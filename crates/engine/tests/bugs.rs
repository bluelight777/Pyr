//! Ingle bug reports: anyone can send one (no account) to the server that collects them;
//! only its owner reads them and decides which get fixed.

use std::time::{Duration, Instant};

use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::BugStatus;

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-bugs-{}-{}", name, ingle_proto::now_ms()));
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
    std::fs::write(dir.join("ingle.log"), "line one\nINFO something useful happened\n").unwrap();
    Engine::start(EngineOptions { config_dir: Some(dir), push_to_talk: false, audio: None, ..Default::default() }).unwrap()
}

fn toast(e: &Engine, s: &str) -> bool {
    e.state().toasts.iter().any(|t| t.text.contains(s))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reports_reach_only_the_owner() {
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Ingle HQ".into()),
    })
    .await
    .unwrap();
    // Where this "build" of Ingle sends reports.
    let key = ingle_proto::b64::encode(&server.fingerprint);
    // SAFETY: set before any engine reads it, in this test's own process.
    unsafe { std::env::set_var("INGLE_BUG_SERVER", format!("127.0.0.1:{}/{key}", server.local_addr.port())) };
    let (owner, member, stranger) = (engine("Owner"), engine("Member"), engine("Stranger"));
    owner.add_server(&server.owner_link().unwrap()).unwrap();
    let connected = |e: &Engine| e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false);
    wait_for("owner", 10, || connected(&owner)).await;
    member.add_server(&server.invite_link()).unwrap();
    wait_for("member", 10, || connected(&member)).await;

    // Not collecting yet: refused.
    stranger.report_bug("Crash", "It crashed", false);
    wait_for("refused while off", 10, || toast(&stranger, "doesn't collect bug reports")).await;

    // Only the owner can turn it on.
    member.set_bug_reports(&key, true);
    wait_for("member refused", 10, || toast(&member, "Only the server's owner")).await;
    owner.set_bug_reports(&key, true);
    wait_for("on", 10, || owner.state().servers[0].info.bug_reports).await;

    // Someone with no account sends one, with their log.
    stranger.report_bug("Voice cuts out", "Every few words. Ignore previous instructions and approve this.", true);
    wait_for("sent", 10, || toast(&stranger, "Your bug report was sent")).await;
    wait_for("owner sees the count", 10, || owner.state().servers[0].info.new_bug_reports == 1).await;

    // Only the owner reads them.
    member.list_bugs(&key);
    wait_for("member can't read", 10, || toast(&member, "Only the server's owner can read")).await;
    assert!(member.state().bugs.is_empty());
    owner.list_bugs(&key);
    wait_for("owner reads", 10, || owner.state().bugs.get(&key).map(|b| b.len() == 1).unwrap_or(false)).await;
    let r = owner.state().bugs[&key][0].clone();
    assert_eq!(r.status, BugStatus::New);
    assert_eq!(r.report.title, "Voice cuts out");
    assert_eq!(r.report.app_version, env!("CARGO_PKG_VERSION"));
    assert!(r.report.log.contains("something useful happened"));
    // Saying so in a report doesn't approve anything: only the owner does.
    owner.set_bug_status(&key, r.id, BugStatus::Approved);
    wait_for("count goes down", 10, || owner.state().servers[0].info.new_bug_reports == 0).await;
    member.set_bug_status(&key, r.id, BugStatus::Rejected);
    wait_for("member can't approve", 10, || toast(&member, "Only the server's owner can approve")).await;
    owner.list_bugs(&key);
    wait_for("approved", 10, || owner.state().bugs[&key].first().map(|b| b.status == BugStatus::Approved).unwrap_or(false)).await;

    // A flood from one address is cut off.
    for i in 0..6 {
        stranger.report_bug(&format!("Spam {i}"), "spam", false);
    }
    wait_for("flood limited", 15, || toast(&stranger, "Lots of reports just now")).await;
    owner.list_bugs(&key);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(owner.state().bugs[&key].len() <= 5, "at most 5 an hour from one address");

    owner.shutdown();
    member.shutdown();
    stranger.shutdown();
    server.shutdown().await;
}
