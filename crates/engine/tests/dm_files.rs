//! Files of any kind in direct messages and groups (0.13.13): sent in encrypted parts, saved
//! where the other person picks, byte for byte; the server holds only scrambled pieces, takes
//! them only in direct messages, and a file that's gone from it shows as expired.

use std::time::{Duration, Instant};

use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-dmfiles-{}-{}", name, pyr_proto::now_ms()));
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

fn refusals(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

/// Bytes nothing else could be: a counter run through a little mixing, with a marker.
fn file_bytes(len: usize) -> Vec<u8> {
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut v: Vec<u8> = (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    v[..20].copy_from_slice(b"FILE-MARKER-31415926");
    v
}

fn all_bytes(dir: &std::path::Path) -> Vec<u8> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(all_bytes(&p));
        } else if let Ok(b) = std::fs::read(&p) {
            out.extend(b);
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_file_goes_in_parts_and_is_saved_whole() {
    let data = temp_dir("server");
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: data.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("File Camp".into()),
    })
    .await
    .unwrap();
    let (a, b) = (client("Alice"), client("Bob"));
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice connected", 10, || a.state().servers[0].status == ConnStatus::Connected).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("bob connected", 10, || b.state().servers[0].status == ConnStatus::Connected).await;
    let key = a.state().servers[0].key.clone();
    assert!(a.state().servers[0].has(pyr_proto::control::DM_FILES_FEATURE));
    let (alice_id, bob_id) = (a.state().my_uid.clone(), b.state().my_uid.clone());
    a.open_dm(&key, &bob_id);
    wait_for("dm opened", 10, || a.state().servers[0].dm_with(&bob_id).is_some()).await;
    let dm = a.state().servers[0].dm_with(&bob_id).unwrap().id;
    wait_for("bob sees the dm", 10, || b.state().servers[0].dm_with(&alice_id).is_some()).await;
    b.load_history(&key, dm, false);

    // A file a bit over two parts (8 MB each): three parts.
    let dir = temp_dir("files");
    let bytes = file_bytes(2 * 8 * 1024 * 1024 + 12_345);
    let path = dir.join("holiday video.mov");
    std::fs::write(&path, &bytes).unwrap();
    a.send_dm_file(&key, dm, path.clone()).unwrap();
    wait_for("alice's file is sent", 60, || a.state().servers[0].dm_sends.is_empty()).await;
    let file_of = |e: &Engine| {
        e.state().servers[0].logs.get(&dm).and_then(|l| l.messages.iter().flat_map(|m| m.attachments.iter()).find(|a| a.parts > 0).cloned())
    };
    wait_for("bob gets the message", 15, || file_of(&b).is_some()).await;
    let f = file_of(&b).unwrap();
    assert_eq!((f.name.as_str(), f.mime.as_str(), f.size, f.parts, f.expired), ("holiday video.mov", "video/quicktime", bytes.len() as u64, 3, false));

    // Bob saves it where he picks: byte for byte.
    let saved = dir.join("saved.mov");
    b.save_dm_file(&key, &f.id, saved.clone()).unwrap();
    assert_eq!(b.save_dm_file(&key, &f.id, saved.clone()).unwrap_err(), "It's being saved already.");
    wait_for("bob's copy is saved", 60, || b.state().servers[0].dm_saves.get(&f.id).is_some_and(|d| d.finished || d.failed.is_some())).await;
    let d = b.state().servers[0].dm_saves.get(&f.id).cloned().unwrap();
    assert_eq!((d.failed, d.done), (None, bytes.len() as u64));
    assert!(std::fs::read(&saved).unwrap() == bytes, "the saved file is the file sent");
    // Alice can save her own too.
    let hers = dir.join("hers.mov");
    a.save_dm_file(&key, &f.id, hers.clone()).unwrap();
    wait_for("alice's copy is saved", 60, || a.state().servers[0].dm_saves.get(&f.id).is_some_and(|d| d.finished || d.failed.is_some())).await;
    assert!(std::fs::read(&hers).unwrap() == bytes);

    // The server has only scrambled pieces.
    let everything = all_bytes(&data);
    assert!(!everything.windows(20).any(|w| w == b"FILE-MARKER-31415926"), "the server has the file in the clear");
    assert!(!everything.windows(17).any(|w| w == b"holiday video.mov"), "the server has the file's name");

    // Not in a channel: only direct messages and groups.
    let general = a.state().servers[0].channels.iter().find(|c| c.name == "General").unwrap().id;
    assert!(a.send_dm_file(&key, general, path.clone()).unwrap_err().contains("direct messages"));
    // Nothing empty, nothing that isn't a file.
    let empty = dir.join("empty.txt");
    std::fs::write(&empty, b"").unwrap();
    assert_eq!(a.send_dm_file(&key, dm, empty).unwrap_err(), "That file is empty.");
    assert!(a.send_dm_file(&key, dm, dir.clone()).unwrap_err().starts_with("That isn't a file"));

    // Cancelling one stops it (and nothing is sent).
    let big = dir.join("big.bin");
    std::fs::write(&big, file_bytes(5 * 8 * 1024 * 1024)).unwrap();
    let id = a.send_dm_file(&key, dm, big).unwrap();
    a.cancel_dm_file(&key, id);
    assert!(a.state().servers[0].dm_sends.is_empty());
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let files_in_dm = b.state().servers[0].logs.get(&dm).map(|l| l.messages.iter().filter(|m| m.attachments.iter().any(|a| a.parts > 0)).count()).unwrap_or(0);
    assert_eq!(files_in_dm, 1, "the cancelled one never arrives");

    // Gone from the server (a week on): it shows as expired, and can't be saved.
    let gone = server.hub.db.take_expired_dm_files(-1, -1);
    assert!(gone.len() >= 3, "{gone:?}");
    server.hub.wipe_files(&gone);
    // (Bob loads the conversation again, as after a restart.)
    b.dismiss_dm_save(&key, &f.id);
    wait_for("it shows as expired", 15, || {
        b.load_history(&key, dm, false);
        file_of(&b).is_some_and(|f| f.expired)
    })
    .await;
    assert_eq!(b.save_dm_file(&key, &f.id, dir.join("late.mov")).unwrap_err(), "That file has expired: files are kept 7 days.");
    assert!(!refusals(&a).contains("didn't go") && !refusals(&b).contains("couldn't be saved"), "{} / {}", refusals(&a), refusals(&b));
    a.shutdown();
    b.shutdown();
    server.shutdown().await;
}
