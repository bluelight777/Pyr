//! Bots that fill seats. They play fair: a bot sees what a player in its seat would (its own
//! cards, the board, the chips) and nothing else. Each level plays its own way:
//!
//! - **Rookie**: loose and passive, calls too much, rarely bluffs. Good to learn against.
//! - **Regular**: tight and aggressive, plays its strong hands hard and folds the rest.
//! - **Shark**: works out its chances against the hands others are likely to hold, thinks about
//!   the price of a call and where it sits, bets for value and bluffs at the right times.
//!
//! Their chances come from dealing out the rest of the hand many times (Monte Carlo), so they
//! play Omaha as well as Hold'em.

use serde::{Deserialize, Serialize};

use crate::cards::{Card, CardSet};
use crate::eval::{self, Value};
use crate::fair::Stream;
use crate::game::{Action, Legal, Street, Table, Variant};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Rookie,
    Regular,
    Shark,
}

impl Level {
    pub fn name(self) -> &'static str {
        match self {
            Level::Rookie => "Rookie",
            Level::Regular => "Regular",
            Level::Shark => "Shark",
        }
    }

    fn style(self) -> Style {
        match self {
            // tight: the chances it wants before playing on; aggression: how often it bets or
            // raises with a good hand rather than calling; bluff: how often it bets with nothing;
            // noise: how far off its sense of its chances is.
            Level::Rookie => Style { tight: 0.75, aggression: 0.25, bluff: 0.03, noise: 0.15, sims: 150 },
            Level::Regular => Style { tight: 1.0, aggression: 0.6, bluff: 0.08, noise: 0.07, sims: 300 },
            Level::Shark => Style { tight: 1.05, aggression: 0.75, bluff: 0.16, noise: 0.03, sims: 600 },
        }
    }
}

struct Style {
    tight: f64,
    aggression: f64,
    bluff: f64,
    noise: f64,
    sims: usize,
}

/// What a bot sees when it's its turn.
#[derive(Clone, Debug)]
pub struct View {
    pub variant: Variant,
    pub hole: Vec<Card>,
    pub board: Vec<Card>,
    pub street: Street,
    pub legal: Legal,
    /// All chips in the middle (including this round's).
    pub pot: u64,
    /// Others still in the hand.
    pub opponents: usize,
    pub big_blind: u64,
    /// Chips it has behind.
    pub stack: u64,
    /// Chips in front of it this round.
    pub in_front: u64,
    /// Whether it acts last this round (on the button, or the last one left to act).
    pub in_position: bool,
    /// Whether someone raised before the flop (others' hands are stronger then).
    pub raised_preflop: bool,
}

impl Table {
    /// What the player in `seat` sees, for a bot deciding there.
    pub fn bot_view(&self, seat: usize) -> Option<View> {
        let hs = self.hand.as_ref()?;
        let legal = self.legal(seat)?;
        let me = hs.player(seat)?;
        let live: Vec<usize> = hs.players.iter().filter(|p| !p.folded).map(|p| p.seat).collect();
        // Last to act after the flop: the live seat closest before the button (inclusive).
        let n = self.seats.len();
        let last = (0..n).map(|d| (hs.button + n - d) % n).find(|s| live.contains(s));
        Some(View {
            variant: self.rules.variant,
            hole: me.hole.clone(),
            board: hs.board.clone(),
            street: hs.street,
            legal,
            pot: hs.pot(),
            opponents: live.len().saturating_sub(1),
            big_blind: self.rules.big_blind,
            stack: self.behind(seat),
            in_front: me.street_bet,
            in_position: last == Some(seat),
            raised_preflop: hs.street != Street::Preflop && hs.last_aggressor.is_some() || hs.raises > 1,
        })
    }
}

