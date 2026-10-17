//! Calls in direct messages, end to end, like Discord's: a real server and three headless
//! clients (each "microphone" a tone of its own). Calling rings the other person (and only
//! them); declining tells the caller; answering connects the two, end-to-end encrypted; nobody
//! else sees the call or can get into it; a moderator's server mute doesn't reach it; hanging
//! up before an answer is a missed call; and blocking ends it.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use pyr_engine::audio::{AudioInput, AudioOutput};
use pyr_engine::config::{NoiseSuppression, Settings};
use pyr_engine::state::{ConnStatus, RingState};
use pyr_engine::{Engine, EngineOptions};
use pyr_proto::control::{ChannelId, ChannelKind, DM_CALLS_FEATURE};
use parking_lot::Mutex;

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-dmcalls-{}-{}", name, pyr_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A copy of Pyr with a tone for a microphone, recording what it hears.
fn bot(dir: &Path, name: &str, tone_hz: f32, heard: Arc<Mutex<Vec<f32>>>) -> Engine {
    let cfg = pyr_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    pyr_engine::config::save_config(dir, &cfg).unwrap();
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
            for i in 0..480 {
                h.push(stereo[2 * i]);
            }
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
    s.vad_auto = false;
    s.vad_threshold_db = -40.0;
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

fn loud(v: &[f32]) -> bool {
    v.iter().map(|x| x.abs()).fold(0.0, f32::max) > 0.05
}

async fn hears(heard: &Arc<Mutex<Vec<f32>>>, what: &str) {
    heard.lock().clear();
    wait_for(what, 10, || loud(&heard.lock())).await;
}

fn toasts(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

async fn connected(e: &Engine, what: &str) {
    wait_for(what, 15, || e.state().servers.first().map(|s| s.status == ConnStatus::Connected).unwrap_or(false)).await;
}

/// The direct message between `e` and `with`, opened from `e`'s side.
async fn dm_with(e: &Engine, key: &str, with: &str) -> ChannelId {
    e.open_dm(key, with);
    let mut opened = None;
    wait_for("conversation opened", 10, || {
        opened = opened.take().or_else(|| e.take_opened_dm());
        opened.is_some()
    })
    .await;
    opened.unwrap().1
}

fn in_call(e: &Engine, channel: ChannelId) -> bool {
    e.state().call.as_ref().is_some_and(|c| c.channel_id == channel)
}

fn rings(e: &Engine, channel: ChannelId) -> bool {
    e.state().incoming_calls.iter().any(|c| c.channel_id == channel)
}

/// Anyone `seer` sees in voice in `channel`.
fn sees_call(seer: &Engine, channel: ChannelId) -> bool {
    seer.state().servers[0].voice.values().any(|p| p.channel_id == channel)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn friends_call_each_other_privately() {
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Call Camp".into()),
    })
    .await
    .unwrap();
    let (ha, hb, hc) = (Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())), Arc::new(Mutex::new(Vec::new())));
    let a = bot(&temp_dir("alice"), "Alice", 440.0, ha.clone());
    let b = bot(&temp_dir("bob"), "Bob", 660.0, hb.clone());
    let c = bot(&temp_dir("carol"), "Carol", 880.0, hc.clone());
    a.add_server(&server.owner_link().unwrap()).unwrap();
    connected(&a, "alice connected").await;
    for (e, what) in [(&b, "bob connected"), (&c, "carol connected")] {
        e.add_server(&server.invite_link()).unwrap();
        connected(e, what).await;
    }
    let key = a.state().servers[0].key.clone();
    assert!(a.state().servers[0].has(DM_CALLS_FEATURE));
    let (alice, bob, carol) = (a.state().my_uid.clone(), b.state().my_uid.clone(), c.state().my_uid.clone());
    let dm = dm_with(&a, &key, &bob).await;
    wait_for("bob knows the conversation", 10, || b.state().servers[0].dm(dm).is_some()).await;

    // ---- Alice calls Bob: she's in the call, it rings for Bob (only).
    a.start_dm_call(&key, dm, false);
    wait_for("alice in the call", 10, || in_call(&a, dm)).await;
    wait_for("it rings for bob", 10, || rings(&b, dm)).await;
    assert_eq!(b.state().incoming_calls[0].from, alice);
    assert_eq!(a.state().outgoing_call.as_ref().map(|o| o.state), Some(RingState::Ringing));
    wait_for("bob sees alice waiting in the call", 10, || sees_call(&b, dm)).await;
    // Carol neither rings nor sees it.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(c.state().incoming_calls.is_empty(), "carol rings");
    assert!(!sees_call(&c, dm), "carol sees someone else's call");

    // ---- Bob declines: Alice sees it; nothing rings any more.
    b.decline_call(&key, dm);
    wait_for("alice sees bob declined", 10, || a.state().outgoing_call.as_ref().is_some_and(|o| o.state == RingState::Declined)).await;
    assert!(!rings(&b, dm));

    // ---- Alice rings again, and Bob answers: the two of them talk, end-to-end encrypted.
    a.ring_again();
    wait_for("it rings for bob again", 10, || rings(&b, dm)).await;
    b.answer_call(&key, dm, false);
    wait_for("bob in the call", 10, || in_call(&b, dm)).await;
    wait_for("no longer ringing either side", 10, || !rings(&b, dm) && a.state().outgoing_call.is_none()).await;
    wait_for("keys exchanged", 15, || {
        a.state().call.as_ref().is_some_and(|c| c.e2ee_ready) && b.state().call.as_ref().is_some_and(|c| c.e2ee_ready)
    })
    .await;
    assert_eq!(a.state().call.as_ref().unwrap().call_code, b.state().call.as_ref().unwrap().call_code);
    hears(&hb, "bob hears alice").await;
    hears(&ha, "alice hears bob").await;
    assert!(!sees_call(&c, dm), "carol still doesn't see the call");

    // ---- Carol can't get in.
    c.join_voice(&key, dm);
    wait_for("carol refused", 10, || toasts(&c).contains("No such voice channel")).await;
    assert!(c.state().call.is_none());

    // ---- A moderator's server mute doesn't reach a direct message's call.
    a.server_mute(&key, &bob, true);
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(a.state().servers[0].voice.values().any(|p| p.user_id == bob && p.channel_id == dm && !p.server_mute));
    hears(&ha, "alice still hears bob").await;
    a.server_mute(&key, &bob, false);

    // ---- Bob hangs up: Alice is alone in it; she leaves too.
    b.leave_voice();
    wait_for("bob gone from alice's call", 10, || !a.state().servers[0].voice.values().any(|p| p.user_id == bob)).await;
    a.leave_voice();
    wait_for("alice out", 10, || a.state().call.is_none()).await;

    // ---- Carol calls Bob and hangs up before he answers: he missed a call.
    let dm_cb = dm_with(&c, &key, &bob).await;
    c.start_dm_call(&key, dm_cb, false);
    wait_for("it rings for bob (carol)", 10, || rings(&b, dm_cb)).await;
    assert!(!rings(&a, dm_cb) && !sees_call(&a, dm_cb), "alice sees carol and bob's call");
    c.leave_voice();
    wait_for("bob missed it", 10, || !rings(&b, dm_cb) && toasts(&b).contains("missed a call from Carol")).await;
    assert_eq!(b.state().missed_calls, 1);

    // ---- Blocking ends a call.
    a.start_dm_call(&key, dm, false);
    wait_for("rings for bob once more", 10, || rings(&b, dm)).await;
    b.answer_call(&key, dm, false);
    wait_for("both in the call again", 10, || in_call(&a, dm) && in_call(&b, dm)).await;
    b.block(&alice);
    wait_for("the call ended for both", 10, || a.state().call.is_none() && b.state().call.is_none()).await;
    assert!(toasts(&a).contains("The call ended"), "{}", toasts(&a));
    // And she can't call him now.
    a.start_dm_call(&key, dm, false);
    wait_for("alice refused", 10, || toasts(&a).contains("can't call this person")).await;
    assert!(a.state().outgoing_call.is_none(), "nobody to ring");
    let _ = (carol, ChannelKind::Text);
}

