//! Friends and blocking, like Discord: requests, accepting, unfriending; a block stops
//! direct messages and friend requests and hides the blocked person's messages.

use std::time::{Duration, Instant};

use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::{ChannelKind, RelationKind};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-friends-{}-{}", name, ingle_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn client(name: &str) -> Engine {
    let dir = temp_dir(name);
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(&dir, &cfg).unwrap();
    Engine::start(EngineOptions { config_dir: Some(dir), push_to_talk: false, ..Default::default() }).unwrap()
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn friends_and_blocking() {
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Friend Camp".into()),
    })
    .await
    .unwrap();
    let (a, b, c) = (client("Alice"), client("Bob"), client("Carol"));
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice", 10, || a.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    for e in [&b, &c] {
        e.add_server(&server.invite_link()).unwrap();
        wait_for("joined", 10, || e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    }
    wait_for("everyone known", 5, || a.state().servers[0].members.len() == 3).await;
    let (ua, ub, uc) = (a.state().my_uid.clone(), b.state().my_uid.clone(), c.state().my_uid.clone());
    let key = a.state().servers[0].key.clone();

    // Request, accept, unfriend.
    a.add_friend(&ub);
    wait_for("bob gets the request", 5, || b.state().relation(&ua) == Some(RelationKind::Incoming)).await;
    assert_eq!(a.state().relation(&ub), Some(RelationKind::Outgoing));
    assert!(toasts(&b).contains("Alice sent you a friend request"), "{}", toasts(&b));
    b.accept_friend(&ua);
    wait_for("friends", 5, || a.state().relation(&ub) == Some(RelationKind::Friend) && b.state().relation(&ua) == Some(RelationKind::Friend)).await;
    b.remove_friend(&ua);
    wait_for("unfriended", 5, || a.state().relation(&ub).is_none() && b.state().relation(&ua).is_none()).await;
    // Both asking at once makes friends.
    a.add_friend(&uc);
    wait_for("carol asked", 5, || c.state().relation(&ua) == Some(RelationKind::Incoming)).await;
    c.add_friend(&ua);
    wait_for("mutual", 5, || a.state().relation(&uc) == Some(RelationKind::Friend)).await;

    // Alice blocks Carol: friendship ends, Carol can't message or re-add her, Carol isn't told.
    a.block(&uc);
    wait_for("blocked", 5, || a.state().relation(&uc) == Some(RelationKind::Blocked) && c.state().relation(&ua).is_none()).await;
    c.open_dm(&key, &ua);
    wait_for("carol can't DM", 5, || toasts(&c).contains("can't message")).await;
    c.add_friend(&ua);
    wait_for("carol can't add", 5, || toasts(&c).contains("can't add")).await;
    // Carol's messages in a channel still arrive for Alice, but don't count as unread.
    let text = a.state().servers[0].channels.iter().find(|ch| ch.kind == ChannelKind::Text).unwrap().id;
    a.load_history(&key, text, false);
    wait_for("history", 5, || a.state().servers[0].logs.get(&text).map(|l| l.loaded).unwrap_or(false)).await;
    c.send_message(&key, text, "hello alice", None);
    wait_for("arrives", 5, || a.state().servers[0].logs[&text].messages.iter().any(|m| m.content == "hello alice")).await;
    assert!(!a.state().servers[0].unread.contains(&text));

    // Unblock: Carol can message again.
    a.unblock(&uc);
    wait_for("unblocked", 5, || a.state().relation(&uc).is_none()).await;
    c.open_dm(&key, &ua);
    wait_for("carol can DM", 5, || c.state().servers[0].dm_with(&ua).is_some()).await;
    server.shutdown().await;
}
