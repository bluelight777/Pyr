//! Something asked for in the second or two a server takes to restart (an update) waits for
//! it to come back, then goes through, instead of "not connected right now" (bug report 138).

use std::time::{Duration, Instant};

use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_proto::control::ChannelKind;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-held-{}-{}", name, pyr_proto::now_ms()));
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_during_a_restart_goes_through_after() {
    let data = temp_dir("server");
    let cfg = |bind| pyr_server::ServerConfig { data_dir: data.clone(), bind, public_host: Some("127.0.0.1".into()), name: Some("Held".into()) };
    let server = pyr_server::start(cfg("127.0.0.1:0".parse().unwrap())).await.unwrap();
    let addr = server.local_addr;
    let dir = temp_dir("owner");
    pyr_engine::config::save_config(&dir, &pyr_engine::config::Config { display_name: "Owner".into(), ..Default::default() }).unwrap();
    let a = Engine::start(EngineOptions { config_dir: Some(dir), ..Default::default() }).unwrap();
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("connected", 10, || a.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected)).await;
    let key = a.state().servers[0].key.clone();

    server.shutdown().await;
    wait_for("noticed", 5, || a.state().servers[0].status != ConnStatus::Connected).await;
    a.create_channel(&key, "made-while-down", ChannelKind::Text);
    tokio::time::sleep(Duration::from_millis(800)).await;
    let mut server = None;
    for _ in 0..20 {
        if let Ok(s) = pyr_server::start(cfg(addr)).await {
            server = Some(s);
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    let server = server.expect("server restarts");
    wait_for("the channel made", 15, || a.state().servers[0].channels.iter().any(|c| c.name == "made-while-down")).await;
    let toasts: Vec<String> = a.state().toasts.iter().map(|t| t.text.clone()).collect();
    assert!(!toasts.iter().any(|t| t.contains("not connected")), "no 'not connected': {toasts:?}");
    server.shutdown().await;
}
