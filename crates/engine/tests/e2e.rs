//! End-to-end: a real server and two headless clients exchange chat, end-to-end encrypted
//! voice, webcam video and a screen share over QUIC on localhost.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_engine::audio::{AudioInput, AudioOutput};
use ingle_engine::config::{NoiseSuppression, Settings};
use ingle_engine::state::{AppState, ConnStatus, VideoKey};
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::ChannelKind;
use ingle_proto::media::track;
use parking_lot::Mutex;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-e2e-{}-{}", name, ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A client whose "microphone" plays a sine tone and whose "speakers" record.
fn bot(name: &str, tone_hz: f32, heard: Arc<Mutex<Vec<f32>>>) -> Engine {
    let dir = temp_dir(name);
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(&dir, &cfg).unwrap();
    let factory = move |_s: &Settings| {
        let mut phase = 0u64;
        let heard = heard.clone();
        let input = AudioInput::Synthetic(Box::new(move |buf: &mut [f32]| {
            for v in buf.iter_mut() {
                *v = (phase as f32 / 48_000.0 * tone_hz * std::f32::consts::TAU).sin() * 0.3;
                phase += 1;
            }
        }));
        let output = AudioOutput::Synthetic(Box::new(move |stereo: &[f32]| {
            let mut h = heard.lock();
            for i in 0..480 {
                h.push(stereo[2 * i]);
            }
        }));
        (input, output)
    };
    let engine = Engine::start(EngineOptions {
        config_dir: Some(dir),
        audio: Some(Box::new(factory)),
        push_to_talk: false,
        ..Default::default()
    })
    .unwrap();
    let mut s = engine.state().settings.clone();
    // A pure tone isn't speech, so gate on level instead of RNNoise's voice detector.
    s.noise_suppression = NoiseSuppression::Off;
    s.vad_auto = false;
    s.vad_threshold_db = -40.0;
    s.echo_cancellation = false;
    s.auto_gain = false;
    s.camera_device = Some("test:camera".into());
    engine.update_settings(s);
    engine
}

