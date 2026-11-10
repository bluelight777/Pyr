//! The AFK channel, like Discord's, end to end: a real server and two headless Pyrs that never
//! talk, the server's clock looked ahead with `Hub::afk_check`. (0.13.28: Maanix came into a
//! voice channel with his status set to Idle, his Pyr connected well over an hour, and was moved
//! to AFK at once, "idle for 60 minutes".) Idle set by hand never moves anyone, however long;
//! an hour away does; and Pyr itself goes along only once it has seen that hour, so a server
//! that counts Idle as away (one from before 0.13.28) can't move someone who's there.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pyr_engine::audio::{AudioInput, AudioOutput};
use pyr_engine::config::{NoiseSuppression, Settings};
use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_proto::control::{AWAY_FEATURE, ChannelId, ChannelKind};

const MIN: Duration = Duration::from_secs(60);

fn temp_dir(name: &str) -> PathBuf {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("pyr-afk-{}-{}-{n}", name, pyr_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A copy of Pyr whose microphone is silent (so it never talks) and whose speaker goes nowhere.
fn quiet_pyr(dir: &Path, name: &str) -> Engine {
    let cfg = pyr_engine::config::Config { display_name: name.to_string(), ..Default::default() };
    pyr_engine::config::save_config(dir, &cfg).unwrap();
    let factory = move |_s: &Settings| {
        let input = AudioInput::Synthetic(Box::new(|buf: &mut [f32]| buf.fill(0.0)));
        let output = AudioOutput::Synthetic(Box::new(|_stereo: &[f32]| {}));
        (input, output)
    };
    let engine = Engine::start(EngineOptions { config_dir: Some(dir.to_path_buf()), audio: Some(Box::new(factory)), push_to_talk: false, ..Default::default() }).unwrap();
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
    wait_for(what, 15, || e.state().servers.first().is_some_and(|s| s.status == ConnStatus::Connected)).await;
}

fn toasts(e: &Engine) -> String {
    e.state().toasts.iter().map(|t| t.text.clone()).collect::<Vec<_>>().join(" | ")
}

/// The voice channel `e` is in, as its own Pyr has it.
fn in_call(e: &Engine) -> Option<ChannelId> {
    e.state().call.as_ref().map(|c| c.channel_id)
}

/// The voice channel `uid` is in, as `seer`'s Pyr hears from the server.
fn seen_in(seer: &Engine, uid: &str) -> Option<ChannelId> {
    seer.state().servers[0].voice.values().find(|p| p.user_id == uid).map(|p| p.channel_id)
}

/// The server looks for anyone away long enough as if it were `ahead` from now, and `e`
/// (`uid`) stays in `ch` a good while later, for itself and for the server (as `seer` hears),
/// and says nothing of a move: one it went along with would have taken milliseconds here.
async fn not_moved(server: &pyr_server::RunningServer, ahead: Duration, e: &Engine, seer: &Engine, uid: &str, ch: ChannelId, what: &str) {
    let said = toasts(e).matches("moved").count();
    server.hub.afk_check(Instant::now() + ahead);
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert_eq!(in_call(e), Some(ch), "{what}: still in the channel");
    assert_eq!(seen_in(seer, uid), Some(ch), "{what}: the server has them there too");
    assert_eq!(toasts(e).matches("moved").count(), said, "{what}: {}", toasts(e));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn idle_by_hand_never_moves_anyone_to_afk_and_an_hour_away_does() {
    let server = pyr_server::start(pyr_server::ServerConfig {
        data_dir: temp_dir("server"),
        bind: "127.0.0.1:0".parse().unwrap(),
        public_host: Some("127.0.0.1".into()),
        name: Some("Brethren".into()),
    })
    .await
    .unwrap();
    let a = quiet_pyr(&temp_dir("blue"), "Bluelight");
    let b = quiet_pyr(&temp_dir("maanix"), "Maanix");
    a.add_server(&server.owner_link().unwrap()).unwrap();
    connected(&a, "bluelight connected").await;
    b.add_server(&server.invite_link()).unwrap();
    connected(&b, "maanix connected").await;
    let key = a.state().servers[0].key.clone();
    let maanix = b.state().my_uid.clone();
    assert!(b.state().servers[0].has(AWAY_FEATURE));

    // The Tavern, and a voice channel called AFK: the AFK channel, after an hour, and everyone's
    // Pyr is told so as it's made.
    a.create_channel(&key, "The Tavern", ChannelKind::Voice);
    a.create_channel(&key, "AFK", ChannelKind::Voice);
    let chan = |name: &str| b.state().servers[0].channels.iter().find(|c| c.name == name).map(|c| c.id);
    wait_for("both channels", 10, || chan("The Tavern").is_some() && chan("AFK").is_some()).await;
    let (tavern, afk) = (chan("The Tavern").unwrap(), chan("AFK").unwrap());
    wait_for("maanix's pyr told the afk channel", 10, || {
        let info = &b.state().servers[0].info;
        info.afk_channel == Some(afk) && info.afk_minutes == 60
    })
    .await;

    // ---- Maanix sets Idle and comes into the Tavern: everyone sees his moon.
    b.set_idle_by_hand(true);
    wait_for("bluelight sees maanix's moon", 10, || a.state().servers[0].members.get(&maanix).is_some_and(|m| m.idle)).await;
    b.join_voice(&key, tavern);
    wait_for("maanix in the tavern", 10, || in_call(&b) == Some(tavern) && seen_in(&a, &maanix) == Some(tavern)).await;
    assert!(!server.hub.is_away(&maanix), "idle by hand isn't away");
    // Three hours on by the server's clock (before 0.13.28, an hour of Idle had him moved the
    // moment he came in): still in the Tavern.
    not_moved(&server, 180 * MIN, &b, &a, &maanix, tavern, "idle by hand, three hours on").await;

    // ---- Away ten minutes, still Idle: the moon stays, and the server hears he's away.
    b.as_if_away_for(10 * MIN);
    wait_for("the server hears maanix is away", 10, || server.hub.is_away(&maanix)).await;
    assert!(b.state().my_away && b.state().my_idle);
    assert!(a.state().servers[0].members.get(&maanix).is_some_and(|m| m.idle));
    // The server's clock an hour on, but Maanix's Pyr has seen ten quiet minutes: it stays put
    // (as it would when a server from before 0.13.28 counted his Idle as away).
    not_moved(&server, 61 * MIN, &b, &a, &maanix, tavern, "ten minutes away here").await;

    // ---- Back (a key or the mouse): the server hears at once; the moon stays (set by hand).
    b.note_activity();
    wait_for("the server hears maanix is back", 10, || !server.hub.is_away(&maanix)).await;
    assert!(!b.state().my_away && b.state().my_idle);
    not_moved(&server, 180 * MIN, &b, &a, &maanix, tavern, "back, idle by hand").await;

    // ---- Really away an hour: moved to AFK, and told why in plain words.
    b.as_if_away_for(61 * MIN);
    wait_for("the server hears maanix is away again", 10, || server.hub.is_away(&maanix)).await;
    server.hub.afk_check(Instant::now() + 61 * MIN);
    wait_for("maanix moved to afk", 10, || in_call(&b) == Some(afk) && seen_in(&a, &maanix) == Some(afk)).await;
    wait_for("told why", 5, || toasts(&b).contains("You were away for an hour (no keys, mouse or talking), so you were moved to AFK.")).await;

    // ---- Online (not Idle) and away an hour: the same.
    b.set_idle_by_hand(false);
    b.join_voice(&key, tavern);
    wait_for("maanix back in the tavern", 10, || in_call(&b) == Some(tavern) && seen_in(&a, &maanix) == Some(tavern)).await;
    wait_for("not away", 10, || !server.hub.is_away(&maanix)).await;
    wait_for("no moon", 10, || a.state().servers[0].members.get(&maanix).is_some_and(|m| !m.idle)).await;
    not_moved(&server, 180 * MIN, &b, &a, &maanix, tavern, "here, online").await;
    b.as_if_away_for(61 * MIN);
    wait_for("away: the moon", 10, || server.hub.is_away(&maanix) && a.state().servers[0].members.get(&maanix).is_some_and(|m| m.idle)).await;
    server.hub.afk_check(Instant::now() + 61 * MIN);
    wait_for("moved to afk again", 10, || in_call(&b) == Some(afk) && seen_in(&a, &maanix) == Some(afk)).await;

    a.shutdown();
    b.shutdown();
    server.shutdown().await;
}
