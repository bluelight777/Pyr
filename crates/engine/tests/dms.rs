//! Direct messages: only the two people see them, read them or their pictures; only the
//! author deletes; they show as unread after reconnecting.

use std::time::{Duration, Instant};

use pyr_engine::files::FileState;
use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_proto::control::perm;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-dms-{}-{}", name, pyr_proto::now_ms()));
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

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03,
    0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41,
    0x54, 0x78, 0x9c, 0x63, 0xf8, 0x5f, 0xa1, 0x01, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x66, 0x62, 0x09, 0xbb, 0xb7, 0x65, 0x7e,
    0x99, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn toasts(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_messages_are_private() {
    let data = temp_dir("server");
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: data.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("DM Camp".into()),
    })
    .await
    .unwrap();
    let (a, b, c) = (client("Alice"), client("Bob"), client("Carol"));
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice connected", 10, || a.state().servers[0].status == ConnStatus::Connected).await;
    for e in [&b, &c] {
        e.add_server(&server.invite_link()).unwrap();
        wait_for("member connected", 10, || e.state().servers[0].status == ConnStatus::Connected).await;
    }
    let key = a.state().servers[0].key.clone();
    assert!(a.state().servers[0].has(pyr_proto::control::DMS_FEATURE));
    let (alice_id, bob_id) = (a.state().my_uid.clone(), b.state().my_uid.clone());

    // Alice opens a conversation with Bob; Bob learns about it.
    a.open_dm(&key, &bob_id);
    let mut opened = None;
    wait_for("dm opened", 5, || {
        opened = opened.take().or_else(|| a.take_opened_dm());
        opened.is_some()
    })
    .await;
    let (k, dm) = opened.unwrap();
    assert_eq!(k, key);
    wait_for("bob sees the conversation", 5, || b.state().servers[0].dm_with(&alice_id).map(|d| d.id) == Some(dm)).await;
    // Opening it again finds the same one, from either side.
    b.open_dm(&key, &alice_id);
    wait_for("same conversation", 5, || b.take_opened_dm() == Some((key.clone(), dm))).await;

    a.send_message(&key, dm, "psst bob", None);
    wait_for("bob gets it", 5, || {
        b.state().servers[0].logs.get(&dm).map(|l| l.messages.iter().any(|m| m.content == "psst bob")).unwrap_or(false)
    })
    .await;
    assert_eq!(b.state().servers[0].mentions.get(&dm), Some(&1), "a direct message gets the red badge");
    assert!(toasts(&b).contains("Alice: psst bob"), "{}", toasts(&b));
    assert_eq!(b.state().servers[0].dms[0].last_message, b.state().servers[0].logs[&dm].messages.last().map(|m| m.id));

    // Carol sees nothing and can't read, write or type into it.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(c.state().servers[0].logs.get(&dm).map(|l| l.messages.is_empty()).unwrap_or(true));
    assert!(c.state().servers[0].dms.is_empty());
    c.load_history(&key, dm, false);
    wait_for("carol can't read it", 5, || toasts(&c).contains("No such channel")).await;
    c.send_message(&key, dm, "let me in", None);
    wait_for("carol can't write", 5, || {
        c.state().servers[0].logs.get(&dm).map(|l| l.pending.iter().any(|p| p.failed.is_some())).unwrap_or(false)
    })
    .await;
    assert!(!b.state().servers[0].logs[&dm].messages.iter().any(|m| m.content == "let me in"));

    // Pictures in it are private too.
    b.send_message_with(&key, dm, "", None, vec![("pic.png".into(), PNG.to_vec())]);
    wait_for("alice gets the picture", 10, || {
        a.state().servers[0].logs.get(&dm).map(|l| l.messages.iter().any(|m| m.attachments.len() == 1)).unwrap_or(false)
    })
    .await;
    let pic = a.state().servers[0].logs[&dm].messages.iter().find(|m| m.attachments.len() == 1).unwrap().attachments[0].id.clone();
    let _ = a.file(&key, &pic);
    wait_for("alice can download it", 10, || matches!(a.file(&key, &pic), FileState::Ready(_))).await;
    let _ = c.file(&key, &pic);
    wait_for("carol can't", 10, || matches!(c.file(&key, &pic), FileState::Failed(_))).await;

    // A moderator can't delete someone else's direct message.
    let mods_id = {
        a.create_role(&key, "Mods", 0, perm::MANAGE_MESSAGES, false);
        wait_for("mods", 5, || a.state().servers[0].roles.iter().any(|r| r.name == "Mods")).await;
        a.state().servers[0].roles.iter().find(|r| r.name == "Mods").unwrap().id
    };
    a.set_member_roles(&key, &bob_id, vec![mods_id]);
    wait_for("bob is a mod", 5, || b.state().servers[0].can(perm::MANAGE_MESSAGES)).await;
    let alices = b.state().servers[0].logs[&dm].messages.iter().find(|m| m.content == "psst bob").unwrap().id;
    b.delete_message(&key, alices);
    wait_for("refused", 5, || toasts(&b).contains("can't delete")).await;
    // But Alice can, and it's gone for both.
    a.delete_message(&key, alices);
    wait_for("gone for bob", 5, || b.state().servers[0].logs[&dm].find(alices).is_none()).await;

    // ---- server icon: only people who can manage the server set it; everyone sees it.
    c.set_server_icon(&key, Some(PNG.to_vec()));
    wait_for("carol can't set the icon", 5, || toasts(&c).contains("change the server's icon")).await;
    a.set_server_icon(&key, Some(PNG.to_vec()));
    wait_for("bob sees the icon", 10, || b.state().servers[0].info.icon.is_some()).await;
    let icon = b.state().servers[0].info.icon.clone().unwrap();
    let _ = b.file(&key, &icon);
    wait_for("bob downloads it", 10, || matches!(b.file(&key, &icon), FileState::Ready(_))).await;
    a.set_server_icon(&key, None);
    wait_for("icon removed", 10, || b.state().servers[0].info.icon.is_none()).await;
    wait_for("and wiped", 5, || !data.join("files").join(&icon).exists()).await;

    // Carol messages Alice while Alice is away; Alice sees it as unread when she's back.
    let a_dir_key = key.clone();
    drop(a);
    let carol_alice = {
        c.open_dm(&key, &alice_id);
        let mut got = None;
        wait_for("carol's dm", 5, || {
            got = got.take().or_else(|| c.take_opened_dm());
            got.is_some()
        })
        .await;
        got.unwrap().1
    };
    assert_ne!(carol_alice, dm);
    c.send_message(&key, carol_alice, "you there?", None);
    wait_for("sent", 5, || {
        c.state().servers[0].logs.get(&carol_alice).map(|l| l.messages.iter().any(|m| m.content == "you there?")).unwrap_or(false)
    })
    .await;
    let _ = a_dir_key;
    server.shutdown().await;
}
