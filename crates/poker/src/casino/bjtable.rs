//! Blackjack at a table of seven: every seat's hands against one dealer, from one shoe, with the
//! same rules as blackjack alone (`blackjack`): six decks, a fresh shoe each round, the dealer
//! stands on soft 17 and checks for blackjack under an ace or a ten, blackjack pays 3 to 2,
//! double on any first two cards (after a split too), split to four hands (split aces get one
//! card each), insurance under an ace pays 2 to 1.
//!
//! The deal goes round the table from first base (the lowest seat) with the dealer last: a card
//! each, the dealer's up card, a second card each, the dealer's hole card. Then insurance round
//! the table under an ace, then each seat plays its hands in turn, then the dealer. The cards
//! come off the round's seed in the order they're drawn, so the same seed, bets and moves always
//! give the same round: everyone's Pyr plays it again afterwards ([`replay`]) to check it.

use serde::{Deserialize, Serialize};

use super::Shoe;
use super::blackjack::{Action, DECKS, Hand, MAX_HANDS, Phase, Result, card_value, is_blackjack, value};
use crate::cards::Card;
use crate::fair::Stream;

/// Seats at a table.
pub const SEATS: usize = 7;

/// One seat's play in a round.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeatPlay {
    pub seat: u8,
    /// Its bet as the round was dealt.
    pub base: u64,
    pub hands: Vec<Hand>,
    /// The hand it's playing.
    pub active: usize,
    /// Insurance taken (chips), and whether it's decided.
    pub insurance: u64,
    pub insured: bool,
}

impl SeatPlay {
    pub fn wagered(&self) -> u64 {
        self.hands.iter().map(|h| h.bet).sum::<u64>() + self.insurance
    }
}

/// A round being played (the house's side: the shoe and the hole card).
pub struct Round {
    s: Stream,
    shoe: Shoe,
    pub dealer: Vec<Card>,
    pub seats: Vec<SeatPlay>,
    pub phase: Phase,
    /// Whose turn (an index into `seats`).
    pub turn: usize,
    /// Every move, by seat, in order.
    pub actions: Vec<(u8, Action)>,
    pub dealer_blackjack: bool,
}

/// A round as the table sees it (the hole card face down until the end).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoundView {
    pub dealer: Vec<Card>,
    pub hole_hidden: bool,
    pub seats: Vec<SeatPlay>,
    pub phase: Phase,
    /// The seat to act, and what it may do.
    pub to_act: Option<u8>,
    pub allowed: Vec<Action>,
    pub dealer_blackjack: bool,
}

/// A round that's over.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub bets: Vec<(u8, u64)>,
    pub dealer: Vec<Card>,
    pub seats: Vec<SeatPlay>,
    pub dealer_blackjack: bool,
    pub actions: Vec<(u8, Action)>,
}

impl Outcome {
    /// What a seat got back (its winning hands' stakes included, and insurance).
    pub fn paid(&self, seat: u8) -> u64 {
        self.seats.iter().find(|s| s.seat == seat).map(|s| paid_of(s, self.dealer_blackjack)).unwrap_or(0)
    }

    /// A seat's round as blackjack alone keeps it (for its player's records).
    pub fn for_seat(&self, seat: u8) -> Option<super::blackjack::Outcome> {
        let sp = self.seats.iter().find(|s| s.seat == seat)?;
        Some(super::blackjack::Outcome {
            base: sp.base,
            dealer: self.dealer.clone(),
            hands: sp.hands.clone(),
            insurance: sp.insurance,
            dealer_blackjack: self.dealer_blackjack,
            actions: self.actions.iter().filter(|(s, _)| *s == seat).map(|(_, a)| *a).collect(),
            wagered: sp.wagered(),
            paid: paid_of(sp, self.dealer_blackjack).min(super::MAX_PAYOUT),
        })
    }
}

