//! Blackjack, six decks, a fresh shoe each hand.
//!
//! The dealer stands on soft 17 and checks for blackjack under an ace or a ten. Blackjack pays
//! 3 to 2. Double on any first two cards (after a split too); split pairs (tens of any kind
//! together) up to four hands; split aces get one card each. Insurance under a dealer's ace pays
//! 2 to 1. The house keeps about half a percent, played well.
//!
//! A hand goes a step at a time: [`Round::deal`], then [`Round::act`] until it's over. The same
//! seed and the same moves always give the same hand, so the player's Pyr plays it again after
//! (see [`replay`]) to check it.

use serde::{Deserialize, Serialize};

use super::Shoe;
use crate::cards::Card;
use crate::fair::Stream;

pub const DECKS: usize = 6;
pub const MAX_HANDS: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Hit,
    Stand,
    Double,
    Split,
    /// Insurance (half the bet) or not, under a dealer's ace.
    Insure,
    NoInsurance,
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::Hit => "Hit",
            Action::Stand => "Stand",
            Action::Double => "Double",
            Action::Split => "Split",
            Action::Insure => "Insurance",
            Action::NoInsurance => "No insurance",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    /// The dealer shows an ace: insurance, or not.
    Insurance,
    Playing,
    Over,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Result {
    Blackjack,
    Win,
    Push,
    Lose,
    Bust,
}

impl Result {
    pub fn name(self) -> &'static str {
        match self {
            Result::Blackjack => "Blackjack!",
            Result::Win => "Win",
            Result::Push => "Push",
            Result::Lose => "Lose",
            Result::Bust => "Bust",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hand {
    pub cards: Vec<Card>,
    pub bet: u64,
    #[serde(default)]
    pub doubled: bool,
    #[serde(default)]
    pub from_split: bool,
    #[serde(default)]
    pub split_aces: bool,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub result: Option<Result>,
    #[serde(default)]
    pub paid: u64,
}

impl Hand {
    fn new(cards: Vec<Card>, bet: u64) -> Hand {
        Hand { cards, bet, doubled: false, from_split: false, split_aces: false, done: false, result: None, paid: 0 }
    }

    pub fn value(&self) -> (u8, bool) {
        value(&self.cards)
    }

    pub fn is_blackjack(&self) -> bool {
        !self.from_split && is_blackjack(&self.cards)
    }
}

/// A card's count: 2 to 9, tens and faces 10, aces 11 (or 1).
pub fn card_value(c: Card) -> u8 {
    match c.rank() {
        12 => 11,
        r if r >= 8 => 10,
        r => r + 2,
    }
}

/// A hand's total, and whether it's soft (an ace counting 11).
pub fn value(cards: &[Card]) -> (u8, bool) {
    let mut total: u32 = cards.iter().map(|c| card_value(*c) as u32).sum();
    let mut aces = cards.iter().filter(|c| c.rank() == 12).count();
    while total > 21 && aces > 0 {
        total -= 10;
        aces -= 1;
    }
    (total.min(255) as u8, aces > 0)
}

pub fn is_blackjack(cards: &[Card]) -> bool {
    cards.len() == 2 && value(cards).0 == 21
}

/// What the player sees of a hand in play.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub base: u64,
    /// The dealer's cards showing (the hole card is left out until it's turned over).
    pub dealer: Vec<Card>,
    pub hole_hidden: bool,
    pub hands: Vec<Hand>,
    pub active: usize,
    pub phase: Phase,
    pub insurance: u64,
    pub allowed: Vec<Action>,
    pub wagered: u64,
}

/// A hand that's over.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub base: u64,
    pub dealer: Vec<Card>,
    pub hands: Vec<Hand>,
    pub insurance: u64,
    pub dealer_blackjack: bool,
    /// The player's moves, in order (to play it again).
    pub actions: Vec<Action>,
    pub wagered: u64,
    pub paid: u64,
}

impl Outcome {
    pub fn headline(&self) -> String {
        if self.hands.len() == 1 {
            let h = &self.hands[0];
            match h.result {
                Some(Result::Blackjack) => "Blackjack!".into(),
                Some(r) => format!("{} with {}", r.name(), value(&h.cards).0),
                None => "a hand".into(),
            }
        } else {
            let won = self.hands.iter().filter(|h| matches!(h.result, Some(Result::Win | Result::Blackjack))).count();
            format!("won {won} of {} hands", self.hands.len())
        }
    }
}

