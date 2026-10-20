//! Craps: the pass line and its odds, don't pass, place bets, hardways, and the one-roll field,
//! any seven and any craps.
//!
//! A shooter's first roll (the come-out) wins the pass line on 7 or 11 and loses it on 2, 3 or
//! 12; anything else becomes the point. Then the pass line wins if the point comes again before
//! a 7. Place bets and hardways stay up until they lose (they're off on the come-out roll).
//!
//! Each roll is its own fair round: the dice come from that roll's seeds.

use serde::{Deserialize, Serialize};

use crate::fair::Stream;

pub const PLACE_NUMBERS: [u8; 6] = [4, 5, 6, 8, 9, 10];
pub const HARD_NUMBERS: [u8; 4] = [4, 6, 8, 10];
/// The most on the table at once.
pub const MAX_ON_TABLE: u64 = 50_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bets {
    #[serde(default)]
    pub pass: u64,
    #[serde(default)]
    pub pass_odds: u64,
    #[serde(default)]
    pub dont_pass: u64,
    #[serde(default)]
    pub field: u64,
    #[serde(default)]
    pub any_seven: u64,
    #[serde(default)]
    pub any_craps: u64,
    /// On 4, 5, 6, 8, 9 and 10.
    #[serde(default)]
    pub place: [u64; 6],
    /// Hard 4, 6, 8 and 10.
    #[serde(default)]
    pub hard: [u64; 4],
}

impl Bets {
    pub fn total(&self) -> u64 {
        let mut t = self.pass.saturating_add(self.pass_odds).saturating_add(self.dont_pass).saturating_add(self.field).saturating_add(self.any_seven).saturating_add(self.any_craps);
        for p in self.place.iter().chain(self.hard.iter()) {
            t = t.saturating_add(*p);
        }
        t
    }

    pub fn is_empty(&self) -> bool {
        self.total() == 0
    }
}

/// The table between rolls.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Table {
    pub point: Option<u8>,
    pub bets: Bets,
    /// Points made by this shooter (a seven-out starts over).
    #[serde(default)]
    pub streak: u32,
}

/// How many times the pass line the odds may be: 3, 4, 5 times on 4/10, 5/9, 6/8.
pub fn max_odds(point: u8) -> u64 {
    match point {
        4 | 10 => 3,
        5 | 9 => 4,
        _ => 5,
    }
}

/// What a winning bet of `stake` wins (not counting the stake), true odds for the pass odds.
fn odds_win(point: u8, stake: u64) -> u64 {
    match point {
        4 | 10 => stake * 2,
        5 | 9 => stake * 3 / 2,
        _ => stake * 6 / 5,
    }
}

fn place_win(n: u8, stake: u64) -> u64 {
    match n {
        4 | 10 => stake * 9 / 5,
        5 | 9 => stake * 7 / 5,
        _ => stake * 7 / 6,
    }
}

fn hard_win(n: u8, stake: u64) -> u64 {
    match n {
        4 | 10 => stake * 7,
        _ => stake * 9,
    }
}

pub fn place_name(i: usize) -> String {
    format!("Place {}", PLACE_NUMBERS[i])
}

pub fn hard_name(i: usize) -> String {
    format!("Hard {}", HARD_NUMBERS[i])
}

/// Whether the table may go from its bets to `new` before the next roll.
pub fn check_change(t: &Table, new: &Bets) -> Result<(), String> {
    if new.total() > MAX_ON_TABLE {
        return Err(format!("the most on the table at once is {MAX_ON_TABLE}"));
    }
    match t.point {
        None => {
            if new.pass_odds > 0 {
                return Err("odds go behind the pass line once there's a point".into());
            }
        }
        Some(p) => {
            if new.pass != t.bets.pass {
                return Err("the pass line can't be changed once there's a point".into());
            }
            if new.pass_odds > new.pass * max_odds(p) {
                return Err(format!("odds on {p} go up to {} times the pass line", max_odds(p)));
            }
            if new.dont_pass > t.bets.dont_pass {
                return Err("don't pass goes down before the come-out roll".into());
            }
        }
    }
    Ok(())
}

pub fn roll(s: &mut Stream) -> (u8, u8) {
    (1 + s.below(6) as u8, 1 + s.below(6) as u8)
}

/// One bet decided by a roll.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decided {
    pub bet: String,
    pub stake: u64,
    pub won: bool,
    /// Chips back to the player (winnings, and the stake when it comes down too).
    pub returned: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub dice: (u8, u8),
    pub point_before: Option<u8>,
    pub point_after: Option<u8>,
    pub before: Bets,
    pub after: Bets,
    pub decided: Vec<Decided>,
    /// Stakes the roll decided, chips back to the player, and the stakes of winners that stay
    /// up (place bets, hardways).
    pub wagered: u64,
    pub paid: u64,
    pub kept: u64,
    /// A shooter's points in a row, after this roll.
    pub streak: u32,
}