/// The chance of winning (ties count by share) against `opponents` random hands, dealing the
/// rest of the board out `sims` times.
pub fn equity(variant: Variant, hole: &[Card], board: &[Card], opponents: usize, sims: usize, rng: &mut Stream) -> f64 {
    if opponents == 0 {
        return 1.0;
    }
    let used = CardSet::of(&[hole, board].concat());
    let rest: Vec<Card> = Card::all().filter(|c| !used.contains(*c)).collect();
    let k = hole.len();
    let need = opponents * k + (5 - board.len());
    if need > rest.len() {
        return 1.0 / (opponents + 1) as f64;
    }
    let mut score = 0.0;
    let mut deck = rest.clone();
    for _ in 0..sims {
        // A partial shuffle: just the cards needed.
        for i in 0..need {
            let j = i + rng.below((deck.len() - i) as u32) as usize;
            deck.swap(i, j);
        }
        let mut b = board.to_vec();
        b.extend(&deck[opponents * k..need]);
        let val = |h: &[Card]| -> Value {
            match variant {
                Variant::Holdem => eval::evaluate(&[h, &b].concat()),
                Variant::Omaha => eval::omaha_value(h, &b),
            }
        };
        let mine = val(hole);
        let mut best_other = Value(0);
        let mut ties = 0;
        for o in 0..opponents {
            let v = val(&deck[o * k..o * k + k]);
            if v > best_other {
                best_other = v;
                ties = 0;
            }
            if v == mine {
                ties += 1;
            }
        }
        if mine > best_other {
            score += 1.0;
        } else if mine == best_other {
            score += 1.0 / (ties + 1) as f64;
        }
    }
    score / sims as f64
}

/// A quick sense of a Hold'em starting hand (the Chen formula), 0 to 1.
pub fn preflop_strength(hole: &[Card]) -> f64 {
    if hole.len() != 2 {
        return 0.5;
    }
    let (a, b) = (hole[0], hole[1]);
    let (hi, lo) = if a.rank() >= b.rank() { (a, b) } else { (b, a) };
    let base = |r: u8| -> f64 {
        match r {
            12 => 10.0,
            11 => 8.0,
            10 => 7.0,
            9 => 6.0,
            r => (r as f64 + 2.0) / 2.0,
        }
    };
    let mut s = base(hi.rank());
    if hi.rank() == lo.rank() {
        s = (s * 2.0).max(5.0);
    } else {
        if hi.suit() == lo.suit() {
            s += 2.0;
        }
        let gap = hi.rank() - lo.rank() - 1;
        s -= match gap {
            0 => 0.0,
            1 => 1.0,
            2 => 2.0,
            3 => 4.0,
            _ => 5.0,
        };
        if gap <= 1 && hi.rank() < 10 {
            s += 1.0;
        }
    }
    // Chen scores run from about -1 (72o) to 20 (AA).
    ((s + 1.0) / 21.0).clamp(0.0, 1.0)
}

