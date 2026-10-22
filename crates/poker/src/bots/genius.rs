//! Genius: the strongest bots, playing the way good players do, from what anyone at the table
//! could see (and its own cards).
//!
//! - Before the flop: a range for where it sits (fewer players behind, more hands), opening to
//!   a size that suits the spot, three-betting its best hands and a few bluffs in balance, and
//!   pushing or folding when its stack is short.
//! - Its chances come from playing the hand out many times against what each opponent's
//!   actions say they hold: a raise before the flop narrows them to strong hands, a bet or a
//!   call after it to made hands and draws (and some bluffs), and each player's habits over
//!   past hands (their reads: how often they play, raise and bet) shade how far.
//! - It weighs those chances against the price of a call, bets for value and bluffs at about
//!   the rate that makes its bets hard to play against, uses the same sizes for both, and
//!   commits when the pot's big next to what's left.

use std::sync::OnceLock;

use super::{Persona, Public, Seen, Spot, hand_value};
use crate::cards::{Card, CardSet};
use crate::eval::{Category, Value};
use crate::fair::{Stream, seed_from};
use crate::game::{Action, Legal, Limit, Street, Variant};

/// Times it plays a hand out to know its chances.
const SIMS: usize = 700;

fn roll(rng: &mut Stream) -> f64 {
    rng.below(1_000_000) as f64 / 1_000_000.0
}

/// A Hold'em starting hand's class: 169 of them (pairs, suited and offsuit).
fn class(a: Card, b: Card) -> usize {
    let (hi, lo) = if a.rank() >= b.rank() { (a, b) } else { (b, a) };
    let (h, l) = (hi.rank() as usize, lo.rank() as usize);
    if h == l || hi.suit() == lo.suit() { h * 13 + l } else { l * 13 + h }
}

/// Each class's place among all 1,326 starting hands, from 0 (aces, the best) to 1 (the
/// worst): worked out once, by playing each class against a random hand.
fn places() -> &'static [f32; 169] {
    static P: OnceLock<[f32; 169]> = OnceLock::new();
    P.get_or_init(|| {
        let mut rng = Stream::new(seed_from(0x6e_6975_73));
        let mut eq = [(0f64, 0usize, 0u32); 169];
        for r in 0..13u8 {
            for c in 0..13u8 {
                // (Pairs on the diagonal; suited with the higher rank first; offsuit the other
                // way round.)
                let (a, b, combos) = if r == c {
                    (Card::new(r, 0), Card::new(r, 1), 6)
                } else if r > c {
                    (Card::new(r, 0), Card::new(c, 0), 4)
                } else {
                    (Card::new(c, 0), Card::new(r, 1), 12)
                };
                let i = r as usize * 13 + c as usize;
                eq[i] = (super::equity(Variant::Holdem, &[a, b], &[], 1, 1_200, &mut rng), i, combos);
            }
        }
        let mut order = eq;
        order.sort_by(|x, y| y.0.total_cmp(&x.0));
        let mut out = [0f32; 169];
        let mut before = 0u32;
        for (_, i, combos) in order {
            out[i] = ((before as f64 + combos as f64 / 2.0) / 1_326.0) as f32;
            before += combos;
        }
        out
    })
}

/// A starting hand's place among all of them (0 the best, 1 the worst). Omaha: a sense of it
/// from its pairs, suits and how its cards work together.
fn place(variant: Variant, hole: &[Card]) -> f64 {
    match variant {
        Variant::Holdem if hole.len() == 2 => places()[class(hole[0], hole[1])] as f64,
        _ => 1.0 - omaha_strength(hole),
    }
}

