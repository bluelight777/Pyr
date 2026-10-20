//! Pyr Sevens: five reels, three rows, twenty lines.
//!
//! Wins pay left to right on a line, three of a kind or more. The flame is wild (it stands in
//! for anything but the star) and pays on its own; five flames on a line win the Phoenix
//! Jackpot (a pool that grows by 1% of every spin's bet). Three or more stars anywhere pay and
//! start free spins, every win in them tripled, and more stars in them add more.
//!
//! Over a long run the reels pay back 94.5% of what's bet, and the jackpot pays back the 1% it
//! takes: about 95.5% in all (see the tests, which work it out exactly).

use serde::{Deserialize, Serialize};

use crate::fair::Stream;

pub const REELS: usize = 5;
pub const ROWS: usize = 3;
pub const LINES: usize = 20;
/// The most a line may bet (20 lines: 10,000 a spin).
pub const MAX_LINE_BET: u64 = 500;
/// Free spins in one round, at most (stars keep adding them; the round has to end).
pub const MAX_FREE_SPINS: usize = 100;
/// Wins in free spins are multiplied by this.
pub const FREE_SPIN_MULTIPLIER: u64 = 3;
/// The jackpot's share of every spin's bet, in hundredths of a percent; and what it starts at.
pub const JACKPOT_SHARE_BP: u64 = 100;
pub const JACKPOT_SEED: u64 = 50_000;

/// The symbols.
pub const CHERRY: u8 = 0;
pub const LEMON: u8 = 1;
pub const PLUM: u8 = 2;
pub const GRAPES: u8 = 3;
pub const BELL: u8 = 4;
pub const BAR: u8 = 5;
pub const DIAMOND: u8 = 6;
pub const SEVEN: u8 = 7;
pub const WILD: u8 = 8;
pub const STAR: u8 = 9;
pub const SYMBOLS: usize = 10;

pub fn symbol_name(s: u8) -> &'static str {
    ["Cherry", "Lemon", "Plum", "Grapes", "Bell", "BAR", "Diamond", "Seven", "Wild flame", "Star"].get(s as usize).copied().unwrap_or("?")
}

/// What three, four and five of a kind pay, times the line bet (the wild's own too).
pub const PAYS: [[u64; 3]; 9] = [
    [4, 12, 40],
    [4, 12, 40],
    [5, 15, 60],
    [6, 15, 60],
    [10, 30, 120],
    [15, 45, 150],
    [25, 75, 300],
    [40, 120, 600],
    [60, 400, 1500],
];

/// Stars anywhere: what 3, 4 and 5 pay (times the whole spin's bet) and the free spins they
/// start.
pub const STAR_PAYS: [u64; 3] = [2, 10, 50];
pub const STAR_SPINS: [usize; 3] = [10, 15, 25];

const CH: u8 = CHERRY;
const LE: u8 = LEMON;
const PL: u8 = PLUM;
const GR: u8 = GRAPES;
const BE: u8 = BELL;
const BA: u8 = BAR;
const DI: u8 = DIAMOND;
const SE: u8 = SEVEN;
const W: u8 = WILD;
const S: u8 = STAR;

/// The reels, top to bottom (each spin stops each reel at a random place; the three rows show
/// the symbol there and the next two).
pub const STRIPS: [&[u8]; REELS] = [
    &[S, LE, PL, W, CH, BE, GR, BA, BE, CH, PL, DI, PL, LE, SE, LE, CH, PL, GR, SE, LE, CH, GR, BA, CH, LE, DI, BA, BE, GR],
    &[LE, BE, PL, CH, BE, LE, BA, CH, GR, DI, CH, LE, CH, PL, GR, W, PL, BA, W, LE, S, BA, PL, DI, CH, GR, W, BE, SE, LE, GR, SE],
    &[SE, LE, GR, PL, GR, CH, W, BA, W, BE, GR, CH, BE, CH, PL, CH, DI, LE, SE, PL, LE, DI, CH, PL, S, BE, GR, LE, W, BA, LE, BA],
    &[CH, LE, BE, CH, BA, SE, LE, W, CH, S, PL, GR, DI, PL, GR, PL, LE, BE, BA, SE, BE, DI, GR, CH, BA, LE, W, LE, GR, CH, W, PL],
    &[GR, S, LE, BA, CH, SE, LE, BA, GR, BE, PL, CH, BE, W, BA, LE, GR, CH, DI, GR, BE, PL, LE, DI, PL, SE, PL, CH, LE, CH],
];

/// The twenty lines: the row (0 top, 1 middle, 2 bottom) on each reel.
pub const PAYLINES: [[u8; REELS]; LINES] = [
    [1, 1, 1, 1, 1],
    [0, 0, 0, 0, 0],
    [2, 2, 2, 2, 2],
    [0, 1, 2, 1, 0],
    [2, 1, 0, 1, 2],
    [0, 0, 1, 0, 0],
    [2, 2, 1, 2, 2],
    [1, 2, 2, 2, 1],
    [1, 0, 0, 0, 1],
    [1, 0, 1, 0, 1],
    [1, 2, 1, 2, 1],
    [0, 1, 0, 1, 0],
    [2, 1, 2, 1, 2],
    [1, 1, 0, 1, 1],
    [1, 1, 2, 1, 1],
    [0, 1, 1, 1, 0],
    [2, 1, 1, 1, 2],
    [0, 2, 0, 2, 0],
    [2, 0, 2, 0, 2],
    [0, 2, 2, 2, 0],
];

