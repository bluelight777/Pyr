//! What you haven't read stays marked: a message in the conversation you have open counts as
//! read only while Pyr's window is in front of you. One that comes while you're in another
//! window (or Pyr is minimized or in the tray) gets the red badge and a "NEW" line, and the
//! line stays until you leave the conversation. Read markers survive a restart.

use std::time::{Duration, Instant};

use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-unread-{}-{}", name, pyr_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn client_in(dir: &std::path::Path, name: &str) -> Engine {
    if !dir.join("config.json").exists() {
        let cfg = pyr_engine::config::Config { display_name: name.to_string(), ..Default::default() };
        pyr_engine::config::save_config(dir, &cfg).unwrap();
    }
    Engine::start(EngineOptions { config_dir: Some(dir.to_path_buf()), push_to_talk: false, ..Default::default() }).unwrap()
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

fn has(e: &Engine, dm: i64, text: &str) -> bool {
    e.state().servers[0].logs.get(&dm).map(|l| l.messages.iter().any(|m| m.content == text)).unwrap_or(false)
}

fn id_of(e: &Engine, dm: i64, text: &str) -> i64 {
    e.state().servers[0].logs[&dm].messages.iter().find(|m| m.content == text).unwrap().id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unread_while_away_from_the_window() {
    let data = temp_dir("server");
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: data.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Unread Camp".into()),
    })
    .await
    .unwrap();
    let alice_dir = temp_dir("alice");
    let a = client_in(&alice_dir, "Alice");
    let b = client_in(&temp_dir("bob"), "Bob");
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice connected", 10, || a.state().servers[0].status == ConnStatus::Connected).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("bob connected", 10, || b.state().servers[0].status == ConnStatus::Connected).await;
    let key = a.state().servers[0].key.clone();
    let bob_id = b.state().my_uid.clone();

    a.open_dm(&key, &bob_id);
    let mut opened = None;
    wait_for("dm opened", 5, || {
        opened = opened.take().or_else(|| a.take_opened_dm());
        opened.is_some()
    })
    .await;
    let dm = opened.unwrap().1;
    wait_for("bob has it", 5, || b.state().servers[0].dm(dm).is_some()).await;

    // Alice has the conversation open, in front of her: Bob's message is read as it comes.
    a.set_window_focused(true);
    a.set_active_channel(&key, dm);
    b.send_message(&key, dm, "one", None);
    wait_for("alice gets one", 5, || has(&a, dm, "one")).await;
    {
        let st = a.state();
        let s = &st.servers[0];
        assert!(!s.unread.contains(&dm) && !s.mentions.contains_key(&dm), "read while looking");
        assert!(!s.new_since.contains_key(&dm), "no NEW line for what you saw arrive");
    }

    // Alice switches to another window. The conversation is still open behind it, but what
    // comes now is unread: red badge, toast with who it's from, and the NEW line.
    a.set_window_focused(false);
    b.send_message(&key, dm, "two", None);
    wait_for("alice gets two", 5, || has(&a, dm, "two")).await;
    let two = id_of(&a, dm, "two");
    {
        let st = a.state();
        let s = &st.servers[0];
        assert!(s.unread.contains(&dm), "unread while she's in another window");
        assert_eq!(s.mentions.get(&dm), Some(&1), "the red badge");
        assert_eq!(s.new_since.get(&dm), Some(&two), "the NEW line goes above it");
        assert!(st.toasts.iter().any(|t| t.text.contains("Bob: two")), "says who it's from");
    }
    // The app keeps drawing the conversation behind the other window: that doesn't read it.
    a.set_active_channel(&key, dm);
    assert!(a.state().servers[0].unread.contains(&dm), "not read until she's back");

    // A second one adds to the badge; the line stays above the first.
    b.send_message(&key, dm, "three", None);
    wait_for("alice gets three", 5, || has(&a, dm, "three")).await;
    assert_eq!(a.state().servers[0].mentions.get(&dm), Some(&2));
    assert_eq!(a.state().servers[0].new_since.get(&dm), Some(&two));

    // Back to Pyr: the badge goes, the NEW line stays while the conversation is open.
    a.set_window_focused(true);
    a.set_active_channel(&key, dm);
    {
        let st = a.state();
        let s = &st.servers[0];
        assert!(!s.unread.contains(&dm) && !s.mentions.contains_key(&dm), "read now she's back");
        assert_eq!(s.new_since.get(&dm), Some(&two), "the NEW line stays");
    }

    // Away again, still in the conversation: the line moves down to the next one she hasn't
    // seen (she read the ones above it).
    a.set_window_focused(false);
    b.send_message(&key, dm, "three and a half", None);
    wait_for("alice gets three and a half", 5, || has(&a, dm, "three and a half")).await;
    let half = id_of(&a, dm, "three and a half");
    assert_eq!(a.state().servers[0].new_since.get(&dm), Some(&half));
    a.set_window_focused(true);
    a.set_active_channel(&key, dm);
    assert!(!a.state().servers[0].unread.contains(&dm));

    // Leaving the conversation takes the line away.
    a.clear_active_channel();
    assert!(!a.state().servers[0].new_since.contains_key(&dm));

    // Not looking at the conversation at all (the friends list), window in front: unread.
    b.send_message(&key, dm, "four", None);
    wait_for("alice gets four", 5, || has(&a, dm, "four")).await;
    let four = id_of(&a, dm, "four");
    assert_eq!(a.state().servers[0].mentions.get(&dm), Some(&1));
    assert_eq!(a.state().servers[0].new_since.get(&dm), Some(&four));
    // She opens it, and answers: no NEW line above her own reply.
    a.set_active_channel(&key, dm);
    assert!(!a.state().servers[0].unread.contains(&dm));
    a.send_message(&key, dm, "hi bob", None);
    wait_for("her reply", 5, || has(&a, dm, "hi bob")).await;
    assert!(!a.state().servers[0].new_since.contains_key(&dm), "writing in it clears the line");

    // The window's own word wins over the last frame's: hidden in the tray, the app runs no
    // frames, so the engine asks the system (the probe) instead.
    a.set_focus_probe(|| Some(false));
    b.send_message(&key, dm, "five", None);
    wait_for("alice gets five", 5, || has(&a, dm, "five")).await;
    assert!(a.state().servers[0].unread.contains(&dm), "hidden in the tray: unread, though the last frame was in front");
    a.set_focus_probe(|| Some(true));
    a.set_active_channel(&key, dm);
    assert!(!a.state().servers[0].unread.contains(&dm));

    // While she's looking, Bob writes again; then Alice restarts. Everything she saw stays read
    // (it used to come back as unread after a restart or a server update).
    b.send_message(&key, dm, "six", None);
    wait_for("alice gets six", 5, || has(&a, dm, "six")).await;
    assert!(!a.state().servers[0].unread.contains(&dm));
    let six = id_of(&a, dm, "six");
    // Quitting (as the tray's Quit does): saves and closes the connection.
    a.shutdown();
    drop(a);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let a = client_in(&alice_dir, "Alice");
    wait_for("alice back", 10, || a.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    assert!(!a.state().servers[0].unread.contains(&dm), "read messages stay read after a restart");

    // But one that came while she was gone is unread, with its NEW line, after the restart.
    a.shutdown();
    drop(a);
    tokio::time::sleep(Duration::from_millis(500)).await;
    b.send_message(&key, dm, "seven", None);
    wait_for("sent seven", 5, || has(&b, dm, "seven")).await;
    let seven = id_of(&b, dm, "seven");
    let a = client_in(&alice_dir, "Alice");
    wait_for("alice back again", 10, || a.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
    {
        let st = a.state();
        let s = &st.servers[0];
        assert!(s.unread.contains(&dm), "came while she was away");
        let since = s.new_since.get(&dm).copied();
        assert!(since.map(|x| x > six && x <= seven).unwrap_or(false), "NEW line right above it: {since:?} ({six}, {seven}]");
    }
    drop(a);
    server.shutdown().await;
}

fn channel_named(e: &Engine, name: &str) -> i64 {
    e.state().servers[0].channels.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("no #{name}")).id
}