async fn wait_for(what: &str, secs: u64, mut cond: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(secs);
    while Instant::now() < deadline {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for: {what}");
}

/// Energy of `freq` in `x` (Goertzel), normalised by total energy.
fn tone_ratio(x: &[f32], freq: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * freq / 48_000.0;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0f32, 0f32);
    for v in x {
        let s0 = v + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    let power = s1 * s1 + s2 * s2 - coeff * s1 * s2;
    let total: f32 = x.iter().map(|v| v * v).sum::<f32>() * x.len() as f32 / 2.0;
    if total <= 0.0 { 0.0 } else { power / total }
}

fn server_key(st: &AppState) -> String {
    st.servers[0].key.clone()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn chat_voice_video_end_to_end() {
    let _ = tracing_subscriber_init();
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Test Camp".into()),
    })
    .await
    .unwrap();

    let heard_a = Arc::new(Mutex::new(Vec::new()));
    let heard_b = Arc::new(Mutex::new(Vec::new()));
    let a = bot("Alice", 440.0, heard_a.clone());
    let b = bot("Bob", 660.0, heard_b.clone());

    // ---- join: owner link for Alice, normal invite for Bob
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice connected", 10, || a.state().servers[0].status == ConnStatus::Connected).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("bob connected", 10, || b.state().servers[0].status == ConnStatus::Connected).await;
    let key = server_key(&a.state());
    assert_eq!(a.state().servers[0].name, "Test Camp");
    assert!(a.state().servers[0].is_admin(), "owner link makes Alice the owner");
    assert!(!b.state().servers[0].is_admin());
    wait_for("alice sees bob online", 5, || {
        a.state().servers[0].members.values().any(|m| m.name == "Bob" && m.online)
    })
    .await;

    // ---- a stranger without an invite is refused
    {
        let stranger = bot("Mallory", 100.0, Arc::default());
        let mut link = ingle_proto::invite::InviteLink::parse(&server.invite_link()).unwrap();
        link.code = Some("not-a-real-code".into());
        stranger.add_server(&link.to_url()).unwrap();
        wait_for("stranger refused", 10, || matches!(stranger.state().servers[0].status, ConnStatus::Failed(_))).await;
    }

    // ---- text chat
    let general = a.state().servers[0].channels.iter().find(|c| c.name == "general").unwrap().id;
    b.load_history(&key, general, false);
    a.send_message(&key, general, "hello from **Alice**", None);
    wait_for("bob gets message", 5, || {
        b.state().servers[0].logs.get(&general).map(|l| l.messages.iter().any(|m| m.content.contains("Alice"))).unwrap_or(false)
    })
    .await;
    assert!(b.state().servers[0].unread.contains(&general), "bob isn't viewing general, so it's unread");
    let first_id = b.state().servers[0].logs[&general].messages.last().unwrap().id;
    b.send_message(&key, general, "hi @Alice!", Some(first_id));
    wait_for("alice gets reply + mention", 5, || {
        let st = a.state();
        let s = &st.servers[0];
        s.logs.get(&general).map(|l| l.messages.iter().any(|m| m.reply_to == Some(first_id))).unwrap_or(false)
            && s.mentions.get(&general).copied().unwrap_or(0) >= 1
    })
    .await;
    b.react(&key, first_id, "🔥", true);
    wait_for("reaction reaches alice", 5, || {
        a.state().servers[0].logs[&general].find(first_id).map(|m| m.reactions.iter().any(|r| r.emoji == "🔥")).unwrap_or(false)
    })
    .await;
    a.edit_message(&key, first_id, "hello (edited)");
    wait_for("edit reaches bob", 5, || {
        b.state().servers[0].logs[&general].find(first_id).map(|m| m.edited_at.is_some()).unwrap_or(false)
    })
    .await;
    // history survives: a fresh fetch returns both messages in order
    let reply_ok = {
        a.load_history(&key, general, false);
        tokio::time::sleep(Duration::from_millis(400)).await;
        let st = a.state();
        let msgs = &st.servers[0].logs[&general].messages;
        msgs.len() == 2 && msgs[0].id < msgs[1].id
    };
    assert!(reply_ok, "history reload");

    // ---- admin: create a channel
    a.create_channel(&key, "Clips And Links 2", ChannelKind::Text);
    wait_for("bob sees new channel", 5, || b.state().servers[0].channels.iter().any(|c| c.name == "clips-and-links-2")).await;

    // ---- voice: both join Lounge
    let lounge = a.state().servers[0].channels.iter().find(|c| c.name == "Lounge").unwrap().id;
    a.join_voice(&key, lounge);
    wait_for("alice in call", 10, || a.state().call.is_some()).await;
    b.join_voice(&key, lounge);
    wait_for("bob in call", 10, || b.state().call.is_some()).await;
    wait_for("e2ee keys exchanged", 10, || {
        a.state().call.as_ref().map(|c| c.e2ee_ready).unwrap_or(false)
            && b.state().call.as_ref().map(|c| c.e2ee_ready).unwrap_or(false)
    })
    .await;
    let (code_a, code_b) =
        (a.state().call.as_ref().unwrap().call_code.clone(), b.state().call.as_ref().unwrap().call_code.clone());
    assert_eq!(code_a, code_b, "both sides compute the same call code");

    heard_a.lock().clear();
    heard_b.lock().clear();
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let (ha, hb) = (heard_a.lock().clone(), heard_b.lock().clone());
    assert!(ha.len() > 48_000, "alice's speaker ran ({} samples)", ha.len());
    let tail_a = &ha[ha.len() - 48_000..];
    let tail_b = &hb[hb.len() - 48_000..];
    let (a660, a440) = (tone_ratio(tail_a, 660.0), tone_ratio(tail_a, 440.0));
    let (b440, b660) = (tone_ratio(tail_b, 440.0), tone_ratio(tail_b, 660.0));
    println!("alice hears 660Hz ratio {a660:.3} (440: {a440:.3}); bob hears 440Hz ratio {b440:.3} (660: {b660:.3})");
    assert!(a660 > 0.5 && a440 < 0.05, "Alice should hear Bob's 660 Hz tone, not herself");
    assert!(b440 > 0.5 && b660 < 0.05, "Bob should hear Alice's 440 Hz tone, not himself");
    {
        let st = a.state();
        let call = st.call.as_ref().unwrap();
        let bob_session = st.servers[0].voice.values().find(|p| p.user_id != st.my_uid).unwrap().session;
        assert!(call.speaking.contains(&bob_session), "Bob shows as speaking");
        // Alice was already in the call: everything Bob sent was decryptable.
        assert_eq!(call.stats.undecryptable, 0);
        println!("alice stats: {:?}", call.stats);
        // Bob joined second: he may miss Alice's last ~300 ms on the key she used before he came.
        let bob_lost = b.state().call.as_ref().unwrap().stats.undecryptable;
        println!("bob undecryptable at join: {bob_lost}");
        assert!(bob_lost < 40);
    }

    // ---- mute: Bob mutes, Alice stops hearing him
    b.set_mute(true);
    tokio::time::sleep(Duration::from_millis(600)).await;
    heard_a.lock().clear();
    tokio::time::sleep(Duration::from_millis(800)).await;
    let quiet: f32 = heard_a.lock().iter().map(|v| v * v).sum();
    assert!(quiet < 1.0, "muted audio still audible ({quiet})");
    b.set_mute(false);

    // ---- webcam: Alice turns on her (test pattern) camera; Bob sees it automatically
    a.start_camera();
    let a_session = a.state().call.as_ref().unwrap().session;
    let cam = VideoKey { session: a_session, track: track::CAMERA };
    wait_for("bob decodes alice's camera", 15, || b.video_frame(cam).is_some()).await;
    tokio::time::sleep(Duration::from_millis(800)).await;
    let f = b.video_frame(cam).unwrap();
    assert_eq!((f.width, f.height), (1280, 720));
    let px = |x: usize, y: usize| {
        let o = (y * f.width as usize + x) * 4;
        (f.rgba[o] as i32, f.rgba[o + 1] as i32, f.rgba[o + 2] as i32)
    };
    let (r, g, bl) = px(40, 700);
    println!("camera pixel {r},{g},{bl} (expected ~255,122,40)");
    assert!((r - 255).abs() < 30 && (g - 122).abs() < 30 && (bl - 40).abs() < 30);
    assert!(a.video_frame(cam).is_some(), "alice has a self-view");

    // ---- screen share: not received until Bob chooses to watch
    a.start_screen("test:pattern");
    wait_for("screen track published", 10, || {
        b.state().servers[0].voice.get(&a_session).map(|p| p.tracks.iter().any(|t| t.id == track::SCREEN)).unwrap_or(false)
    })
    .await;
    let scr = VideoKey { session: a_session, track: track::SCREEN };
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(b.video_frame(scr).is_none(), "screen share is opt-in");
    b.watch(a_session, track::SCREEN, true);
    wait_for("bob decodes the screen share", 15, || b.video_frame(scr).is_some()).await;
    let g0 = b.video_frame(scr).unwrap().generation;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let s = b.video_frame(scr).unwrap();
    assert_eq!((s.width, s.height), (1920, 1080));
    let frames_in_window = (s.generation - g0) as f32;
    println!("screen share: {}x{}, ~{} frame updates in 1.5 s across tiles", s.width, s.height, frames_in_window);
    {
        let st = a.state();
        println!("alice sending video at {} kbps", st.call.as_ref().unwrap().stats.video_send_kbps);
    }

    // ---- Bob leaves; Alice is alone and the call stays healthy
    b.leave_voice();
    wait_for("bob gone from alice's view", 5, || a.state().servers[0].voice.len() == 1).await;
    assert!(b.state().call.is_none());

    // ---- moderation: Alice kicks Bob
    let bob_uid = b.state().my_uid.clone();
    a.kick(&key, &bob_uid, false);
    wait_for("bob kicked", 5, || matches!(b.state().servers[0].status, ConnStatus::Failed(_))).await;
    wait_for("bob removed from member list", 5, || !a.state().servers[0].members.contains_key(&bob_uid)).await;

    a.shutdown();
    b.shutdown();
    server.shutdown().await;
}