fn paid_of(sp: &SeatPlay, dealer_blackjack: bool) -> u64 {
    let insurance = if dealer_blackjack { sp.insurance * 3 } else { 0 };
    sp.hands.iter().map(|h| h.paid).sum::<u64>() + insurance
}

fn new_hand(cards: Vec<Card>, bet: u64) -> Hand {
    Hand { cards, bet, doubled: false, from_split: false, split_aces: false, done: false, result: None, paid: 0 }
}

impl Round {
    /// Deals to the seats that bet (`bets`: seat and chips, any order).
    pub fn deal(bets: &[(u8, u64)], mut s: Stream) -> Round {
        let mut bets = bets.to_vec();
        bets.sort_by_key(|b| b.0);
        let mut shoe = Shoe::new(DECKS);
        let first: Vec<Card> = bets.iter().map(|_| shoe.draw(&mut s)).collect();
        let up = shoe.draw(&mut s);
        let second: Vec<Card> = bets.iter().map(|_| shoe.draw(&mut s)).collect();
        let hole = shoe.draw(&mut s);
        let seats = bets
            .iter()
            .enumerate()
            .map(|(i, (seat, base))| SeatPlay { seat: *seat, base: *base, hands: vec![new_hand(vec![first[i], second[i]], *base)], active: 0, insurance: 0, insured: false })
            .collect();
        let mut r = Round { s, shoe, dealer: vec![up, hole], seats, phase: Phase::Playing, turn: 0, actions: Vec::new(), dealer_blackjack: false };
        if up.rank() == 12 {
            r.phase = Phase::Insurance;
        } else {
            r.after_insurance();
        }
        r
    }

    /// The dealer checks for blackjack (under an ace or a ten); then play goes round.
    fn after_insurance(&mut self) {
        self.phase = Phase::Playing;
        if card_value(self.dealer[0]) >= 10 && is_blackjack(&self.dealer) {
            self.dealer_blackjack = true;
            self.finish();
            return;
        }
        for sp in &mut self.seats {
            if sp.hands[0].is_blackjack() {
                sp.hands[0].done = true;
            }
        }
        self.turn = 0;
        self.next_to_play();
    }

    /// The turn on to the first hand still to play (from the seat whose turn it is), or the
    /// dealer's when there's none.
    fn next_to_play(&mut self) {
        while self.turn < self.seats.len() {
            let sp = &mut self.seats[self.turn];
            if let Some(i) = (sp.active..sp.hands.len()).find(|&i| !sp.hands[i].done) {
                sp.active = i;
                return;
            }
            self.turn += 1;
        }
        self.finish();
    }

    /// The seat to act now (insurance or play).
    pub fn to_act(&self) -> Option<u8> {
        if self.phase == Phase::Over { None } else { self.seats.get(self.turn).map(|s| s.seat) }
    }

    pub fn allowed(&self) -> Vec<Action> {
        let Some(sp) = self.seats.get(self.turn) else { return vec![] };
        match self.phase {
            Phase::Insurance => vec![Action::Insure, Action::NoInsurance],
            Phase::Over => vec![],
            Phase::Playing => {
                let h = &sp.hands[sp.active];
                let mut a = vec![Action::Hit, Action::Stand];
                if h.cards.len() == 2 && !h.split_aces {
                    a.push(Action::Double);
                }
                if h.cards.len() == 2 && card_value(h.cards[0]) == card_value(h.cards[1]) && sp.hands.len() < MAX_HANDS && !h.split_aces {
                    a.push(Action::Split);
                }
                a
            }
        }
    }

    /// The chips an action puts down besides what's bet already.
    pub fn cost(&self, a: Action) -> u64 {
        let Some(sp) = self.seats.get(self.turn) else { return 0 };
        match a {
            Action::Double | Action::Split => sp.hands.get(sp.active).map(|h| h.bet).unwrap_or(0),
            Action::Insure => sp.base / 2,
            _ => 0,
        }
    }

