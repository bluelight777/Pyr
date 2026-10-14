//! A moderator's Server Mute and Server Deafen, end to end, like Discord's: a real server and
//! two headless clients talking (each "microphone" a tone of its own). Muted, nobody hears
//! them (and their soundboard sounds don't play); deafened, they hear nobody but can still be
//! heard; both stay when they leave voice and come back; and only who may can do it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_engine::audio::{AudioInput, AudioOutput};
use ingle_engine::config::{NoiseSuppression, Settings};
use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions, soundboard};
use ingle_proto::control::{ChannelKind, VOICE_MOD_FEATURE, VoiceParticipant};
use parking_lot::Mutex;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-voicemod-{}-{}", name, ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A copy of Ingle with a tone for a microphone, recording what it hears.
fn bot(dir: &Path, name: &str, tone_hz: f32, heard: Arc<Mutex<Vec<f32>>>) -> Engine {
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(dir, &cfg).unwrap();
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
    let mut s = engine.state().settings.clone();
    s.noise_suppression = NoiseSuppression::Off;
    s.vad_auto = false;
    s.vad_threshold_db = -40.0;
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

fn loud(v: &[f32]) -> bool {
    v.iter().map(|x| x.abs()).fold(0.0, f32::max) > 0.05
}

fn toasts(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

/// How `seer` sees `uid` in voice.
fn seen(seer: &Engine, uid: &str) -> Option<VoiceParticipant> {
    seer.state().servers[0].voice.values().find(|p| p.user_id == uid).cloned()
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

/// Something comes through (from now on).
async fn hears(heard: &Arc<Mutex<Vec<f32>>>, what: &str) {
    heard.lock().clear();
    wait_for(what, 10, || loud(&heard.lock())).await;
}

async fn connected(e: &Engine, what: &str) {
    wait_for(what, 15, || e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_mute_and_deafen() {
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Mod Camp".into()),
    })
    .await
    .unwrap();
    let (ha, hb) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
    let a = bot(&temp_dir("alice"), "Alice", 440.0, ha.clone());
    let b = bot(&temp_dir("bob"), "Bob", 660.0, hb.clone());
    a.add_server(&server.owner_link().unwrap()).unwrap();
    connected(&a, "alice connected").await;
    b.add_server(&server.invite_link()).unwrap();
    connected(&b, "bob connected").await;
    let key = a.state().servers[0].key.clone();
    assert!(a.state().servers[0].has(VOICE_MOD_FEATURE));
    let lounge = a.state().servers[0].channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
    let (alice, bob) = (a.state().my_uid.clone(), b.state().my_uid.clone());
    a.join_voice(&key, lounge);
    b.join_voice(&key, lounge);
    wait_for("both in the call", 10, || a.state().call.is_some() && b.state().call.is_some()).await;
    hears(&hb, "bob hears alice").await;
    hears(&ha, "alice hears bob").await;

    // Bob isn't a moderator.
    b.server_mute(&key, &alice, true);
    b.server_deafen(&key, &alice, true);
    wait_for("bob refused", 10, || toasts(&b).contains("permission")).await;
    assert!(seen(&b, &alice).is_some_and(|p| !p.server_mute && !p.server_deaf));

    // ---- Server Mute: nobody hears Bob (and his sounds don't play); he still hears Alice.
    a.server_mute(&key, &bob, true);
    wait_for("both see bob server muted", 10, || {
        seen(&a, &bob).is_some_and(|p| p.server_mute && !p.self_mute) && seen(&b, &bob).is_some_and(|p| p.server_mute)
    })
    .await;
    silent_for(&ha, "alice, bob server muted").await;
    hears(&hb, "server muted bob still hears alice").await;
    b.play_soundboard(soundboard::builtin("ding").unwrap().sound_ref());
    wait_for("bob's sound refused", 10, || toasts(&b).contains("muted you")).await;
    a.server_mute(&key, &bob, false);
    wait_for("bob unmuted", 10, || seen(&b, &bob).is_some_and(|p| !p.server_mute)).await;
    hears(&ha, "alice hears bob again").await;

    // ---- Server Deafen: Bob hears nobody, but it doesn't mute him (unlike deafening himself).
    a.server_deafen(&key, &bob, true);
    wait_for("both see bob server deafened", 10, || {
        seen(&a, &bob).is_some_and(|p| p.server_deaf && !p.server_mute) && seen(&b, &bob).is_some_and(|p| p.server_deaf)
    })
    .await;
    silent_for(&hb, "bob, server deafened").await;
    hears(&ha, "alice still hears server deafened bob").await;

    // It stays when he leaves voice and comes back.
    b.leave_voice();
    wait_for("bob gone", 10, || seen(&a, &bob).is_none() && b.state().call.is_none()).await;
    b.join_voice(&key, lounge);
    wait_for("bob back, still server deafened", 15, || {
        b.state().call.is_some() && seen(&a, &bob).is_some_and(|p| p.server_deaf) && seen(&b, &bob).is_some_and(|p| p.server_deaf)
    })
    .await;
    silent_for(&hb, "bob back, server deafened").await;
    hears(&ha, "alice hears bob back").await;

    // Undeafened: he hears Alice again.
    a.server_deafen(&key, &bob, false);
    wait_for("bob undeafened", 10, || seen(&b, &bob).is_some_and(|p| !p.server_deaf)).await;
    hears(&hb, "bob hears alice again").await;
}
