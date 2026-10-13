//! A server you own is backed up to your PC, and comes back exactly as it was — the same
//! server, with its messages, files and identity — on another machine at another address.

use std::time::{Duration, Instant};

use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::ChannelKind;

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00, 0x03,
    0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1, 0x4d, 0x00, 0x00, 0x00, 0x10, 0x49, 0x44, 0x41,
    0x54, 0x78, 0x9c, 0x63, 0xf8, 0x5f, 0xa1, 0x01, 0x41, 0x0c, 0x70, 0x16, 0x00, 0x66, 0x62, 0x09, 0xbb, 0xb7, 0x65, 0x7e,
    0x99, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn toasts(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-backup-{}-{}", name, ingle_proto::now_ms()));
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
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(&dir, &cfg).unwrap();
    Engine::start(EngineOptions { config_dir: Some(dir), push_to_talk: false, audio: None, ..Default::default() }).unwrap()
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        std::fs::copy(e.path(), to.join(e.file_name())).unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn back_up_and_restore_elsewhere() {
    let data = temp_dir("server");
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: data.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Brethren".into()),
    })
    .await
    .unwrap();
    let (a, b) = (engine("Owner"), engine("Member"));
    a.add_server(&server.owner_link().unwrap()).unwrap();
    let connected = |e: &Engine| e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false);
    wait_for("owner connected", 10, || connected(&a)).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("member connected", 10, || connected(&b)).await;
    let key = a.state().servers[0].key.clone();
    let general = a.state().servers[0].channels.iter().find(|c| c.kind == ChannelKind::Text).unwrap().id;
    b.send_message(&key, general, "remember this", None);
    a.set_server_icon(&key, Some(PNG.to_vec()));
    wait_for("icon set", 10, || a.state().servers[0].info.icon.is_some()).await;
    wait_for("message there", 10, || {
        a.state().servers[0].logs.get(&general).map(|l| l.messages.iter().any(|m| m.content == "remember this")).unwrap_or(false)
    })
    .await;
    // Only the owner may back up; the member's app doesn't offer it.
    wait_for("owner can back up", 10, || a.can_back_up(&key)).await;
    assert!(!b.can_back_up(&key));

    let backups = temp_dir("backups");
    a.set_backup_dir(Some(backups.clone()));
    a.backup_now(&key);
    let folder = a.backup_folder(&key);
    wait_for("backed up", 30, || ingle_engine::backup::info(&folder).is_some()).await;
    let info = ingle_engine::backup::info(&folder).unwrap();
    assert_eq!(info.key, key);
    assert_eq!(info.name, "Brethren");
    assert_eq!(info.files, 1, "the icon");
    assert!(folder.join("database/latest.db").is_file());
    assert!(folder.join("identity/server-key.der").is_file());
    // A second backup only adds what's new (and keeps dated copies).
    a.backup_now(&key);
    wait_for("second backup", 30, || a.state().backups.get(&key).map(|s| !s.running && s.last_ok.unwrap_or(0) > info.last_backup).unwrap_or(false)).await;

    // The VPS is gone.
    server.shutdown().await;
    let _ = std::fs::remove_dir_all(&data);

    // A new machine: its data folder from the backup (as the setup script's --restore does).
    let new_data = temp_dir("restored");
    std::fs::copy(folder.join("database/latest.db"), new_data.join("ingle.db")).unwrap();
    copy_dir(&folder.join("identity"), &new_data);
    copy_dir(&folder.join("files"), &new_data.join("files"));
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: new_data.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: None,
    })
    .await
    .unwrap();
    // The same server: same identity (so everyone's app trusts it), same name.
    assert_eq!(ingle_proto::b64::encode(&server.fingerprint), key);
    // Members follow the new invite link (the address changed); it's the same entry.
    b.add_server(&server.invite_link()).unwrap();
    wait_for("member back", 15, || connected(&b)).await;
    assert_eq!(b.state().servers.len(), 1, "the same server, not a second one");
    // Someone new joining sees the history: it really came from the restored server.
    let c = engine("Newcomer");
    c.add_server(&server.invite_link()).unwrap();
    wait_for("newcomer connected", 15, || connected(&c)).await;
    c.load_history(&key, general, false);
    wait_for("history back", 10, || {
        c.state().servers[0].logs.get(&general).map(|l| l.messages.iter().any(|m| m.content == "remember this")).unwrap_or(false)
    })
    .await;
    assert_eq!(c.state().servers[0].info.name, "Brethren");
    assert!(c.state().servers[0].members.values().any(|m| m.name == "Member"), "members kept");
    let icon = c.state().servers[0].info.icon.clone().expect("icon kept");
    let _ = c.file(&key, &icon);
    wait_for("icon file back", 10, || matches!(c.file(&key, &icon), ingle_engine::files::FileState::Ready(_))).await;
    // And the owner is still the owner.
    a.add_server(&server.invite_link()).unwrap();
    wait_for("owner back", 15, || connected(&a)).await;
    wait_for("still owner", 10, || a.can_back_up(&key)).await;
    a.shutdown();
    b.shutdown();
    c.shutdown();
    server.shutdown().await;
}
