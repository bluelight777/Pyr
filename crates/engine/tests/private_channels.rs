//! Private and read-only channels, like Discord's, end to end: a real server and three
//! headless clients. A private channel is seen (listed, read, written in, talked in) only by
//! the roles it's for, admins and the owner: nobody else is even told it's there. Handing out
//! or taking away the role shows or hides it right away (taking it away leaves its voice). A
//! read-only channel is read by everyone and written in only by the roles it names.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ingle_engine::audio::{AudioInput, AudioOutput};
use ingle_engine::config::{NoiseSuppression, Settings};
use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::{ChannelId, ChannelKind, PRIVATE_CHANNELS_FEATURE, RoleId};
use ingle_proto::now_ms;
use parking_lot::Mutex;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ingle-private-{}-{}", name, now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A copy of Ingle with a tone for a microphone.
fn bot(dir: &Path, name: &str, tone_hz: f32) -> Engine {
    let cfg = ingle_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    ingle_engine::config::save_config(dir, &cfg).unwrap();
    let heard = Arc::new(Mutex::new(Vec::<f32>::new()));
    let factory = move |_s: &Settings| {
        let mut phase = 0u64;
        let heard = heard.clone();
        let input = AudioInput::Synthetic(Box::new(move |buf: &mut [f32]| {
            for v in buf.iter_mut() {
                *v = (phase as f32 / 48_000.0 * tone_hz * std::f32::consts::TAU).sin() * 0.3;
                phase += 1;
            }
        }));
        let output = AudioOutput::Synthetic(Box::new(move |stereo: &[f32]| {
            let mut h = heard.lock();
            h.clear();
            h.extend_from_slice(&stereo[..2]);
        }));
        (input, output)
    };
    let engine = Engine::start(EngineOptions {
        config_dir: Some(dir.to_path_buf()),
        audio: Some(Box::new(factory)),
        push_to_talk: false,
        ..Default::default()
    })
    .unwrap();
    let mut s = engine.state().settings.clone();
    s.noise_suppression = NoiseSuppression::Off;
    s.echo_cancellation = false;
    s.auto_gain = false;
    s.sounds = false;
    engine.update_settings(s);
    engine
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

async fn connected(e: &Engine, what: &str) {
    wait_for(what, 15, || e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
}

fn channel_named(e: &Engine, name: &str) -> Option<ChannelId> {
    e.state().servers[0].channels.iter().find(|c| c.name == name).map(|c| c.id)
}

fn sees(e: &Engine, ch: ChannelId) -> bool {
    e.state().servers[0].channel(ch).is_some()
}

fn has_message(e: &Engine, ch: ChannelId, text: &str) -> bool {
    e.state().servers[0].logs.get(&ch).is_some_and(|l| l.messages.iter().any(|m| m.content == text))
}

fn failed_because(e: &Engine, ch: ChannelId, text: &str) -> Option<String> {
    e.state().servers[0].logs.get(&ch).and_then(|l| l.pending.iter().find(|p| p.content == text).and_then(|p| p.failed.clone()))
}

fn in_voice(e: &Engine, ch: ChannelId) -> bool {
    e.state().call.as_ref().is_some_and(|c| c.channel_id == ch)
}

fn toasts(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

fn my_roles(e: &Engine) -> Vec<RoleId> {
    e.state().servers[0].me.as_ref().map(|m| m.roles.clone()).unwrap_or_default()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn private_channels_like_discords() {
    let server = ingle_server::start(ingle_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Back Room".into()),
    })
    .await
    .unwrap();
    let a = bot(&temp_dir("alice"), "Alice", 440.0);
    let b = bot(&temp_dir("bob"), "Bob", 660.0);
    let c = bot(&temp_dir("carol"), "Carol", 550.0);
    a.add_server(&server.owner_link().unwrap()).unwrap();
    connected(&a, "alice connected").await;
    b.add_server(&server.invite_link()).unwrap();
    c.add_server(&server.invite_link()).unwrap();
    connected(&b, "bob connected").await;
    connected(&c, "carol connected").await;
    let key = a.state().servers[0].key.clone();
    assert!(a.state().servers[0].has(PRIVATE_CHANNELS_FEATURE));
    let (bob, carol) = (b.state().my_uid.clone(), c.state().my_uid.clone());
    wait_for("everyone sees everyone", 10, || [&a, &b, &c].iter().all(|e| e.state().servers[0].members.len() == 3)).await;

    // ---- A Mods role, for Bob.
    a.create_role(&key, "Mods", 0x3498db, 0, false);
    wait_for("the role", 10, || a.state().servers[0].roles.iter().any(|r| r.name == "Mods")).await;
    let mods = a.state().servers[0].roles.iter().find(|r| r.name == "Mods").unwrap().id;
    a.set_member_roles(&key, &bob, vec![mods]);
    wait_for("bob is a mod", 10, || my_roles(&b) == vec![mods]).await;

    // ---- A private text channel and a private voice channel, for Mods. Bob sees them; Carol
    // isn't told they're there.
    a.create_channel_with(&key, "mods-only", ChannelKind::Text, true, vec![mods]);
    a.create_channel_with(&key, "Mods Voice", ChannelKind::Voice, true, vec![mods]);
    wait_for("alice has them", 10, || channel_named(&a, "mods-only").is_some() && channel_named(&a, "Mods Voice").is_some()).await;
    let (secret, secret_voice) = (channel_named(&a, "mods-only").unwrap(), channel_named(&a, "Mods Voice").unwrap());
    {
        let st = a.state();
        let ch = st.servers[0].channel(secret).unwrap();
        assert!(ch.private && ch.view_roles == vec![mods], "made private: {ch:?}");
    }
    wait_for("bob has them", 10, || sees(&b, secret) && sees(&b, secret_voice)).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!sees(&c, secret) && !sees(&c, secret_voice), "carol isn't told about them");
    assert!(channel_named(&c, "mods-only").is_none());

    // Bob writes in it: Alice gets it, Carol doesn't.
    b.send_message(&key, secret, "for mods only", None);
    wait_for("alice reads it", 10, || has_message(&a, secret, "for mods only")).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!has_message(&c, secret, "for mods only"), "carol doesn't get it");
    assert!(!c.state().servers[0].unread.contains(&secret), "nor is it unread for her");

    // Carol can't write in it (she'd need its id: the server acts as if there's no such channel).
    c.send_message(&key, secret, "let me in", None);
    wait_for("carol's message refused", 10, || failed_because(&c, secret, "let me in").is_some()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!has_message(&a, secret, "let me in"));

    // Bob talks in the private voice channel: Alice sees him there, Carol doesn't, and Carol
    // can't get in.
    b.join_voice(&key, secret_voice);
    wait_for("bob in voice", 10, || in_voice(&b, secret_voice)).await;
    wait_for("alice sees bob in voice", 10, || !a.state().servers[0].voice_in(secret_voice).is_empty()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(c.state().servers[0].voice_in(secret_voice).is_empty(), "carol doesn't see who's talking in it");
    c.join_voice(&key, secret_voice);
    wait_for("carol refused", 10, || toasts(&c).contains("Couldn't join voice")).await;
    assert!(!in_voice(&c, secret_voice));

    // ---- A read-only channel: everyone reads it; only Mods (and admins) write in it.
    a.create_channel(&key, "announcements", ChannelKind::Text);
    wait_for("everyone has it", 10, || [&a, &b, &c].iter().all(|e| channel_named(e, "announcements").is_some())).await;
    let news = channel_named(&a, "announcements").unwrap();
    a.set_channel_access(&key, news, false, Vec::new(), true, vec![mods]);
    wait_for("it's read-only", 10, || {
        c.state().servers[0].channel(news).is_some_and(|ch| ch.read_only) && b.state().servers[0].channel(news).is_some_and(|ch| ch.read_only)
    })
    .await;
    assert!(a.state().servers[0].can_post(news), "admins always can");
    assert!(b.state().servers[0].can_post(news), "mods can");
    assert!(!c.state().servers[0].can_post(news), "carol can't");
    a.send_message(&key, news, "welcome", None);
    b.send_message(&key, news, "rules are pinned", None);
    wait_for("carol reads them", 10, || has_message(&c, news, "welcome") && has_message(&c, news, "rules are pinned")).await;
    c.send_message(&key, news, "first!", None);
    wait_for("carol's post refused", 10, || failed_because(&c, news, "first!").is_some_and(|e| e.contains("permission"))).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!has_message(&a, news, "first!"));

    // ---- Carol becomes a mod: the private channels show up for her (with what was said), and
    // she can write in the announcements.
    a.set_member_roles(&key, &carol, vec![mods]);
    wait_for("carol sees them now", 10, || sees(&c, secret) && sees(&c, secret_voice)).await;
    wait_for("and can post", 10, || c.state().servers[0].can_post(news)).await;
    c.load_history(&key, secret, false);
    wait_for("carol reads the history", 10, || has_message(&c, secret, "for mods only")).await;
    wait_for("and sees bob talking", 10, || !c.state().servers[0].voice_in(secret_voice).is_empty()).await;

    // ---- Bob's not a mod any more: they're gone for him, and he's out of their voice.
    a.set_member_roles(&key, &bob, Vec::new());
    wait_for("bob lost them", 10, || !sees(&b, secret) && !sees(&b, secret_voice)).await;
    wait_for("bob's out of voice", 10, || !in_voice(&b, secret_voice)).await;
    wait_for("nobody sees him there", 10, || {
        a.state().servers[0].voice_in(secret_voice).is_empty() && c.state().servers[0].voice_in(secret_voice).is_empty()
    })
    .await;
    assert!(!b.state().servers[0].can_post(news), "nor can he post announcements");
    b.send_message(&key, secret, "still here?", None);
    wait_for("bob refused", 10, || failed_because(&b, secret, "still here?").is_some()).await;

    // ---- Deleting the role: only admins see the private channels now.
    a.delete_role(&key, mods);
    wait_for("carol lost them", 10, || !sees(&c, secret) && !sees(&c, secret_voice)).await;
    assert!(sees(&a, secret), "the owner always sees everything");

    // ---- Making it public again: everyone sees it.
    a.set_channel_access(&key, secret, false, Vec::new(), false, Vec::new());
    wait_for("everyone sees it", 10, || sees(&b, secret) && sees(&c, secret)).await;
    assert!(!b.state().servers[0].channel(secret).unwrap().private);
    assert!(!sees(&b, secret_voice), "the other one's still private");
}
