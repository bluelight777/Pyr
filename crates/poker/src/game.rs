//! A table and the hands played at it: seats, the button, blinds and antes, betting rounds with
//! no limit, pot limit or fixed limit, all-ins and side pots, the showdown and who wins what.
//!
//! The table is only the rules: it doesn't keep time, network or pick cards. Whoever runs it
//! (the Pyr server) gives it the shuffled deck for each hand, passes it each player's action,
//! and sends on the [`Event`]s it returns (each says who may see it: hole cards only go to
//! their owner until the showdown).

use serde::{Deserialize, Serialize};

use crate::cards::Card;
use crate::eval::{self, Value};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Variant {
    /// Texas Hold'em: two cards each, any five of seven.
    Holdem,
    /// Omaha: four cards each, exactly two of them and three from the board.
    Omaha,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Limit {
    NoLimit,
    PotLimit,
    /// Bets and raises of one size (the big blind on the first two rounds, twice that on the
    /// last two), at most four a round.
    FixedLimit,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Rules {
    pub variant: Variant,
    pub limit: Limit,
    pub small_blind: u64,
    pub big_blind: u64,
    #[serde(default)]
    pub ante: u64,
    /// 2 to 9.
    pub seats: usize,
}

impl Rules {
    pub fn holdem(small_blind: u64, big_blind: u64, seats: usize) -> Rules {
        Rules { variant: Variant::Holdem, limit: Limit::NoLimit, small_blind, big_blind, ante: 0, seats }
    }

    pub fn check(&self) -> Result<(), String> {
        if !(2..=9).contains(&self.seats) {
            return Err("a table has 2 to 9 seats".into());
        }
        if self.big_blind == 0 || self.small_blind == 0 || self.small_blind > self.big_blind {
            return Err("the blinds have to be more than nothing, and the small one no bigger than the big one".into());
        }
        if self.ante > self.big_blind {
            return Err("the ante can't be more than the big blind".into());
        }
        Ok(())
    }

    pub fn hole_cards(&self) -> usize {
        match self.variant {
            Variant::Holdem => 2,
            Variant::Omaha => 4,
        }
    }

    /// "No Limit Hold'em 5/10".
    pub fn name(&self) -> String {
        let limit = match self.limit {
            Limit::NoLimit => "No Limit",
            Limit::PotLimit => "Pot Limit",
            Limit::FixedLimit => "Limit",
        };
        let game = match self.variant {
            Variant::Holdem => "Hold'em",
            Variant::Omaha => "Omaha",
        };
        format!("{limit} {game} {}/{}", self.small_blind, self.big_blind)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Street {
    Preflop,
    Flop,
    Turn,
    River,
}

impl Street {
    pub fn name(self) -> &'static str {
        match self {
            Street::Preflop => "Preflop",
            Street::Flop => "Flop",
            Street::Turn => "Turn",
            Street::River => "River",
        }
    }
}

/// What a player does when it's their turn.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Action {
    Fold,
    Check,
    Call,
    /// Bet or raise so this round's chips in front of them come to `to` (all-in when that's
    /// everything they have).
    Raise { to: u64 },
    /// Everything they have.
    AllIn,
}

/// What a player may do now.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Legal {
    pub can_check: bool,
    /// Chips to call (0 when they can check); all-in for less when that's what they have.
    pub call: u64,
    /// Whether they may bet or raise at all.
    pub can_raise: bool,
    /// The smallest and largest totals for this round they may raise to (the largest is
    /// all-in in no limit; a raise to less than the smallest is only allowed all-in).
    pub min_to: u64,
    pub max_to: u64,
    /// Whether nobody has bet this round (so it's a "bet", not a "raise").
    pub is_bet: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Seat {
    /// Who sits here (a Pyr user id, or a bot's id).
    pub player: String,
    pub name: String,
    pub stack: u64,
    /// Sitting out: dealt no hands until they come back.
    #[serde(default)]
    pub sitting_out: bool,
    /// At a showdown, a hand that can't win anything isn't shown (mucked).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub muck: bool,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct InHand {
    pub seat: usize,
    pub hole: Vec<Card>,
    /// Chips in front of them this round.
    pub street_bet: u64,
    /// Chips they've put in this hand.
    pub total: u64,
    pub folded: bool,
    pub all_in: bool,
    /// Has acted this round (the blinds haven't, by posting).
    pub acted: bool,
    /// The number of full raises this round when they last acted (a raise all-in for less
    /// than a full raise doesn't give back the right to raise to those who already acted).
    seen_raises: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandState {
    pub number: u64,
    pub street: Street,
    pub board: Vec<Card>,
    pub players: Vec<InHand>,
    /// The seat whose turn it is (None: nobody's, the hand is being dealt out or is over).
    pub to_act: Option<usize>,
    pub button: usize,
    /// The highest chips in front of anyone this round.
    pub current_bet: u64,
    /// The size of the last full bet or raise this round (the next raise is at least this much more).
    pub last_raise: u64,
    /// Full bets and raises this round (fixed limit stops at four).
    pub raises: u32,
    /// The deck as shuffled, and how many cards have come off it.
    #[serde(skip)]
    deck: Vec<Card>,
    pub next_card: usize,
    /// Who bet or raised last on the last round (they show first at the showdown).
    pub last_aggressor: Option<usize>,
    /// Every action so far this hand, in order (everyone at the table saw them all).
    #[serde(default)]
    pub actions: Vec<Played>,
}

/// An action as everyone saw it: on which street, by whom, and the chips it put in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Played {
    pub street: Street,
    pub seat: usize,
    pub action: Action,
    pub put: u64,
}

impl HandState {
    pub fn player(&self, seat: usize) -> Option<&InHand> {
        self.players.iter().find(|p| p.seat == seat)
    }

    fn player_mut(&mut self, seat: usize) -> Option<&mut InHand> {
        self.players.iter_mut().find(|p| p.seat == seat)
    }

    /// All the chips put in this hand.
    pub fn pot(&self) -> u64 {
        self.players.iter().map(|p| p.total).sum()
    }

    fn live(&self) -> impl Iterator<Item = &InHand> {
        self.players.iter().filter(|p| !p.folded)
    }

    fn draw(&mut self) -> Card {
        let c = self.deck[self.next_card];
        self.next_card += 1;
        c
    }
}

/// Who may see an event.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum To {
    Everyone,
    /// Only whoever sits in this seat.
    Seat(usize),
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    HandStarted { number: u64, button: usize, seats: Vec<usize> },
    Posted { seat: usize, what: Posting, amount: u64 },
    /// A player's own cards (to them only).
    Dealt { seat: usize, cards: Vec<Card> },
    /// Everyone else sees that cards went to these seats.
    DealtFaceDown { seats: Vec<usize>, count: usize },
    Acted { seat: usize, action: Action, amount: u64, stack: u64 },
    /// Chips nobody called, back to whoever put them in.
    Returned { seat: usize, amount: u64 },
    Street { street: Street, cards: Vec<Card>, board: Vec<Card> },
    /// Whose turn it is, and what they may do.
    Turn { seat: usize, legal: Legal },
    /// Cards turned over at the showdown (or when everyone's all-in).
    Shown { seat: usize, cards: Vec<Card>, hand: String },
    Mucked { seat: usize },
    Won { pot: usize, seats: Vec<usize>, amounts: Vec<u64>, hand: Option<String>, best: Vec<Card> },
    HandEnded {
        number: u64,
        stacks: Vec<(usize, u64)>,
        /// Chips the house took from the pot (the rake: it goes to the Friday Night pool).
        #[serde(default, skip_serializing_if = "is_zero")]
        rake: u64,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Posting {
    Ante,
    SmallBlind,
    BigBlind,
}

/// A pot: its chips and who can win it.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Pot {
    pub amount: u64,
    pub eligible: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Table {
    pub rules: Rules,
    pub seats: Vec<Option<Seat>>,
    pub button: Option<usize>,
    pub hand: Option<HandState>,
    pub hands_played: u64,
    /// Whether this hand is raked (the room decides as it deals).
    #[serde(default)]
    pub raked: bool,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// The rake: 5% of the pot, at most 3 big blinds, and none when there's no flop.
pub const RAKE_PERCENT: u64 = 5;
pub const RAKE_CAP_BB: u64 = 3;

/// What the house takes from a pot of `pot` chips (`flop`: the flop came).
pub fn rake_for(pot: u64, big_blind: u64, flop: bool) -> u64 {
    if !flop {
        return 0;
    }
    (pot * RAKE_PERCENT / 100).min(RAKE_CAP_BB * big_blind)
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Error {
    NoHand,
    NotYourTurn,
    Illegal(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NoHand => write!(f, "no hand is being played"),
            Error::NotYourTurn => write!(f, "it isn't your turn"),
            Error::Illegal(s) => write!(f, "{s}"),
        }
    }
}

pub type Out = Vec<(To, Event)>;

impl Table {
    pub fn new(rules: Rules) -> Table {
        let n = rules.seats;
        Table { rules, seats: vec![None; n], button: None, hand: None, hands_played: 0, raked: false }
    }

    pub fn sit(&mut self, seat: usize, player: &str, name: &str, chips: u64) -> Result<(), String> {
        if seat >= self.seats.len() {
            return Err("there's no such seat".into());
        }
        if self.seats[seat].is_some() {
            return Err("that seat is taken".into());
        }
        if self.seats.iter().flatten().any(|s| s.player == player) {
            return Err("you're already at this table".into());
        }
        self.seats[seat] = Some(Seat { player: player.into(), name: name.into(), stack: chips, sitting_out: false, muck: false });
        Ok(())
    }

    /// Leaves the table (in a hand, they fold first: the caller does that with [`Table::act`]).
    pub fn stand(&mut self, seat: usize) -> Option<Seat> {
        if self.hand.as_ref().is_some_and(|h| h.player(seat).is_some_and(|p| !p.folded)) {
            return None;
        }
        self.seats.get_mut(seat).and_then(|s| s.take())
    }

    pub fn seat_of(&self, player: &str) -> Option<usize> {
        self.seats.iter().position(|s| s.as_ref().is_some_and(|s| s.player == player))
    }

    /// Seats that would be dealt in now.
    pub fn ready(&self) -> Vec<usize> {
        self.seats.iter().enumerate().filter(|(_, s)| s.as_ref().is_some_and(|s| !s.sitting_out && s.stack > 0)).map(|(i, _)| i).collect()
    }

    fn next_from(&self, from: usize, among: &[usize]) -> usize {
        let n = self.seats.len();
        (1..=n).map(|d| (from + d) % n).find(|s| among.contains(s)).unwrap_or(from)
    }

    /// Starts a hand with this shuffled deck (at least 2 players ready).
    pub fn start_hand(&mut self, number: u64, deck: Vec<Card>) -> Result<Out, String> {
        if self.hand.is_some() {
            return Err("a hand is already being played".into());
        }
        let ready = self.ready();
        if ready.len() < 2 {
            return Err("it takes two players".into());
        }
        if deck.len() != 52 {
            return Err("that isn't a deck".into());
        }
        let n = self.seats.len();
        let button = match self.button {
            None => ready[0],
            Some(b) => self.next_from(b, &ready),
        };
        let _ = n;
        self.button = Some(button);
        let mut out: Out = Vec::new();
        // Heads-up the button is the small blind and acts first before the flop.
        let (sb, bb) = if ready.len() == 2 {
            (button, self.next_from(button, &ready))
        } else {
            let sb = self.next_from(button, &ready);
            (sb, self.next_from(sb, &ready))
        };
        // Players in the order they're dealt (from the small blind round).
        let mut order = Vec::new();
        let mut s = sb;
        for _ in 0..ready.len() {
            order.push(s);
            s = self.next_from(s, &ready);
        }
        let mut hs = HandState {
            number,
            street: Street::Preflop,
            board: Vec::new(),
            players: order
                .iter()
                .map(|&seat| InHand { seat, hole: Vec::new(), street_bet: 0, total: 0, folded: false, all_in: false, acted: false, seen_raises: 0 })
                .collect(),
            to_act: None,
            button,
            current_bet: 0,
            last_raise: self.rules.big_blind,
            // (The big blind is the round's first bet.)
            raises: 1,
            deck,
            next_card: 0,
            last_aggressor: None,
            actions: Vec::new(),
        };
        out.push((To::Everyone, Event::HandStarted { number, button, seats: order.clone() }));
        // Antes, then the blinds (all-in for less when that's all they have).
        if self.rules.ante > 0 {
            for &seat in &order {
                let amt = self.post(&mut hs, seat, self.rules.ante, false);
                out.push((To::Everyone, Event::Posted { seat, what: Posting::Ante, amount: amt }));
            }
        }
        let a = self.post(&mut hs, sb, self.rules.small_blind, true);
        out.push((To::Everyone, Event::Posted { seat: sb, what: Posting::SmallBlind, amount: a }));
        let a = self.post(&mut hs, bb, self.rules.big_blind, true);
        out.push((To::Everyone, Event::Posted { seat: bb, what: Posting::BigBlind, amount: a }));
        hs.current_bet = self.rules.big_blind.max(hs.players.iter().map(|p| p.street_bet).max().unwrap_or(0));
        // The cards: one at a time round the table.
        let k = self.rules.hole_cards();
        for _ in 0..k {
            for i in 0..hs.players.len() {
                let c = hs.draw();
                hs.players[i].hole.push(c);
            }
        }
        for p in &hs.players {
            out.push((To::Seat(p.seat), Event::Dealt { seat: p.seat, cards: p.hole.clone() }));
        }
        out.push((To::Everyone, Event::DealtFaceDown { seats: order.clone(), count: k }));
        // First to act: after the big blind.
        let first = self.next_actor(&hs, bb);
        hs.to_act = first;
        self.hand = Some(hs);
        self.hands_played += 1;
        self.advance(&mut out);
        Ok(out)
    }

    /// Puts chips in front of a player (up to what they have); returns how much went in.
    fn post(&self, hs: &mut HandState, seat: usize, amount: u64, street: bool) -> u64 {
        let stack = self.seats[seat].as_ref().map(|s| s.stack).unwrap_or(0) - hs.player(seat).map(|p| p.total).unwrap_or(0);
        let amt = amount.min(stack);
        let p = hs.player_mut(seat).unwrap();
        p.total += amt;
        if street {
            p.street_bet += amt;
        }
        if amt == stack {
            p.all_in = true;
        }
        amt
    }

    /// What's left in front of a player during a hand (their stack less what they've put in).
    pub fn behind(&self, seat: usize) -> u64 {
        let stack = self.seats.get(seat).and_then(|s| s.as_ref()).map(|s| s.stack).unwrap_or(0);
        let put = self.hand.as_ref().and_then(|h| h.player(seat)).map(|p| p.total).unwrap_or(0);
        stack - put
    }

    /// The next player after `from` who can still act.
    fn next_actor(&self, hs: &HandState, from: usize) -> Option<usize> {
        let can: Vec<usize> = hs.players.iter().filter(|p| !p.folded && !p.all_in).map(|p| p.seat).collect();
        if can.is_empty() {
            return None;
        }
        Some(self.next_from(from, &can))
    }

    /// What the player whose turn it is may do.
    pub fn legal(&self, seat: usize) -> Option<Legal> {
        let hs = self.hand.as_ref()?;
        if hs.to_act != Some(seat) {
            return None;
        }
        let p = hs.player(seat)?;
        let behind = self.behind(seat);
        let to_call = hs.current_bet.saturating_sub(p.street_bet);
        let call = to_call.min(behind);
        let all_in_to = p.street_bet + behind;
        let reopened = !p.acted || p.seen_raises < hs.raises;
        // Is anyone else still able to call a raise?
        let others_can_act = hs.players.iter().any(|q| q.seat != seat && !q.folded && !q.all_in);
        let mut can_raise = reopened && behind > to_call && others_can_act;
        let bb = self.rules.big_blind;
        let (min_to, max_to) = match self.rules.limit {
            Limit::NoLimit => ((hs.current_bet + hs.last_raise.max(bb)).min(all_in_to), all_in_to),
            Limit::PotLimit => {
                let min = (hs.current_bet + hs.last_raise.max(bb)).min(all_in_to);
                // Raise by the pot after calling: the pot, plus the call, plus the call again.
                let pot_after_call = hs.pot() + to_call;
                let max = (hs.current_bet + pot_after_call).max(min).min(all_in_to);
                (min, max)
            }
            Limit::FixedLimit => {
                let size = if hs.street >= Street::Turn { 2 * bb } else { bb };
                if hs.raises >= 4 {
                    can_raise = false;
                }
                let to = (hs.current_bet + size).min(all_in_to);
                (to, to)
            }
        };
        Some(Legal { can_check: to_call == 0, call, can_raise, min_to, max_to, is_bet: hs.current_bet == 0 })
    }

    /// A player's action. Returns what happened (including the next deals, the showdown and
    /// the end of the hand when this ends a round).
    pub fn act(&mut self, seat: usize, action: Action) -> Result<Out, Error> {
        let Some(hs) = self.hand.as_ref() else { return Err(Error::NoHand) };
        if hs.to_act != Some(seat) {
            return Err(Error::NotYourTurn);
        }
        let legal = self.legal(seat).ok_or(Error::NotYourTurn)?;
        let behind = self.behind(seat);
        let mut out = Vec::new();
        let hs = self.hand.as_mut().unwrap();
        let street_bet = hs.player(seat).unwrap().street_bet;
        let raises = hs.raises;
        let (action, put) = match action {
            Action::Fold => (Action::Fold, 0),
            Action::Check => {
                if !legal.can_check {
                    return Err(Error::Illegal("there's a bet to call".into()));
                }
                (Action::Check, 0)
            }
            Action::Call => {
                if legal.can_check {
                    (Action::Check, 0)
                } else {
                    (Action::Call, legal.call)
                }
            }
            Action::AllIn => {
                let to = street_bet + behind;
                if to <= hs.current_bet || !legal.can_raise {
                    // All-in to call (or for less): a call.
                    (Action::Call, legal.call)
                } else {
                    (Action::Raise { to }, behind)
                }
            }
            Action::Raise { to } => {
                if !legal.can_raise {
                    return Err(Error::Illegal("you can't raise now".into()));
                }
                let all_in_to = street_bet + behind;
                if to > legal.max_to {
                    return Err(Error::Illegal(format!("you can raise to {} at most", legal.max_to)));
                }
                if to < legal.min_to && to != all_in_to {
                    return Err(Error::Illegal(format!("a raise has to be to {} at least", legal.min_to)));
                }
                if to <= hs.current_bet {
                    return Err(Error::Illegal("that's not a raise".into()));
                }
                (Action::Raise { to }, to - street_bet)
            }
        };
        let p = hs.player_mut(seat).unwrap();
        p.acted = true;
        p.seen_raises = raises;
        match action {
            Action::Fold => p.folded = true,
            _ => {
                p.street_bet += put;
                p.total += put;
                if put == behind && put > 0 {
                    p.all_in = true;
                }
            }
        }
        if let Action::Raise { to } = action {
            let size = to - hs.current_bet;
            // A full raise reopens the betting; an all-in for less doesn't.
            if size >= hs.last_raise {
                hs.last_raise = size;
                hs.raises += 1;
                let r = hs.raises;
                if let Some(p) = hs.player_mut(seat) {
                    p.seen_raises = r;
                }
            }
            hs.current_bet = to;
            hs.last_aggressor = Some(seat);
        }
        let stack_left = {
            let stack = self.seats[seat].as_ref().unwrap().stack;
            stack - self.hand.as_ref().unwrap().player(seat).unwrap().total
        };
        let hs = self.hand.as_mut().unwrap();
        let street = hs.street;
        hs.actions.push(Played { street, seat, action, put });
        out.push((To::Everyone, Event::Acted { seat, action, amount: put, stack: stack_left }));
        // Next player, or the end of the round.
        let hs = self.hand.as_ref().unwrap();
        let next = self.next_actor(hs, seat);
        self.hand.as_mut().unwrap().to_act = next;
        self.advance(&mut out);
        Ok(out)
    }

    /// Whether the betting round is over.
    fn round_over(&self) -> bool {
        let hs = self.hand.as_ref().unwrap();
        if hs.live().count() <= 1 {
            return true;
        }
        let can: Vec<&InHand> = hs.players.iter().filter(|p| !p.folded && !p.all_in).collect();
        match can.len() {
            0 => true,
            // Everyone else is all-in: the last one only acts if they're short of the bet.
            1 => can[0].street_bet >= hs.current_bet,
            _ => can.iter().all(|p| p.acted && p.street_bet >= hs.current_bet),
        }
    }

    /// Moves the hand along: says whose turn it is, or ends the round, deals the next
    /// street, and finishes the hand.
    fn advance(&mut self, out: &mut Out) {
        loop {
            if self.hand.is_none() {
                return;
            }
            let live = self.hand.as_ref().unwrap().live().count();
            if live <= 1 {
                self.finish(out);
                return;
            }
            if !self.round_over() {
                let hs = self.hand.as_ref().unwrap();
                if let Some(seat) = hs.to_act
                    && let Some(legal) = self.legal(seat)
                {
                    out.push((To::Everyone, Event::Turn { seat, legal }));
                    return;
                }
            }
            // The round is over: return what nobody could call, then the next street.
            let hs = self.hand.as_mut().unwrap();
            if hs.street == Street::River {
                self.showdown(out);
                return;
            }
            let next = match hs.street {
                Street::Preflop => Street::Flop,
                Street::Flop => Street::Turn,
                _ => Street::River,
            };
            hs.street = next;
            let _burn = hs.draw();
            let n = if next == Street::Flop { 3 } else { 1 };
            let cards: Vec<Card> = (0..n).map(|_| hs.draw()).collect();
            hs.board.extend(&cards);
            for p in &mut hs.players {
                p.street_bet = 0;
                p.acted = false;
                p.seen_raises = 0;
            }
            hs.current_bet = 0;
            hs.raises = 0;
            hs.last_raise = self.rules.big_blind;
            let board = hs.board.clone();
            out.push((To::Everyone, Event::Street { street: next, cards, board }));
            let button = hs.button;
            let hs = self.hand.as_ref().unwrap();
            // After the flop, the first live player left of the button acts first.
            let first = self.next_actor(hs, button);
            self.hand.as_mut().unwrap().to_act = first;
            // Fewer than two who can bet: no betting, deal on (the loop goes round again).
            let can = self.hand.as_ref().unwrap().players.iter().filter(|p| !p.folded && !p.all_in).count();
            if can < 2 {
                self.hand.as_mut().unwrap().to_act = None;
                // Everyone left is all-in (or one isn't): turn the cards over now, once.
                self.reveal_all_in(out);
            }
        }
    }

    /// When the betting is over with players all-in, everyone's cards are shown before the rest
    /// of the board comes.
    fn reveal_all_in(&mut self, out: &mut Out) {
        let hs = self.hand.as_ref().unwrap();
        let already = out.iter().any(|(_, e)| matches!(e, Event::Shown { .. }));
        if already {
            return;
        }
        for p in hs.live() {
            let hand = self.describe(&p.hole, &hs.board);
            out.push((To::Everyone, Event::Shown { seat: p.seat, cards: p.hole.clone(), hand }));
        }
    }

    fn value(&self, hole: &[Card], board: &[Card]) -> (Value, Vec<Card>) {
        match self.rules.variant {
            Variant::Holdem => eval::best_five(&[hole, board].concat()),
            Variant::Omaha => eval::best_omaha(hole, board),
        }
    }

    fn describe(&self, hole: &[Card], board: &[Card]) -> String {
        if board.len() < 3 {
            return String::new();
        }
        self.value(hole, board).0.describe()
    }

    /// The pots, from what everyone put in: a main pot and side pots for all-ins.
    pub fn pots(hs: &HandState) -> Vec<Pot> {
        let mut levels: Vec<u64> = hs.players.iter().filter(|p| p.all_in && !p.folded).map(|p| p.total).collect();
        levels.push(hs.players.iter().map(|p| p.total).max().unwrap_or(0));
        levels.sort();
        levels.dedup();
        let mut pots: Vec<Pot> = Vec::new();
        let mut prev = 0;
        for lvl in levels {
            if lvl <= prev {
                continue;
            }
            let amount: u64 = hs.players.iter().map(|p| p.total.min(lvl).saturating_sub(prev)).sum();
            let eligible: Vec<usize> = hs.players.iter().filter(|p| !p.folded && p.total >= lvl).map(|p| p.seat).collect();
            if amount > 0 {
                // A pot only one player can win, with nobody else's chips in it, is chips back.
                match pots.last_mut() {
                    Some(last) if last.eligible == eligible => last.amount += amount,
                    _ => pots.push(Pot { amount, eligible }),
                }
            }
            prev = lvl;
        }
        pots
    }

    fn showdown(&mut self, out: &mut Out) {
        let hs = self.hand.as_ref().unwrap();
        // Everyone left shows, starting with the last to bet or raise on the river (or the
        // first left of the button). Hands shown when everyone was all-in aren't shown again.
        // Someone set to muck shows only a hand that could still win something: one that beats
        // or ties the best shown so far in a pot they're in; otherwise it goes in face down.
        if !out.iter().any(|(_, e)| matches!(e, Event::Shown { .. })) {
            let live: Vec<usize> = hs.live().map(|p| p.seat).collect();
            let first = hs.last_aggressor.filter(|s| live.contains(s)).unwrap_or_else(|| self.next_from(hs.button, &live));
            let pots = Self::pots(hs);
            let mut best: Vec<Option<Value>> = vec![None; pots.len()];
            let mut s = first;
            for _ in 0..live.len() {
                let p = hs.player(s).unwrap();
                let v = self.value(&p.hole, &hs.board).0;
                let mucks = self.seats.get(s).and_then(|x| x.as_ref()).is_some_and(|x| x.muck);
                let could_win = pots.iter().zip(&best).any(|(pot, b)| pot.eligible.contains(&s) && b.is_none_or(|b| v >= b));
                if !mucks || could_win {
                    out.push((To::Everyone, Event::Shown { seat: s, cards: p.hole.clone(), hand: self.describe(&p.hole, &hs.board) }));
                    for (pot, b) in pots.iter().zip(best.iter_mut()) {
                        if pot.eligible.contains(&s) && b.is_none_or(|x| v > x) {
                            *b = Some(v);
                        }
                    }
                } else {
                    out.push((To::Everyone, Event::Mucked { seat: s }));
                }
                s = self.next_from(s, &live);
            }
        }
        self.finish(out);
    }

    /// Pays the pots and ends the hand.
    fn finish(&mut self, out: &mut Out) {
        let mut hs = self.hand.take().unwrap();
        let mut won: Vec<u64> = vec![0; self.seats.len()];
        // Chips nobody called go back to whoever put them in.
        let top = hs.players.iter().map(|p| p.total).max().unwrap_or(0);
        let tops: Vec<usize> = hs.players.iter().enumerate().filter(|(_, p)| p.total == top).map(|(i, _)| i).collect();
        if tops.len() == 1 {
            let second = hs.players.iter().enumerate().filter(|(i, _)| *i != tops[0]).map(|(_, p)| p.total).max().unwrap_or(0);
            let back = top - second;
            if back > 0 {
                // (What they get back counts as never put in.)
                let p = &mut hs.players[tops[0]];
                p.total -= back;
                out.push((To::Everyone, Event::Returned { seat: p.seat, amount: back }));
            }
        }
        let mut pots = Table::pots(&hs);
        // The rake, from the main pot first.
        let rake = if self.raked { rake_for(pots.iter().map(|p| p.amount).sum(), self.rules.big_blind, hs.board.len() >= 3) } else { 0 };
        let mut left = rake;
        for pot in pots.iter_mut() {
            let take = left.min(pot.amount);
            pot.amount -= take;
            left -= take;
        }
        for (i, pot) in pots.iter().enumerate() {
            if pot.amount == 0 {
                continue;
            }
            let contenders: Vec<usize> = pot.eligible.clone();
            if contenders.len() == 1 {
                // Everyone else in it folded.
                let seat = contenders[0];
                won[seat] += pot.amount;
                out.push((To::Everyone, Event::Won { pot: i, seats: vec![seat], amounts: vec![pot.amount], hand: None, best: vec![] }));
                continue;
            }
            let vals: Vec<(usize, Value, Vec<Card>)> = contenders
                .iter()
                .map(|&s| {
                    let p = hs.player(s).unwrap();
                    let (v, five) = self.value(&p.hole, &hs.board);
                    (s, v, five)
                })
                .collect();
            let best = vals.iter().map(|x| x.1).max().unwrap();
            let mut winners: Vec<usize> = vals.iter().filter(|x| x.1 == best).map(|x| x.0).collect();
            // Odd chips go to the first winner left of the button.
            let n = self.seats.len();
            winners.sort_by_key(|&s| (s + n - hs.button - 1) % n);
            let share = pot.amount / winners.len() as u64;
            let mut odd = pot.amount % winners.len() as u64;
            let mut amounts = Vec::new();
            for &w in &winners {
                let extra = if odd > 0 {
                    odd -= 1;
                    1
                } else {
                    0
                };
                won[w] += share + extra;
                amounts.push(share + extra);
            }
            let five = vals.iter().find(|x| x.0 == winners[0]).map(|x| x.2.clone()).unwrap_or_default();
            out.push((To::Everyone, Event::Won { pot: i, seats: winners, amounts, hand: Some(best.describe()), best: five }));
        }
        // Stacks: less what they put in, plus what they won.
        for p in &hs.players {
            if let Some(s) = self.seats[p.seat].as_mut() {
                s.stack = s.stack - p.total + won[p.seat];
            }
        }
        let stacks = self.seats.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, s.stack))).collect();
        out.push((To::Everyone, Event::HandEnded { number: hs.number, stacks, rake }));
        // Keep the finished hand's cards for the dealer's check (the caller has them in the events).
    }
}

#[cfg(test)]
mod tests;