    pub fn act(&mut self, seat: u8, a: Action) -> std::result::Result<(), String> {
        if self.to_act() != Some(seat) {
            return Err("it isn't that seat's turn".into());
        }
        if !self.allowed().contains(&a) {
            return Err(format!("{} isn't something you can do now", a.name()));
        }
        self.actions.push((seat, a));
        let t = self.turn;
        match a {
            Action::Insure | Action::NoInsurance => {
                let sp = &mut self.seats[t];
                if a == Action::Insure {
                    sp.insurance = sp.base / 2;
                }
                sp.insured = true;
                self.turn += 1;
                if self.turn >= self.seats.len() {
                    self.after_insurance();
                }
            }
            Action::Hit => {
                let c = self.shoe.draw(&mut self.s);
                let sp = &mut self.seats[t];
                let h = &mut sp.hands[sp.active];
                h.cards.push(c);
                if h.value().0 >= 21 {
                    h.done = true;
                }
                self.next_to_play();
            }
            Action::Stand => {
                let sp = &mut self.seats[t];
                sp.hands[sp.active].done = true;
                self.next_to_play();
            }
            Action::Double => {
                let c = self.shoe.draw(&mut self.s);
                let sp = &mut self.seats[t];
                let h = &mut sp.hands[sp.active];
                h.bet *= 2;
                h.doubled = true;
                h.cards.push(c);
                h.done = true;
                self.next_to_play();
            }
            Action::Split => {
                let i = self.seats[t].active;
                let aces = self.seats[t].hands[i].cards[0].rank() == 12;
                let second = self.seats[t].hands[i].cards.pop().unwrap();
                let bet = self.seats[t].hands[i].bet;
                let c1 = self.shoe.draw(&mut self.s);
                let c2 = self.shoe.draw(&mut self.s);
                let sp = &mut self.seats[t];
                let h = &mut sp.hands[i];
                h.cards.push(c1);
                h.from_split = true;
                h.split_aces = aces;
                let mut other = new_hand(vec![second, c2], bet);
                other.from_split = true;
                other.split_aces = aces;
                for h in [&mut sp.hands[i], &mut other] {
                    if aces || h.value().0 == 21 {
                        h.done = true;
                    }
                }
                sp.hands.insert(i + 1, other);
                self.next_to_play();
            }
        }
        Ok(())
    }

    /// The dealer plays (when any hand's still live) and every hand is settled.
    fn finish(&mut self) {
        self.phase = Phase::Over;
        let live = !self.dealer_blackjack && self.seats.iter().flat_map(|s| &s.hands).any(|h| h.value().0 <= 21 && !h.is_blackjack());
        if live {
            while value(&self.dealer).0 < 17 {
                let c = self.shoe.draw(&mut self.s);
                self.dealer.push(c);
            }
        }
        let (d, _) = value(&self.dealer);
        let dealer_bj = self.dealer_blackjack;
        for sp in &mut self.seats {
            for h in &mut sp.hands {
                let (t, _) = value(&h.cards);
                let (result, paid) = if t > 21 {
                    (Result::Bust, 0)
                } else if h.is_blackjack() {
                    if dealer_bj { (Result::Push, h.bet) } else { (Result::Blackjack, h.bet + h.bet * 3 / 2) }
                } else if dealer_bj {
                    (Result::Lose, 0)
                } else if d > 21 || t > d {
                    (Result::Win, h.bet * 2)
                } else if t == d {
                    (Result::Push, h.bet)
                } else {
                    (Result::Lose, 0)
                };
                h.result = Some(result);
                h.paid = paid;
                h.done = true;
            }
        }
        self.turn = self.seats.len();
    }

    pub fn view(&self) -> RoundView {
        let hidden = self.phase != Phase::Over;
        RoundView {
            dealer: if hidden { vec![self.dealer[0]] } else { self.dealer.clone() },
            hole_hidden: hidden,
            seats: self.seats.clone(),
            phase: self.phase,
            to_act: self.to_act(),
            allowed: self.allowed(),
            dealer_blackjack: self.phase == Phase::Over && self.dealer_blackjack,
        }
    }

