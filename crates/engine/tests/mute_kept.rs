//! Muted or deafened stays that way through an update: Ingle restarting (the app's update),
//! the server restarting (its update), and the update from a version that didn't remember
//! (the server tells the app how it left).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_engine::audio::{AudioInput, AudioOutput};
use ingle_engine::config::{NoiseSuppression, SelfVoice, Settings};
use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::{ChannelId, ChannelKind, VoiceParticipant};
use parking_lot::Mutex;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-mute-{}-{}", name, ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A copy of Ingle with a tone for a microphone, recording what it hears. `fresh` starts a
/// new person; otherwise it's the same person's Ingle starting again (same settings folder).
fn bot(dir: &Path, name: &str, tone_hz: f32, heard: Arc<Mutex<Vec<f32>>>, fresh: bool) -> Engine {
    if fresh {
        let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
        ingle_engine::config::save_config(dir, &cfg).unwrap();
    }
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
        config_dir: Some(dir.to_path_buf()),
        audio: Some(Box::new(factory)),
        push_to_talk: false,
        ..Default::default()
    })
    .unwrap();
    if fresh {
        let mut s = engine.state().settings.clone();
        s.noise_suppression = NoiseSuppression::Off;
        s.vad_auto = false;
        s.vad_threshold_db = -40.0;
        s.echo_cancellation = false;
        s.auto_gain = false;
        engine.update_settings(s);
    }
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

fn loud(v: &[f32]) -> bool {
    v.iter().map(|x| x.abs()).fold(0.0, f32::max) > 0.05
}

/// How `seer` sees `uid` in voice (one entry at most: one voice session per person).
fn seen(seer: &Engine, uid: &str) -> Option<VoiceParticipant> {
    let st = seer.state();
    let mine: Vec<_> = st.servers[0].voice.values().filter(|p| p.user_id == uid).cloned().collect();
    assert!(mine.len() <= 1, "two voice sessions for one person: {mine:?}");
    mine.into_iter().next()
}

/// Nothing comes through for a while (after what was already on its way has played).
async fn silent_for(heard: &Arc<Mutex<Vec<f32>>>, what: &str) {
    tokio::time::sleep(Duration::from_millis(400)).await;
    heard.lock().clear();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let h = heard.lock();
    assert!(h.len() > 48_000, "{what}: the speaker should be running ({} samples)", h.len());
    assert!(!loud(&h), "{what}: should hear nothing");
}

