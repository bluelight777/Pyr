//! Development: plays a craps table on a server from the command line, every kind of bet at once,
//! to see exactly how it settles them (with `PYR_DEV_FRAME_LOG=<file>` set, every message from
//! the server is written there as it came). The probe sits at the table (the shooter, when it's
//! alone) and each roll puts down: the pass line and don't pass on the come-out, full odds and
//! lay odds behind them on a point, place bets, hardways, buys and lays on every number, and the
//! field, any seven and any craps; buys working on every other come-out.
//!
//!   PYR_DEV_FRAME_LOG=frames.log cargo run --release -p pyr-engine --example craps_probe -- \
//!       --join "pyr://…" --name Probe --table 1 --rolls 20
//!
//! `--try '<bets json>'` instead sends those bets once (on the coming roll) and prints the answer.

use std::time::{Duration, Instant};

use pyr_engine::state::ConnStatus;
use pyr_engine::{Engine, EngineOptions};
use pyr_poker::casino::craps::{Bets, max_odds};
use pyr_poker::net::ToServer;

fn main() -> anyhow::Result<()> {
    let mut join = None;
    let mut name = "Probe".to_string();
    let mut table = 1u64;
    let mut rolls = 10u32;
    let mut tries: Vec<String> = Vec::new();
    let mut unit = 20u64;
    // (Odds and lay odds as many times the line as given: more than the most, to see the refusal.)
    let mut odds_x: Option<u64> = None;
    let mut lay_x = 6u64;
    // (A seat to sit in, when it's free; and only buys and lays, one chip of `unit` each.)
    let mut want_seat: Option<u8> = None;
    let mut strips = false;
    // (Only the pass line, `unit` on it each come-out: someone at the table, so its bots play;
    // and bots in every other seat, asked to leave and new ones seated every so many rolls, to
    // see each kind bet.)
    let mut line = false;
    let mut bots_every: Option<u32> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--join" => join = args.next(),
            "--name" => name = args.next().unwrap_or(name),
            "--table" => table = args.next().and_then(|n| n.parse().ok()).unwrap_or(table),
            "--rolls" => rolls = args.next().and_then(|n| n.parse().ok()).unwrap_or(rolls),
            "--unit" => unit = args.next().and_then(|n| n.parse().ok()).unwrap_or(unit),
            "--odds-x" => odds_x = args.next().and_then(|n| n.parse().ok()),
            "--lay-x" => lay_x = args.next().and_then(|n| n.parse().ok()).unwrap_or(lay_x),
            "--try" => tries.push(args.next().unwrap_or_default()),
            "--seat" => want_seat = args.next().and_then(|n| n.parse().ok()),
            "--strips" => strips = true,
            "--line" => line = true,
            "--bots" => bots_every = args.next().and_then(|n| n.parse().ok()),
            other => anyhow::bail!("what's {other}?"),
        }
    }
    let join = join.ok_or_else(|| anyhow::anyhow!("--join <invite link> is required"))?;
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
    engine.poker(&key, ToServer::CrapsTables);
    std::thread::sleep(Duration::from_millis(600));
    engine.poker(&key, ToServer::CrapsWatch { table });
    let view = |e: &Engine| e.state().server(&key).and_then(|s| s.poker.craps.as_ref().filter(|c| c.view.id == table).map(|c| c.view.clone()));
    let error = |e: &Engine| e.state().server(&key).and_then(|s| s.poker.error.clone());
    let wait = |e: &Engine, what: &str, f: &dyn Fn(&pyr_poker::net::CrapsView) -> bool| -> anyhow::Result<pyr_poker::net::CrapsView> {
        let start = Instant::now();
        loop {
            if let Some(v) = view(e).filter(|v| f(v)) {
                return Ok(v);
            }
            if start.elapsed() > Duration::from_secs(40) {
                anyhow::bail!("timed out waiting for {what}");
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let v = wait(&engine, "the table", &|_| true)?;
    let seat = match v.my_seat {
        Some(s) => s,
        None => {
            let first = v.seats.iter().position(|s| s.is_none()).ok_or_else(|| anyhow::anyhow!("no seat free"))? as u8;
            let free = want_seat.filter(|s| v.seats.get(*s as usize).is_some_and(|x| x.is_none())).unwrap_or(first);
            engine.poker(&key, ToServer::CrapsSit { table, seat: free });
            wait(&engine, "my seat", &|v| v.my_seat.is_some())?;
            free
        }
    };
    eprintln!("sitting in seat {seat}");
    if !tries.is_empty() {
        for t in &tries {
            let bets: Bets = serde_json::from_str(t)?;
            let v = wait(&engine, "betting", &|v| matches!(v.stage, pyr_poker::net::CrapsStage::Betting | pyr_poker::net::CrapsStage::Waiting))?;
            engine.clear_poker_error(&key);
            std::thread::sleep(Duration::from_millis(200));
            engine.craps_bets(&key, table, v.roll_no, bets, false);
            std::thread::sleep(Duration::from_millis(1200));
            let mine = view(&engine).and_then(|v| v.seats[seat as usize].clone()).map(|s| serde_json::to_string(&s.bets).unwrap_or_default());
            println!("try {t}\n  -> error: {:?}\n  -> on the layout: {}", error(&engine), mine.unwrap_or_default());
            // (Taken back down, for the next.)
            engine.craps_bets(&key, table, v.roll_no, Bets::default(), false);
            std::thread::sleep(Duration::from_millis(600));
        }
        engine.poker(&key, ToServer::CrapsLeave { table });
        std::thread::sleep(Duration::from_millis(500));
        return Ok(());
    }
    let u = unit;
    let mut last_roll = 0;
    for n in 0..rolls {
        let v = wait(&engine, "betting", &|v| v.roll_no > last_roll && matches!(v.stage, pyr_poker::net::CrapsStage::Betting | pyr_poker::net::CrapsStage::Waiting))?;
        last_roll = v.roll_no;
        let up = v.seats[seat as usize].as_ref().map(|s| s.bets.clone()).unwrap_or_default();
        let mut b = up.clone();
        if let Some(every) = bots_every
            && n % every.max(1) == 0
        {
            for (i, x) in v.seats.iter().enumerate() {
                if x.as_ref().is_some_and(|x| x.bot) {
                    engine.poker(&key, ToServer::CrapsBot { table, seat: i as u8, add: false });
                }
            }
            std::thread::sleep(Duration::from_millis(300));
            for i in 0..v.seats.len() {
                if i != seat as usize && v.seats[i].as_ref().is_none_or(|x| x.bot) {
                    engine.poker(&key, ToServer::CrapsBot { table, seat: i as u8, add: true });
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        if line {
            if v.point.is_none() && b.pass == 0 {
                b.pass = u;
            }
            engine.clear_poker_error(&key);
            engine.craps_bets(&key, table, v.roll_no, b.clone(), true);
            std::thread::sleep(Duration::from_millis(300));
            if let Some(e) = error(&engine) {
                println!("roll {} refused: {e}", v.roll_no);
            }
            let after = wait(&engine, "the dice", &|w| w.last.as_ref().is_some_and(|l| l.roll == v.roll_no))?;
            println!("roll {} point {:?} dice {:?}", v.roll_no, v.point, after.last.as_ref().map(|l| l.dice));
            continue;
        }
        if strips {
            for i in 0..6 {
                b.buy[i] = u;
                b.lay[i] = u;
            }
            engine.clear_poker_error(&key);
            engine.craps_bets(&key, table, v.roll_no, b.clone(), false);
            std::thread::sleep(Duration::from_millis(700));
            if let Some(e) = error(&engine) {
                println!("roll {} refused: {e}", v.roll_no);
            }
            let after = wait(&engine, "the dice", &|w| w.last.as_ref().is_some_and(|l| l.roll == v.roll_no))?;
            println!("roll {} dice {:?}", v.roll_no, after.last.as_ref().map(|l| l.dice));
            continue;
        }
        match v.point {
            None => {
                b.pass = if b.pass == 0 { u } else { b.pass };
                b.dont_pass = if b.dont_pass == 0 { u } else { b.dont_pass };
                b.pass_odds = 0;
                b.dont_odds = 0;
                b.buys_working = n % 2 == 1;
            }
            Some(p) => {
                b.pass_odds = b.pass * odds_x.unwrap_or(max_odds(p));
                b.dont_odds = b.dont_pass * lay_x;
            }
        }
        for i in 0..6 {
            if b.place[i] == 0 {
                b.place[i] = if i == 2 || i == 3 { u + u / 5 } else { u };
            }
            if b.buy[i] == 0 {
                b.buy[i] = u;
            }
            if b.lay[i] == 0 {
                b.lay[i] = u + u / 2;
            }
        }
        for i in 0..4 {
            if b.hard[i] == 0 {
                b.hard[i] = u / 2;
            }
        }
        b.field = u / 2;
        b.any_seven = u / 2;
        b.any_craps = u / 2;
        engine.clear_poker_error(&key);
        engine.craps_bets(&key, table, v.roll_no, b.clone(), true);
        std::thread::sleep(Duration::from_millis(700));
        if let Some(e) = error(&engine) {
            println!("roll {} refused: {e}\n  bets {}", v.roll_no, serde_json::to_string(&b)?);
        }
        if v.shooter == Some(seat) {
            engine.poker(&key, ToServer::CrapsThrow { table });
        }
        let after = wait(&engine, "the dice", &|w| w.last.as_ref().is_some_and(|l| l.roll == v.roll_no))?;
        let me = after.seats[seat as usize].as_ref();
        println!(
            "roll {} point {:?} dice {:?}: {}",
            v.roll_no,
            v.point,
            after.last.as_ref().map(|l| l.dice),
            me.map(|s| serde_json::to_string(&s.decided).unwrap_or_default()).unwrap_or_default()
        );
    }
    engine.poker(&key, ToServer::CrapsLeave { table });
    std::thread::sleep(Duration::from_millis(500));
    Ok(())
}
