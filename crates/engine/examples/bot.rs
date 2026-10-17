//! A headless test participant. Handy for trying a call on your own:
//!
//!   cargo run --release -p ingle-engine --example bot -- --join "ingle://…" --name Mara \
//!       --voice Lounge --camera --screen --say "hi from the bot"
//!
//! The bot "talks" with a tone (on/off like speech), and can share a test-pattern
//! camera and screen (the screen share comes with a quiet test chord unless you pass
//! --no-sound). Ctrl+C to stop.
//!
//! `--dm NAME` sends the `--say` lines as direct messages to that member instead, after
//! `--wait SECONDS` (time to switch windows), `--gap SECONDS` apart. `--image PATH` sends a
//! picture (in order with the `--say` lines).
//!
//! `--check-isolation` only loads voice isolation's model and times it on this PC, then exits.

use std::time::{Duration, Instant};

use ingle_engine::audio::{AudioInput, AudioOutput};
use ingle_engine::config::{Config, NoiseSuppression, Settings};
use ingle_engine::state::ConnStatus;
use ingle_engine::{Engine, EngineOptions};
use ingle_proto::control::ChannelKind;

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
            _ => {}
        }
    }
    if std::env::args().any(|a| a == "--check-isolation") {
        return check_isolation();
    }
    let join = join.ok_or_else(|| anyhow::anyhow!("--join <invite link> is required"))?;
    let dir = std::env::temp_dir().join(format!("ingle-bot-{}", name.to_lowercase().replace(' ', "-")));
    std::fs::create_dir_all(&dir)?;
    let mut cfg = ingle_engine::config::load_config(&dir);
    cfg.display_name = name.clone();
    cfg.settings = Settings {
        noise_suppression: NoiseSuppression::Off,
        vad_auto: false,
        vad_threshold_db: -45.0,
        echo_cancellation: false,
        auto_gain: false,
        camera_device: Some("test:camera".into()),
        share_audio: stream_sound,
        ..Default::default()
    };
    ingle_engine::config::save_config(&dir, &Config { ..cfg })?;

    let engine = Engine::start(EngineOptions {
        config_dir: Some(dir),
        push_to_talk: false,
        audio: Some(Box::new(move |_s: &Settings| {
            let mut n = 0u64;
            let input = AudioInput::Synthetic(Box::new(move |buf: &mut [f32]| {
                for v in buf.iter_mut() {
                    // talk for ~1.6 s, pause ~1.1 s, with a little vibrato so it sounds less robotic
                    let t = n as f32 / 48_000.0;
                    let on = !quiet && (t % 2.7) < 1.6;
                    let f = tone * (1.0 + 0.03 * (t * 5.0).sin());
                    *v = if on { (t * f * std::f32::consts::TAU).sin() * amp } else { 0.0 };
                    n += 1;
                }
            }));
            (input, AudioOutput::None)
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
            engine.start_screen("test:pattern");
        }
    }
    let _ = rt;
    let deadline = seconds.map(|s| Instant::now() + Duration::from_secs(s));
    let mut watching = std::collections::HashSet::new();
    let mut last_report = Instant::now();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if watch {
            // Watch every screen share in our voice channel and report what we decode.
            let shares: Vec<u32> = {
                let st = engine.state();
                match (st.call.as_ref(), st.server(&key)) {
                    (Some(c), Some(s)) => s
                        .voice_in(c.channel_id)
                        .iter()
                        .filter(|p| p.session != c.session && p.tracks.iter().any(|t| t.id == ingle_proto::media::track::SCREEN))
                        .map(|p| p.session)
                        .collect(),
                    _ => Vec::new(),
                }
            };
            for s in shares {
                if watching.insert(s) {
                    println!("{name}: watching screen share from session {s}");
                    engine.watch(s, ingle_proto::media::track::SCREEN, true);
                }
            }
            if last_report.elapsed() > Duration::from_secs(3) {
                last_report = Instant::now();
                for s in &watching {
                    let key = ingle_engine::state::VideoKey { session: *s, track: ingle_proto::media::track::SCREEN };
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

/// Load voice isolation's model (built into the program) and time 3 s of audio through it.
fn check_isolation() -> anyhow::Result<()> {
    let t = Instant::now();
    let mut d = ingle_engine::audio::dfn::Dfn::new().map_err(|e| anyhow::anyhow!("voice isolation model failed: {e}"))?;
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
