//! Keno: pick 1 to 10 numbers from 1 to 80; the house draws 20. What it pays depends on how many
//! of your picks were drawn (see [`PAYS`]: about 95% comes back over time, whatever you pick).

use serde::{Deserialize, Serialize};

use crate::fair::Stream;

pub const NUMBERS: u8 = 80;
pub const DRAWN: usize = 20;
pub const MAX_PICKS: usize = 10;

/// What each catch pays, times ten (so 3.8 times the bet is 38), for 1 to 10 picks:
/// `PAYS[picks - 1][caught]`.
pub const PAYS: [[u64; 11]; 10] = [
    [0, 38, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 160, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 20, 480, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 10, 90, 1_100, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 20, 220, 8_000, 0, 0, 0, 0, 0],
    [0, 0, 0, 20, 90, 900, 12_000, 0, 0, 0, 0],
    [0, 0, 0, 20, 40, 210, 1_200, 50_000, 0, 0, 0],
    [0, 0, 0, 0, 25, 140, 1_000, 9_000, 250_000, 0, 0],
    [0, 0, 0, 0, 15, 60, 480, 3_000, 30_000, 500_000, 0],
    [0, 0, 0, 0, 0, 50, 220, 1_500, 10_000, 80_000, 1_000_000],
];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub chips: u64,
    pub picks: Vec<u8>,
    /// The twenty drawn, in the order they came out.
    pub drawn: Vec<u8>,
    /// Picks that were drawn.
    pub hits: Vec<u8>,
    pub paid: u64,
}

pub fn check_picks(picks: &[u8]) -> Result<(), String> {
    if picks.is_empty() || picks.len() > MAX_PICKS {
        return Err("pick 1 to 10 numbers".into());
    }
    let mut seen = [false; NUMBERS as usize + 1];
    for &p in picks {
        if p == 0 || p > NUMBERS {
            return Err("numbers go from 1 to 80".into());
        }
        if seen[p as usize] {
            return Err("each number once".into());
        }
        seen[p as usize] = true;
    }
    Ok(())
}

/// The twenty numbers drawn.
pub fn draw(s: &mut Stream) -> Vec<u8> {
    let mut balls: Vec<u8> = (1..=NUMBERS).collect();
    for i in 0..DRAWN {
        let j = i + s.below((balls.len() - i) as u32) as usize;
        balls.swap(i, j);
    }
    balls.truncate(DRAWN);
    balls
}

/// What `caught` of `picks` pays back (stake included).
pub fn payout(chips: u64, picks: usize, caught: usize) -> u64 {
    if picks == 0 || picks > MAX_PICKS {
        return 0;
    }
    chips * PAYS[picks - 1][caught.min(10)] / 10
}

pub fn play(chips: u64, picks: &[u8], s: &mut Stream) -> Outcome {
    let drawn = draw(s);
    let hits: Vec<u8> = picks.iter().copied().filter(|p| drawn.contains(p)).collect();
    let paid = payout(chips, picks.len(), hits.len()).min(super::MAX_PAYOUT);
    Outcome { chips, picks: picks.to_vec(), drawn, hits, paid }
}
