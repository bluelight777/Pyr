//! Dice: a roll from 0.00 to 99.99. Pick a chance; the roll has to come in under it (or, rolling
//! over, in the top of the range). It pays 99 over the chance: 2% pays 49.5 times the bet, 50%
//! pays 1.98 times. The house keeps 1%.

use serde::{Deserialize, Serialize};

use crate::fair::Stream;

/// Chances, in hundredths of a percent.
pub const MIN_CHANCE: u16 = 200;
pub const MAX_CHANCE: u16 = 9_800;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub chips: u64,
    pub chance: u16,
    pub over: bool,
    /// 0 to 9,999 (shown as 0.00 to 99.99).
    pub roll: u16,
    pub won: bool,
    pub paid: u64,
}

/// What a win pays back for a chance (stake included).
pub fn payout(chips: u64, chance: u16) -> u64 {
    ((chips as u128 * 990_000 / chance as u128) / 100) as u64
}

/// The multiplier for a chance, in hundredths ("x1.98").
pub fn multiplier_x100(chance: u16) -> u64 {
    990_000 / chance as u64
}

/// Where a roll has to land to win: below `chance` (under), or at `10,000 - chance` or more.
pub fn wins(roll: u16, chance: u16, over: bool) -> bool {
    if over { roll >= 10_000 - chance } else { roll < chance }
}

pub fn show(roll: u16) -> String {
    format!("{}.{:02}", roll / 100, roll % 100)
}

pub fn play(chips: u64, chance: u16, over: bool, s: &mut Stream) -> Outcome {
    let roll = s.below(10_000) as u16;
    let won = wins(roll, chance, over);
    let paid = if won { payout(chips, chance).min(super::MAX_PAYOUT) } else { 0 };
    Outcome { chips, chance, over, roll, won, paid }
}
