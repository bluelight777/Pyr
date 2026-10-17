//! A headless test participant. Handy for trying a call on your own:
//!
//!   cargo run --release -p pyr-engine --example bot -- --join "pyr://…" --name Mara \
//!       --voice Lounge --camera --screen --say "hi from the bot"
//!
//! The bot "talks" with a tone (on/off like speech), and can share a test-pattern
//! camera and screen (the screen share comes with a quiet test chord unless you pass
//! --no-sound). Ctrl+C to stop.
//!
//! `--friend NAME` sends that member a friend request.
//!
//! `--dm NAME` sends the `--say` lines as direct messages to that member instead, after
//! `--wait SECONDS` (time to switch windows), `--gap SECONDS` apart. `--image PATH` sends a
//! picture (in order with the `--say` lines).
//!
//! `--check-isolation` only loads voice isolation's model and times it on this PC, then exits.
//!
//! For measuring shares over slow or far-away networks (with the `pathsim` example between the
//! bot and the server): `--source motion|text` shares a synthetic screen (a busy moving picture,
//! or a page of text that scrolls now and then) whose frames carry the time they were made, and
//! `--preset saver|standard|smooth|sharp` picks the share's quality. `--beeps` makes the bot's
//! voice a short beep every 2 s of the clock. A watching bot with `--measure FILE` writes a line
//! for every new picture it sees (`v <time ms> <latency ms> <width> <height>`), for every beep it
//! hears (`a <time ms> <latency ms>`), and its call stats twice a second (`s ...`).
//! `--media-log FILE` writes the engine's media events (frames sent, acknowledged, decoded,
//! keyframe requests, the connection's round trip) there.

use std::time::{Duration, Instant};

use std::io::Write;
use std::sync::{Arc, Mutex};

use pyr_engine::audio::{AudioInput, AudioOutput};
use pyr_engine::config::{Config, NoiseSuppression, ScreenPreset, Settings};
use pyr_engine::video::testsrc;
use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_proto::control::ChannelKind;

