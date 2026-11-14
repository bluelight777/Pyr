//! A server tells Pyrs from before the new release key (0.13.28 and older, which can't update to
//! the rebuilt Pyr by themselves) where to get it, a moment after they log in; this Pyr, which
//! updates with the new key, is never told. (0.13.28's app seen getting it: REBUILD.md.)

use std::time::{Duration, Instant};

use pyr_engine::audio::{AudioInput, AudioOutput};
use pyr_engine::config::Settings;
use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-newpyr-{}-{}", name, pyr_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn this_pyr_is_not_told_to_get_a_new_one() {
    let data = temp_dir("server");
    let server = pyr_server::start(pyr_server::ServerConfig { data_dir: data.clone(), bind: "127.0.0.1:0".parse().unwrap(), public_host: Some("127.0.0.1".into()), name: None })
        .await
        .unwrap();
    let silent = |_s: &Settings| (AudioInput::Synthetic(Box::new(|b: &mut [f32]| b.fill(0.0))), AudioOutput::Synthetic(Box::new(|_: &[f32]| {})));
    let a = Engine::start(EngineOptions { config_dir: Some(temp_dir("a")), push_to_talk: false, audio: Some(Box::new(silent)), ..Default::default() }).unwrap();
    a.add_server(&server.owner_link().unwrap()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !a.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected) {
        assert!(Instant::now() < deadline, "never connected");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // (The note would come three seconds in.)
    tokio::time::sleep(Duration::from_millis(4500)).await;
    let told: Vec<String> = a.state().toasts.iter().map(|t| t.text.clone()).filter(|t| t.contains("new Pyr")).collect();
    assert!(told.is_empty(), "{told:?}");
    a.shutdown();
    drop(a);
    server.shutdown().await;
    let _ = std::fs::remove_dir_all(&data);
}