/// A line that won.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineWin {
    /// Which line (0 to 19).
    pub line: u8,
    /// The symbol it paid for, and how many in a row from the left.
    pub symbol: u8,
    pub count: u8,
    /// Chips (multiplier included).
    pub pay: u64,
}

/// One spin (the first of a round, or a free one).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spin {
    /// Where each reel stopped (the top row's place on its strip).
    pub stops: [u8; REELS],
    pub wins: Vec<LineWin>,
    /// Stars showing, and what they paid.
    pub stars: u8,
    pub star_pay: u64,
    /// Free spins this spin started (or added).
    pub free_spins: u8,
    pub free: bool,
    /// Everything this spin paid.
    pub pay: u64,
}

impl Spin {
    /// The symbols showing: `grid[reel][row]`.
    pub fn grid(&self) -> [[u8; ROWS]; REELS] {
        grid(&self.stops)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub line_bet: u64,
    /// The spin bought, then any free spins it led to.
    pub spins: Vec<Spin>,
    /// The jackpot's pool as the round began, and what of it the round won.
    pub jackpot_pool: u64,
    pub jackpot_won: u64,
    pub paid: u64,
}

pub fn grid(stops: &[u8; REELS]) -> [[u8; ROWS]; REELS] {
    let mut g = [[0u8; ROWS]; REELS];
    for (r, strip) in STRIPS.iter().enumerate() {
        for (row, cell) in g[r].iter_mut().enumerate() {
            *cell = strip[(stops[r] as usize + row) % strip.len()];
        }
    }
    g
}

/// What a line of symbols wins: (symbol, how many, times the line bet). The longer of a run of
/// wilds (paid as wilds) and a run of the first other symbol with wilds standing in.
pub fn line_win(syms: [u8; REELS]) -> Option<(u8, u8, u64)> {
    let wilds = syms.iter().take_while(|&&s| s == WILD).count();
    let mut best: Option<(u8, u8, u64)> = (wilds >= 3).then(|| (WILD, wilds as u8, PAYS[WILD as usize][wilds - 3]));
    if let Some(&first) = syms.iter().find(|&&s| s != WILD)
        && first != STAR
    {
        let n = syms.iter().take_while(|&&s| s == first || s == WILD).count();
        if n >= 3 {
            let pay = PAYS[first as usize][n - 3];
            if best.is_none_or(|b| pay > b.2) {
                best = Some((first, n as u8, pay));
            }
        }
    }
    best
}

/// One spin from the stops. `jackpot`: what five wilds on a line win instead of their line
/// pay (if more), once a round; set to 0 once it's won.
fn evaluate(stops: [u8; REELS], line_bet: u64, free: bool, jackpot: &mut u64, jackpot_won: &mut u64) -> Spin {
    let g = grid(&stops);
    let mult = if free { FREE_SPIN_MULTIPLIER } else { 1 };
    let mut wins = Vec::new();
    let mut pay = 0u64;
    for (i, line) in PAYLINES.iter().enumerate() {
        let syms: [u8; REELS] = std::array::from_fn(|r| g[r][line[r] as usize]);
        if let Some((symbol, count, times)) = line_win(syms) {
            let mut p = times * line_bet * mult;
            if symbol == WILD && count == 5 && *jackpot > 0 {
                // The Phoenix Jackpot: the pool, shared by how much of the biggest line bet this
                // one is (the fixed pay if that's more).
                let share = (*jackpot as u128 * line_bet as u128 / MAX_LINE_BET as u128) as u64;
                if share > p {
                    *jackpot_won = share;
                    p = share;
                }
                *jackpot = 0;
            }
            pay += p;
            wins.push(LineWin { line: i as u8, symbol, count, pay: p });
        }
    }
    let stars = g.iter().flatten().filter(|&&s| s == STAR).count();
    let (star_pay, free_spins) = if stars >= 3 {
        let k = stars.min(5) - 3;
        (STAR_PAYS[k] * line_bet * LINES as u64 * mult, STAR_SPINS[k] as u8)
    } else {
        (0, 0)
    };
    pay += star_pay;
    Spin { stops, wins, stars: stars as u8, star_pay, free_spins, free, pay }
}

fn stops(s: &mut Stream) -> [u8; REELS] {
    std::array::from_fn(|r| s.below(STRIPS[r].len() as u32) as u8)
}

/// A round: the spin bought and any free spins after it.
pub fn play(line_bet: u64, s: &mut Stream, jackpot_pool: u64) -> Outcome {
    let mut jackpot = jackpot_pool;
    let mut jackpot_won = 0;
    let first = evaluate(stops(s), line_bet, false, &mut jackpot, &mut jackpot_won);
    let mut left = first.free_spins as usize;
    let mut spins = vec![first];
    let mut played = 0;
    while left > 0 && played < MAX_FREE_SPINS {
        left -= 1;
        played += 1;
        let spin = evaluate(stops(s), line_bet, true, &mut jackpot, &mut jackpot_won);
        left += spin.free_spins as usize;
        spins.push(spin);
    }
    let paid = spins.iter().map(|x| x.pay).sum::<u64>().min(super::MAX_PAYOUT);
    Outcome { line_bet, spins, jackpot_pool, jackpot_won, paid }
}

/// The jackpot's share of a spin's bet.
pub fn jackpot_share(line_bet: u64) -> u64 {
    (line_bet * LINES as u64 * JACKPOT_SHARE_BP).div_ceil(10_000)
}

#[cfg(test)]
pub fn tests_evaluate(stops: [u8; REELS], line_bet: u64, jackpot: &mut u64, jackpot_won: &mut u64) -> Spin {
    evaluate(stops, line_bet, false, jackpot, jackpot_won)
}
