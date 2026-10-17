//! The server lets go of a connection once it ends. (It used to keep a task and the closed
//! connection's memory for every app that disconnected outside a voice channel, for as long
//! as the server ran.) Seen from outside: the port is free the moment the server stops.

use std::time::{Duration, Instant};

use pyr_engine::audio::{AudioInput, AudioOutput};
use pyr_engine::config::Settings;
use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_proto::control::ChannelKind;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-letsgo-{}-{}", name, pyr_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
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

/// Someone connects (and maybe joins voice), quits, then the server stops: its port must be
/// free straight away, i.e. nothing of that connection is left behind.
async fn quits_then_server_stops(voice: bool) {
    let data = temp_dir(&format!("server-{voice}"));
    let cfg = |bind| pyr_server::ServerConfig { data_dir: data.clone(), bind, public_host: Some("127.0.0.1".into()), name: None };
    let server = pyr_server::start(cfg("127.0.0.1:0".parse().unwrap())).await.unwrap();
    let addr = server.local_addr;
    let silent = |_s: &Settings| {
        (AudioInput::Synthetic(Box::new(|b: &mut [f32]| b.fill(0.0))), AudioOutput::Synthetic(Box::new(|_: &[f32]| {})))
    };
    let a = Engine::start(EngineOptions {
        config_dir: Some(temp_dir(&format!("a-{voice}"))),
        push_to_talk: false,
        audio: Some(Box::new(silent)),
        ..Default::default()
    })
    .unwrap();
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("connected", 10, || a.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    if voice {
        let key = a.state().servers[0].key.clone();
        let lounge = a.state().servers[0].channels.iter().find(|c| c.kind == ChannelKind::Voice).unwrap().id;
        a.join_voice(&key, lounge);
        wait_for("in the call", 10, || a.state().call.is_some()).await;
    }
    a.shutdown();
    drop(a);
    tokio::time::sleep(Duration::from_millis(500)).await;
    server.shutdown().await;
    let t = Instant::now();
    let mut why = String::new();
    while t.elapsed() < Duration::from_secs(3) {
        match pyr_server::start(cfg(addr)).await {
            Ok(s) => {
                s.shutdown().await;
                return;
            }
            Err(e) => why = format!("{e:#}"),
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the server kept its port (voice: {voice}): {why}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_left_after_someone_quits() {
    quits_then_server_stops(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nothing_left_after_someone_quits_from_voice() {
    quits_then_server_stops(true).await;
}