/// A hand being played (the house's side: the shoe and the hole card).
pub struct Round {
    s: Stream,
    shoe: Shoe,
    pub base: u64,
    pub dealer: Vec<Card>,
    pub hands: Vec<Hand>,
    pub active: usize,
    pub phase: Phase,
    pub insurance: u64,
    pub actions: Vec<Action>,
    dealer_blackjack: bool,
}

impl Round {
    /// Deals: the player, the dealer's up card, the player, the dealer's hole card.
    pub fn deal(chips: u64, mut s: Stream) -> Round {
        let mut shoe = Shoe::new(DECKS);
        let p1 = shoe.draw(&mut s);
        let d1 = shoe.draw(&mut s);
        let p2 = shoe.draw(&mut s);
        let d2 = shoe.draw(&mut s);
        let mut r = Round {
            s,
            shoe,
            base: chips,
            dealer: vec![d1, d2],
            hands: vec![Hand::new(vec![p1, p2], chips)],
            active: 0,
            phase: Phase::Playing,
            insurance: 0,
            actions: Vec::new(),
            dealer_blackjack: false,
        };
        if d1.rank() == 12 {
            r.phase = Phase::Insurance;
        } else {
            r.after_insurance();
        }
        r
    }

    /// The dealer checks for blackjack (under an ace or a ten), then a player's blackjack is
    /// paid; otherwise play goes on.
    fn after_insurance(&mut self) {
        self.phase = Phase::Playing;
        if card_value(self.dealer[0]) >= 10 && is_blackjack(&self.dealer) {
            self.dealer_blackjack = true;
            self.finish();
            return;
        }
        if self.hands[0].is_blackjack() {
            self.hands[0].done = true;
            self.finish();
        }
    }

    pub fn allowed(&self) -> Vec<Action> {
        match self.phase {
            Phase::Insurance => vec![Action::Insure, Action::NoInsurance],
            Phase::Over => vec![],
            Phase::Playing => {
                let h = &self.hands[self.active];
                let mut a = vec![Action::Hit, Action::Stand];
                if h.cards.len() == 2 && !h.split_aces {
                    a.push(Action::Double);
                }
                if h.cards.len() == 2 && card_value(h.cards[0]) == card_value(h.cards[1]) && self.hands.len() < MAX_HANDS && !h.split_aces {
                    a.push(Action::Split);
                }
                a
            }
        }
    }

    /// The chips an action puts down besides what's already bet.
    pub fn cost(&self, a: Action) -> u64 {
        match a {
            Action::Double | Action::Split => self.hands.get(self.active).map(|h| h.bet).unwrap_or(0),
            Action::Insure => self.base / 2,
            _ => 0,
        }
    }

    pub fn act(&mut self, a: Action) -> std::result::Result<(), String> {
        if !self.allowed().contains(&a) {
            return Err(format!("{} isn't something you can do now", a.name()));
        }
        self.actions.push(a);
        match a {
            Action::Insure | Action::NoInsurance => {
                if a == Action::Insure {
                    self.insurance = self.base / 2;
                }
                self.after_insurance();
            }
            Action::Hit => {
                let c = self.shoe.draw(&mut self.s);
                let h = &mut self.hands[self.active];
                h.cards.push(c);
                if h.value().0 >= 21 {
                    h.done = true;
                }
                self.advance();
            }
            Action::Stand => {
                self.hands[self.active].done = true;
                self.advance();
            }
            Action::Double => {
                let c = self.shoe.draw(&mut self.s);
                let h = &mut self.hands[self.active];
                h.bet *= 2;
                h.doubled = true;
                h.cards.push(c);
                h.done = true;
                self.advance();
            }
            Action::Split => {
                let i = self.active;
                let aces = self.hands[i].cards[0].rank() == 12;
                let second = self.hands[i].cards.pop().unwrap();
                let bet = self.hands[i].bet;
                let c1 = self.shoe.draw(&mut self.s);
                let c2 = self.shoe.draw(&mut self.s);
                let h = &mut self.hands[i];
                h.cards.push(c1);
                h.from_split = true;
                h.split_aces = aces;
                let mut other = Hand::new(vec![second, c2], bet);
                other.from_split = true;
                other.split_aces = aces;
                for h in [&mut self.hands[i], &mut other] {
                    if aces || h.value().0 == 21 {
                        h.done = true;
                    }
                }
                self.hands.insert(i + 1, other);
                self.advance();
            }
        }
        Ok(())
    }

    /// On to the next hand still to play, or the dealer's turn.
    fn advance(&mut self) {
        if !self.hands[self.active].done {
            return;
        }
        if let Some(next) = (self.active + 1..self.hands.len()).find(|&i| !self.hands[i].done) {
            self.active = next;
            return;
        }
        // (Hands split later in the list are dealt done when they're aces; any other hand
        // before them was played in turn.)
        self.finish();
    }

