//! Friends and blocking, like Discord: requests, accepting, unfriending; a block stops
//! direct messages and friend requests and hides the blocked person's messages.

use std::time::{Duration, Instant};

use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_proto::control::{ChannelKind, RelationKind};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-friends-{}-{}", name, pyr_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn client(name: &str) -> Engine {
    let dir = temp_dir(name);
    let cfg = pyr_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    pyr_engine::config::save_config(&dir, &cfg).unwrap();
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
    let server = pyr_server::start(pyr_server::ServerConfig {
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
    // News, not an error ("Something went wrong" with Report on it, as 0.7.6 showed it): a
    // notice that knows who sent it (for its Accept and Ignore), and goes once it's answered.
    {
        let st = b.state();
        let t = st.toasts.iter().find(|t| t.text.contains("sent you a friend request")).unwrap();
        assert!(!t.error && !t.report, "a friend request isn't an error");
        assert_eq!(t.friend.as_deref(), Some(ua.as_str()));
    }
    b.accept_friend(&ua);
    wait_for("friends", 5, || a.state().relation(&ub) == Some(RelationKind::Friend) && b.state().relation(&ua) == Some(RelationKind::Friend)).await;
    assert!(!b.state().toasts.iter().any(|t| t.friend.is_some()), "answered: its notice went");
    assert!(toasts(&a).contains("Bob accepted your friend request"), "{}", toasts(&a));
    assert!(!a.state().toasts.iter().any(|t| t.error), "{}", toasts(&a));
    b.remove_friend(&ua);
    wait_for("unfriended", 5, || a.state().relation(&ub).is_none() && b.state().relation(&ua).is_none()).await;
    // A request taken back takes its notice with it.
    a.add_friend(&ub);
    wait_for("asked again", 5, || b.state().toasts.iter().any(|t| t.friend.as_deref() == Some(ua.as_str()))).await;
    a.remove_friend(&ub);
    wait_for("taken back", 5, || b.state().relation(&ua).is_none()).await;
    assert!(!b.state().toasts.iter().any(|t| t.friend.is_some()), "{}", toasts(&b));
    // Both asking at once makes friends.
    a.add_friend(&uc);
    wait_for("carol asked", 5, || c.state().relation(&ua) == Some(RelationKind::Incoming)).await;
    c.add_friend(&ua);
    wait_for("mutual", 5, || a.state().relation(&uc) == Some(RelationKind::Friend)).await;
    wait_for("carol's notice went", 5, || !c.state().toasts.iter().any(|t| t.friend.is_some())).await;

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