fn connected(e: &Engine) -> bool {
    e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)
}

/// Channels too, not just direct messages: what comes in while you're elsewhere stays unread
/// (with its NEW line) through a restart, and what came while Pyr was closed shows as unread
/// when it's back: in text channels and in voice channels' chats. Muted channels stay quiet,
/// your own messages aren't unread, and a channel this PC has never seen starts out read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn channels_keep_unread_across_restarts() {
    let data = temp_dir("server2");
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: data.clone(),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Unread Camp".into()),
    })
    .await
    .unwrap();
    let alice_dir = temp_dir("alice2");
    let a = client_in(&alice_dir, "Alice");
    let b = client_in(&temp_dir("bob2"), "Bob");
    a.add_server(&server.owner_link().unwrap()).unwrap();
    wait_for("alice connected", 10, || connected(&a)).await;
    b.add_server(&server.invite_link()).unwrap();
    wait_for("bob connected", 10, || connected(&b)).await;
    let key = a.state().servers[0].key.clone();
    let general = channel_named(&a, "General");
    let clips = channel_named(&a, "Links and clips");
    let lounge = channel_named(&a, "Lounge");

    // Alice is in #clips, in front of it; Bob writes in #general.
    a.set_window_focused(true);
    a.set_active_channel(&key, clips);
    b.send_message(&key, general, "hello camp", None);
    wait_for("alice gets it", 5, || has(&a, general, "hello camp")).await;
    let hello = id_of(&a, general, "hello camp");
    assert!(a.state().servers[0].unread.contains(&general));
    assert_eq!(a.state().servers[0].new_since.get(&general), Some(&hello));

    // She restarts before reading it: still unread, NEW line above it.
    a.shutdown();
    drop(a);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let a = client_in(&alice_dir, "Alice");
    wait_for("alice back", 10, || connected(&a)).await;
    {
        let st = a.state();
        let s = &st.servers[0];
        assert!(s.unread.contains(&general), "unread after a restart");
        assert_eq!(s.new_since.get(&general), Some(&hello));
        assert!(!s.unread.contains(&clips) && !s.unread.contains(&lounge));
    }

    // She reads it (the app opens it and loads its messages), then quits. Bob writes in
    // #general and in the Lounge's chat while she's gone.
    a.set_window_focused(true);
    a.set_active_channel(&key, general);
    a.load_history(&key, general, false);
    wait_for("history", 5, || has(&a, general, "hello camp")).await;
    wait_for("read mark", 5, || a.shared.config.lock().last_read.get(&key).and_then(|m| m.get(&general)).copied() == Some(hello)).await;
    assert!(!a.state().servers[0].unread.contains(&general));
    a.shutdown();
    drop(a);
    tokio::time::sleep(Duration::from_millis(500)).await;
    b.send_message(&key, general, "while you were out", None);
    b.send_message(&key, lounge, "in the lounge chat", None);
    wait_for("bob sent both", 5, || has(&b, general, "while you were out") && has(&b, lounge, "in the lounge chat")).await;
    let out = id_of(&b, general, "while you were out");
    let a = client_in(&alice_dir, "Alice");
    wait_for("alice back", 10, || connected(&a)).await;
    {
        let st = a.state();
        let s = &st.servers[0];
        assert!(s.unread.contains(&general), "what came while she was gone is unread");
        let since = s.new_since.get(&general).copied();
        assert!(since.is_some_and(|x| x > hello && x <= out), "NEW line right above it: {since:?} ({hello}, {out}]");
        assert!(s.unread.contains(&lounge), "a voice channel's chat too");
        assert!(!s.unread.contains(&clips), "nothing new there");
    }

    // A muted channel stays quiet; her own last message isn't unread.
    a.set_muted(&key, Some(clips), true);
    a.set_window_focused(true);
    a.set_active_channel(&key, lounge);
    a.send_message(&key, lounge, "back now", None);
    wait_for("her message", 5, || has(&a, lounge, "back now")).await;
    a.clear_active_channel();
    let a = {
        a.shutdown();
        drop(a);
        tokio::time::sleep(Duration::from_millis(500)).await;
        b.send_message(&key, clips, "a clip", None);
        wait_for("bob's clip", 5, || has(&b, clips, "a clip")).await;
        let a = client_in(&alice_dir, "Alice");
        wait_for("alice back", 10, || connected(&a)).await;
        a
    };
    {
        let st = a.state();
        let s = &st.servers[0];
        assert!(!s.unread.contains(&clips), "muted: no unread");
        assert!(!s.unread.contains(&lounge), "her own message last: read");
    }

    // Someone new (no marks on this PC yet): the server's history doesn't all show as unread.
    let c = client_in(&temp_dir("carol"), "Carol");
    c.add_server(&server.invite_link()).unwrap();
    wait_for("carol connected", 10, || connected(&c)).await;
    assert!(c.state().servers[0].unread.is_empty(), "a first visit starts out read: {:?}", c.state().servers[0].unread);
    drop(a);
    drop(c);
    server.shutdown().await;
}