/// An Omaha starting hand, 0 (hopeless) to 1 (aces double-suited).
fn omaha_strength(hole: &[Card]) -> f64 {
    let mut s = 0.0;
    let mut ranks = [0u8; 13];
    let mut suits = [0u8; 4];
    for c in hole {
        ranks[c.rank() as usize] += 1;
        suits[c.suit() as usize] += 1;
        s += c.rank() as f64 / 12.0 * 0.07;
    }
    for (r, n) in ranks.iter().enumerate() {
        if *n >= 2 {
            s += 0.06 + r as f64 / 12.0 * 0.14;
        }
        if *n >= 3 {
            // (Trips in the hand: one of them is dead.)
            s -= 0.12;
        }
    }
    for (su, n) in suits.iter().enumerate() {
        if *n >= 2 {
            s += 0.06;
            if hole.iter().any(|c| c.suit() as usize == su && c.rank() == 12) {
                s += 0.04;
            }
        }
    }
    // Cards that run together.
    let present: u16 = hole.iter().fold(0, |m, c| m | 1 << c.rank());
    let best_run = (0..9).map(|lo| (present >> lo & 0b11111).count_ones()).max().unwrap_or(0);
    s += best_run.saturating_sub(1) as f64 * 0.05;
    s.clamp(0.0, 1.0)
}

/// How an opponent has played the hand after the flop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Line {
    /// Bet or raised.
    Aggressive,
    /// Called a bet.
    Called,
    /// Checked (or hasn't acted yet).
    Passive,
}

/// What an opponent might hold: the best `top` share of starting hands (by place), and how
/// they've played since the flop. `bluffy`: how often a bet of theirs is light.
#[derive(Clone, Copy, Debug)]
struct Range {
    top: f64,
    line: Line,
    bluffy: f64,
}

/// A hand's strength with the board for range-reading: what it's made, and whether it's
/// drawing to a flush or a straight.
fn made_and_draw(variant: Variant, hole: &[Card], board: &[Card]) -> (Category, bool) {
    if board.len() < 3 {
        return (Category::HighCard, false);
    }
    let made = hand_value(variant, hole, board).category();
    if board.len() >= 5 {
        return (made, false);
    }
    // (Omaha: two of the hand's cards with the board, roughly: any of them counts here.)
    let all: Vec<Card> = hole.iter().chain(board).copied().collect();
    let mut suits = [0u8; 4];
    for c in &all {
        suits[c.suit() as usize] += 1;
    }
    let flush_draw = suits.iter().enumerate().any(|(s, n)| *n == 4 && hole.iter().any(|c| c.suit() as usize == s));
    // (Ranks as bits from the two up, with the ace below the two as well: a five-high run.)
    let present: u16 = all.iter().fold(0, |m, c| m | 1 << c.rank());
    let ranks: u16 = (present << 1) | (present >> 12 & 1);
    let straight_draw = (0..=9).any(|lo| (ranks >> lo & 0b11111).count_ones() == 4);
    (made, flush_draw || straight_draw)
}

/// Whether an opponent's sampled hand fits how they've played (a draw on `rng`'s chance).
fn fits(r: &Range, variant: Variant, hole: &[Card], board: &[Card], rng: &mut Stream) -> bool {
    if place(variant, hole) > r.top {
        return false;
    }
    if board.len() < 3 {
        return true;
    }
    let (made, draw) = made_and_draw(variant, hole, board);
    let keep = match r.line {
        Line::Aggressive => match made {
            Category::HighCard if draw => 0.7,
            Category::HighCard => r.bluffy,
            Category::Pair => 0.75,
            _ => 1.0,
        },
        Line::Called => match made {
            Category::HighCard if draw => 0.85,
            Category::HighCard => 0.3,
            _ => 1.0,
        },
        Line::Passive => match made {
            // (A big hand would usually have bet.)
            c if c >= Category::TwoPair => 0.55,
            _ => 1.0,
        },
    };
    keep >= 1.0 || roll(rng) < keep
}

