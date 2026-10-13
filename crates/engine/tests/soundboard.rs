//! The soundboard end to end: a real server and two headless clients in a call. Ingle's own
//! sounds, a server's own (added from a cut of a file), one from another server (sent to the
//! call's server first), muting someone's sounds, stopping them, editing and deleting, who may
//! add them, and the join sound.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_engine::audio::{AudioInput, AudioOutput};
use ingle_engine::config::{NoiseSuppression, Settings, SoundRef};
use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions, soundboard};
use ingle_proto::control::{ChannelKind, SOUNDBOARD_FEATURE};
use parking_lot::Mutex;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-sb-{}-{}", name, ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A client with a silent "microphone" whose "speakers" record what it plays.
fn bot(name: &str, heard: Arc<Mutex<Vec<f32>>>) -> Engine {
    let dir = temp_dir(name);
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(&dir, &cfg).unwrap();
    let factory = move |_s: &Settings| {
        let heard = heard.clone();
        let input = AudioInput::Synthetic(Box::new(|buf: &mut [f32]| buf.fill(0.0)));
        let output = AudioOutput::Synthetic(Box::new(move |stereo: &[f32]| {
            let mut h = heard.lock();
            for i in 0..480 {
                h.push(stereo[2 * i]);
            }
        }));
        (input, output)
    };
    let engine = Engine::start(EngineOptions { config_dir: Some(dir), audio: Some(Box::new(factory)), push_to_talk: false, ..Default::default() }).unwrap();
    let mut s = engine.state().settings.clone();
    s.noise_suppression = NoiseSuppression::Off;
    s.echo_cancellation = false;
    s.auto_gain = false;
    s.sounds = false;
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

fn toasts(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

/// Energy of `freq` in `x` (Goertzel), against all of `x`'s energy.
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

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// What `heard` gets over the next `ms`.
async fn listen(heard: &Arc<Mutex<Vec<f32>>>, ms: u64) -> Vec<f32> {
    heard.lock().clear();
    tokio::time::sleep(Duration::from_millis(ms)).await;
    heard.lock().clone()
}

/// `secs` of a tone at `hz`, 48 kHz stereo (as a picked file is read).
fn tone(secs: f32, hz: f32) -> Arc<Vec<f32>> {
    let n = (secs * 48_000.0) as usize;
    Arc::new((0..n).flat_map(|i| {
        let v = (i as f32 / 48_000.0 * hz * std::f32::consts::TAU).sin() * 0.4;
        [v, v]
    }).collect())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn soundboard_end_to_end() {
    let start = |name: &str| {
        let name = name.to_string();
        async move {
            ingle_server::start(ingle_server::ServerConfig {
                data_dir: temp_dir(&name),
                bind: "127.0.0.1:0".parse().unwrap(),
                public_host: Some("127.0.0.1".into()),
                name: Some(name),
            })
            .await
            .unwrap()
        }
    };
    let camp = start("Sound Camp").await;
    let other = start("Other Camp").await;
    let (heard_a, heard_b) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
    let (a, b) = (bot("Alice", heard_a.clone()), bot("Bob", heard_b.clone()));
    a.add_server(&camp.owner_link().unwrap()).unwrap();
    wait_for("alice on camp", 10, || a.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected)).await;
    a.add_server(&other.owner_link().unwrap()).unwrap();
    wait_for("alice on other", 10, || a.state().servers.len() == 2 && a.state().servers.iter().all(|s| s.status == ConnStatus::Connected)).await;
    b.add_server(&camp.invite_link()).unwrap();
    wait_for("bob on camp", 10, || b.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected)).await;
    let st = a.state();
    let camp_key = st.servers.iter().find(|s| s.name == "Sound Camp").unwrap().key.clone();
    let other_key = st.servers.iter().find(|s| s.name == "Other Camp").unwrap().key.clone();
    assert!(st.servers.iter().all(|s| s.has(SOUNDBOARD_FEATURE)));
    let lounge = st.server(&camp_key).unwrap().channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
    drop(st);

    // ---- who may add sounds: not a member, only who manages emoji (the owner here)
    b.add_sound_from(&camp_key, "Sneaky", "🥷", 1.0, tone(0.5, 700.0), 0.0, 0.5);
    wait_for("bob refused", 10, || toasts(&b).contains("permission")).await;
    assert!(a.state().server(&camp_key).unwrap().info.sounds.is_empty());

    // ---- both in the call
    a.join_voice(&camp_key, lounge);
    wait_for("alice in call", 10, || a.state().call.is_some()).await;
    b.join_voice(&camp_key, lounge);
    wait_for("bob in call", 10, || b.state().call.is_some()).await;
    wait_for("keys", 10, || {
        a.state().call.as_ref().is_some_and(|c| c.e2ee_ready) && b.state().call.as_ref().is_some_and(|c| c.e2ee_ready)
    })
    .await;
    let alice_session = a.state().call.as_ref().unwrap().session;
    let alice_uid = a.state().my_uid.clone();
    let quiet = listen(&heard_b, 600).await;
    assert!(rms(&quiet) < 0.001, "the call is quiet to start with ({})", rms(&quiet));

    // ---- one of Ingle's own: everyone hears it (Alice too), and sees who played it
    let ding = soundboard::builtin("ding").unwrap().sound_ref();
    heard_a.lock().clear();
    heard_b.lock().clear();
    a.play_soundboard(ding.clone());
    wait_for("bob sees alice's ding", 5, || b.state().call.as_ref().is_some_and(|c| c.sounds.iter().any(|s| s.session == alice_session && s.name == "Ding"))).await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let (ha, hb) = (heard_a.lock().clone(), heard_b.lock().clone());
    println!("ding: alice hears rms {:.4}, bob {:.4}", rms(&ha), rms(&hb));
    assert!(rms(&hb) > 0.01, "bob hears the ding");
    assert!(rms(&ha) > 0.01, "alice hears her own ding");
    assert!(a.state().settings.sound_recent.first().is_some_and(|r| r.same(&ding)), "it's at the top of her recently played");

    // ---- a server's own: cut from a "file", evened out, uploaded; then played
    a.add_sound_from(&camp_key, "Beep", "📟", 1.0, tone(3.0, 1000.0), 1.0, 2.0);
    wait_for("bob sees Beep", 15, || b.state().server(&camp_key).unwrap().info.sounds.iter().any(|x| x.name == "Beep")).await;
    let beep = b.state().server(&camp_key).unwrap().info.sounds.iter().find(|x| x.name == "Beep").cloned().unwrap();
    assert!((beep.length_ms as i32 - 1000).abs() <= 40, "the cut is 1 s ({} ms)", beep.length_ms);
    assert_eq!(beep.emoji, "📟");
    let beep_ref = SoundRef { server: camp_key.clone(), id: beep.id.clone(), name: beep.name.clone(), emoji: beep.emoji.clone(), volume: 1.0 };
    heard_b.lock().clear();
    b.play_soundboard(beep_ref.clone());
    tokio::time::sleep(Duration::from_millis(1400)).await;
    let ha = heard_a.lock().clone();
    let tail = &ha[ha.len().saturating_sub(48_000)..];
    // (Alice hears Bob play it.)
    let loud: Vec<f32> = ha.chunks(4800).map(rms).collect();
    println!("beep, as alice hears it, per 100 ms: {loud:.3?}");
    let best = ha.windows(24_000).step_by(4800).map(|w| tone_ratio(w, 1000.0)).fold(0f32, f32::max);
    assert!(best > 0.5, "alice hears bob's 1 kHz beep (ratio {best:.3}, tail rms {:.4})", rms(tail));

    // ---- one from another server (Bob isn't on it): sent to this call's server first
    a.add_sound_from(&other_key, "Chirp", "🐦", 1.0, tone(0.8, 1500.0), 0.0, 0.8);
    wait_for("chirp on other", 15, || a.state().server(&other_key).unwrap().info.sounds.iter().any(|x| x.name == "Chirp")).await;
    let chirp = a.state().server(&other_key).unwrap().info.sounds.iter().find(|x| x.name == "Chirp").cloned().unwrap();
    let chirp_ref = SoundRef { server: other_key.clone(), id: chirp.id.clone(), name: chirp.name.clone(), emoji: chirp.emoji.clone(), volume: 1.0 };
    heard_b.lock().clear();
    a.play_soundboard(chirp_ref.clone());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let hb = heard_b.lock().clone();
    let best = hb.windows(24_000).step_by(4800).map(|w| tone_ratio(w, 1500.0)).fold(0f32, f32::max);
    println!("chirp from the other server, bob's best 1.5 kHz ratio {best:.3}; toasts: {}", toasts(&a));
    assert!(best > 0.5, "bob hears a sound from a server he isn't on");
    // Played again, it isn't sent again (the call's server keeps it).
    heard_b.lock().clear();
    a.play_soundboard(chirp_ref.clone());
    wait_for("second chirp", 5, || b.state().call.as_ref().is_some_and(|c| c.sounds.iter().filter(|s| s.name == "Chirp").count() >= 2)).await;

    // ---- muting Alice's soundboard: Bob still sees it, doesn't hear it
    let mut s = b.state().settings.clone();
    s.soundboard_muted.push(alice_uid.clone());
    b.update_settings(s);
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let before = b.state().call.as_ref().unwrap().sounds.len();
    heard_b.lock().clear();
    a.play_soundboard(ding.clone());
    wait_for("bob sees the muted ding", 5, || b.state().call.as_ref().is_some_and(|c| c.sounds.len() > before)).await;
    tokio::time::sleep(Duration::from_millis(1000)).await;
    let hb = heard_b.lock().clone();
    println!("muted ding: bob hears rms {:.5}", rms(&hb));
    assert!(rms(&hb) < 0.002, "muted: bob doesn't hear alice's sounds");
    let mut s = b.state().settings.clone();
    s.soundboard_muted.clear();
    b.update_settings(s);

    // ---- stopping: a long sound stops at once (anyone's)
    a.add_sound_from(&camp_key, "Drone", "🐝", 1.0, tone(9.0, 400.0), 0.0, 9.0);
    wait_for("drone", 15, || b.state().server(&camp_key).unwrap().info.sounds.iter().any(|x| x.name == "Drone")).await;
    let drone = b.state().server(&camp_key).unwrap().info.sounds.iter().find(|x| x.name == "Drone").cloned().unwrap();
    a.play_soundboard(SoundRef { server: camp_key.clone(), id: drone.id.clone(), name: drone.name.clone(), emoji: drone.emoji.clone(), volume: 1.0 });
    wait_for("drone playing at bob's", 10, || b.sounds_playing()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    b.stop_sounds();
    assert!(!b.sounds_playing());
    tokio::time::sleep(Duration::from_millis(300)).await;
    let after = listen(&heard_b, 800).await;
    println!("after stop: bob hears rms {:.5}", rms(&after));
    assert!(rms(&after) < 0.002, "stopped");

    // ---- editing and deleting (the file goes from the server)
    a.edit_sound(&camp_key, &beep.id, Some("Boop".into()), Some("🤖".into()), Some(0.5));
    wait_for("renamed", 5, || b.state().server(&camp_key).unwrap().info.sounds.iter().any(|x| x.name == "Boop" && x.emoji == "🤖" && (x.volume - 0.5).abs() < 1e-3)).await;
    b.remove_sound(&camp_key, &beep.id);
    wait_for("bob can't delete", 5, || toasts(&b).matches("permission").count() >= 2).await;
    a.remove_sound(&camp_key, &beep.id);
    wait_for("deleted", 5, || !b.state().server(&camp_key).unwrap().info.sounds.iter().any(|x| x.id == beep.id)).await;

    // ---- a join sound: Bob's plays for everyone when he joins
    let tada = soundboard::builtin("ta-da").unwrap().sound_ref();
    let mut s = b.state().settings.clone();
    s.join_sound = Some(tada);
    b.update_settings(s);
    b.leave_voice();
    wait_for("bob left", 5, || a.state().server(&camp_key).unwrap().voice.len() == 1).await;
    b.join_voice(&camp_key, lounge);
    wait_for("bob's ta-da", 10, || a.state().call.as_ref().is_some_and(|c| c.sounds.iter().any(|s| s.name == "Ta-da"))).await;

    camp.shutdown().await;
    other.shutdown().await;
}