fn tracing_subscriber_init() -> Option<()> {
    // Quiet by default; set RUST_LOG=debug to see everything.
    None
}

/// Someone joining a call that's already running must get everyone's keys right away
/// (regression test: keys sent in reaction to our join used to be dropped).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn late_joiner_gets_keys_audio_and_video() {
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server2"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: None,
    })
    .await
    .unwrap();
    let heard_c = Arc::new(Mutex::new(Vec::new()));
    let a = bot("Ann", 440.0, Arc::default());
    let b = bot("Ben", 660.0, Arc::default());
    let c = bot("Cat", 550.0, heard_c.clone());
    for (e, link) in [(&a, server.owner_link().unwrap()), (&b, server.invite_link()), (&c, server.invite_link())] {
        e.add_server(&link).unwrap();
    }
    for e in [&a, &b, &c] {
        wait_for("connected", 10, || e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    }
    let key = server_key(&a.state());
    let lounge = a.state().servers[0].channels.iter().find(|ch| ch.name == "Lounge").unwrap().id;
    a.join_voice(&key, lounge);
    b.join_voice(&key, lounge);
    wait_for("a+b keyed", 10, || {
        [&a, &b].iter().all(|e| e.state().call.as_ref().map(|c| c.e2ee_ready).unwrap_or(false))
            && a.state().servers[0].voice.len() == 2
    })
    .await;
    a.start_camera();
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Cat arrives late.
    c.join_voice(&key, lounge);
    let t0 = Instant::now();
    wait_for("late joiner keyed", 5, || c.state().call.as_ref().map(|x| x.e2ee_ready).unwrap_or(false)).await;
    println!("late joiner had all keys after {:?}", t0.elapsed());
    let a_session = a.state().call.as_ref().unwrap().session;
    wait_for("late joiner sees camera", 10, || c.video_frame(VideoKey { session: a_session, track: track::CAMERA }).is_some()).await;
    heard_c.lock().clear();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let h = heard_c.lock().clone();
    let tail = &h[h.len().saturating_sub(48_000)..];
    let (r440, r660) = (tone_ratio(tail, 440.0), tone_ratio(tail, 660.0));
    println!("late joiner hears 440: {r440:.3}, 660: {r660:.3}");
    assert!(r440 > 0.2 && r660 > 0.2, "should hear both earlier participants");
    let all_three = c.state().call.as_ref().unwrap().call_code.clone();
    wait_for("call codes agree", 5, || {
        a.state().call.as_ref().unwrap().call_code == all_three && b.state().call.as_ref().unwrap().call_code == all_three
    })
    .await;
    for e in [&a, &b, &c] {
        e.shutdown();
    }
    server.shutdown().await;
}