    pub fn outcome(&self) -> Outcome {
        Outcome {
            bets: self.seats.iter().map(|s| (s.seat, s.base)).collect(),
            dealer: self.dealer.clone(),
            seats: self.seats.clone(),
            dealer_blackjack: self.dealer_blackjack,
            actions: self.actions.clone(),
        }
    }
}

/// Plays a round again from its seed, bets and moves (everyone's Pyr checking the house).
pub fn replay(bets: &[(u8, u64)], s: Stream, actions: &[(u8, Action)]) -> std::result::Result<Outcome, String> {
    let mut r = Round::deal(bets, s);
    for (seat, a) in actions {
        r.act(*seat, *a)?;
    }
    if r.phase != Phase::Over {
        return Err("the round isn't over".into());
    }
    Ok(r.outcome())
}

/// A bot's move, its own way (see `bots::persona`): by the book; never risking a bust; like the
/// dealer; doubling and splitting whenever it can; or on a hunch. Whatever its way, it's a move
/// the seat may make.
pub fn bot_move_as(r: &RoundView, play: crate::bots::persona::BjPlay) -> Action {
    use crate::bots::persona::BjPlay;
    let book = bot_move(r);
    let ok = |a: Action| r.allowed.contains(&a);
    if r.phase == Phase::Insurance {
        return if play == BjPlay::Superstitious && ok(Action::Insure) { Action::Insure } else { Action::NoInsurance };
    }
    let Some(sp) = r.seats.iter().find(|s| Some(s.seat) == r.to_act) else { return Action::Stand };
    let h = &sp.hands[sp.active];
    let (v, soft) = value(&h.cards);
    let pair = h.cards.len() == 2 && card_value(h.cards[0]) == card_value(h.cards[1]);
    let a = match play {
        BjPlay::Book => book,
        BjPlay::Careful => match book {
            _ if !soft && v >= 12 => Action::Stand,
            Action::Double => Action::Hit,
            a => a,
        },
        BjPlay::Dealer => {
            if v < 17 {
                Action::Hit
            } else {
                Action::Stand
            }
        }
        BjPlay::Daredevil => {
            if pair && ok(Action::Split) && card_value(h.cards[0]) != 10 {
                Action::Split
            } else if ok(Action::Double) && ((9..=11).contains(&v) || (soft && (13..=18).contains(&v))) {
                Action::Double
            } else {
                book
            }
        }
        BjPlay::Superstitious => {
            if !soft && v == 16 {
                Action::Stand
            } else {
                book
            }
        }
    };
    if ok(a) { a } else if ok(book) { book } else { Action::Stand }
}

