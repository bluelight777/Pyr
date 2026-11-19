//! Development: sends a bug report the way anyone's Pyr does (no account), to a test server
//! on this machine, for trying the owner's Bug reports window on a test screen
//! (`rebuild/tools/release-0.16.1/e2e`). Only ever to this machine: `PYR_BUG_SERVER` must be
//! set, and name 127.0.0.1.
//!
//!   PYR_BUG_SERVER=127.0.0.1:17790/<server key> cargo run -p pyr-engine --example send_report -- \
//!       --title "The wheel's last number is cut off" --body "…" --log 47000 --count 2
//!
//! `--log`: bytes of made-up log to send along (0: none). `--count`: how many reports, their
//! titles numbered from the second on.

use std::time::{Duration, Instant};

use pyr_engine::state::BugSend;
use pyr_engine::{Engine, EngineOptions};

fn main() -> anyhow::Result<()> {
    let (mut title, mut body, mut log, mut count) = ("A made-up report".to_string(), "Made up, for a test.".to_string(), 0usize, 1u32);
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--title" => title = args.next().unwrap_or(title),
            "--body" => body = args.next().unwrap_or(body),
            "--log" => log = args.next().and_then(|n| n.parse().ok()).unwrap_or(log),
            "--count" => count = args.next().and_then(|n| n.parse().ok()).unwrap_or(count),
            other => anyhow::bail!("what's {other}?"),
        }
    }
    // (Never the bug server Pyr is built with: only one this machine runs, named when running.)
    let to = std::env::var("PYR_BUG_SERVER").map_err(|_| anyhow::anyhow!("PYR_BUG_SERVER=127.0.0.1:<port>/<server key> is required"))?;
    anyhow::ensure!(to.trim().starts_with("127.0.0.1:"), "only to a test server on this machine (127.0.0.1)");
    anyhow::ensure!(pyr_engine::bugs::server().is_some(), "PYR_BUG_SERVER isn't host:port/<server key>");
    let dir = std::env::temp_dir().join(format!("pyr-send-report-{}", pyr_proto::now_ms()));
    std::fs::create_dir_all(&dir)?;
    let cfg = pyr_engine::config::Config { display_name: "Reporter".to_string(), ..Default::default() };
    pyr_engine::config::save_config(&dir, &cfg)?;
    let mut lines = String::new();
    while lines.len() < log {
        lines.push_str(&format!("2026-11-19T21:{:02}:{:02}.000000Z  INFO pyr::wheel: made-up line {} of a made-up log\n", (lines.len() / 4000) % 60, (lines.len() / 70) % 60, lines.len() / 70));
    }
    std::fs::write(dir.join("pyr.log"), &lines)?;
    let engine = Engine::start(EngineOptions { config_dir: Some(dir.clone()), push_to_talk: false, audio: None, ..Default::default() })?;
    for i in 0..count {
        let title = if i == 0 { title.clone() } else { format!("{title} ({})", i + 1) };
        let before = engine.state().bug_sends_done;
        engine.report_bug(&title, &body, log > 0, None);
        let start = Instant::now();
        while engine.state().bug_sends_done == before {
            anyhow::ensure!(start.elapsed() < Duration::from_secs(20), "no answer");
            std::thread::sleep(Duration::from_millis(50));
        }
        match engine.state().bug_send.clone() {
            BugSend::Sent => println!("sent: {title}"),
            other => anyhow::bail!("not sent: {other:?}"),
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
