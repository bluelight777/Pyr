//! GIF search is set up per server, by its owner only, and then works for everyone in it.
//! (GIPHY and KLIPY want each app to search them itself, so every member's app gets the key.)

use std::time::{Duration, Instant};

use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-giphy-{}-{}", name, pyr_proto::now_ms()));
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

fn engine(name: &str) -> Engine {
    let dir = temp_dir(name);
    let cfg = pyr_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    pyr_engine::config::save_config(&dir, &cfg).unwrap();
    Engine::start(EngineOptions { config_dir: Some(dir), push_to_talk: false, audio: None, ..Default::default() }).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn only_the_owner_sets_up_gif_search() {
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: None,
    })
    .await
    .unwrap();
    let (a, b) = (engine("Owner"), engine("Member"));
    a.add_server(&server.owner_link().unwrap()).unwrap();
    let connected = |e: &Engine| e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false);
    wait_for("owner", 10, || connected(&a)).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("member", 10, || connected(&b)).await;
    let key = a.state().servers[0].key.clone();
    assert!(b.state().servers[0].info.giphy_key.is_empty(), "off to begin with");

    // A member can't.
    b.set_giphy_key(&key, "MemberKey123");
    wait_for("refused", 10, || b.state().toasts.iter().any(|t| t.error && t.text.contains("Only the server's owner"))).await;
    assert!(b.state().servers[0].info.giphy_key.is_empty());
    // Not a key.
    a.set_giphy_key(&key, "not a key; rm -rf");
    wait_for("rejected", 10, || a.state().toasts.iter().any(|t| t.error && t.text.contains("doesn't look like"))).await;
    // The owner can, and everyone gets it.
    a.set_giphy_key(&key, "OwnerKey456");
    wait_for("member has it", 10, || b.state().servers[0].info.giphy_key == "OwnerKey456").await;
    // And off again.
    a.set_giphy_key(&key, "");
    wait_for("off again", 10, || b.state().servers[0].info.giphy_key.is_empty()).await;
    a.shutdown();
    b.shutdown();
    server.shutdown().await;
}
