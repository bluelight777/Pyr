//! MMR: one rating for everyone who plays poker on a server, people and bots alike (each bot a
//! named player of its own, its rating kept like anyone's).
//!
//! It moves the way chess ratings do (Elo), result against expectation: beating someone rated
//! higher moves it more than beating someone rated lower, and losing to them costs less.
//!
//! - Tournaments and Flash Pots: by finishing place, every pair of entrants a game between
//!   them (the better placed won it).
//! - Cash hands: chips won against chips lost, each winner against each loser, weighted by the
//!   big blinds between them (a pot fought over moves ratings; a walk barely does).
//!
//! New players move faster for their first games (until the rating has found its level).

use serde::{Deserialize, Serialize};

use crate::bots::Level;

/// Where everyone starts.
pub const START: f64 = 1_000.0;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Rating {
    pub mmr: f64,
    pub peak: f64,
    /// Tournaments (and Flash Pots) finished, and cash hands played, rated.
    pub tourneys: u32,
    pub hands: u32,
}

impl Default for Rating {
    fn default() -> Rating {
        Rating { mmr: START, peak: START, tourneys: 0, hands: 0 }
    }
}

impl Rating {
    /// A bot's first rating: what its level plays like.
    pub fn for_bot(level: Level) -> Rating {
        let mmr = match level {
            Level::Rookie => 800.0,
            Level::Regular => 1_000.0,
            Level::Shark => 1_150.0,
            Level::Genius => 1_450.0,
        };
        Rating { mmr, peak: mmr, ..Rating::default() }
    }

    /// Still finding its level: a tournament counts as much as fifty hands.
    pub fn placing(&self) -> bool {
        self.tourneys * 50 + self.hands < 300
    }

    fn apply(&mut self, delta: f64) {
        self.mmr = (self.mmr + delta).max(0.0);
        self.peak = self.peak.max(self.mmr);
    }

    pub fn tier(&self) -> Tier {
        Tier::of(self.mmr)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Bronze,
    Silver,
    Gold,
    Platinum,
    Diamond,
    Master,
    Grandmaster,
}

impl Tier {
    pub const ALL: [Tier; 7] = [Tier::Bronze, Tier::Silver, Tier::Gold, Tier::Platinum, Tier::Diamond, Tier::Master, Tier::Grandmaster];

    pub fn of(mmr: f64) -> Tier {
        match mmr as i64 {
            ..900 => Tier::Bronze,
            900..1_100 => Tier::Silver,
            1_100..1_300 => Tier::Gold,
            1_300..1_500 => Tier::Platinum,
            1_500..1_700 => Tier::Diamond,
            1_700..1_900 => Tier::Master,
            _ => Tier::Grandmaster,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tier::Bronze => "Bronze",
            Tier::Silver => "Silver",
            Tier::Gold => "Gold",
            Tier::Platinum => "Platinum",
            Tier::Diamond => "Diamond",
            Tier::Master => "Master",
            Tier::Grandmaster => "Grandmaster",
        }
    }

    /// Where it starts (its lowest rating).
    pub fn floor(self) -> f64 {
        match self {
            Tier::Bronze => 0.0,
            Tier::Silver => 900.0,
            Tier::Gold => 1_100.0,
            Tier::Platinum => 1_300.0,
            Tier::Diamond => 1_500.0,
            Tier::Master => 1_700.0,
            Tier::Grandmaster => 1_900.0,
        }
    }
}

/// The chance `a` beats `b`, by their ratings.
pub fn expected(a: f64, b: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((b - a) / 400.0))
}

/// A tournament's K (the most one result can move a rating), faster while placing.
fn k_tourney(r: &Rating) -> f64 {
    if r.placing() { 48.0 } else { 24.0 }
}

fn k_hand(r: &Rating) -> f64 {
    if r.placing() { 10.0 } else { 5.0 }
}

/// Rates a finished tournament: `places[i]` is where `ratings[i]` finished (1 the winner; a
/// shared place is a draw). Every pair is a game between them. Changes the ratings in place.
pub fn rate_places(ratings: &mut [Rating], places: &[usize]) {
    let n = ratings.len();
    if n < 2 || places.len() != n {
        return;
    }
    let before: Vec<f64> = ratings.iter().map(|r| r.mmr).collect();
    let deltas: Vec<f64> = (0..n)
        .map(|i| {
            let k = k_tourney(&ratings[i]);
            let sum: f64 = (0..n)
                .filter(|j| *j != i)
                .map(|j| {
                    let s = match places[i].cmp(&places[j]) {
                        std::cmp::Ordering::Less => 1.0,
                        std::cmp::Ordering::Equal => 0.5,
                        std::cmp::Ordering::Greater => 0.0,
                    };
                    s - expected(before[i], before[j])
                })
                .sum();
            k * sum / (n - 1) as f64
        })
        .collect();
    for (r, d) in ratings.iter_mut().zip(deltas) {
        r.apply(d);
        r.tourneys += 1;
    }
}