async fn connected(e: &Engine, what: &str) {
    wait_for(what, 15, || e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
}

/// Ingle closing (as for an update) and starting again, then rejoining the call the way the
/// app does after an update.
async fn restart(old: Engine, dir: &Path, heard: &Arc<Mutex<Vec<f32>>>, key: &str, lounge: ChannelId, uid: &str, other: &Engine) -> Engine {
    old.shutdown();
    drop(old);
    wait_for("the others see alice go", 10, || seen(other, uid).is_none()).await;
    let e = bot(dir, "Alice", 440.0, heard.clone(), false);
    connected(&e, "alice reconnected").await;
    e.join_voice(key, lounge);
    wait_for("alice back in the call", 15, || e.state().call.as_ref().map(|c| c.channel_id == lounge).unwrap_or(false)).await;
    e
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mute_and_deafen_survive_updates() {
    let data = temp_dir("server");
    let cfg = |bind| ingle_server::ServerConfig {
        data_dir: data.clone(),
        bind,
        public_host: Some("127.0.0.1".into()),
        name: Some("Quiet Camp".into()),
    };
    let server = ingle_server::start(cfg("127.0.0.1:0".parse().unwrap())).await.unwrap();
    let addr = server.local_addr;
    let (adir, bdir) = (temp_dir("alice"), temp_dir("bob"));
    let (ha, hb) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
    let a = bot(&adir, "Alice", 440.0, ha.clone(), true);
    let b = bot(&bdir, "Bob", 660.0, hb.clone(), true);
    a.add_server(&server.owner_link().unwrap()).unwrap();
    connected(&a, "alice connected").await;
    b.add_server(&server.invite_link()).unwrap();
    connected(&b, "bob connected").await;
    let key = a.state().servers[0].key.clone();
    let lounge = a.state().servers[0].channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
    let alice = a.state().my_uid.clone();
    assert_eq!(a.voice_prefs(), (false, false), "a new person starts unmuted");
    a.join_voice(&key, lounge);
    b.join_voice(&key, lounge);
    wait_for("both in the call", 10, || a.state().call.is_some() && b.state().call.is_some()).await;
    wait_for("bob hears alice", 10, || loud(&hb.lock())).await;

    // Alice mutes.
    a.set_mute(true);
    wait_for("bob sees alice muted", 10, || seen(&b, &alice).map(|p| p.self_mute && !p.self_deaf).unwrap_or(false)).await;
    silent_for(&hb, "bob, alice muted").await;

    // 1. Alice's Ingle updates (closes and starts again): still muted, before a sound goes out.
    let a = restart(a, &adir, &ha, &key, lounge, &alice, &b).await;
    assert_eq!(a.voice_prefs(), (true, false), "muted after the restart");
    assert!(a.state().call.as_ref().unwrap().muted);
    wait_for("bob sees alice back, muted", 10, || seen(&b, &alice).map(|p| p.self_mute && !p.self_deaf).unwrap_or(false)).await;
    silent_for(&hb, "bob, alice muted after her update").await;
    ha.lock().clear();
    wait_for("muted alice still hears bob", 10, || loud(&ha.lock())).await;

    // 2. Deafened, and Ingle updates again: still deafened (and so muted).
    a.set_deafen(true);
    wait_for("bob sees alice deafened", 10, || seen(&b, &alice).map(|p| p.self_mute && p.self_deaf).unwrap_or(false)).await;
    silent_for(&ha, "deafened alice").await;
    let a = restart(a, &adir, &ha, &key, lounge, &alice, &b).await;
    assert_eq!(a.voice_prefs(), (true, true), "deafened after the restart");
    wait_for("bob sees alice back, deafened", 10, || seen(&b, &alice).map(|p| p.self_mute && p.self_deaf).unwrap_or(false)).await;
    silent_for(&ha, "deafened alice after her update").await;
    silent_for(&hb, "bob, alice deafened after her update").await;

    // 3. The server updates (restarts): everyone rejoins by themselves, alice still deafened.
    server.shutdown().await;
    wait_for("alice notices", 10, || a.state().call.is_none()).await;
    let mut server = None;
    let mut why = String::new();
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        match ingle_server::start(cfg(addr)).await {
            Ok(s) => {
                server = Some(s);
                break;
            }
            Err(e) => why = format!("{e:#}"),
        }
    }
    let server = server.unwrap_or_else(|| panic!("server restarts: {why}"));
    wait_for("both back in the call", 20, || a.state().call.is_some() && b.state().call.is_some()).await;
    assert_eq!(a.voice_prefs(), (true, true));
    wait_for("bob sees alice deafened again", 10, || seen(&b, &alice).map(|p| p.self_mute && p.self_deaf).unwrap_or(false)).await;
    silent_for(&hb, "bob, alice deafened after the server's update").await;

    // Undeafening goes back to muted (she was muted before deafening); unmuting is heard.
    a.set_deafen(false);
    assert_eq!(a.voice_prefs(), (true, false));
    wait_for("bob sees alice muted, not deafened", 10, || seen(&b, &alice).map(|p| p.self_mute && !p.self_deaf).unwrap_or(false)).await;
    ha.lock().clear();
    wait_for("alice hears bob again", 10, || loud(&ha.lock())).await;
    a.set_mute(false);
    hb.lock().clear();
    wait_for("bob hears alice again", 10, || loud(&hb.lock())).await;
    assert_eq!(ingle_engine::config::load_config(&adir).self_voice, Some(SelfVoice { muted: false, deafened: false }));

    // 4. The update from a version that didn't remember (0.6.0): its settings say nothing about
    // mute, but the server saw how alice left and tells her app.
    a.set_mute(true);
    wait_for("bob sees alice muted (4)", 10, || seen(&b, &alice).map(|p| p.self_mute).unwrap_or(false)).await;
    a.shutdown();
    drop(a);
    wait_for("bob sees alice go (4)", 10, || seen(&b, &alice).is_none()).await;
    let path = adir.join("config.json");
    let mut old: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(old.as_object_mut().unwrap().remove("self_voice").is_some());
    std::fs::write(&path, serde_json::to_vec_pretty(&old).unwrap()).unwrap();
    let a = bot(&adir, "Alice", 440.0, ha.clone(), false);
    assert_eq!(a.voice_prefs(), (false, false), "nothing saved about mute yet");
    connected(&a, "alice reconnected (4)").await;
    a.join_voice(&key, lounge);
    wait_for("alice back in the call (4)", 15, || a.state().call.is_some()).await;
    assert_eq!(a.voice_prefs(), (true, false), "muted the way the server saw her leave");
    assert!(a.state().call.as_ref().unwrap().muted);
    wait_for("bob sees alice muted (4b)", 10, || seen(&b, &alice).map(|p| p.self_mute && !p.self_deaf).unwrap_or(false)).await;
    silent_for(&hb, "bob, alice muted after updating from 0.6.0").await;
    assert_eq!(ingle_engine::config::load_config(&adir).self_voice, Some(SelfVoice { muted: true, deafened: false }));

    // 5. Once this PC remembers, it's what counts: unmuting while out of the call isn't undone
    // by the server remembering her leaving muted.
    a.leave_voice();
    wait_for("bob sees alice go (5)", 10, || seen(&b, &alice).is_none()).await;
    a.set_mute(false);
    a.join_voice(&key, lounge);
    wait_for("alice back in the call (5)", 15, || a.state().call.is_some()).await;
    hb.lock().clear();
    wait_for("bob hears alice (5)", 10, || loud(&hb.lock())).await;
    assert_eq!(a.voice_prefs(), (false, false));
    assert!(!seen(&b, &alice).unwrap().self_mute);

    // 6. From a version that didn't remember again, but she'd left deafened (bug report #3: she
    // came back silently deafened, and it looked like the update had muted her): her
    // microphone stays off, she hears everyone again, and she's told why she's muted.
    a.set_deafen(true);
    wait_for("bob sees alice deafened (6)", 10, || seen(&b, &alice).map(|p| p.self_mute && p.self_deaf).unwrap_or(false)).await;
    a.shutdown();
    drop(a);
    wait_for("bob sees alice go (6)", 10, || seen(&b, &alice).is_none()).await;
    let mut old: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert!(old.as_object_mut().unwrap().remove("self_voice").is_some());
    std::fs::write(&path, serde_json::to_vec_pretty(&old).unwrap()).unwrap();
    let a = bot(&adir, "Alice", 440.0, ha.clone(), false);
    connected(&a, "alice reconnected (6)").await;
    a.join_voice(&key, lounge);
    wait_for("alice back in the call (6)", 15, || a.state().call.is_some()).await;
    assert_eq!(a.voice_prefs(), (true, false), "muted, not deafened");
    wait_for("bob sees alice muted, not deafened (6)", 10, || seen(&b, &alice).map(|p| p.self_mute && !p.self_deaf).unwrap_or(false)).await;
    ha.lock().clear();
    wait_for("alice hears bob (6)", 10, || loud(&ha.lock())).await;
    silent_for(&hb, "bob, alice still muted (6)").await;
    let told = a.state().toasts.iter().any(|t| t.text.contains("Click the microphone to talk"));
    assert!(told, "alice is told why she's muted");

    a.shutdown();
    b.shutdown();
    server.shutdown().await;
}
