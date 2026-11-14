//! Development: a table cue rendered many times into one raw file (48 kHz stereo f32,
//! interleaved), each take starting `gap` seconds after the last, to measure the rebuilt table's
//! sounds against 0.13.28's recordings without playing them through the app.
//!
//!   cargo run -p pyr-engine --example cue_dump -- <cue> <takes> <pan> <gap s> <out.raw>
//!
//! Cues: deal, card, flip, fold, chips, check, pot, allin, win, turn, tick, tickup, timebank,
//! timeout, hurry.

use pyr_engine::poker_sfx::{Cue, render};

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().skip(1).collect();
    anyhow::ensure!(a.len() == 5, "cue_dump <cue> <takes> <pan> <gap s> <out.raw>");
    let cue = match a[0].as_str() {
        "deal" => Cue::Deal,
        "card" => Cue::Card,
        "flip" => Cue::Flip,
        "fold" => Cue::Fold,
        "chips" => Cue::Chips,
        "check" => Cue::Check,
        "pot" => Cue::Pot,
        "allin" => Cue::AllIn,
        "win" => Cue::Win,
        "turn" => Cue::Turn,
        "tick" => Cue::Tick,
        "tickup" => Cue::TickUp,
        "timebank" => Cue::TimeBank,
        "timeout" => Cue::TimeOut,
        "hurry" => Cue::Hurry,
        other => anyhow::bail!("what's {other}?"),
    };
    let takes: usize = a[1].parse()?;
    let pan: f32 = a[2].parse()?;
    let gap = (a[3].parse::<f32>()? * 48_000.0) as usize * 2;
    let mut out = vec![0.0f32; gap * takes + 48_000 * 4];
    for k in 0..takes {
        for (i, x) in render(cue, pan).into_iter().enumerate() {
            if let Some(o) = out.get_mut(k * gap + i) {
                *o += x;
            }
        }
    }
    let bytes: Vec<u8> = out.iter().flat_map(|x| x.to_le_bytes()).collect();
    std::fs::write(&a[4], bytes)?;
    Ok(())
}
