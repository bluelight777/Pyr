//! Development: sends requests to a server exactly as written, to see what it answers to
//! messages this Pyr doesn't know yet. With `PYR_DEV_FRAME_LOG=<file>` set, every message from
//! the server is written there as it came.
//!
//!   PYR_DEV_FRAME_LOG=frames.log cargo run --release -p pyr-engine --example raw_probe -- \
//!       --dir <a profile folder> [--join "pyr://…"] --wait 2 \
//!       --send '{"op":"poker","msg":{"op":"staff"}}' --send '{"op":"poker","msg":{"op":"cms","req":{"op":"overview"}}}'
//!
//! `--dir` is a Pyr profile (its identity is who asks; a test profile, never anyone's own).
//! Without `--join` it uses the first server already in that profile.
//!
//! `--stdin`: after the `--send`s, each line read is sent as a request the same way (another
//! program drives it, reading the answers in the frame log); `wait <seconds>` pauses; the end
//! of the input ends it.

use std::time::{Duration, Instant};

use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};

fn main() -> anyhow::Result<()> {
    let mut dir = None;
    let mut join = None;
    let mut sends: Vec<String> = Vec::new();
    let mut wait = 2.0f64;
    let mut stdin = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => dir = args.next(),
            "--join" => join = args.next(),
            "--wait" => wait = args.next().and_then(|n| n.parse().ok()).unwrap_or(wait),
            "--stdin" => stdin = true,
            // (Checked to be JSON, but sent exactly as written: the keys' order kept.)
            "--send" => {
                let req = args.next().unwrap_or_default();
                serde_json::from_str::<serde_json::Value>(&req)?;
                sends.push(req);
            }
            other => anyhow::bail!("what's {other}?"),
        }
    }
    let dir = std::path::PathBuf::from(dir.ok_or_else(|| anyhow::anyhow!("--dir <profile folder> is required"))?);
    let engine = Engine::start(EngineOptions { config_dir: Some(dir), ..Default::default() })?;
    let key = match join {
        Some(link) => engine.add_server(&link)?,
        None => {
            let start = Instant::now();
            loop {
                if let Some(s) = engine.state().servers.first() {
                    break s.key.clone();
                }
                if start.elapsed() > Duration::from_secs(10) {
                    anyhow::bail!("that profile has no server (give --join)");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    };
    let start = Instant::now();
    loop {
        match engine.state().server(&key).map(|s| s.status.clone()) {
            Some(ConnStatus::Connected) => break,
            Some(ConnStatus::Failed(e)) => anyhow::bail!("couldn't connect: {e}"),
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(20) {
            anyhow::bail!("timed out connecting");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("connected");
    // (A moment for what a connection gets first, so the answers stand apart in the log.)
    std::thread::sleep(Duration::from_millis(1500));
    for req in sends {
        eprintln!("sending {req}");
        engine.dev_send_raw(&key, req);
        std::thread::sleep(Duration::from_secs_f64(wait));
    }
    if stdin {
        for line in std::io::stdin().lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(s) = line.strip_prefix("wait ") {
                std::thread::sleep(Duration::from_secs_f64(s.trim().parse().unwrap_or(1.0)));
                continue;
            }
            if serde_json::from_str::<serde_json::Value>(line).is_err() {
                eprintln!("not JSON, not sent: {line}");
                continue;
            }
            eprintln!("sending {line}");
            engine.dev_send_raw(&key, line.to_string());
        }
    }
    Ok(())
}
