//! End-to-end for 0.3.0 features: custom roles and their permissions, images in chat
//! (upload, download, refusing non-images, wiping on delete) and profile pictures.

use std::time::{Duration, Instant};

use ingle_engine::files::FileState;
use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::{ChannelKind, perm};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-v03-{}-{}", name, ingle_proto::now_ms()));
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

/// A 3×2 orange PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03,
    0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41,
    0x54, 0x78, 0x9c, 0x63, 0xf8, 0x5f, 0xa1, 0x01, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x66, 0x62, 0x09, 0xbb, 0xb7, 0x65, 0x7e,
    0x99, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];
/// A 4×3 GIF.
const GIF: &[u8] = b"GIF89a\x04\x00\x03\x00\x80\x00\x00\xff\x00\x00\x00\x00\x00,\x00\x00\x00\x00\x04\x00\x03\x00\x00\x02\x03\x84\x8f\x59\x00;";

fn toasts(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn roles_images_and_avatars() {
    let data = temp_dir("server");
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: data.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Roles Camp".into()),
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
    assert!(a.state().servers[0].has(ingle_proto::control::ROLES_FEATURE));
    let (bob_id, carol_id, alice_id) = (b.state().my_uid.clone(), c.state().my_uid.clone(), a.state().my_uid.clone());
    let text = a.state().servers[0].channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id;

    // ---- roles: only people allowed to can make them
    b.create_role(&key, "Sneaky", 0, perm::ALL, false);
    wait_for("bob is refused", 5, || toasts(&b).contains("permission")).await;
    a.create_role(&key, "Mods", 0x3498db, perm::MANAGE_MESSAGES | perm::KICK | perm::MANAGE_ROLES, true);
    wait_for("everyone sees Mods", 5, || b.state().servers[0].roles.iter().any(|r| r.name == "Mods" && r.hoist)).await;
    let mods = b.state().servers[0].roles.iter().find(|r| r.name == "Mods").unwrap().id;
    assert!(!b.state().servers[0].roles.iter().any(|r| r.name == "Sneaky"));
    a.set_member_roles(&key, &bob_id, vec![mods]);
    wait_for("bob has Mods", 5, || b.state().servers[0].can(perm::KICK)).await;
    assert_eq!(c.state().servers[0].name_color(&bob_id), Some(0x3498db), "his name shows in the role's color");
    assert!(!b.state().servers[0].can(perm::BAN));

    // Bob can't create a role with powers he doesn't have, or touch Mods (his own top role).
    b.create_role(&key, "Banners", 0, perm::BAN, false);
    wait_for("bob can't hand out ban", 5, || toasts(&b).contains("permissions you don't have")).await;
    b.set_member_roles(&key, &carol_id, vec![mods]);
    wait_for("bob can't hand out his own top role", 5, || toasts(&b).contains("at or above your own highest role")).await;
    // But he can make and hand out a role below his own.
    b.create_role(&key, "Helpers", 0x2ecc71, perm::MANAGE_MESSAGES, false);
    wait_for("Helpers exists", 5, || b.state().servers[0].roles.iter().any(|r| r.name == "Helpers")).await;
    let helpers = b.state().servers[0].roles.iter().find(|r| r.name == "Helpers").unwrap().id;
    b.set_member_roles(&key, &carol_id, vec![helpers]);
    wait_for("carol is a helper", 5, || c.state().servers[0].can(perm::MANAGE_MESSAGES)).await;
    assert_eq!(b.state().servers[0].roles[0].name, "Mods", "new roles go below existing ones");

    // Mods can delete other people's messages.
    c.send_message(&key, text, "carol was here", None);
    wait_for("bob sees carol's message", 5, || {
        b.state().servers[0].logs.get(&text).map(|l| l.messages.iter().any(|m| m.content == "carol was here")).unwrap_or(false)
    })
    .await;
    let mid = b.state().servers[0].logs[&text].messages.iter().find(|m| m.content == "carol was here").unwrap().id;
    b.delete_message(&key, mid);
    wait_for("carol's message is gone for carol", 5, || {
        c.state().servers[0].logs.get(&text).map(|l| l.find(mid).is_none()).unwrap_or(true)
    })
    .await;

    // Bob can kick Carol but not Alice (the owner).
    b.kick(&key, &alice_id, false);
    wait_for("bob can't kick the owner", 5, || toasts(&b).contains("same or a higher role")).await;
    b.kick(&key, &carol_id, false);
    wait_for("carol was kicked", 5, || matches!(c.state().servers[0].status, ConnStatus::Failed(_))).await;

    // ---- images
    a.send_message_with(&key, text, "look", None, vec![("dot.png".into(), PNG.to_vec()), ("tiny.gif".into(), GIF.to_vec())]);
    wait_for("bob gets the images", 10, || {
        b.state().servers[0].logs[&text].messages.iter().any(|m| m.content == "look" && m.attachments.len() == 2)
    })
    .await;
    let msg = b.state().servers[0].logs[&text].messages.iter().find(|m| m.content == "look").unwrap().clone();
    assert_eq!((msg.attachments[0].width, msg.attachments[0].height, msg.attachments[0].mime.as_str()), (3, 2, "image/png"));
    assert_eq!((msg.attachments[1].width, msg.attachments[1].height, msg.attachments[1].mime.as_str()), (4, 3, "image/gif"));
    let png_id = msg.attachments[0].id.clone();
    let _ = b.file(&key, &png_id);
    wait_for("bob downloads the png", 10, || matches!(b.file(&key, &png_id), FileState::Ready(_))).await;
    match b.file(&key, &png_id) {
        FileState::Ready(bytes) => assert_eq!(&bytes[..], PNG),
        other => panic!("{other:?}"),
    }
    let on_disk = data.join("files").join(&png_id);
    assert!(on_disk.exists());
    // An image-only message (no text) is fine; a program is not.
    a.send_message_with(&key, text, "", None, vec![("only.png".into(), PNG.to_vec())]);
    wait_for("image-only message arrives", 10, || {
        b.state().servers[0].logs[&text].messages.iter().any(|m| m.content.is_empty() && m.attachments.len() == 1)
    })
    .await;
    a.send_message_with(&key, text, "run me", None, vec![("evil.exe".into(), b"MZ\x90\x00 not an image".to_vec())]);
    wait_for("the exe is refused", 10, || {
        a.state().servers[0].logs[&text].pending.iter().any(|p| p.failed.as_deref().unwrap_or("").contains("Only PNG"))
    })
    .await;

    // Deleting the message wipes its images from the server's disk.
    a.delete_message(&key, msg.id);
    wait_for("image file wiped", 5, || !on_disk.exists()).await;
    wait_for("bob forgets the image", 5, || !matches!(b.file(&key, &png_id), FileState::Ready(_))).await;

    // ---- profile pictures
    a.set_avatar(Some(PNG.to_vec()));
    let want = ingle_proto::control::avatar_id(&alice_id, PNG);
    wait_for("bob sees alice's picture", 10, || {
        b.state().servers[0].members.get(&alice_id).and_then(|m| m.avatar.clone()) == Some(want.clone())
    })
    .await;
    wait_for("bob downloads it", 10, || matches!(b.file(&key, &want), FileState::Ready(_))).await;
    assert!(data.join("files").join(&want).exists());
    a.set_avatar(None);
    wait_for("picture removed", 10, || b.state().servers[0].members.get(&alice_id).map(|m| m.avatar.is_none()).unwrap_or(false))
        .await;
    wait_for("and wiped", 5, || !data.join("files").join(&want).exists()).await;

    // ---- deleting a role takes it off everyone
    a.delete_role(&key, mods);
    wait_for("Mods gone", 5, || {
        !b.state().servers[0].roles.iter().any(|r| r.id == mods) && !b.state().servers[0].can(perm::KICK)
    })
    .await;
    server.shutdown().await;
}