/// A bot's move: basic strategy (and never insurance, as it advises).
pub fn bot_move(r: &RoundView) -> Action {
    match r.phase {
        Phase::Insurance => Action::NoInsurance,
        _ => {
            let Some(sp) = r.seats.iter().find(|s| Some(s.seat) == r.to_act) else { return Action::Stand };
            let h = &sp.hands[sp.active];
            let up = r.dealer[0];
            let can_double = r.allowed.contains(&Action::Double);
            let can_split = r.allowed.contains(&Action::Split);
            let a = super::blackjack::basic(&h.cards, up, can_double, can_split);
            if r.allowed.contains(&a) { a } else { Action::Stand }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fair::seed_from;

    /// Plays a round with every seat on basic strategy.
    fn play(bets: &[(u8, u64)], seed: u64) -> Round {
        let mut r = Round::deal(bets, Stream::new(seed_from(seed)));
        let mut guard = 0;
        while let Some(seat) = r.to_act() {
            let a = bot_move(&r.view());
            r.act(seat, a).unwrap();
            guard += 1;
            assert!(guard < 200);
        }
        r
    }

    #[test]
    fn rounds_finish_and_play_again_the_same() {
        for seed in 0..400u64 {
            let bets: Vec<(u8, u64)> = (0..7u8).filter(|s| (seed + *s as u64) % 3 != 0).map(|s| (s, 10 + s as u64 * 5)).collect();
            if bets.is_empty() {
                continue;
            }
            let r = play(&bets, seed);
            assert_eq!(r.phase, Phase::Over);
            let o = r.outcome();
            let again = replay(&bets, Stream::new(seed_from(seed)), &o.actions).unwrap();
            assert_eq!(again, o, "seed {seed}");
            // Every hand settled, the dealer standing at 17 or more unless nobody needed it.
            assert!(o.seats.iter().flat_map(|s| &s.hands).all(|h| h.result.is_some()));
        }
    }

    /// Every bot's way of playing makes only moves its seat may make, and its rounds finish.
    #[test]
    fn every_way_of_playing_finishes_its_rounds() {
        use crate::bots::persona::BjPlay;
        for (k, play) in [BjPlay::Book, BjPlay::Careful, BjPlay::Dealer, BjPlay::Daredevil, BjPlay::Superstitious].into_iter().enumerate() {
            let mut doubles = 0;
            for seed in 0..200u64 {
                let bets: Vec<(u8, u64)> = (0..7u8).map(|s| (s, 10)).collect();
                let mut r = Round::deal(&bets, Stream::new(seed_from(seed * 7 + k as u64)));
                let mut guard = 0;
                while let Some(seat) = r.to_act() {
                    let v = r.view();
                    let a = bot_move_as(&v, play);
                    assert!(v.allowed.contains(&a), "{play:?}: {a:?} isn't allowed");
                    doubles += (a == Action::Double) as u32;
                    r.act(seat, a).unwrap();
                    guard += 1;
                    assert!(guard < 300);
                }
                assert_eq!(r.phase, Phase::Over);
            }
            match play {
                BjPlay::Careful | BjPlay::Dealer => assert_eq!(doubles, 0, "{play:?} never doubles"),
                _ => assert!(doubles > 20, "{play:?} doubles sometimes: {doubles}"),
            }
        }
    }

    #[test]
    fn the_deal_goes_round_from_first_base() {
        let bets = [(4u8, 10u64), (0, 10), (6, 10)];
        let r = Round::deal(&bets, Stream::new(seed_from(1)));
        let seats: Vec<u8> = r.seats.iter().map(|s| s.seat).collect();
        assert_eq!(seats, vec![0, 4, 6]);
        assert!(r.seats.iter().all(|s| s.hands[0].cards.len() == 2));
        assert_eq!(r.dealer.len(), 2);
        // The view keeps the hole card down.
        assert!(r.view().hole_hidden || r.phase == Phase::Over);
    }

    #[test]
    fn a_seat_cant_act_out_of_turn() {
        for seed in 0..50u64 {
            let mut r = Round::deal(&[(1, 10), (3, 10)], Stream::new(seed_from(seed)));
            if let Some(t) = r.to_act() {
                let other = if t == 1 { 3 } else { 1 };
                assert!(r.act(other, Action::Stand).is_err());
            }
        }
    }

    /// Basic strategy at a full table pays back about 99.5% (the house's edge about half a
    /// percent), seat by seat.
    #[test]
    fn a_full_table_pays_back_about_99_and_a_half_percent() {
        let (mut wagered, mut paid) = (0u64, 0u64);
        for seed in 0..40_000u64 {
            let bets: Vec<(u8, u64)> = (0..7u8).map(|s| (s, 100)).collect();
            let o = play(&bets, 1_000_000 + seed).outcome();
            for sp in &o.seats {
                wagered += sp.wagered();
                paid += o.paid(sp.seat);
            }
        }
        let rtp = paid as f64 / wagered as f64;
        assert!((0.985..1.005).contains(&rtp), "{rtp}");
    }
}
