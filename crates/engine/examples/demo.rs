//! A demo community, for screenshots (the README's): a server with channels, roles, an icon
//! and its own emoji, and four friends with profile pictures who've been chatting, reacting,
//! sending a picture, adding you as a friend and messaging you. Then, with `live`, they stay
//! online and sit in voice: one with a camera, one sharing a screen, one talking.
//!
//!   ingle-server --data /tmp/demo-server --name "Firelit Friends" --public-host 127.0.0.1 \
//!       --bind 127.0.0.1:7791 run
//!   cargo run -p ingle-engine --example demo -- setup --owner "<owner link>" \
//!       --invite "<invite link>" --me <app profile config folder> --assets <folder>
//!   cargo run -p ingle-engine --example demo -- live --invite "<invite link>" --assets <folder>
//!
//! `--me` is the config folder of the app profile the screenshots are taken with: `setup`
//! uses its identity to make the server as its owner ("Friend"), so run it before opening
//! that profile. `--assets` holds friend.png, mara.png, theo.png, rae.png, kai.png (profile
//! pictures), icon.png (the server's icon), campfire.png (an emoji) and lake.jpg (a photo).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use ingle_engine::audio::{AudioInput, AudioOutput};
use ingle_engine::config::{Config, NoiseSuppression, Settings};
use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::{ChannelId, ChannelKind, MessageId, perm};

const FRIENDS: [&str; 4] = ["Mara", "Theo", "Rae", "Kai"];

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter().position(|x| x == name).and_then(|i| a.get(i + 1).cloned())
}