/// Settles a roll.
pub fn settle(t: &Table, dice: (u8, u8)) -> (Table, Outcome) {
    let sum = dice.0 + dice.1;
    let hard = dice.0 == dice.1;
    let mut b = t.bets.clone();
    let mut decided = Vec::new();
    let mut point = t.point;
    let mut streak = t.streak;
    let mut kept = 0;
    let decide = |name: String, stake: u64, won: bool, returned: u64, decided: &mut Vec<Decided>| {
        if stake > 0 {
            decided.push(Decided { bet: name, stake, won, returned });
        }
    };
    match t.point {
        None => {
            match sum {
                7 | 11 => {
                    decide("Pass line".into(), b.pass, true, b.pass * 2, &mut decided);
                    decide("Don't pass".into(), b.dont_pass, false, 0, &mut decided);
                    b.pass = 0;
                    b.dont_pass = 0;
                }
                2 | 3 | 12 => {
                    decide("Pass line".into(), b.pass, false, 0, &mut decided);
                    b.pass = 0;
                    if sum == 12 {
                        // (Don't pass pushes on 12.)
                    } else {
                        decide("Don't pass".into(), b.dont_pass, true, b.dont_pass * 2, &mut decided);
                        b.dont_pass = 0;
                    }
                }
                n => point = Some(n),
            }
        }
        Some(p) => {
            if sum == p {
                let odds = b.pass_odds;
                decide("Pass line".into(), b.pass, true, b.pass * 2, &mut decided);
                decide("Odds".into(), odds, true, odds + odds_win(p, odds), &mut decided);
                decide("Don't pass".into(), b.dont_pass, false, 0, &mut decided);
                b.pass = 0;
                b.pass_odds = 0;
                b.dont_pass = 0;
                point = None;
                streak += 1;
            } else if sum == 7 {
                decide("Pass line".into(), b.pass, false, 0, &mut decided);
                decide("Odds".into(), b.pass_odds, false, 0, &mut decided);
                decide("Don't pass".into(), b.dont_pass, true, b.dont_pass * 2, &mut decided);
                b.pass = 0;
                b.pass_odds = 0;
                b.dont_pass = 0;
                point = None;
                streak = 0;
            }
            // Place bets and hardways (working once there's a point).
            for (i, n) in PLACE_NUMBERS.iter().enumerate() {
                let stake = b.place[i];
                if stake == 0 {
                    continue;
                }
                if sum == *n {
                    let w = place_win(*n, stake);
                    kept += stake;
                    decide(place_name(i), stake, true, w, &mut decided);
                } else if sum == 7 {
                    decide(place_name(i), stake, false, 0, &mut decided);
                    b.place[i] = 0;
                }
            }
            for (i, n) in HARD_NUMBERS.iter().enumerate() {
                let stake = b.hard[i];
                if stake == 0 {
                    continue;
                }
                if sum == *n && hard {
                    kept += stake;
                    decide(hard_name(i), stake, true, hard_win(*n, stake), &mut decided);
                } else if sum == *n || sum == 7 {
                    decide(hard_name(i), stake, false, 0, &mut decided);
                    b.hard[i] = 0;
                }
            }
        }
    }
    // One-roll bets, every roll.
    let field_win = match sum {
        2 => Some(2),
        12 => Some(3),
        3 | 4 | 9 | 10 | 11 => Some(1),
        _ => None,
    };
    decide("Field".into(), b.field, field_win.is_some(), field_win.map(|m| b.field * (m + 1)).unwrap_or(0), &mut decided);
    b.field = 0;
    decide("Any seven".into(), b.any_seven, sum == 7, if sum == 7 { b.any_seven * 5 } else { 0 }, &mut decided);
    b.any_seven = 0;
    let craps = matches!(sum, 2 | 3 | 12);
    decide("Any craps".into(), b.any_craps, craps, if craps { b.any_craps * 8 } else { 0 }, &mut decided);
    b.any_craps = 0;
    let wagered = decided.iter().map(|d| d.stake).sum();
    let paid = decided.iter().map(|d| d.returned).sum::<u64>().min(super::MAX_PAYOUT);
    let after = Table { point, bets: b.clone(), streak };
    let o = Outcome { dice, point_before: t.point, point_after: point, before: t.bets.clone(), after: b, decided, wagered, paid, kept, streak };
    (after, o)
}

pub fn play(t: &Table, s: &mut Stream) -> (Table, Outcome) {
    settle(t, roll(s))
}