/// Group calls, like Discord's: adding people to a call in a direct message makes a group and
/// moves the call there (the other person comes along); the people added ring; only the
/// group's members see it, its messages or its call; people can be added into a call going on;
/// and leaving the group leaves its call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn adding_people_to_a_call_makes_a_group() {
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: temp_dir("gserver"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Group Camp".into()),
    })
    .await
    .unwrap();
    let heard: Vec<Arc<Mutex<Vec<f32>>>> = (0..4).map(|_| Arc::new(Mutex::new(Vec::new()))).collect();
    let a = bot(&temp_dir("galice"), "Alice", 440.0, heard[0].clone());
    let b = bot(&temp_dir("gbob"), "Bob", 550.0, heard[1].clone());
    let c = bot(&temp_dir("gcarol"), "Carol", 660.0, heard[2].clone());
    let d = bot(&temp_dir("gdave"), "Dave", 770.0, heard[3].clone());
    a.add_server(&server.owner_link().unwrap()).unwrap();
    connected(&a, "alice connected").await;
    for (e, what) in [(&b, "bob connected"), (&c, "carol connected"), (&d, "dave connected")] {
        e.add_server(&server.invite_link()).unwrap();
        connected(e, what).await;
    }
    let key = a.state().servers[0].key.clone();
    assert!(a.state().servers[0].has(pyr_proto::control::GROUPS_FEATURE));
    let (bob, carol, dave) = (b.state().my_uid.clone(), c.state().my_uid.clone(), d.state().my_uid.clone());

    // Alice and Bob in a call in their direct message.
    let dm = dm_with(&a, &key, &bob).await;
    wait_for("bob knows the conversation", 10, || b.state().servers[0].dm(dm).is_some()).await;
    a.start_dm_call(&key, dm, false);
    wait_for("it rings for bob", 10, || rings(&b, dm)).await;
    b.answer_call(&key, dm, false);
    wait_for("both in the call", 10, || in_call(&a, dm) && in_call(&b, dm)).await;

    // ---- Alice adds Carol: a group of three, and the call moves there.
    a.create_group(&key, vec![bob.clone(), carol.clone()], Some(false), Some(dm));
    let mut opened = None;
    wait_for("the group opened for alice", 10, || {
        opened = opened.take().or_else(|| a.take_opened_dm());
        opened.is_some()
    })
    .await;
    let group = opened.unwrap().1;
    // (The app does this every frame.)
    a.take_group_call();
    wait_for("alice in the group's call", 10, || in_call(&a, group)).await;
    wait_for("bob came along", 10, || in_call(&b, group)).await;
    wait_for("it rings for carol", 10, || rings(&c, group)).await;
    assert!(!rings(&b, group), "bob was moved, not rung");
    // Bob's there, so (for Alice) it rings on just for Carol.
    wait_for("alice rings on for carol", 10, || a.state().outgoing_call.as_ref().is_some_and(|o| o.to == [carol.clone()] && o.state == RingState::Ringing)).await;
    for e in [&b, &c] {
        let g = e.state().servers[0].group(group).cloned().expect("the group, for its members");
        assert_eq!(g.members.len(), 3);
    }
    // Dave doesn't see the group, its call, or a ring.
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(d.state().servers[0].group(group).is_none() && !sees_call(&d, group) && d.state().incoming_calls.is_empty());

    // ---- Carol answers: the three of them talk.
    c.answer_call(&key, group, false);
    wait_for("carol in the call", 10, || in_call(&c, group)).await;
    wait_for("nobody left to ring", 10, || a.state().outgoing_call.is_none()).await;
    wait_for("keys for all three", 15, || [&a, &b, &c].iter().all(|e| e.state().call.as_ref().is_some_and(|x| x.e2ee_ready))).await;
    hears(&heard[2], "carol hears them").await;
    hears(&heard[0], "alice hears them").await;

    // ---- Messages in the group: its members get them, Dave doesn't.
    a.send_message(&key, group, "hello group", None);
    for (e, what) in [(&b, "bob"), (&c, "carol")] {
        wait_for(what, 10, || e.state().servers[0].logs.get(&group).is_some_and(|l| l.messages.iter().any(|m| m.content == "hello group"))).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!d.state().servers[0].logs.contains_key(&group), "dave got the group's message");

    // ---- Bob adds Dave into the call: Dave rings, and answers.
    b.add_to_group(&key, group, vec![dave.clone()], true);
    wait_for("dave is in the group", 10, || d.state().servers[0].group(group).is_some()).await;
    wait_for("it rings for dave", 10, || rings(&d, group)).await;
    wait_for("dave sees the call going on", 10, || sees_call(&d, group)).await;
    d.answer_call(&key, group, false);
    wait_for("four in the call", 10, || in_call(&d, group) && a.state().servers[0].voice_in(group).len() == 4).await;

    // ---- Carol leaves the group: out of its call too, and it's gone from her list.
    c.leave_group(&key, group);
    wait_for("carol out", 10, || c.state().call.is_none() && c.state().servers[0].group(group).is_none()).await;
    wait_for("the rest see three", 10, || a.state().servers[0].group(group).is_some_and(|g| g.members.len() == 3 && !g.members.contains(&carol))).await;
    assert!(!a.state().servers[0].voice.values().any(|p| p.user_id == carol));

    // ---- Bob leaves while it's ringing for him: it stops ringing there (no missed call).
    for e in [&a, &b, &d] {
        e.leave_voice();
    }
    wait_for("nobody in the call", 10, || a.state().call.is_none() && a.state().servers[0].voice_in(group).is_empty()).await;
    a.start_dm_call(&key, group, false);
    wait_for("it rings for bob", 10, || rings(&b, group)).await;
    b.leave_group(&key, group);
    wait_for("not ringing for bob any more", 10, || !rings(&b, group) && b.state().servers[0].group(group).is_none()).await;
    assert_eq!(b.state().missed_calls, 0);
}
