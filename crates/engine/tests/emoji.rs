//! A server's own emoji: who can add them, everyone seeing them, reacting with them,
//! renaming and removing (which wipes the picture).

use std::time::{Duration, Instant};

use pyr_engine::files::FileState;
use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_proto::control::ChannelKind;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-emoji-{}-{}", name, pyr_proto::now_ms()));
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

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03,
    0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41,
    0x54, 0x78, 0x9c, 0x63, 0xf8, 0x5f, 0xa1, 0x01, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x66, 0x62, 0x09, 0xbb, 0xb7, 0x65, 0x7e,
    0x99, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn server_emoji() {
    let data = temp_dir("server");
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: data.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Emoji Camp".into()),
    })
    .await
    .unwrap();
    let (a, b) = (client("Alice"), client("Bob"));
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice", 10, || a.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("bob", 10, || b.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    let key = a.state().servers[0].key.clone();
    assert!(a.state().servers[0].has(pyr_proto::control::EMOJI_FEATURE));
    let text = a.state().servers[0].channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id;

    // Members can't add emoji; the owner can, and everyone gets it.
    b.add_emoji(&key, "sneaky", PNG.to_vec(), false);
    wait_for("bob refused", 5, || toasts(&b).contains("permission")).await;
    a.add_emoji(&key, "party_cat", PNG.to_vec(), false);
    wait_for("bob sees :party_cat:", 5, || b.state().servers[0].info.emojis.iter().any(|e| e.name == "party_cat")).await;
    let id = b.state().servers[0].info.emojis[0].id.clone();
    let _ = b.file(&key, &id);
    wait_for("bob downloads it", 10, || matches!(b.file(&key, &id), FileState::Ready(_))).await;
    // Same name again is refused, bad names too.
    a.add_emoji(&key, "party_cat", vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3], false);
    a.add_emoji(&key, "no spaces", PNG.to_vec(), false);
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(a.state().servers[0].info.emojis.len(), 1, "{}", toasts(&a));

    // React with it; an emoji this server doesn't have is refused.
    let tag = format!("<:party_cat:{id}>");
    b.send_message(&key, text, &format!("hi {tag}"), None);
    wait_for("message", 5, || a.state().servers[0].logs.get(&text).map(|l| !l.messages.is_empty()).unwrap_or(false)).await;
    let mid = a.state().servers[0].logs[&text].messages.last().unwrap().id;
    a.react(&key, mid, &tag, true);
    wait_for("reaction", 5, || {
        b.state().servers[0].logs.get(&text).and_then(|l| l.messages.last()).map(|m| m.reactions.iter().any(|r| r.emoji == tag)).unwrap_or(false)
    })
    .await;
    b.react(&key, mid, "<:fake:abcdefgh12345>", true);
    wait_for("fake refused", 5, || toasts(&b).contains("not an emoji on this server")).await;

    // Rename, then remove: the picture is wiped from the server.
    a.rename_emoji(&key, &id, "partycat");
    wait_for("renamed", 5, || b.state().servers[0].info.emojis.iter().any(|e| e.name == "partycat")).await;
    a.remove_emoji(&key, &id);
    wait_for("removed", 5, || b.state().servers[0].info.emojis.is_empty()).await;
    wait_for("wiped", 5, || !data.join("files").join(&id).exists()).await;
    server.shutdown().await;
}
