//! The server restarting (an update) mid-call: everyone's app rejoins the call by itself and
//! they hear each other again. A camera that was on stays off until its owner turns it back on
//! (the app offers to).

use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_engine::audio::{AudioInput, AudioOutput};
use ingle_engine::config::{NoiseSuppression, Settings};
use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::ChannelKind;
use parking_lot::Mutex;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-rejoin-{}-{}", name, ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

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
    let engine =
        Engine::start(EngineOptions { config_dir: Some(dir), audio: Some(Box::new(factory)), push_to_talk: false, ..Default::default() })
            .unwrap();
    let mut s = engine.state().settings.clone();
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

fn loud(v: &[f32]) -> bool {
    v.iter().map(|x| x.abs()).fold(0.0, f32::max) > 0.05
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn call_survives_a_server_restart() {
    let data = temp_dir("server");
    let cfg = |bind| ingle_server::ServerConfig {
        data_dir: data.clone(),
        bind,
        public_host: Some("127.0.0.1".into()),
        name: Some("Restart Camp".into()),
    };
    let server = ingle_server::start(cfg("127.0.0.1:0".parse().unwrap())).await.unwrap();
    let addr = server.local_addr;
    let (ha, hb) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
    let (a, b) = (bot("Alice", 440.0, ha.clone()), bot("Bob", 660.0, hb.clone()));
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice connected", 10, || a.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("bob connected", 10, || b.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    let key = a.state().servers[0].key.clone();
    let lounge = a.state().servers[0].channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
    a.join_voice(&key, lounge);
    b.join_voice(&key, lounge);
    wait_for("both in the call", 10, || a.state().call.is_some() && b.state().call.is_some()).await;
    a.start_camera();
    wait_for("alice's camera on", 10, || a.state().call.as_ref().map(|c| c.camera_on).unwrap_or(false)).await;
    wait_for("bob hears alice", 10, || loud(&hb.lock())).await;

    // The update: the server stops and the new one starts a moment later on the same port.
    server.shutdown().await;
    wait_for("alice notices", 5, || a.state().call.is_none()).await;
    assert!(a.state().voice_rejoin.is_some(), "alice should be waiting to rejoin");
    let toasts: String = a.state().toasts.iter().map(|t| t.text.clone()).collect();
    assert!(!toasts.contains("You left"), "no 'you left' message on a restart: {toasts}");
    tokio::time::sleep(Duration::from_secs(1)).await;
    let server = ingle_server::start(cfg(addr)).await.unwrap();
    let t0 = Instant::now();

    wait_for("both back in the call", 20, || {
        [&a, &b].iter().all(|e| {
            let st = e.state();
            st.voice_rejoin.is_none() && st.call.as_ref().map(|c| c.server_key == key && c.channel_id == lounge).unwrap_or(false)
        })
    })
    .await;
    eprintln!("back in the call {:?} after the server came back", t0.elapsed());
    // Her camera isn't back on by itself: she's offered it.
    wait_for("alice is offered her camera", 10, || {
        a.state().resume_offer.as_ref().is_some_and(|o| o.camera && !o.screen && o.server_key == key && o.channel_id == lounge)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!a.state().call.as_ref().is_some_and(|c| c.camera_on), "the camera stays off until she says so");
    assert!(b.state().resume_offer.is_none(), "bob had nothing on");
    // She takes it.
    a.settle_resume_offer(true, false);
    a.start_camera();
    wait_for("alice's camera back on", 10, || a.state().call.as_ref().map(|c| c.camera_on).unwrap_or(false)).await;
    assert!(a.state().resume_offer.is_none());
    wait_for("bob sees two people in the lounge", 10, || {
        b.state().servers[0].voice.values().filter(|p| p.channel_id == lounge).count() == 2
    })
    .await;
    hb.lock().clear();
    ha.lock().clear();
    wait_for("bob hears alice again", 10, || loud(&hb.lock())).await;
    wait_for("alice hears bob again", 10, || loud(&ha.lock())).await;

    // Leaving while it's reconnecting means leaving: no surprise rejoin.
    server.shutdown().await;
    wait_for("dropped again", 5, || b.state().voice_rejoin.is_some()).await;
    b.leave_voice();
    // The old socket can take a moment to be let go of.
    let mut server = None;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Ok(s) = ingle_server::start(cfg(addr)).await {
            server = Some(s);
            break;
        }
    }
    let server = server.expect("server restarts");
    wait_for("alice back", 20, || a.state().call.is_some()).await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(b.state().call.is_none() && b.state().voice_rejoin.is_none(), "bob left, so bob stays out");
    server.shutdown().await;
}