    /// The dealer plays (if anyone's still in) and every hand is settled.
    fn finish(&mut self) {
        self.phase = Phase::Over;
        let anyone_in = self.hands.iter().any(|h| h.value().0 <= 21) && !self.dealer_blackjack;
        let only_blackjack = self.hands.len() == 1 && self.hands[0].is_blackjack();
        if anyone_in && !only_blackjack {
            loop {
                let (t, _) = value(&self.dealer);
                if t >= 17 {
                    break;
                }
                let c = self.shoe.draw(&mut self.s);
                self.dealer.push(c);
            }
        }
        let (d, _) = value(&self.dealer);
        let dealer_bj = self.dealer_blackjack;
        for h in &mut self.hands {
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

    pub fn wagered(&self) -> u64 {
        self.hands.iter().map(|h| h.bet).sum::<u64>() + self.insurance
    }

    pub fn paid(&self) -> u64 {
        if self.phase != Phase::Over {
            return 0;
        }
        let insurance = if self.dealer_blackjack { self.insurance * 3 } else { 0 };
        self.hands.iter().map(|h| h.paid).sum::<u64>() + insurance
    }

    pub fn view(&self) -> View {
        let hidden = self.phase != Phase::Over;
        View {
            base: self.base,
            dealer: if hidden { vec![self.dealer[0]] } else { self.dealer.clone() },
            hole_hidden: hidden,
            hands: self.hands.clone(),
            active: self.active,
            phase: self.phase,
            insurance: self.insurance,
            allowed: self.allowed(),
            wagered: self.wagered(),
        }
    }

    pub fn outcome(&self) -> Outcome {
        Outcome {
            base: self.base,
            dealer: self.dealer.clone(),
            hands: self.hands.clone(),
            insurance: self.insurance,
            dealer_blackjack: self.dealer_blackjack,
            actions: self.actions.clone(),
            wagered: self.wagered(),
            paid: self.paid().min(super::MAX_PAYOUT),
        }
    }
}

/// Plays a hand again from its seed and moves (the player's Pyr checking the house).
pub fn replay(chips: u64, s: Stream, actions: &[Action]) -> std::result::Result<Outcome, String> {
    let mut r = Round::deal(chips, s);
    for a in actions {
        r.act(*a)?;
    }
    if r.phase != Phase::Over {
        return Err("the hand isn't over".into());
    }
    Ok(r.outcome())
}

/// Basic strategy for these rules (six decks, dealer stands on soft 17, double after split, no
/// surrender): the move a careful player makes. The app shows it as a hint; the tests use it.
pub fn basic(cards: &[Card], dealer_up: Card, can_double: bool, can_split: bool) -> Action {
    let up = card_value(dealer_up);
    let up = if up == 11 { 1 } else { up };
    let (t, soft) = value(cards);
    if can_split && cards.len() == 2 {
        let p = card_value(cards[0]);
        let split = match p {
            11 => true,
            10 => false,
            9 => !matches!(up, 7 | 10 | 1),
            8 => true,
            7 => (2..=7).contains(&up),
            6 => (2..=6).contains(&up),
            5 => false,
            4 => (5..=6).contains(&up),
            _ => (2..=7).contains(&up),
        };
        if split {
            return Action::Split;
        }
    }
    let double = |ok: bool| if ok && can_double { Action::Double } else { Action::Hit };
    if soft && cards.len() >= 2 {
        return match t {
            20.. => Action::Stand,
            19 => Action::Stand,
            18 => {
                if (3..=6).contains(&up) && can_double {
                    Action::Double
                } else if (2..=8).contains(&up) {
                    Action::Stand
                } else {
                    Action::Hit
                }
            }
            17 => double((3..=6).contains(&up)),
            15 | 16 => {
                if (4..=6).contains(&up) {
                    double(true)
                } else {
                    Action::Hit
                }
            }
            _ => {
                if (5..=6).contains(&up) {
                    double(true)
                } else {
                    Action::Hit
                }
            }
        };
    }
    match t {
        17.. => Action::Stand,
        13..=16 => {
            if (2..=6).contains(&up) {
                Action::Stand
            } else {
                Action::Hit
            }
        }
        12 => {
            if (4..=6).contains(&up) {
                Action::Stand
            } else {
                Action::Hit
            }
        }
        11 => double(up != 1),
        10 => double((2..=9).contains(&up)),
        9 => {
            if (3..=6).contains(&up) {
                double(true)
            } else {
                Action::Hit
            }
        }
        _ => Action::Hit,
    }
}