/// Waits for `f` to give something.
fn wait<T>(what: &str, secs: u64, mut f: impl FnMut() -> Option<T>) -> Result<T> {
    let until = Instant::now() + Duration::from_secs(secs);
    loop {
        if let Some(v) = f() {
            return Ok(v);
        }
        if Instant::now() > until {
            bail!("timed out waiting for {what}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Someone, in `dir`; `voice`: their "microphone" talks with a tone now and then.
fn person(dir: &Path, name: &str, voice: Option<f32>) -> Result<Engine> {
    std::fs::create_dir_all(dir)?;
    let mut cfg = ingle_engine::config::load_config(dir);
    cfg.display_name = name.to_string();
    cfg.settings = Settings {
        noise_suppression: NoiseSuppression::Off,
        vad_auto: false,
        vad_threshold_db: -45.0,
        echo_cancellation: false,
        auto_gain: false,
        camera_device: Some("test:camera".into()),
        sounds: false,
        ..cfg.settings
    };
    ingle_engine::config::save_config(dir, &Config { ..cfg })?;
    let tone = voice.unwrap_or(0.0);
    Engine::start(EngineOptions {
        config_dir: Some(dir.to_path_buf()),
        push_to_talk: false,
        audio: Some(Box::new(move |_s: &Settings| {
            let mut n = 0u64;
            let input = AudioInput::Synthetic(Box::new(move |buf: &mut [f32]| {
                for v in buf.iter_mut() {
                    let t = n as f32 / 48_000.0;
                    let on = tone > 0.0 && (t % 4.0) < 2.2;
                    let f = tone * (1.0 + 0.03 * (t * 5.0).sin());
                    *v = if on { (t * f * std::f32::consts::TAU).sin() * 0.15 } else { 0.0 };
                    n += 1;
                }
            }));
            (input, AudioOutput::None)
        })),
        ..Default::default()
    })
}

/// Joins the server in `link` (or finds it, if already joined) and waits until connected.
fn join(e: &Engine, link: &str) -> Result<String> {
    let key = match e.add_server(link) {
        Ok(k) => k,
        Err(_) => e.state().servers.first().map(|s| s.key.clone()).ok_or_else(|| anyhow!("couldn't join"))?,
    };
    wait("connecting", 30, || e.state().server(&key).filter(|s| matches!(s.status, ConnStatus::Connected)).map(|_| ()))?;
    Ok(key)
}

fn channel(e: &Engine, key: &str, name: &str) -> Option<ChannelId> {
    e.state().server(key)?.channels.iter().find(|c| c.name.eq_ignore_ascii_case(name)).map(|c| c.id)
}

fn uid_of(e: &Engine, key: &str, name: &str) -> Option<String> {
    e.state().server(key)?.members.values().find(|m| m.name == name).map(|m| m.user_id.clone())
}

/// Says something in `ch` and returns the message once it's there.
fn say(e: &Engine, key: &str, ch: ChannelId, text: &str, reply_to: Option<MessageId>) -> Result<MessageId> {
    let me = e.state().my_uid.clone();
    let before = e.state().server(key).and_then(|s| s.logs.get(&ch)).map(|l| l.messages.len()).unwrap_or(0);
    e.send_message(key, ch, text, reply_to);
    let id = wait("the message", 15, || {
        let st = e.state();
        let log = st.server(key)?.logs.get(&ch)?;
        (log.messages.len() > before).then(|| log.messages.iter().rev().find(|m| m.author == me && m.content == text).map(|m| m.id)).flatten()
    })?;
    // (Servers limit how fast one person posts.)
    std::thread::sleep(Duration::from_millis(1300));
    Ok(id)
}

fn png(assets: &Path, name: &str) -> Result<Vec<u8>> {
    std::fs::read(assets.join(name)).with_context(|| format!("{}", assets.join(name).display()))
}

fn dir_of(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("ingle-demo-{}", name.to_lowercase()))
}

fn setup() -> Result<()> {
    let owner = arg("--owner").ok_or_else(|| anyhow!("--owner <owner link>"))?;
    let invite = arg("--invite").ok_or_else(|| anyhow!("--invite <invite link>"))?;
    let me_dir = PathBuf::from(arg("--me").ok_or_else(|| anyhow!("--me <app profile config folder>"))?);
    let assets = PathBuf::from(arg("--assets").ok_or_else(|| anyhow!("--assets <folder>"))?);

    // You ("Friend"), the owner: the server's look, channels, roles and emoji.
    let me = person(&me_dir, "Friend", None)?;
    me.set_avatar(Some(png(&assets, "friend.png")?));
    let key = join(&me, &owner)?;
    me.set_server_icon(&key, Some(png(&assets, "icon.png")?));
    me.add_emoji(&key, "campfire", png(&assets, "campfire.png")?, false);
    me.create_channel(&key, "patch-notes", ChannelKind::Text);
    if channel(&me, &key, "Gaming").is_none() {
        me.create_channel(&key, "Gaming", ChannelKind::Voice);
    }
    wait("channels", 10, || channel(&me, &key, "patch-notes").and(channel(&me, &key, "Gaming")))?;
    let general = channel(&me, &key, "general").ok_or_else(|| anyhow!("no #general"))?;
    me.update_channel(&key, general, None, Some("Say hi! Game night is Fridays at 8.".into()));
    me.create_role(&key, "Moderators", 0x6cc070, perm::MANAGE_MESSAGES | perm::KICK | perm::MANAGE_EMOJI, true);
    me.create_role(&key, "Crew", 0xffa94d, 0, false);
    let role = |name: &str| me.state().server(&key).and_then(|s| s.roles.iter().find(|r| r.name == name).map(|r| r.id));
    let (mods, crew) = wait("roles", 10, || role("Moderators").zip(role("Crew")))?;
    let campfire = wait("the emoji", 10, || {
        me.state().server(&key).and_then(|s| s.info.emojis.iter().find(|e| e.name == "campfire").map(|e| format!("<:campfire:{}>", e.id)))
    })?;

    // The friends, with their pictures.
    let mut friends = Vec::new();
    for name in FRIENDS {
        let e = person(&dir_of(name), name, None)?;
        e.set_avatar(Some(png(&assets, &format!("{}.png", name.to_lowercase()))?));
        let k = join(&e, &invite)?;
        friends.push((name, e, k));
    }
    let f = |n: &str| &friends.iter().find(|x| x.0 == n).expect("a friend").1;
    for n in FRIENDS {
        wait("everyone", 15, || uid_of(&me, &key, n))?;
    }
    me.set_member_roles(&key, &uid_of(&me, &key, "Mara").unwrap(), vec![mods]);
    for n in ["Theo", "Rae"] {
        me.set_member_roles(&key, &uid_of(&me, &key, n).unwrap(), vec![crew]);
    }
    let me_uid = me.state().my_uid.clone();
    let (mara, theo, rae, kai) = (f("Mara"), f("Theo"), f("Rae"), f("Kai"));

    // An evening in #general.
    say(mara, &key, general, "Game night Friday? I set up the **Gaming** voice channel 🎉", None)?;
    say(mara, &key, general, "Bracket is in #links-and-clips, sign up by Thursday", None)?;
    let sharp = say(theo, &key, general, "The screen share in here is *so* much sharper than Discord's free tier", None)?;
    for e in [mara, rae, &me] {
        e.react(&key, sharp, "🔥", true);
    }
    kai.react(&key, sharp, "💯", true);
    say(theo, &key, general, "Hosting is one command: `ingle-server --public-host play.example.com`", None)?;
    let before = rae.state().server(&key).and_then(|s| s.logs.get(&general)).map(|l| l.messages.len()).unwrap_or(0);
    rae.send_message_with(&key, general, "Sunset at the lake last weekend 🌅", None, vec![("lake.jpg".into(), png(&assets, "lake.jpg")?)]);
    let photo = wait("the photo", 30, || {
        let st = rae.state();
        let log = st.server(&key)?.logs.get(&general)?;
        (log.messages.len() > before).then(|| log.messages.iter().rev().find(|m| !m.attachments.is_empty()).map(|m| m.id)).flatten()
    })?;
    std::thread::sleep(Duration::from_millis(1300));
    me.react(&key, photo, &campfire, true);
    kai.react(&key, photo, &campfire, true);
    mara.react(&key, photo, "😍", true);
    say(kai, &key, general, "that sky though 😍 wallpaper material", Some(photo))?;
    let omw = say(rae, &key, general, "@Friend joining Lounge in 5, grabbing headphones", None)?;
    say(&me, &key, general, "On my way! Hosting tips are in the README :)", Some(omw))?;

    // Elsewhere, unread.
    if let Some(links) = channel(mara, &key, "links-and-clips") {
        say(mara, &key, links, "Friday bracket: reply here to sign up 👇", None)?;
    }
    if let Some(notes) = channel(theo, &key, "patch-notes") {
        say(theo, &key, notes, "@Friend 0.6.7 is out: channels open steady now, no more jumping", None)?;
    }

    // Friends: Rae and Kai (accepted), Theo (waiting for you), and Mara messages you.
    for e in [rae, kai, theo] {
        e.add_friend(&me_uid);
    }
    for n in ["Rae", "Kai"] {
        let uid = uid_of(&me, &key, n).unwrap();
        wait("a friend request", 10, || me.state().relation(&uid).map(|_| ()))?;
        me.accept_friend(&uid);
    }
    mara.open_dm(&key, &me_uid);
    let (_, dm) = wait("a conversation", 10, || mara.take_opened_dm())?;
    say(mara, &key, dm, "you still up for game night? 🎮", None)?;
    say(mara, &key, dm, "I'll bring the snacks this time", None)?;
    wait("Friend sees it", 10, || me.state().server(&key).and_then(|s| s.dm(dm)).map(|_| ()))?;
    say(&me, &key, dm, "yes! count me in 🍿", None)?;
    say(mara, &key, dm, "perfect, see you Friday", None)?;

    std::thread::sleep(Duration::from_secs(2));
    println!("demo: set up {key}");
    for (_, e, _) in &friends {
        e.shutdown();
    }
    me.shutdown();
    Ok(())
}

/// The friends online: Theo sharing his screen, Mara (with her camera on, given `--camera`)
/// and Rae talking, in Lounge; Kai just online.
fn live() -> Result<()> {
    let invite = arg("--invite").ok_or_else(|| anyhow!("--invite <invite link>"))?;
    let secs: u64 = arg("--seconds").and_then(|s| s.parse().ok()).unwrap_or(600);
    let mut people = Vec::new();
    for (name, tone) in [("Mara", None), ("Theo", None), ("Rae", Some(300.0)), ("Kai", None)] {
        let e = person(&dir_of(name), name, tone)?;
        let key = join(&e, &invite)?;
        people.push((name, e, key));
    }
    for (name, e, key) in &people {
        if *name == "Kai" {
            continue;
        }
        let lounge = channel(e, key, "Lounge").ok_or_else(|| anyhow!("no Lounge"))?;
        e.join_voice(key, lounge);
        wait("voice", 15, || e.state().call.as_ref().map(|_| ()))?;
        match *name {
            "Mara" if std::env::args().any(|a| a == "--camera") => e.start_camera(),
            "Theo" => {
                e.set_mute(true);
                // (720p30: what a small test machine keeps up with while it also draws the app.)
                let mut s = e.state().settings.clone();
                s.screen_preset = ingle_engine::config::ScreenPreset::Saver;
                e.update_settings(s);
                e.start_screen("test:pattern")
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    println!("demo: live for {secs} s");
    std::thread::sleep(Duration::from_secs(secs));
    Ok(())
}

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("setup") => setup(),
        Some("live") => live(),
        _ => bail!("usage: demo setup|live …"),
    }
}