/// Its chance of winning against opponents holding the hands their `ranges` say, playing the
/// rest of the hand out `sims` times.
fn equity_vs(variant: Variant, hole: &[Card], board: &[Card], ranges: &[Range], sims: usize, rng: &mut Stream) -> f64 {
    if ranges.is_empty() {
        return 1.0;
    }
    let used = CardSet::of(&[hole, board].concat());
    let rest: Vec<Card> = Card::all().filter(|c| !used.contains(*c)).collect();
    let k = hole.len();
    if ranges.len() * k + (5 - board.len()) > rest.len() {
        return 1.0 / (ranges.len() + 1) as f64;
    }
    let mut score = 0.0;
    let mut deck = rest;
    for _ in 0..sims {
        // Deal each opponent a hand that fits them (as near as a few tries get), then the
        // board, from the front of the deck.
        let mut at = 0;
        let mut hands: Vec<Vec<Card>> = Vec::with_capacity(ranges.len());
        for r in ranges {
            let mut tries = 0;
            loop {
                for i in at..at + k {
                    let j = i + rng.below((deck.len() - i) as u32) as usize;
                    deck.swap(i, j);
                }
                tries += 1;
                if tries >= 25 || fits(r, variant, &deck[at..at + k], board, rng) {
                    break;
                }
            }
            hands.push(deck[at..at + k].to_vec());
            at += k;
        }
        let need = 5 - board.len();
        for i in at..at + need {
            let j = i + rng.below((deck.len() - i) as u32) as usize;
            deck.swap(i, j);
        }
        let mut b = board.to_vec();
        b.extend(&deck[at..at + need]);
        let mine = hand_value(variant, hole, &b);
        let mut best_other = Value(0);
        let mut ties = 0;
        for h in &hands {
            let v = hand_value(variant, h, &b);
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

/// Players still to act after it before the flop, when nobody's raised yet (the blinds act
/// last).
fn behind(sp: &Spot) -> usize {
    let n = sp.players.len();
    let i = sp.players.iter().position(|p| p.seat == sp.seat).unwrap_or(0);
    let left = if n == 2 {
        if i == 0 { 1 } else { 0 }
    } else if i == 0 {
        1
    } else if i == 1 {
        0
    } else {
        (n - 1 - i) + 2
    };
    // (Only those still in.)
    left.min(sp.opponents().count())
}

/// How much of its hands a player opens with when `behind` players are left to act.
fn open_range(behind: usize) -> f64 {
    match behind {
        0 | 1 => 0.42,
        2 => 0.30,
        3 => 0.22,
        4 => 0.17,
        5 => 0.14,
        _ => 0.12,
    }
}

/// The share of its hands to push all in with, `bb` big blinds deep, `behind` players still
/// to act (one behind is heads-up: wide; more behind, tighter).
fn push_range(bb: f64, behind: usize) -> f64 {
    let heads_up = (0.95 - (bb - 3.0) * 0.05).clamp(0.3, 0.95);
    (heads_up * 0.62f64.powi(behind.saturating_sub(1) as i32)).max(0.04)
}

/// What an opponent's actions say they hold before the flop.
fn preflop_top(sp: &Spot, seen: &Seen, seat: usize) -> f64 {
    let reads = seen.reads_of(seat);
    let pfr = reads.and_then(|r| r.pfr()).map(|p| p.clamp(0.04, 0.6));
    let vpip = reads.and_then(|r| r.vpip()).map(|p| p.clamp(0.08, 0.9));
    let mut raises_seen = 0;
    let mut top: Option<f64> = None;
    let mut called = false;
    for a in sp.actions.iter().filter(|a| a.street == Street::Preflop) {
        if a.seat == seat {
            match a.action {
                Action::Raise { .. } => {
                    // An open, a three-bet, a four-bet: each a tighter range.
                    top = Some(match raises_seen {
                        0 => pfr.unwrap_or(0.18),
                        1 => pfr.map(|p| p * 0.35).unwrap_or(0.06),
                        _ => 0.03,
                    });
                }
                Action::Call => called = true,
                _ => {}
            }
        }
        if matches!(a.action, Action::Raise { .. }) {
            raises_seen += 1;
        }
    }
    top.unwrap_or(if called { if raises_seen > 0 { vpip.unwrap_or(0.3).min(0.4) } else { vpip.unwrap_or(0.45) } } else { 0.85 })
}

/// How an opponent's played the hand since the flop.
fn line_of(sp: &Spot, seat: usize) -> Line {
    let mut line = Line::Passive;
    for a in sp.actions.iter().filter(|a| a.street != Street::Preflop && a.seat == seat) {
        match a.action {
            Action::Raise { .. } => return Line::Aggressive,
            Action::Call => line = Line::Called,
            _ => {}
        }
    }
    line
}

/// Raise (or bet) so this round's chips in front of it come to about `to` (a little more or
/// less each time, so its sizes don't give it away), within what's allowed; all in when that
/// is what it comes to.
fn raise(l: Legal, to: f64, rng: &mut Stream) -> Action {
    let wobble = 0.93 + roll(rng) * 0.14;
    let to = ((to * wobble) as u64).clamp(l.min_to, l.max_to);
    if !l.can_raise {
        return if l.can_check { Action::Check } else { Action::Call };
    }
    if to >= l.max_to { Action::AllIn } else { Action::Raise { to } }
}

/// A bet of `frac` of the pot (after calling what's owed).
fn bet_pot(sp: &Spot, me: &Public, frac: f64, rng: &mut Stream) -> Action {
    let l = sp.legal;
    let after_call = (sp.pot() + l.call) as f64;
    raise(l, (me.street_bet + l.call) as f64 + after_call * frac, rng)
}

/// What it does, its own way (`who`: how loose, how hard, how often it bluffs, how sticky).
pub fn decide(seen: &Seen, who: &Persona, rng: &mut Stream) -> Action {
    let sp = &seen.spot;
    let a = if sp.street == Street::Preflop { preflop(seen, who, rng) } else { postflop(seen, who, rng) };
    legal_or_safe(sp.legal, a)
}

/// What it chose, if it may; otherwise the nearest thing it may do.
fn legal_or_safe(l: Legal, a: Action) -> Action {
    match a {
        Action::Check if !l.can_check => Action::Fold,
        Action::Fold if l.can_check => Action::Check,
        Action::Raise { .. } | Action::AllIn if !l.can_raise => {
            if l.can_check {
                Action::Check
            } else {
                Action::Call
            }
        }
        a => a,
    }
}

fn preflop(seen: &Seen, who: &Persona, rng: &mut Stream) -> Action {
    let sp = &seen.spot;
    let l = sp.legal;
    let bb = sp.big_blind.max(1) as f64;
    let me = sp.me();
    let my_place = place(sp.variant, &seen.hole);
    let raises: Vec<_> = sp.preflop_raises().copied().collect();
    let pot = sp.pot() as f64;
    let call = l.call as f64;
    // The stack that matters: its own, or the biggest it can lose to.
    let biggest_other = sp.opponents().map(|p| p.behind + p.street_bet).max().unwrap_or(0);
    let eff_bb = ((me.behind + me.street_bet).min(biggest_other)) as f64 / bb;
    let n_behind = behind(sp);
    // Short: push or fold.
    if sp.limit == Limit::NoLimit && eff_bb <= 12.0 {
        if raises.is_empty() {
            if my_place <= push_range(eff_bb, n_behind.max(1)) {
                return Action::AllIn;
            }
            return if l.can_check { Action::Check } else { Action::Fold };
        }
        // Facing a raise or a push: call (or push over it) when its chances against their
        // range beat the price.
        let raiser = raises.last().map(|r| r.seat).unwrap_or(sp.seat);
        let range = Range { top: preflop_top(sp, seen, raiser).max(0.08), line: Line::Passive, bluffy: 0.0 };
        let eq = equity_vs(sp.variant, &seen.hole, &[], &[range], 400, rng);
        let price = call / (pot + call).max(1.0);
        if eq > price + 0.02 {
            return if my_place <= 0.12 && l.can_raise { Action::AllIn } else { Action::Call };
        }
        return Action::Fold;
    }
    let limpers = sp.actions.iter().filter(|a| a.street == Street::Preflop && a.action == Action::Call).count() as f64;
    match raises.len() {
        0 => {
            if l.can_check {
                // The big blind, everyone limped: raise its best hands.
                if my_place <= 0.12 {
                    return raise(l, bb * (3.5 + limpers), rng);
                }
                return Action::Check;
            }
            // Open (or raise the limpers): fewer behind, more hands; limpers make it tighter.
            let range = (open_range(n_behind) * who.loose).min(0.85) * (1.0 - 0.18 * limpers).max(0.45);
            if my_place <= range {
                let size = if sp.players.first().is_some_and(|p| p.seat == sp.seat) { 3.0 } else { 2.5 } + limpers;
                return raise(l, bb * size, rng);
            }
            // Behind limpers, a cheap look with a hand that plays well (in position).
            if limpers > 0.0 && my_place <= range * 1.7 && sp.in_position() && call <= bb {
                return Action::Call;
            }
            Action::Fold
        }
        1 => {
            // Facing an open: three-bet the best (and a few bluffs), call what does well
            // against their range at the price, fold the rest.
            let raiser = raises[0].seat;
            let their = preflop_top(sp, seen, raiser);
            let value = (their * 0.22).max(0.025);
            let in_pos = sp.in_position();
            // (Three times the open in position, more out of it, and one more for each caller.)
            let to = raises[0].action_to();
            let callers = sp.actions.iter().filter(|a| a.street == Street::Preflop && a.action == Action::Call).count() as f64;
            let three_bet = |rng: &mut Stream| raise(l, to * (if in_pos { 3.0 } else { 3.6 } + callers), rng);
            if my_place <= value {
                return three_bet(rng);
            }
            // (Bluffs: hands just outside calling strength, about one for two value hands.)
            if my_place <= value * 3.0 && my_place > value * 2.0 && roll(rng) < (0.5 * who.bluff).min(0.9) {
                return three_bet(rng);
            }
            let range = Range { top: their.max(0.05), line: Line::Passive, bluffy: 0.0 };
            let eq = equity_vs(sp.variant, &seen.hole, &[], &[range], 400, rng);
            let price = call / (pot + call);
            // (Position and the chips still to win make a call worth more than its price.)
            let implied = if in_pos { 0.07 } else { 0.03 };
            let crowd = sp.opponents().count().saturating_sub(1) as f64 * 0.04;
            if eq + implied + who.sticky > price + 0.04 + crowd {
                return Action::Call;
            }
            Action::Fold
        }
        _ => {
            // Facing a three-bet or more: the very best go on (all in when it's deep enough to
            // matter), a few strong hands call, the rest fold.
            let last = raises.last().unwrap();
            let their = preflop_top(sp, seen, last.seat).max(0.02);
            if my_place <= 0.025 {
                return raise(l, last.action_to() * 2.4, rng);
            }
            let range = Range { top: their, line: Line::Passive, bluffy: 0.0 };
            let eq = equity_vs(sp.variant, &seen.hole, &[], &[range], 400, rng);
            let price = call / (pot + call);
            if eq > price + 0.05 && my_place <= 0.09 {
                return Action::Call;
            }
            Action::Fold
        }
    }
}

fn postflop(seen: &Seen, who: &Persona, rng: &mut Stream) -> Action {
    let sp = &seen.spot;
    let l = sp.legal;
    let me = sp.me();
    let pot = sp.pot() as f64;
    let call = l.call as f64;
    // What each opponent holds, by their actions (and habits).
    let ranges: Vec<Range> = sp
        .opponents()
        .map(|p| {
            let bluffy = seen.reads_of(p.seat).and_then(|r| r.aggression()).map(|a| (0.15 + a * 0.45).clamp(0.15, 0.6)).unwrap_or(0.33);
            Range { top: preflop_top(sp, seen, p.seat), line: line_of(sp, p.seat), bluffy }
        })
        .collect();
    let opponents = ranges.len().max(1);
    let eq = equity_vs(sp.variant, &seen.hole, &sp.board, &ranges, SIMS, rng);
    let (_, drawing) = made_and_draw(sp.variant, &seen.hole, &sp.board);
    // Chips left against the pot: little left, and it's all going in with a good hand.
    let spr = me.behind as f64 / pot.max(1.0);
    let river = sp.street == Street::River;
    let size = match sp.street {
        Street::Flop => 0.55,
        Street::Turn => 0.66,
        _ => 0.75,
    } * who.sizing;
    // (The river's big bet is for its best hands and some of its bluffs alike.)
    let polar = |rng: &mut Stream, strong: bool| if river && roll(rng) < if strong { 0.5 } else { 0.3 } { 1.1 } else { size };
    let aggressor_before = sp.preflop_raises().last().is_some_and(|r| r.seat == sp.seat);
    if l.can_check {
        // Bet for value when it's ahead of what they'd call with.
        let value_at = if opponents == 1 { 0.62 } else { 0.66 + 0.04 * opponents as f64 } - (who.aggression - 1.0) * 0.08;
        if eq >= value_at {
            // Now and then, a trap with a monster on the flop.
            if sp.street == Street::Flop && eq > 0.9 && roll(rng) < who.trap.max(0.2) {
                return Action::Check;
            }
            if spr < 1.2 {
                return Action::AllIn;
            }
            let f = polar(rng, true);
            return bet_pot(sp, &me, f, rng);
        }
        // Bluff at about the rate its bet size makes fair (for a bet of s pots, s / (1 + 2s)
        // of its bets), mostly with draws, more as the one who raised before the flop, less
        // into a crowd.
        let f = polar(rng, false);
        let fair = f / (1.0 + 2.0 * f);
        let rate = fair * if aggressor_before { 1.5 } else { 0.7 } * if opponents > 1 { 0.35 } else { 1.0 } * who.bluff;
        if eq < 0.5 && (drawing || river || aggressor_before) && roll(rng) < rate {
            return bet_pot(sp, &me, f, rng);
        }
        return Action::Check;
    }
    // Facing a bet.
    let price = call / (pot + call);
    let implied = if !river && drawing { 0.05 } else { 0.0 };
    // Raise its best hands (all in when little's left), and semi-bluff a draw now and then.
    if eq >= 0.8 && l.can_raise {
        if spr < 1.5 {
            return Action::AllIn;
        }
        let to = (me.street_bet + l.call) as f64 + (pot + call) * 0.8;
        return raise(l, to, rng);
    }
    if sp.street == Street::Flop && drawing && eq > 0.3 && opponents == 1 && l.can_raise && roll(rng) < 0.12 {
        let to = (me.street_bet + l.call) as f64 + (pot + call) * 0.75;
        return raise(l, to, rng);
    }
    if eq + implied + who.sticky >= price + 0.015 {
        return Action::Call;
    }
    // Don't give up too easily to a small bet.
    if call * 3.0 <= pot && eq > price * 0.8 && roll(rng) < 0.5 {
        return Action::Call;
    }
    Action::Fold
}

impl crate::game::Played {
    /// The total a raise was to (what it put in, for anything else).
    fn action_to(&self) -> f64 {
        match self.action {
            Action::Raise { to } => to as f64,
            _ => self.put as f64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bots::Level;
    use crate::cards::parse_cards;
    use crate::fair::shuffle;
    use crate::game::{Event, Rules, Table};

    #[test]
    fn starting_hands_are_placed_right() {
        let p = |h: &str| {
            let c = parse_cards(h).unwrap();
            place(Variant::Holdem, &c)
        };
        assert!(p("AsAh") < 0.01, "aces first: {}", p("AsAh"));
        assert!(p("AsAh") < p("KsKh") && p("KsKh") < p("AsKs") && p("AsKs") < p("AsKd"));
        assert!(p("AsKd") < p("9s8s") && p("9s8s") < p("7c2d"));
        assert!(p("7c2d") > 0.9 && p("3c2d") > 0.98, "the worst near the bottom: {} {}", p("7c2d"), p("3c2d"));
        // The same hand, any suits.
        assert_eq!(p("KhQh"), p("KcQc"));
        assert_eq!(p("KhQd"), p("KsQc"));
    }

    /// Plays one hand to the end, every action checked legal; returns each seat's chips won
    /// or lost.
    fn hand(levels: &[Level], deck: Vec<Card>, stack: u64, rng: &mut Stream) -> Vec<i64> {
        let who: Vec<&Persona> = levels.iter().map(|_| &crate::bots::persona::PLAIN).collect();
        hand_as(levels, &who, deck, stack, rng)
    }

    /// `hand`, each seat someone (see `bots::persona`).
    fn hand_as(levels: &[Level], who: &[&Persona], deck: Vec<Card>, stack: u64, rng: &mut Stream) -> Vec<i64> {
        let n = levels.len();
        let mut t = Table::new(Rules::holdem(5, 10, n));
        for i in 0..n {
            t.sit(i, &format!("bot{i}"), "x", stack).unwrap();
        }
        // (The button at the last seat, so the first hand's is at seat 0.)
        t.button = Some(n - 1);
        let mut out = t.start_hand(1, deck).unwrap();
        loop {
            let next = out.iter().rev().find_map(|(_, e)| match e {
                Event::Turn { seat, .. } => Some(Some(*seat)),
                Event::HandEnded { .. } => Some(None),
                _ => None,
            });
            let Some(Some(seat)) = next else { break };
            let seen = t.seen(seat).unwrap();
            let a = crate::bots::decide_as(&seen, levels[seat], who[seat], rng);
            out = t.act(seat, a).unwrap_or_else(|e| panic!("{:?} chose {a:?} with {:?}: {e}", levels[seat], seen.spot.legal));
        }
        t.seats.iter().map(|s| s.as_ref().map(|s| s.stack as i64).unwrap_or(0) - stack as i64).collect()
    }

    /// Genius against Shark, heads-up, played in pairs: each deck twice with the seats
    /// swapped, so the cards' luck mostly cancels out and what's left is how they play.
    #[test]
    fn genius_beats_shark_heads_up() {
        let mut rng = Stream::new(seed_from(2_026));
        let mut genius = 0i64;
        let deals = 1_500u64;
        for d in 0..deals {
            let deck = shuffle(seed_from(10_000 + d));
            let a = hand(&[Level::Genius, Level::Shark], deck.clone(), 1_000, &mut rng);
            let b = hand(&[Level::Shark, Level::Genius], deck, 1_000, &mut rng);
            genius += a[0] + b[1];
        }
        // In big blinds a hundred hands.
        let bb100 = genius as f64 / 10.0 / (deals as f64 * 2.0) * 100.0;
        eprintln!("Genius against Shark, heads-up: {bb100:+.1} bb/100 over {} hands", deals * 2);
        assert!(bb100 > 0.0, "Genius should beat Shark: {bb100:.1} bb/100 over {} hands", deals * 2);
    }

    /// A long run to measure the levels (`cargo test --release -p pyr-poker long_run -- --ignored
    /// --nocapture`): big blinds won a hundred hands at a table of six, every deck dealt with
    /// the seats turned round six ways.
    #[test]
    #[ignore]
    fn long_run() {
        let levels = [Level::Genius, Level::Shark, Level::Regular, Level::Rookie, Level::Shark, Level::Regular];
        let threads = 8u64;
        let per = 1_500u64;
        let handles: Vec<_> = (0..threads)
            .map(|th| {
                std::thread::spawn(move || {
                    let mut rng = Stream::new(seed_from(900 + th));
                    let mut won = [0i64; 6];
                    for d in 0..per {
                        let deck = shuffle(seed_from(1_000_000 * (th + 1) + d));
                        for turn in 0..6usize {
                            let seated: Vec<Level> = (0..6).map(|i| levels[(i + turn) % 6]).collect();
                            let r = hand(&seated, deck.clone(), 1_000, &mut rng);
                            for i in 0..6 {
                                won[(i + turn) % 6] += r[i];
                            }
                        }
                    }
                    won
                })
            })
            .collect();
        let mut won = [0i64; 6];
        for h in handles {
            for (i, w) in h.join().unwrap().iter().enumerate() {
                won[i] += w;
            }
        }
        let hands = (threads * per * 6) as f64;
        for (i, l) in levels.iter().enumerate() {
            eprintln!("{:>8}: {:+.1} bb/100", l.name(), won[i] as f64 / 10.0 / hands * 100.0);
        }
    }

    /// Every personality at Genius against the same table (`cargo test --release -p pyr-poker
    /// personas_at_genius -- --ignored --nocapture`): big blinds won a hundred hands, each deck
    /// dealt with the seats turned round six ways. Each should still win.
    #[test]
    #[ignore]
    fn personas_at_genius() {
        let levels = [Level::Genius, Level::Shark, Level::Regular, Level::Rookie, Level::Shark, Level::Regular];
        let handles: Vec<_> = crate::bots::persona::ALL
            .iter()
            .map(|p| {
                std::thread::spawn(move || {
                    let mut rng = Stream::new(seed_from(77));
                    let plain = &crate::bots::persona::PLAIN;
                    let mut won = 0i64;
                    let per = 400u64;
                    for d in 0..per {
                        let deck = shuffle(seed_from(3_000_000 + d));
                        for turn in 0..6usize {
                            let seated: Vec<Level> = (0..6).map(|i| levels[(i + turn) % 6]).collect();
                            let who: Vec<&Persona> = (0..6).map(|i| if (i + turn) % 6 == 0 { p } else { plain }).collect();
                            let r = hand_as(&seated, &who, deck.clone(), 1_000, &mut rng);
                            won += r[(6 - turn) % 6];
                        }
                    }
                    (p.name, won as f64 / 10.0 / (per * 6) as f64 * 100.0)
                })
            })
            .collect();
        for h in handles {
            let (name, bb100) = h.join().unwrap();
            eprintln!("{name:>8} at Genius: {bb100:+.1} bb/100");
        }
    }

    /// At a full table of every level, Genius comes out on top (each deck dealt six times, the
    /// seats turned round, so the cards' luck mostly cancels out).
    #[test]
    fn genius_wins_at_a_mixed_table() {
        let mut rng = Stream::new(seed_from(7));
        let levels = [Level::Genius, Level::Shark, Level::Regular, Level::Rookie, Level::Shark, Level::Regular];
        let mut won = [0i64; 6];
        for d in 0..250u64 {
            let deck = shuffle(seed_from(50_000 + d));
            for turn in 0..6usize {
                let seated: Vec<Level> = (0..6).map(|i| levels[(i + turn) % 6]).collect();
                let r = hand(&seated, deck.clone(), 1_000, &mut rng);
                for i in 0..6 {
                    won[(i + turn) % 6] += r[i];
                }
            }
        }
        let g = won[0];
        eprintln!("A table of six, chips won (Genius, Shark, Regular, Rookie, Shark, Regular): {won:?}");
        assert!(won.iter().skip(1).all(|w| *w < g), "Genius should win the most: {won:?}");
    }
}
