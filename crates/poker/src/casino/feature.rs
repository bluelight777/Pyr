//! Play Feature (0.13.0): what a slot spin can be besides a spin, the way Stake's slots have
//! it. Pay more for a spin with better chances of the bonus, or buy the bonus outright, a bigger
//! one, a mystery one, or an all-or-nothing shot. Each is priced from the game's exact math so
//! it pays back what the game does (about 99%): a bonus that's worth 59.4 times the bet costs 60.
//!
//! What each slot offers, and what each costs, is the slot's own (`reelrow::price`,
//! `slots::price`, `dragon::price`).

use serde::{Deserialize, Serialize};

/// What's bought with a spin.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Feature {
    /// A spin, as ever.
    #[default]
    Spin,
    /// A spin with the bonus several times as likely.
    Chance,
    /// A spin with a reel all wild (Reel Row: the middle reel; Pyr Sevens: the middle reel of
    /// flames; Ember Dragon: the middle reel of wilds, with the dragon's multiplier).
    Wild,
    /// The bonus, straight away.
    Bonus,
    /// A bigger bonus (Reel Row: every prize tripled; Pyr Sevens: free spins at 10× instead of
    /// 3×; Ember Dragon: Ember Respins with the dragon full grown).
    Super,
    /// A bonus that might be the bigger one.
    Mystery,
    /// One shot at the game's biggest prize, or nothing.
    AllIn,
}

impl Feature {
    pub const ALL: [Feature; 7] = [Feature::Spin, Feature::Chance, Feature::Wild, Feature::Bonus, Feature::Super, Feature::Mystery, Feature::AllIn];

    pub fn is_spin(&self) -> bool {
        *self == Feature::Spin
    }

    /// Whether it plays a spin of the reels (the rest go straight to their bonus or prize).
    pub fn spins(self) -> bool {
        matches!(self, Feature::Spin | Feature::Chance | Feature::Wild)
    }

    /// A short id (settings, statistics).
    pub fn id(self) -> &'static str {
        match self {
            Feature::Spin => "spin",
            Feature::Chance => "chance",
            Feature::Wild => "wild",
            Feature::Bonus => "bonus",
            Feature::Super => "super",
            Feature::Mystery => "mystery",
            Feature::AllIn => "all_in",
        }
    }
}

/// What a spin with `feature` costs at `bet`, its price being `price_h` hundredths of the bet
/// (rounded to the nearest chip).
pub fn cost(bet: u64, price_h: u64) -> u64 {
    ((bet as u128 * price_h as u128 + 50) / 100) as u64
}
