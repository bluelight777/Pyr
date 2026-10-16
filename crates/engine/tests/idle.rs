//! Idle, like Discord: an Ingle that's idle shows its person with a moon on everyone else's
//! Ingle; while they're on another Ingle that isn't idle, they show as online; coming back
//! shows at once.

use std::time::{Duration, Instant};

use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-idle-{}-{}", name, ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// An Ingle named `name`; with `same_as`, one that's the same person (their identity copied,
/// like Ingle on a second PC).
fn client(name: &str, same_as: Option<&std::path::Path>) -> (Engine, std::path::PathBuf) {
    let dir = temp_dir(name);
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(&dir, &cfg).unwrap();
    if let Some(other) = same_as {
        std::fs::copy(other.join("identity.key"), dir.join("identity.key")).unwrap();
    }
    let e = Engine::start(EngineOptions { config_dir: Some(dir.clone()), push_to_talk: false, ..Default::default() }).unwrap();
    (e, dir)
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

fn connected(e: &Engine) -> bool {
    e.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_shows_as_a_moon_on_everyone_elses_ingle() {
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Night Camp".into()),
    })
    .await
    .unwrap();
    let (a, a_dir) = client("Alice", None);
    let (b, _) = client("Bob", None);
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice", 10, || connected(&a)).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("bob", 10, || connected(&b)).await;
    wait_for("everyone known", 5, || b.state().servers[0].members.len() == 2).await;
    let ua = a.state().my_uid.clone();
    let alice_on_bobs = || b.state().servers[0].members.get(&ua).map(|m| (m.online, m.idle));
    assert_eq!(alice_on_bobs(), Some((true, false)), "online, not idle, to start with");
    assert!(!a.state().my_idle);

    // Alice says she's idle: Bob sees the moon, and it's remembered.
    a.set_idle_by_hand(true);
    wait_for("bob sees alice idle", 10, || alice_on_bobs() == Some((true, true))).await;
    assert!(a.state().my_idle);
    let saved = std::fs::read_to_string(a_dir.join("config.json")).unwrap();
    assert!(saved.contains("\"idle_by_hand\": true"), "{saved}");

    // Alice on a second PC, not idle there: she's online again for everyone.
    let (a2, _) = client("Alice", Some(&a_dir));
    a2.add_server(&server.invite_link()).unwrap();
    wait_for("alice's second pc", 10, || connected(&a2)).await;
    assert_eq!(a2.state().my_uid, ua, "the same person");
    wait_for("online while one of her pcs isn't idle", 10, || alice_on_bobs() == Some((true, false))).await;
    // Idle there too: idle everywhere, so idle.
    a2.set_idle_by_hand(true);
    wait_for("idle on both", 10, || alice_on_bobs() == Some((true, true))).await;
    // Back on the first PC.
    a.set_idle_by_hand(false);
    wait_for("back", 10, || alice_on_bobs() == Some((true, false))).await;
    assert!(!a.state().my_idle);
    // The PC that's in use goes away: the one left is idle, so she is.
    a.shutdown();
    wait_for("idle again, with only the idle pc left", 15, || alice_on_bobs() == Some((true, true))).await;
    a2.shutdown();
    wait_for("offline", 15, || alice_on_bobs().is_some_and(|(online, _)| !online)).await;
    assert_eq!(alice_on_bobs(), Some((false, false)), "offline isn't idle");
    b.shutdown();
    server.shutdown().await;
}