/// A client with a silent microphone whose "speakers" record both channels.
fn listener(name: &str, heard: Arc<Mutex<(Vec<f32>, Vec<f32>)>>) -> Engine {
    let dir = temp_dir(name);
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(&dir, &cfg).unwrap();
    let factory = move |_s: &Settings| {
        let heard = heard.clone();
        let input = AudioInput::Synthetic(Box::new(|buf: &mut [f32]| buf.fill(0.0)));
        let output = AudioOutput::Synthetic(Box::new(move |stereo: &[f32]| {
            let mut h = heard.lock();
            for i in 0..480 {
                h.0.push(stereo[2 * i]);
                h.1.push(stereo[2 * i + 1]);
            }
        }));
        (input, output)
    };
    Engine::start(EngineOptions { config_dir: Some(dir), audio: Some(Box::new(factory)), push_to_talk: false, ..Default::default() })
        .unwrap()
}

/// Screen-share sound reaches people watching the stream (in stereo), and nobody else.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn screen_share_sound_goes_to_viewers_only() {
    use ingle_engine::audio::stream::{SYNTH_LEFT_HZ, SYNTH_RIGHT_HZ};
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server3"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: None,
    })
    .await
    .unwrap();
    let viewer_heard: Arc<Mutex<(Vec<f32>, Vec<f32>)>> = Arc::default();
    let other_heard: Arc<Mutex<(Vec<f32>, Vec<f32>)>> = Arc::default();
    let host = listener("Host", Arc::default());
    let viewer = listener("Viewer", viewer_heard.clone());
    let other = listener("Other", other_heard.clone());
    for (e, link) in [(&host, server.owner_link().unwrap()), (&viewer, server.invite_link()), (&other, server.invite_link())] {
        e.add_server(&link).unwrap();
    }
    for e in [&host, &viewer, &other] {
        wait_for("connected", 10, || e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    }
    let key = server_key(&host.state());
    let lounge = host.state().servers[0].channels.iter().find(|ch| ch.name == "Lounge").unwrap().id;
    for e in [&host, &viewer, &other] {
        e.join_voice(&key, lounge);
    }
    wait_for("everyone keyed", 10, || {
        [&host, &viewer, &other].iter().all(|e| e.state().call.as_ref().map(|c| c.e2ee_ready).unwrap_or(false))
            && host.state().servers[0].voice.len() == 3
    })
    .await;

    host.start_screen("test:pattern");
    let host_session = host.state().call.as_ref().unwrap().session;
    wait_for("stream audio running", 10, || host.state().call.as_ref().map(|c| c.stream_audio_on).unwrap_or(false)).await;
    wait_for("others see a stream with sound", 10, || {
        viewer.state().servers[0]
            .voice
            .get(&host_session)
            .map(|p| p.tracks.iter().any(|t| t.id == track::SCREEN) && p.tracks.iter().any(|t| t.id == track::SCREEN_AUDIO))
            .unwrap_or(false)
    })
    .await;
    viewer.watch(host_session, track::SCREEN, true);
    wait_for("viewer decodes the screen", 15, || viewer.video_frame(VideoKey { session: host_session, track: track::SCREEN }).is_some())
        .await;

    let listen = |h: &Arc<Mutex<(Vec<f32>, Vec<f32>)>>| {
        let g = h.lock();
        let n = g.0.len();
        (g.0[n.saturating_sub(48_000)..].to_vec(), g.1[n.saturating_sub(48_000)..].to_vec())
    };
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let (l, r) = listen(&viewer_heard);
    let (ll, lr, rr, rl) =
        (tone_ratio(&l, SYNTH_LEFT_HZ), tone_ratio(&l, SYNTH_RIGHT_HZ), tone_ratio(&r, SYNTH_RIGHT_HZ), tone_ratio(&r, SYNTH_LEFT_HZ));
    println!("viewer: left {ll:.3} (other ch {lr:.3}), right {rr:.3} (other ch {rl:.3})");
    assert!(ll > 0.5 && rr > 0.5, "the viewer hears the stream");
    assert!(lr < 0.05 && rl < 0.05, "left and right stay separate");
    let (ol, or) = listen(&other_heard);
    let energy: f32 = ol.iter().chain(or.iter()).map(|v| v * v).sum();
    println!("non-viewer energy {energy:.4}");
    assert!(energy < 0.01, "someone not watching hears nothing");

    // Turning the stream down silences it without touching anything else.
    let host_uid = host.state().my_uid.clone();
    viewer.set_stream_volume(&host_uid, 0.0);
    tokio::time::sleep(Duration::from_millis(300)).await;
    viewer_heard.lock().0.clear();
    viewer_heard.lock().1.clear();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let (l, _) = listen(&viewer_heard);
    assert!(l.iter().map(|v| v * v).sum::<f32>() < 0.01, "stream volume 0 is silent");
    viewer.set_stream_volume(&host_uid, 1.0);

    // Stopping the share removes the sound track and the sound.
    host.stop_screen();
    wait_for("stream audio track gone", 5, || {
        viewer.state().servers[0].voice.get(&host_session).map(|p| p.tracks.iter().all(|t| t.id != track::SCREEN_AUDIO)).unwrap_or(false)
    })
    .await;
    assert!(!host.state().call.as_ref().unwrap().stream_audio_on);
    tokio::time::sleep(Duration::from_millis(400)).await;
    viewer_heard.lock().0.clear();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let (l, _) = listen(&viewer_heard);
    assert!(l.iter().map(|v| v * v).sum::<f32>() < 0.01, "no sound after the share stops");

    for e in [&host, &viewer, &other] {
        e.shutdown();
    }
    server.shutdown().await;
}
