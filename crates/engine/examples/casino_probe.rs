//! Development: plays casino rounds on a server from the command line, to see exactly what it
//! answers (with `PYR_DEV_FRAME_LOG=<file>` set, every message from the server is written there
//! as it came, the ones this Pyr can't read too).
//!
//!   PYR_DEV_FRAME_LOG=frames.log cargo run --release -p pyr-engine --example casino_probe -- \
//!       --join "pyr://…" --name Probe --rounds 5 \
//!       --bet '{"game":"plinko","chips":100,"risk":"medium","rows":16}'
//!
//! Each round: the casino opened (for the house's fingerprint), a seed of the probe's own, the
//! bet. The seeds it used are printed, one round a line (`round <n> seed <hex>`).

use std::time::{Duration, Instant};

use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_poker::casino::Bet;
use pyr_poker::net::{CasinoReq, ToServer};
use sha2::{Digest, Sha256};

fn main() -> anyhow::Result<()> {
    let mut join = None;
    let mut name = "Probe".to_string();
    let mut bet = None;
    let mut rounds = 1u32;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--join" => join = args.next(),
            "--name" => name = args.next().unwrap_or(name),
            "--bet" => bet = args.next(),
            "--rounds" => rounds = args.next().and_then(|n| n.parse().ok()).unwrap_or(rounds),
            other => anyhow::bail!("what's {other}?"),
        }
    }
    let join = join.ok_or_else(|| anyhow::anyhow!("--join <invite link> is required"))?;
    let bet: Bet = serde_json::from_str(&bet.ok_or_else(|| anyhow::anyhow!("--bet <json> is required"))?)?;
    let dir = std::env::temp_dir().join(format!("pyr-probe-{}", name.to_lowercase().replace(' ', "-")));
    std::fs::create_dir_all(&dir)?;
    let mut cfg = pyr_engine::config::load_config(&dir);
    cfg.display_name = name.clone();
    pyr_engine::config::save_config(&dir, &cfg)?;
    let engine = Engine::start(EngineOptions { config_dir: Some(dir), ..Default::default() })?;
    let key = engine.add_server(&join)?;
    let start = Instant::now();
    loop {
        match engine.state().server(&key).map(|s| s.status.clone()) {
            Some(ConnStatus::Connected) => break,
            Some(ConnStatus::Failed(e)) => anyhow::bail!("couldn't join: {e}"),
            _ => {}
        }
        if start.elapsed() > Duration::from_secs(20) {
            anyhow::bail!("timed out connecting");
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!("{name}: connected");
    let commit_now = |e: &Engine| e.state().server(&key).and_then(|s| s.poker.casino.commit);
    let mut last = None;
    for n in 0..rounds {
        // The house's fingerprint for this round (a new one after every round).
        engine.poker(&key, ToServer::Casino { req: CasinoReq::Open });
        let asked = Instant::now();
        let commit = loop {
            if let Some(c) = commit_now(&engine)
                && Some(c) != last
            {
                break c;
            }
            if asked.elapsed() > Duration::from_secs(8) {
                anyhow::bail!("no new fingerprint from the house for round {n}");
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        last = Some(commit);
        let mut h = Sha256::new();
        h.update(name.as_bytes());
        h.update(n.to_le_bytes());
        h.update(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos().to_le_bytes());
        let seed: [u8; 32] = h.finalize().into();
        println!("round {n} seed {}", seed.iter().map(|b| format!("{b:02x}")).collect::<String>());
        engine.poker(&key, ToServer::Casino { req: CasinoReq::Play { commit, seed, bet: bet.clone() } });
        std::thread::sleep(Duration::from_millis(700));
    }
    std::thread::sleep(Duration::from_millis(1500));
    // How the rounds checked out (this Pyr played each again from the seeds).
    if let Some(s) = engine.state().server(&key) {
        println!("checked {} problems {:?}", s.poker.casino.checked, s.poker.casino.problems);
    }
    Ok(())
}