/// Big blinds between two players that make a hand count fully.
const FULL_POT_BB: f64 = 25.0;

/// Rates a cash hand: `net_bb[i]` is what `ratings[i]` won or lost in it (big blinds). Each
/// winner against each loser, weighted by the chips between them. Changes the ratings in place.
pub fn rate_hand(ratings: &mut [Rating], net_bb: &[f64]) {
    let n = ratings.len();
    if n < 2 || net_bb.len() != n {
        return;
    }
    let before: Vec<f64> = ratings.iter().map(|r| r.mmr).collect();
    let mut deltas = vec![0.0; n];
    for i in 0..n {
        for j in 0..n {
            if i == j || net_bb[i] <= 0.0 || net_bb[j] >= 0.0 {
                continue;
            }
            // i won chips, j lost them: a game i won, as big as what passed between them.
            let at_stake = net_bb[i].min(-net_bb[j]);
            let w = (at_stake / FULL_POT_BB).min(1.0);
            let e = expected(before[i], before[j]);
            deltas[i] += k_hand(&ratings[i]) * w * (1.0 - e);
            deltas[j] -= k_hand(&ratings[j]) * w * (1.0 - e);
        }
    }
    for (r, d) in ratings.iter_mut().zip(deltas) {
        r.apply(d);
        r.hands += 1;
    }
}

/// Someone's rating id: a person's Pyr id; a bot's name and level (the same bot wherever it
/// sits, its rating its own).
pub fn id_of(uid: &str, name: &str, bot: Option<Level>) -> String {
    match bot {
        Some(l) => format!("bot:{}:{}", l.name().to_lowercase(), name.split_whitespace().next().unwrap_or(name)),
        None => uid.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn winners_climb_and_upsets_count_more() {
        let mut r = vec![Rating::default(); 4];
        rate_places(&mut r, &[1, 2, 3, 4]);
        assert!(r[0].mmr > r[1].mmr && r[1].mmr > START && r[2].mmr < START && r[3].mmr < r[2].mmr);
        // What one gains another loses (nobody's rating made from nothing).
        let total: f64 = r.iter().map(|x| x.mmr).sum();
        assert!((total - 4.0 * START).abs() < 1e-6);
        // Beating someone rated higher moves it more than beating someone lower.
        let mut up = vec![Rating { mmr: 1_000.0, ..Default::default() }, Rating { mmr: 1_400.0, ..Default::default() }];
        let mut down = vec![Rating { mmr: 1_000.0, ..Default::default() }, Rating { mmr: 600.0, ..Default::default() }];
        rate_places(&mut up, &[1, 2]);
        rate_places(&mut down, &[1, 2]);
        assert!(up[0].mmr - 1_000.0 > 3.0 * (down[0].mmr - 1_000.0));
        assert_eq!(up[0].tourneys, 1);
    }

    #[test]
    fn hands_move_ratings_by_what_was_at_stake() {
        let mut big = vec![Rating::default(); 2];
        rate_hand(&mut big, &[40.0, -40.0]);
        let mut walk = vec![Rating::default(); 2];
        rate_hand(&mut walk, &[0.5, -0.5]);
        assert!(big[0].mmr - START > 20.0 * (walk[0].mmr - START));
        assert!((big[0].mmr + big[1].mmr - 2.0 * START).abs() < 1e-6, "zero sum");
        // Nobody won or lost: nothing moves (but the hand is counted).
        let mut none = vec![Rating::default(); 3];
        rate_hand(&mut none, &[0.0, 0.0, 0.0]);
        assert!(none.iter().all(|r| r.mmr == START && r.hands == 1));
    }

    /// Ratings find the level each plays at: a strong player rated with weaker ones rises
    /// and settles above them.
    #[test]
    fn ratings_settle_in_order() {
        let mut r = vec![Rating::default(); 3];
        // The first always wins, the second beats the third two times in three.
        for g in 0..200 {
            let places = if g % 3 == 0 { [1, 3, 2] } else { [1, 2, 3] };
            rate_places(&mut r, &places);
        }
        assert!(r[0].mmr > r[1].mmr + 150.0 && r[1].mmr > r[2].mmr);
        assert_eq!(r[0].tier(), Tier::of(r[0].mmr));
        assert!(r[0].peak >= r[0].mmr);
    }

    #[test]
    fn tiers_and_ids() {
        assert_eq!(Tier::of(899.0), Tier::Bronze);
        assert_eq!(Tier::of(1_000.0), Tier::Silver);
        assert_eq!(Tier::of(1_450.0), Tier::Platinum);
        assert_eq!(Tier::of(2_400.0), Tier::Grandmaster);
        for t in Tier::ALL {
            assert_eq!(Tier::of(t.floor()), t);
        }
        assert_eq!(id_of("bot:7:2:123", "Blaze", Some(Level::Shark)), "bot:shark:Blaze");
        assert_eq!(id_of("bot:t3:21", "Ace 2", Some(Level::Rookie)), "bot:rookie:Ace");
        assert_eq!(id_of("u123", "Bluelight", None), "u123");
    }
}