/// What the bot does.
pub fn decide(view: &View, level: Level, rng: &mut Stream) -> Action {
    let st = level.style();
    let l = view.legal;
    let roll = |rng: &mut Stream| rng.below(10_000) as f64 / 10_000.0;
    // Its chances, a little off for the weaker bots, and lower when others showed strength.
    let mut eq = if view.street == Street::Preflop && view.variant == Variant::Holdem {
        // Against several players the best hands still win less often.
        let s = preflop_strength(&view.hole);
        s.powf(1.0 + 0.35 * view.opponents.saturating_sub(1) as f64)
    } else {
        equity(view.variant, &view.hole, &view.board, view.opponents.max(1), st.sims, rng)
    };
    if view.raised_preflop && view.street != Street::Preflop {
        eq *= 0.92;
    }
    eq = (eq + (roll(rng) - 0.5) * 2.0 * st.noise).clamp(0.0, 1.0);
    // The price of a call.
    let call = l.call as f64;
    let pot_odds = if call > 0.0 { call / (view.pot as f64 + call) } else { 0.0 };
    // What a fair share of the pot would be.
    let fair = 1.0 / (view.opponents + 1) as f64;
    let strong = eq > fair * 1.9 * st.tight || eq > 0.72;
    let good = eq > fair * 1.25 * st.tight;
    let position_bonus = if view.in_position { 0.04 } else { 0.0 };

    let raise_to = |frac: f64, rng: &mut Stream| -> u64 {
        // Call, then bet `frac` of the pot that makes (a little more or less, so it doesn't
        // give its hand away), between what's allowed.
        let pot_after = view.pot as f64 + call;
        let wobble = 0.85 + roll(rng) * 0.3;
        let target = view.in_front as f64 + call + pot_after * frac * wobble;
        (target as u64).clamp(l.min_to, l.max_to)
    };

    if strong && l.can_raise && roll(rng) < st.aggression + position_bonus {
        // Value: bigger when very strong; all-in with the nuts-ish when short.
        let frac = if eq > 0.85 { 1.0 } else { 0.66 };
        let to = raise_to(frac, rng);
        if to >= l.max_to {
            return Action::AllIn;
        }
        return Action::Raise { to };
    }
    if l.can_check {
        // Nobody bet: bet good hands sometimes, bluff now and then (more in position).
        if l.can_raise && (good && roll(rng) < st.aggression * 0.7 || roll(rng) < st.bluff + position_bonus) {
            let to = raise_to(0.5, rng);
            return Action::Raise { to };
        }
        return Action::Check;
    }
    // Facing a bet: call when the chances beat the price (rookies call more).
    let margin = match level {
        Level::Rookie => -0.08,
        Level::Regular => 0.02,
        Level::Shark => 0.0,
    };
    if eq + position_bonus > pot_odds + margin || strong {
        // A bluff-raise now and then, from the shark.
        if level == Level::Shark && l.can_raise && view.street >= Street::Flop && roll(rng) < st.bluff * 0.4 {
            return Action::Raise { to: raise_to(0.75, rng) };
        }
        return Action::Call;
    }
    // A cheap call with something to draw to.
    if pot_odds < 0.15 && eq > fair * 0.6 {
        return Action::Call;
    }
    Action::Fold
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::parse_cards;
    use crate::fair::{seed_from, shuffle};
    use crate::game::{Event, Rules};

    #[test]
    fn equity_is_about_right() {
        let mut rng = Stream::new(seed_from(1));
        let aa = parse_cards("AhAd").unwrap();
        let e = equity(Variant::Holdem, &aa, &[], 1, 4000, &mut rng);
        assert!((0.82..0.88).contains(&e), "aces against one random hand: {e}");
        let e = equity(Variant::Holdem, &parse_cards("7c2d").unwrap(), &[], 1, 4000, &mut rng);
        assert!((0.30..0.38).contains(&e), "seven-two: {e}");
        let nuts = equity(Variant::Holdem, &parse_cards("AhKh").unwrap(), &parse_cards("QhJhTh2c3d").unwrap(), 3, 300, &mut rng);
        assert_eq!(nuts, 1.0);
    }

    #[test]
    fn chen_orders_hands() {
        let s = |h: &str| preflop_strength(&parse_cards(h).unwrap());
        assert!(s("AhAd") > s("KhKd"));
        assert!(s("KhKd") > s("AhKh"));
        assert!(s("AhKh") > s("AhKd"));
        assert!(s("AhKd") > s("9h8h"));
        assert!(s("9h8h") > s("7c2d"));
    }

    /// Bots play many hands against each other: everything they do is legal, and the better
    /// bots win chips from the weaker over time.
    #[test]
    fn sharks_beat_rookies() {
        let mut rng = Stream::new(seed_from(9));
        let mut shark_total: i64 = 0;
        let levels = [Level::Shark, Level::Rookie, Level::Rookie, Level::Regular];
        let mut by_level = [0i64; 3];
        for game in 0..40u64 {
            let mut t = Table::new(Rules::holdem(5, 10, 4));
            for i in 0..4 {
                t.sit(i, &format!("bot{i}"), "x", 1000).unwrap();
            }
            for hand in 0..60u64 {
                if t.ready().len() < 2 {
                    break;
                }
                let mut out = t.start_hand(hand, shuffle(seed_from(game * 997 + hand))).unwrap();
                loop {
                    let next = out.iter().rev().find_map(|(_, e)| match e {
                        Event::Turn { seat, .. } => Some(Some(*seat)),
                        Event::HandEnded { .. } => Some(None),
                        _ => None,
                    });
                    let Some(Some(seat)) = next else { break };
                    let view = t.bot_view(seat).unwrap();
                    let a = decide(&view, levels[seat], &mut rng);
                    out = t.act(seat, a).unwrap_or_else(|e| panic!("{a:?} with {:?}: {e}", view.legal));
                }
            }
            for (i, s) in t.seats.iter().enumerate() {
                let won = s.as_ref().map(|s| s.stack as i64).unwrap_or(0) - 1000;
                by_level[levels[i] as usize] += won;
                if levels[i] == Level::Shark {
                    shark_total += won;
                }
            }
        }
        assert!(shark_total > 0, "the shark should win over 40 games: {shark_total} ({by_level:?})");
        assert!(by_level[Level::Rookie as usize] < 0, "rookies lose: {by_level:?}");
    }
}
