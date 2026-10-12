//! A headless test participant. Handy for trying a call on your own:
//!
//!   cargo run --release -p ingle-engine --example bot -- --join "ingle://…" --name Mara \
//!       --voice Lounge --camera --screen --say "hi from the bot"
//!
//! The bot "talks" with a tone (on/off like speech), and can share a test-pattern
//! camera and screen (the screen share comes with a quiet test chord unless you pass
//! --no-sound). Ctrl+C to stop.

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
    let mut say: Vec<String> = Vec::new();
    let mut channel = "general".to_string();
    let mut tone = 330.0f32;
    let mut seconds: Option<u64> = None;
    let mut quiet = false;
    let mut watch = false;
    let mut stream_sound = true;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--join" => join = args.next(),
            "--name" => name = args.next().unwrap_or(name),
            "--voice" => voice = args.next(),
            "--camera" => camera = true,
            "--screen" => screen = true,
            "--say" => say.extend(args.next()),
            "--channel" => channel = args.next().unwrap_or(channel),
            "--tone" => tone = args.next().and_then(|t| t.parse().ok()).unwrap_or(tone),
            "--seconds" => seconds = args.next().and_then(|t| t.parse().ok()),
            "--quiet" => quiet = true,
            "--watch" => watch = true,
            "--no-sound" => stream_sound = false,
            _ => {}
        }
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
                    *v = if on { (t * f * std::f32::consts::TAU).sin() * 0.18 } else { 0.0 };
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
    let find = |kind: ChannelKind, n: &str| {
        engine.state().server(&key).and_then(|s| {
            s.channels.iter().find(|c| c.kind == kind && c.name.eq_ignore_ascii_case(n)).map(|c| c.id)
        })
    };
    if let Some(ch) = find(ChannelKind::Text, &channel).or_else(|| find(ChannelKind::Voice, &channel)) {
        for line in &say {
            engine.send_message(&key, ch, line, None);
            std::thread::sleep(Duration::from_millis(900));
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