fn main() -> anyhow::Result<()> {
    let mut join = None;
    let mut name = "Test Bot".to_string();
    let mut voice: Option<String> = None;
    let mut camera = false;
    let mut screen = false;
    // What to send, in order: (text, a picture to attach).
    let mut say: Vec<(String, Option<std::path::PathBuf>)> = Vec::new();
    let mut channel = "general".to_string();
    let mut tone = 330.0f32;
    let mut seconds: Option<u64> = None;
    let mut quiet = false;
    let mut watch = false;
    let mut stream_sound = true;
    let mut dm_to: Option<String> = None;
    let mut wait = 0.0f32;
    let mut gap = 0.9f32;
    // A profile picture (a square PNG or GIF) to wear.
    let mut avatar: Option<std::path::PathBuf> = None;
    // How loud its voice is (the tone's amplitude).
    let mut amp = 0.18f32;
    let mut source = pyr_engine::video::capture::TEST_PATTERN_ID.to_string();
    let mut preset = ScreenPreset::default();
    let mut measure: Option<std::path::PathBuf> = None;
    let mut media_log: Option<std::path::PathBuf> = None;
    let mut beeps = false;
    // Someone to send a friend request to (by name).
    let mut befriend: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--join" => join = args.next(),
            "--name" => name = args.next().unwrap_or(name),
            "--voice" => voice = args.next(),
            "--camera" => camera = true,
            "--screen" => screen = true,
            "--say" => say.extend(args.next().map(|t| (t, None))),
            "--image" => say.extend(args.next().map(|p| (String::new(), Some(std::path::PathBuf::from(p))))),
            "--channel" => channel = args.next().unwrap_or(channel),
            "--tone" => tone = args.next().and_then(|t| t.parse().ok()).unwrap_or(tone),
            "--seconds" => seconds = args.next().and_then(|t| t.parse().ok()),
            "--quiet" => quiet = true,
            "--watch" => watch = true,
            "--no-sound" => stream_sound = false,
            "--dm" => dm_to = args.next(),
            "--wait" => wait = args.next().and_then(|t| t.parse().ok()).unwrap_or(wait),
            "--gap" => gap = args.next().and_then(|t| t.parse().ok()).unwrap_or(gap),
            "--avatar" => avatar = args.next().map(std::path::PathBuf::from),
            "--amp" => amp = args.next().and_then(|t| t.parse().ok()).unwrap_or(amp),
            "--source" => {
                source = match args.next().as_deref() {
                    Some("motion") => testsrc::MOTION_ID.to_string(),
                    Some("text") => testsrc::TEXT_ID.to_string(),
                    _ => pyr_engine::video::capture::TEST_PATTERN_ID.to_string(),
                }
            }
            "--preset" => {
                preset = match args.next().as_deref() {
                    Some("saver") => ScreenPreset::Saver,
                    Some("standard") => ScreenPreset::Standard,
                    Some("sharp") => ScreenPreset::Sharp,
                    _ => ScreenPreset::Smooth,
                }
            }
            "--measure" => measure = args.next().map(std::path::PathBuf::from),
            "--media-log" => media_log = args.next().map(std::path::PathBuf::from),
            "--beeps" => beeps = true,
            "--friend" => befriend = args.next(),
            _ => {}
        }
    }
    if std::env::args().any(|a| a == "--check-isolation") {
        return check_isolation();
    }
    let join = join.ok_or_else(|| anyhow::anyhow!("--join <invite link> is required"))?;
    if let Some(p) = &media_log {
        let file = std::fs::File::create(p)?;
        tracing_subscriber::fmt()
            .with_writer(Mutex::new(file))
            .with_ansi(false)
            .with_env_filter(tracing_subscriber::EnvFilter::new("pyr_media=debug,warn"))
            .init();
    }
    // What a measuring bot writes (from the main loop and the sound thread).
    let out: Option<Arc<Mutex<std::fs::File>>> = match &measure {
        Some(p) => Some(Arc::new(Mutex::new(std::fs::File::create(p)?))),
        None => None,
    };
    let dir = std::env::temp_dir().join(format!("pyr-bot-{}", name.to_lowercase().replace(' ', "-")));
    std::fs::create_dir_all(&dir)?;
    let mut cfg = pyr_engine::config::load_config(&dir);
    cfg.display_name = name.clone();
    cfg.settings = Settings {
        noise_suppression: NoiseSuppression::Off,
        vad_auto: false,
        vad_threshold_db: -45.0,
        echo_cancellation: false,
        auto_gain: false,
        camera_device: Some("test:camera".into()),
        share_audio: stream_sound,
        screen_preset: preset,
        ..Default::default()
    };
    pyr_engine::config::save_config(&dir, &Config { ..cfg })?;

    let out_for_sound = out.clone();
    let engine = Engine::start(EngineOptions {
        config_dir: Some(dir),
        push_to_talk: false,
        audio: Some(Box::new(move |_s: &Settings| {
            let mut n = 0u64;
            let heard = out_for_sound.clone();
            let input = AudioInput::Synthetic(Box::new(move |buf: &mut [f32]| {
                if beeps {
                    // A 60 ms beep at every even 2 s of the clock (this 10 ms just ended).
                    let start = wall_ms() - 10.0;
                    for (i, v) in buf.iter_mut().enumerate() {
                        let t = start + i as f64 / 48.0;
                        let into = t.rem_euclid(BEEP_EVERY_MS);
                        *v = if into < 60.0 { ((into / 1000.0 * BEEP_HZ * std::f64::consts::TAU).sin() * 0.3) as f32 } else { 0.0 };
                    }
                    return;
                }
                for v in buf.iter_mut() {
                    // talk for ~1.6 s, pause ~1.1 s, with a little vibrato so it sounds less robotic
                    let t = n as f32 / 48_000.0;
                    let on = !quiet && (t % 2.7) < 1.6;
                    let f = tone * (1.0 + 0.03 * (t * 5.0).sin());
                    *v = if on { (t * f * std::f32::consts::TAU).sin() * amp } else { 0.0 };
                    n += 1;
                }
            }));
            // A measuring bot listens for the beeps: when each one starts, against when it was
            // made.
            let output = match heard {
                Some(f) => {
                    let mut last_loud = 0.0f64;
                    // (The beep's pitch picked out of the mix, 2 ms at a time, so a shared
                    // screen's sound playing along doesn't count.)
                    let coeff = 2.0 * (std::f64::consts::TAU * BEEP_HZ / 48_000.0).cos();
                    AudioOutput::Synthetic(Box::new(move |buf: &[f32]| {
                        let start = wall_ms();
                        let frames = buf.len() / 2;
                        for w in 0..frames / 96 {
                            let (mut s1, mut s2) = (0.0f64, 0.0f64);
                            for i in w * 96..(w + 1) * 96 {
                                let s0 = buf[2 * i] as f64 + coeff * s1 - s2;
                                s2 = s1;
                                s1 = s0;
                            }
                            let amp = 2.0 * (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt() / 96.0;
                            if amp > 0.1 {
                                let t = start + (w * 96) as f64 / 48.0;
                                if t - last_loud > 500.0 {
                                    let late = t.rem_euclid(BEEP_EVERY_MS);
                                    let _ = writeln!(f.lock().unwrap(), "a {:.1} {:.1}", t, late);
                                }
                                last_loud = t;
                            }
                        }
                    }))
                }
                None => AudioOutput::None,
            };
            (input, output)
        })),
        ..Default::default()
    })?;
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let key = engine.add_server(&join)?;
    let start = Instant::now();
    loop {
        let status = engine.state().server(&key).map(|s| s.status.clone());
        match status {
            Some(ConnStatus::Connected) => break,
            Some(ConnStatus::Failed(e)) => anyhow::bail!("couldn't join: {e}"),
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(20) {
            anyhow::bail!("timed out connecting");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("{name}: connected");
    if let Some(p) = &avatar {
        engine.set_avatar(Some(std::fs::read(p)?));
    }
    if let Some(who) = &befriend {
        let uid = engine
            .state()
            .server(&key)
            .and_then(|s| s.members.values().find(|m| m.name.eq_ignore_ascii_case(who)).map(|m| m.user_id.clone()))
            .ok_or_else(|| anyhow::anyhow!("nobody called {who} here"))?;
        engine.add_friend(&uid);
        println!("{name}: sent {who} a friend request");
    }
    let find = |kind: ChannelKind, n: &str| {
        engine.state().server(&key).and_then(|s| {
            s.channels.iter().find(|c| c.kind == kind && c.name.eq_ignore_ascii_case(n)).map(|c| c.id)
        })
    };
    let target = match &dm_to {
        Some(who) => {
            let uid = engine
                .state()
                .server(&key)
                .and_then(|s| s.members.values().find(|m| m.name.eq_ignore_ascii_case(who)).map(|m| m.user_id.clone()))
                .ok_or_else(|| anyhow::anyhow!("nobody called {who} here"))?;
            engine.open_dm(&key, &uid);
            let t = Instant::now();
            loop {
                if let Some((_, id)) = engine.take_opened_dm() {
                    break Some(id);
                }
                if t.elapsed() > Duration::from_secs(10) {
                    anyhow::bail!("couldn't open a direct message with {who}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        None => find(ChannelKind::Text, &channel).or_else(|| find(ChannelKind::Voice, &channel)),
    };
    if let Some(ch) = target {
        std::thread::sleep(Duration::from_secs_f32(wait));
        for (line, image) in &say {
            match image {
                Some(p) => {
                    let file = (p.file_name().and_then(|n| n.to_str()).unwrap_or("image.png").to_string(), std::fs::read(p)?);
                    engine.send_message_with(&key, ch, line, None, vec![file]);
                    println!("{name}: sent {}", p.display());
                }
                None => {
                    engine.send_message(&key, ch, line, None);
                    println!("{name}: said {line:?}");
                }
            }
            std::thread::sleep(Duration::from_secs_f32(gap));
        }
    }
    if let Some(v) = &voice {
        let ch = find(ChannelKind::Voice, v).ok_or_else(|| anyhow::anyhow!("no voice channel named {v}"))?;
        engine.join_voice(&key, ch);
        let t = Instant::now();
        while engine.state().call.is_none() && t.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(100));
        }
        println!("{name}: in voice channel {v}");
        if camera {
            engine.start_camera();
        }
        if screen {
            engine.start_screen(&source);
        }
    }
    let _ = rt;
    let deadline = seconds.map(|s| Instant::now() + Duration::from_secs(s));
    let mut watching = std::collections::HashSet::new();
    let mut last_report = Instant::now();
    let mut last_look = Instant::now() - Duration::from_secs(1);
    let mut last_stats = Instant::now();
    // The newest picture seen from each share.
    let mut seen: std::collections::HashMap<u32, u64> = std::collections::HashMap::new();
    loop {
        // Measuring: look for new pictures every 2 ms.
        std::thread::sleep(Duration::from_millis(if measure.is_some() { 2 } else { 500 }));
        if let Some(f) = &out {
            for s in &watching {
                let key = pyr_engine::state::VideoKey { session: *s, track: pyr_proto::media::track::SCREEN };
                if let Some(frame) = engine.video_frame(key) {
                    if seen.get(s) != Some(&frame.generation) {
                        seen.insert(*s, frame.generation);
                        let now = wall_ms();
                        if let Some(ms) = testsrc::read_stamp(&frame.rgba, frame.width, frame.height) {
                            // (The stamp keeps the clock's last 32 bits.)
                            let late = (now as u64 as u32).wrapping_sub(ms) as i32;
                            let _ = writeln!(f.lock().unwrap(), "v {now:.1} {late} {} {}", frame.width, frame.height);
                        }
                    }
                }
            }
            if last_stats.elapsed() >= Duration::from_millis(500) {
                last_stats = Instant::now();
                let line = engine.state().call.as_ref().map(|c| {
                    let st = &c.stats;
                    format!(
                        "s {:.1} rtt={} jitter={:.1} buffer={} audio_lost={} concealed={} video_rx={} send_kbps={}",
                        wall_ms(),
                        st.rtt_ms,
                        st.jitter_ms,
                        st.buffer_ms,
                        st.audio_lost,
                        st.audio_concealed,
                        st.video_frames_received,
                        st.video_send_kbps
                    )
                });
                if let Some(l) = line {
                    let _ = writeln!(f.lock().unwrap(), "{l}");
                }
            }
        }
        if watch && last_look.elapsed() >= Duration::from_millis(500) {
            last_look = Instant::now();
            // Watch every screen share in our voice channel and report what we decode.
            let shares: Vec<u32> = {
                let st = engine.state();
                match (st.call.as_ref(), st.server(&key)) {
                    (Some(c), Some(s)) => s
                        .voice_in(c.channel_id)
                        .iter()
                        .filter(|p| p.session != c.session && p.tracks.iter().any(|t| t.id == pyr_proto::media::track::SCREEN))
                        .map(|p| p.session)
                        .collect(),
                    _ => Vec::new(),
                }
            };
            for s in shares {
                if watching.insert(s) {
                    println!("{name}: watching screen share from session {s}");
                    engine.watch(s, pyr_proto::media::track::SCREEN, true);
                }
            }
            if last_report.elapsed() > Duration::from_secs(3) {
                last_report = Instant::now();
                for s in &watching {
                    let key = pyr_engine::state::VideoKey { session: *s, track: pyr_proto::media::track::SCREEN };
                    if let Some(f) = engine.video_frame(key) {
                        println!("{name}: screen from {s}: {}x{} (frame #{})", f.width, f.height, f.generation);
                    }
                }
            }
        }
        if deadline.map(|d| Instant::now() > d).unwrap_or(false) {
            break;
        }
    }
    engine.shutdown();
    Ok(())
}

/// Beeps (see `--beeps`) start at every multiple of this on the clock,
const BEEP_EVERY_MS: f64 = 2000.0;
/// at this pitch (well above the test screen's sound).
const BEEP_HZ: f64 = 2500.0;

/// The wall clock in milliseconds (the same in every program on this computer).
fn wall_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}

/// Load voice isolation's model (built into the program) and time 3 s of audio through it.
fn check_isolation() -> anyhow::Result<()> {
    let t = Instant::now();
    let mut d = pyr_engine::audio::dfn::Dfn::new().map_err(|e| anyhow::anyhow!("voice isolation model failed: {e}"))?;
    let load = t.elapsed();
    let mut frame = vec![0f32; 480];
    let mut worst = Duration::ZERO;
    let t = Instant::now();
    for i in 0..300usize {
        for (k, s) in frame.iter_mut().enumerate() {
            *s = ((i * 480 + k) as f32 * 0.047).sin() * 0.1;
        }
        let f = Instant::now();
        d.process(&mut frame);
        worst = worst.max(f.elapsed());
    }
    let per = t.elapsed().as_secs_f64() * 1000.0 / 300.0;
    println!("voice isolation model: loaded in {} ms; {per:.2} ms per 10 ms of audio (slowest {:.2} ms)", load.as_millis(), worst.as_secs_f64() * 1000.0);
    Ok(())
}
