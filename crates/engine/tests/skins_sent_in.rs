//! Skins sent in for the Ingle Library, and new versions of what someone sent: what's sent is
//! signed with the sender's Ingle identity, so only they can send a new version of it, and they
//! can see where theirs stand (with the owner's note). (Its own file: each sets
//! INGLE_LIBRARY_SERVER for the whole test program.)

use std::time::{Duration, Instant};

use ingle_engine::state::{BugSend, ConnStatus};
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::{PluginSubmission, SubmissionStatus};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-skins-sent-{}-{}", name, ingle_proto::now_ms()));
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

fn toast(e: &Engine, s: &str) -> bool {
    e.state().toasts.iter().any(|t| t.text.contains(s))
}

async fn send(e: &Engine, p: PluginSubmission, turn_on: Option<&str>) -> BugSend {
    let before = e.state().plugin_sends_done;
    e.submit_plugin(p, turn_on);
    // Sending, until the answer (which, from a server on this machine, can be back already).
    let st = e.state();
    assert!(st.plugin_send == BugSend::Sending || st.plugin_sends_done > before, "{:?}", st.plugin_send);
    drop(st);
    wait_for("the answer", 15, || e.state().plugin_sends_done > before).await;
    e.state().plugin_send.clone()
}

/// A PNG's first bytes (all the checks look at) for a picture `w` × `h`, then `size` bytes in all.
fn png(w: u32, h: u32, size: usize) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    b.extend_from_slice(&w.to_be_bytes());
    b.extend_from_slice(&h.to_be_bytes());
    b.extend_from_slice(&[8, 6, 0, 0, 0]);
    // Not all zeros, so nothing along the way squeezes it.
    let mut x = 0x9e37_79b9u32;
    while b.len() < size {
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        b.push(x as u8);
    }
    b
}

fn skin(version: &str, picture: usize) -> PluginSubmission {
    let json = format!(
        r##"{{ "name": "Blood Moon", "version": "{version}", "summary": "Deep reds for the guild.", "author": "Mara",
              "colors": {{ "ember": "#c0392b", "main": "#1a0f10" }},
              "background": {{ "file": "moon.png", "show": 0.3 }} }}"##
    );
    let files = vec![("moon.png".to_string(), png(1920, 1080, picture)), ("preview.png".to_string(), png(640, 400, 200_000)), ("skin.json".to_string(), json.into_bytes())];
    let report = ingle_engine::skins::check(&files);
    assert!(report.ok(), "{:?}", report.problems);
    ingle_engine::skins::submission(&files, &report, "", 0).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn skins_and_new_versions_come_signed_and_senders_see_where_theirs_stand() {
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server2"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Ingle Library".into()),
    })
    .await
    .unwrap();
    let key = ingle_proto::b64::encode(&server.fingerprint);
    // SAFETY: set before any engine reads it, in this test's own process.
    unsafe { std::env::set_var("INGLE_LIBRARY_SERVER", format!("127.0.0.1:{}/{key}", server.local_addr.port())) };
    let (owner, mara, someone) = (engine("Owner2"), engine("Mara"), engine("Someone"));
    owner.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("owner", 10, || owner.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    owner.set_plugin_submissions(&key, true);
    wait_for("collecting", 10, || owner.state().servers[0].info.plugin_submissions).await;

    // Nothing sent yet.
    mara.my_submissions();
    wait_for("Mara's (none)", 15, || matches!(&mara.state().my_submissions, Some(Ok(l)) if l.is_empty())).await;
    // A skin too big for one message: its files follow in parts.
    assert_eq!(send(&mara, skin("1.0", 650_000), None).await, BugSend::Sent);
    assert!(toast(&mara, "Your skin will be looked at before it's listed"));
    wait_for("the owner's count of skins", 10, || owner.state().servers[0].info.new_skin_submissions == 1).await;
    assert_eq!(owner.state().servers[0].info.new_plugin_submissions, 0, "skins are counted apart");
    owner.list_submissions(&key);
    wait_for("listed", 10, || owner.state().submissions.get(&key).is_some_and(|l| l.len() == 1)).await;
    let first = owner.state().submissions[&key][0].clone();
    assert_eq!((first.kind.as_str(), first.name.as_str(), first.files, first.author_id.clone()), ("skin", "Blood Moon", 3, mara.state().my_uid.clone()));
    // Its files, the big one fetched by itself.
    owner.open_submission(&key, first.id);
    wait_for("files", 15, || owner.state().submission_files.contains_key(&(key.clone(), first.id))).await;
    let files = owner.state().submission_files[&(key.clone(), first.id)].clone();
    assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), vec!["moon.png", "preview.png", "skin.json"]);
    assert_eq!(ingle_proto::b64::decode(&files[0].data).unwrap(), png(1920, 1080, 650_000));

    // Mara sees hers waiting; once approved (with a word from the owner), she sees that too.
    mara.my_submissions();
    wait_for("Mara's waiting", 15, || matches!(&mara.state().my_submissions, Some(Ok(l)) if l.len() == 1 && l[0].status == SubmissionStatus::New)).await;
    owner.set_submission_status(&key, first.id, SubmissionStatus::Approved, "Lovely. It'll be in the Library soon.".into());
    let saved = owner.shared.config_dir.join(ingle_engine::plugins::APPROVED_SKINS_DIR).join(&key).join(format!("{}-blood-moon", first.id));
    wait_for("saved for listing", 10, || saved.join("skin/skin.json").exists()).await;
    assert_eq!(std::fs::read(saved.join("skin/moon.png")).unwrap(), png(1920, 1080, 650_000));
    mara.my_submissions();
    wait_for("Mara's approved", 15, || {
        matches!(&mara.state().my_submissions, Some(Ok(l)) if l.len() == 1 && l[0].status == SubmissionStatus::Approved && l[0].note == "Lovely. It'll be in the Library soon.")
    })
    .await;

    // A new version from Mara is taken, saying what changed.
    let mut v2 = skin("1.1", 640_000);
    v2.update_of = first.id;
    assert_eq!(send(&mara, v2.clone(), None).await, BugSend::Sent);
    owner.list_submissions(&key);
    wait_for("the new version", 10, || owner.state().submissions[&key].len() == 2).await;
    let second = owner.state().submissions[&key].iter().find(|e| e.id != first.id).cloned().unwrap();
    assert_eq!((second.update_of, second.version.as_str(), second.status), (first.id, "1.1", SubmissionStatus::New));
    assert!(second.changes.contains("moon.png") && second.changes.contains("skin.json"), "{}", second.changes);
    // Someone else can't send one in its place.
    match send(&someone, v2, None).await {
        BugSend::Failed { message, .. } => assert!(message.contains("only whoever sent it"), "{message}"),
        other => panic!("only Mara can send a new version of hers: got {other:?}"),
    }
    someone.my_submissions();
    wait_for("Someone's (none)", 15, || matches!(&someone.state().my_submissions, Some(Ok(l)) if l.is_empty())).await;
}
