//! A table as the server runs it: who sits where (people and bots), who's watching, the clock,
//! the shuffle (with every player's Pyr adding to it), bots taking their turns, and what each
//! person is told. It has no network and no clock of its own: the server passes it messages and
//! the time, gives it random numbers, and delivers what it returns.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::bots::{self, Level};
use crate::cards::Card;
use crate::eval;
use crate::fair::{self, Stream};
use crate::game::{Action, Event, Rules, Street, Table, To, Variant};
use crate::net::{BuyIn, HandHistory, PlayerHand, PlayerView, PrizeView, SeatView, Tally, TableId, TableSummary, TableView, ToApp};
use crate::progress::{self, HandResult};

/// How long a player has to act, the time bank they start with and what each hand adds to it.
pub const TURN_MS: u64 = 20_000;
pub const BANK_START_MS: u64 = 30_000;
pub const BANK_PER_HAND_MS: u64 = 2_000;
pub const BANK_MAX_MS: u64 = 60_000;
/// Between hands (longer after a showdown, to see the cards), and for Pyrs to send seeds.
pub const BETWEEN_MS: u64 = 3_000;
pub const AFTER_SHOWDOWN_MS: u64 = 5_000;
pub const SHUFFLE_MS: u64 = 1_200;
/// Turns missed in a row before a player is sat out.
pub const MISSES_TO_SIT_OUT: u32 = 2;
/// How long an away tournament player is waited for.
pub const AWAY_MS: u64 = 1_200;
/// Hands kept for the history.
pub const HISTORY: usize = 30;
/// A Time Bomb gone off: each push, a moment apart.
pub const BOMB_PUSH_MS: u64 = 700;

/// Where the server gets random numbers (its system's generator; tests use a seeded stream).
pub trait Random {
    fn seed(&mut self) -> [u8; 32];
    fn below(&mut self, n: u32) -> u32;
}

impl Random for Stream {
    fn seed(&mut self) -> [u8; 32] {
        let mut s = [0u8; 32];
        for c in s.chunks_mut(4) {
            c.copy_from_slice(&self.next_u32().to_le_bytes());
        }
        s
    }

    fn below(&mut self, n: u32) -> u32 {
        Stream::below(self, n)
    }
}

/// Who a message is for.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Rcpt {
    /// Everyone watching or sitting.
    All,
    User(String),
}

/// What the server does besides sending messages.
#[derive(Clone, PartialEq, Debug)]
pub enum Effect {
    /// Chips back to a player's record (they stood up, or the table closed).
    Credit { uid: String, chips: u64 },
    /// A hand ended: what it meant for each person in it (for their records).
    Hand { uid: String, result: HandResult, big_blind: u64, beat_shark: bool },
    /// Out of a tournament (or won it): where they finished, of how many, and the prize.
    Placed { uid: String, place: usize, entrants: usize, prize: u64, tourney: u64 },
    /// Chips into the Friday Night pool: a hand's rake (or a called-off Friday Night's prize
    /// money, back again).
    Raked { chips: u64 },
    /// A prize box opened by whoever won the hand.
    Prize { uid: String, chips: u64, item: Option<String> },
}

#[derive(Default)]
pub struct Output {
    pub msgs: Vec<(Rcpt, ToApp)>,
    pub effects: Vec<Effect>,
    /// A tournament table: who ran out of chips in the hand just over (taken off the table),
    /// with what they started that hand with (bigger stacks finish higher).
    pub busted: Vec<(String, u64)>,
}

impl Output {
    pub fn extend(&mut self, o: Output) {
        self.msgs.extend(o.msgs);
        self.effects.extend(o.effects);
        self.busted.extend(o.busted);
    }
}

impl Output {
    pub(crate) fn all(&mut self, m: ToApp) {
        self.msgs.push((Rcpt::All, m));
    }

    pub(crate) fn to(&mut self, uid: &str, m: ToApp) {
        self.msgs.push((Rcpt::User(uid.to_string()), m));
    }
}

/// Someone taken off a tournament table to move to another.
#[derive(Clone, Debug)]
pub struct Taken {
    pub uid: String,
    pub name: String,
    pub chips: u64,
    pub look: Look,
    pub bot: Option<Level>,
    pub misses: u32,
}

/// A seated person's looks and level (from their record).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Look {
    pub level: u32,
    pub card_back: String,
    pub frame: String,
}

#[derive(Clone, Debug)]
struct SeatMeta {
    bot: Option<Level>,
    look: Look,
    bank_ms: u64,
    misses: u32,
    /// Asked to leave: they go when the hand is over.
    leaving: bool,
}

enum Phase {
    /// Waiting for players, or for the next hand at `next`.
    Idle { next: Option<u64> },
    /// The dealer has promised its seed; Pyrs send theirs until `deadline`.
    Shuffle { hand: u64, dealer: [u8; 32], commit: [u8; 32], seeds: BTreeMap<String, [u8; 32]>, deadline: u64 },
    Playing(Box<Playing>),
}

struct Playing {
    hand: u64,
    dealer: [u8; 32],
    commit: [u8; 32],
    seeds: Vec<[u8; 32]>,
    deck: Vec<Card>,
    log: Vec<(To, Event)>,
    seats_at_start: Vec<Option<(String, u64)>>,
    button: usize,
    /// When the player to act runs out of time (and into their bank), or a bot acts.
    deadline: u64,
    using_bank: bool,
}

pub struct Room {
    pub id: TableId,
    pub name: String,
    pub owner: String,
    pub buy_in: BuyIn,
    pub table: Table,
    meta: HashMap<usize, SeatMeta>,
    watchers: BTreeSet<String>,
    phase: Phase,
    next_hand: u64,
    history: VecDeque<(HandHistory, Vec<(To, Event)>)>,
    /// How each person (and bot) has played here, and their last hands: (the hand, their
    /// cards, whether everyone saw them).
    tallies: HashMap<String, Tally>,
    /// A prize box waiting for the next hand's winner.
    pub prize: Option<PrizeView>,
    hands_of: HashMap<String, VecDeque<(PlayerHand, Vec<Card>, bool)>>,
    pots: VecDeque<u64>,
    hand_times: VecDeque<u64>,
    /// Part of a tournament: nobody sits, stands or buys chips here themselves (the
    /// tournament seats and moves them), the broke are out, the absent are blinded away.
    pub tourney: Option<u64>,
    /// No new hand until let go (a tournament moving players).
    pub hold: bool,
    /// A Time Bomb gone off: everyone is all in, every hand (a moment for each to see it).
    pub bomb: bool,
    /// Blinds for the next hand (a tournament's level went up mid-hand).
    next_rules: Option<Rules>,
}

impl Room {
    pub fn new(id: TableId, name: &str, owner: &str, rules: Rules, buy_in: BuyIn) -> Result<Room, String> {
        rules.check()?;
        if buy_in.min_bb == 0 || buy_in.min_bb > buy_in.max_bb {
            return Err("the buy-in's least has to be more than nothing and no more than its most".into());
        }
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 40 {
            return Err("a table's name is 1 to 40 characters".into());
        }
        Ok(Room {
            id,
            name: name.into(),
            owner: owner.into(),
            buy_in,
            table: Table::new(rules),
            meta: HashMap::new(),
            watchers: BTreeSet::new(),
            phase: Phase::Idle { next: None },
            next_hand: 1,
            history: VecDeque::new(),
            tallies: HashMap::new(),
            prize: None,
            hands_of: HashMap::new(),
            pots: VecDeque::new(),
            hand_times: VecDeque::new(),
            tourney: None,
            hold: false,
            bomb: false,
            next_rules: None,
        })
    }

    pub fn rules(&self) -> &Rules {
        &self.table.rules
    }

    pub fn min_buy_in(&self) -> u64 {
        self.buy_in.min_bb * self.table.rules.big_blind
    }

    pub fn max_buy_in(&self) -> u64 {
        self.buy_in.max_bb * self.table.rules.big_blind
    }

    pub fn uid_at(&self, seat: usize) -> Option<&str> {
        self.table.seats.get(seat)?.as_ref().map(|s| s.player.as_str())
    }

    fn is_bot(&self, seat: usize) -> bool {
        self.meta.get(&seat).is_some_and(|m| m.bot.is_some())
    }

    pub fn seat_of(&self, uid: &str) -> Option<usize> {
        self.table.seat_of(uid)
    }

    /// People (not bots) sitting here.
    pub fn people(&self) -> Vec<String> {
        self.table.seats.iter().enumerate().filter(|(i, s)| s.is_some() && !self.is_bot(*i)).map(|(_, s)| s.as_ref().unwrap().player.clone()).collect()
    }

    pub fn watchers(&self) -> impl Iterator<Item = &String> {
        self.watchers.iter()
    }

    /// Everyone to tell about what happens: watchers and the people sitting.
    pub fn audience(&self) -> BTreeSet<String> {
        let mut a = self.watchers.clone();
        a.extend(self.people());
        a
    }

    pub fn watch(&mut self, uid: &str) -> ToApp {
        self.watchers.insert(uid.into());
        ToApp::Snapshot { table: self.view(Some(uid)) }
    }

    pub fn unwatch(&mut self, uid: &str) {
        self.watchers.remove(uid);
    }

    pub fn summary(&self) -> TableSummary {
        let seated: Vec<&crate::game::Seat> = self.table.seats.iter().flatten().collect();
        let bots = (0..self.table.seats.len()).filter(|&i| self.is_bot(i)).count();
        let avg = if self.pots.is_empty() { 0 } else { self.pots.iter().sum::<u64>() / self.pots.len() as u64 };
        // Hands in the last hour (from the last 50 hands' times).
        let hph = match (self.hand_times.front(), self.hand_times.back()) {
            (Some(a), Some(b)) if b > a && self.hand_times.len() > 1 => ((self.hand_times.len() - 1) as u64 * 3_600_000 / (b - a)).min(999) as u32,
            _ => 0,
        };
        TableSummary {
            id: self.id,
            name: self.name.clone(),
            rules: self.table.rules.clone(),
            buy_in: self.buy_in,
            players: seated.len() - bots,
            bots,
            seats: self.table.seats.len(),
            average_pot: avg,
            hands_per_hour: hph,
            names: seated.iter().map(|s| s.name.clone()).collect(),
        }
    }

    /// The table as `uid` sees it (their own cards, others' only when shown).
    pub fn view(&self, uid: Option<&str>) -> TableView {
        let my_seat = uid.and_then(|u| self.table.seat_of(u));
        let hs = self.table.hand.as_ref();
        let shown: HashMap<usize, Vec<Card>> = match &self.phase {
            Phase::Playing(p) => p.log.iter().filter_map(|(_, e)| if let Event::Shown { seat, cards, .. } = e { Some((*seat, cards.clone())) } else { None }).collect(),
            _ => HashMap::new(),
        };
        let seats = self
            .table
            .seats
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let s = s.as_ref()?;
                let m = self.meta.get(&i);
                let p = hs.and_then(|h| h.player(i));
                let cards = if Some(i) == my_seat { p.map(|p| p.hole.clone()).unwrap_or_default() } else { shown.get(&i).cloned().unwrap_or_default() };
                Some(SeatView {
                    player: s.player.clone(),
                    name: s.name.clone(),
                    stack: self.table.behind(i),
                    bot: m.and_then(|m| m.bot),
                    sitting_out: s.sitting_out || (self.tourney.is_some() && m.is_some_and(|m| m.bot.is_none() && m.misses >= MISSES_TO_SIT_OUT)),
                    level: m.map(|m| m.look.level).unwrap_or(1),
                    card_back: m.map(|m| m.look.card_back.clone()).unwrap_or_default(),
                    frame: m.map(|m| m.look.frame.clone()).unwrap_or_default(),
                    bet: p.map(|p| p.street_bet).unwrap_or(0),
                    in_hand: p.is_some(),
                    folded: p.is_some_and(|p| p.folded),
                    all_in: p.is_some_and(|p| p.all_in),
                    cards,
                })
            })
            .collect();
        TableView {
            id: self.id,
            name: self.name.clone(),
            rules: self.table.rules.clone(),
            buy_in: self.buy_in,
            owner: self.owner.clone(),
            seats,
            button: self.table.button,
            hand: hs.map(|h| h.number),
            street: hs.map(|h| h.street),
            board: hs.map(|h| h.board.clone()).unwrap_or_default(),
            pot: hs.map(|h| h.pot()).unwrap_or(0),
            to_act: hs.and_then(|h| h.to_act),
            // (A Time Bomb gone off leaves nothing to choose.)
            legal: my_seat.filter(|_| !self.bomb).and_then(|s| self.table.legal(s)),
            my_seat,
            recent: self.history.iter().map(|(h, _)| h.number).collect(),
            tourney: self.tourney,
            prize: self.prize.clone(),
            bomb: self.bomb,
        }
    }

    /// Sits a person down with chips already taken from their record (the caller gives them
    /// back if this fails).
    pub fn sit(&mut self, uid: &str, name: &str, seat: usize, chips: u64, look: Look, now: u64) -> Result<Output, String> {
        if self.tourney.is_some() {
            return Err("this is a tournament table: register for the tournament to play".into());
        }
        if chips < self.min_buy_in() || chips > self.max_buy_in() {
            return Err(format!("sit down with {} to {} chips", self.min_buy_in(), self.max_buy_in()));
        }
        self.table.sit(seat, uid, name, chips)?;
        self.meta.insert(seat, SeatMeta { bot: None, look, bank_ms: BANK_START_MS, misses: 0, leaving: false });
        self.watchers.insert(uid.into());
        let mut out = Output::default();
        self.changed(&mut out);
        self.maybe_start(now, &mut out);
        Ok(out)
    }

    pub fn add_bot(&mut self, seat: usize, level: Level, now: u64, rng: &mut dyn Random) -> Result<Output, String> {
        if self.tourney.is_some() {
            return Err("this is a tournament table".into());
        }
        let names = ["Ace", "Blaze", "Cinder", "Dusk", "Ember", "Flint", "Gale", "Hex", "Ivy", "Jinx", "Koda", "Lux", "Moss", "Nova", "Onyx", "Pike", "Quill", "Rook", "Sage", "Tarn"];
        let taken: Vec<String> = self.table.seats.iter().flatten().map(|s| s.name.clone()).collect();
        let free: Vec<&&str> = names.iter().filter(|n| !taken.iter().any(|t| t.starts_with(**n))).collect();
        let base = free.get(rng.below(free.len().max(1) as u32) as usize).map(|n| n.to_string()).unwrap_or_else(|| "Bot".into());
        let name = base;
        let id = format!("bot:{}:{seat}:{}", self.id, rng.below(1_000_000));
        // Bots sit with the most chips the table allows.
        let chips = self.max_buy_in();
        self.table.sit(seat, &id, &name, chips)?;
        let lvl = match level {
            Level::Rookie => 3,
            Level::Regular => 12,
            Level::Shark => 30,
        };
        self.meta.insert(seat, SeatMeta { bot: Some(level), look: Look { level: lvl, card_back: String::new(), frame: String::new() }, bank_ms: 0, misses: 0, leaving: false });
        let mut out = Output::default();
        self.changed(&mut out);
        self.maybe_start(now, &mut out);
        Ok(out)
    }

    pub fn remove_bot(&mut self, seat: usize) -> Result<Output, String> {
        if self.tourney.is_some() {
            return Err("this is a tournament table".into());
        }
        if !self.is_bot(seat) {
            return Err("there's no bot in that seat".into());
        }
        let mut out = Output::default();
        if self.table.hand.as_ref().is_some_and(|h| h.player(seat).is_some_and(|p| !p.folded)) {
            self.meta.get_mut(&seat).unwrap().leaving = true;
            return Ok(out);
        }
        self.table.stand(seat);
        self.meta.remove(&seat);
        self.changed(&mut out);
        Ok(out)
    }

    /// A person leaves: now, or (in a hand) after folding, when the hand is over.
    pub fn stand(&mut self, uid: &str, now: u64) -> Result<Output, String> {
        if self.tourney.is_some() {
            return Err("in a tournament you play until you're out (sit out to be folded for)".into());
        }
        let seat = self.table.seat_of(uid).ok_or("you aren't sitting here")?;
        let mut out = Output::default();
        let in_hand = self.table.hand.as_ref().is_some_and(|h| h.player(seat).is_some_and(|p| !p.folded));
        if in_hand {
            self.meta.get_mut(&seat).unwrap().leaving = true;
            if self.table.hand.as_ref().and_then(|h| h.to_act) == Some(seat) {
                let hand = self.table.hand.as_ref().unwrap().number;
                return self.act(uid, hand, Action::Fold, now);
            }
            // Fold as soon as it's their turn (see `tick`).
            return Ok(out);
        }
        self.leave(seat, &mut out);
        Ok(out)
    }

    fn leave(&mut self, seat: usize, out: &mut Output) {
        if let Some(s) = self.table.stand(seat) {
            if !self.is_bot(seat) {
                out.effects.push(Effect::Credit { uid: s.player.clone(), chips: s.stack });
            }
            self.meta.remove(&seat);
            self.changed(out);
        }
    }

    pub fn sit_out(&mut self, uid: &str, sitting_out: bool) -> Result<Output, String> {
        let seat = self.table.seat_of(uid).ok_or("you aren't sitting here")?;
        if self.tourney.is_some() {
            // Still dealt in (the blinds come round), folded for at once.
            if let Some(m) = self.meta.get_mut(&seat) {
                m.misses = if sitting_out { MISSES_TO_SIT_OUT } else { 0 };
            }
        } else {
            self.table.seats[seat].as_mut().unwrap().sitting_out = sitting_out;
            if let Some(m) = self.meta.get_mut(&seat) {
                m.misses = 0;
            }
        }
        let mut out = Output::default();
        self.changed(&mut out);
        Ok(out)
    }

    pub fn top_up(&mut self, uid: &str, chips: u64) -> Result<Output, String> {
        if self.tourney.is_some() {
            return Err("no adding chips in a tournament".into());
        }
        let seat = self.table.seat_of(uid).ok_or("you aren't sitting here")?;
        if self.table.hand.as_ref().is_some_and(|h| h.player(seat).is_some()) {
            return Err("you can add chips between hands".into());
        }
        let s = self.table.seats[seat].as_mut().unwrap();
        if s.stack + chips > self.buy_in.max_bb * self.table.rules.big_blind {
            return Err(format!("you can have {} chips at this table at most", self.max_buy_in()));
        }
        s.stack += chips;
        let mut out = Output::default();
        self.changed(&mut out);
        Ok(out)
    }

    /// A player's Pyr's seed for the shuffle that's coming.
    pub fn seed(&mut self, uid: &str, hand: u64, seed: [u8; 32]) {
        if let Phase::Shuffle { hand: h, seeds, .. } = &mut self.phase
            && *h == hand
            && self.table.seat_of(uid).is_some()
        {
            seeds.insert(uid.into(), seed);
        }
    }

    pub fn emote(&mut self, uid: &str, emote: &str) -> Result<Output, String> {
        let seat = self.table.seat_of(uid).ok_or("sit down to send those")?;
        if emote.chars().count() > 16 || emote.is_empty() {
            return Err("that isn't an emote".into());
        }
        let mut out = Output::default();
        out.all(ToApp::Emote { table: self.id, seat, emote: emote.into() });
        Ok(out)
    }

    pub fn time_bank(&mut self, uid: &str, now: u64) -> Result<Output, String> {
        let seat = self.table.seat_of(uid).ok_or("you aren't sitting here")?;
        let mut out = Output::default();
        let bank = self.meta.get(&seat).map(|m| m.bank_ms).unwrap_or(0);
        if let Phase::Playing(p) = &mut self.phase
            && self.table.hand.as_ref().and_then(|h| h.to_act) == Some(seat)
            && !p.using_bank
            && bank > 0
        {
            p.using_bank = true;
            p.deadline = now + bank;
            out.all(ToApp::Clock { table: self.id, seat, ms: bank, bank_ms: 0 });
        }
        Ok(out)
    }

    /// A person's action.
    pub fn act(&mut self, uid: &str, hand: u64, action: Action, now: u64) -> Result<Output, String> {
        let seat = self.table.seat_of(uid).ok_or("you aren't sitting here")?;
        if self.table.hand.as_ref().map(|h| h.number) != Some(hand) {
            return Err("that hand is over".into());
        }
        if self.bomb {
            return Err("the bomb's gone off: everyone's all in, every hand".into());
        }
        let mut out = Output::default();
        if let Some(m) = self.meta.get_mut(&seat) {
            m.misses = 0;
        }
        self.apply(seat, action, now, &mut out).map_err(|e| e.to_string())?;
        Ok(out)
    }

    fn apply(&mut self, seat: usize, action: Action, now: u64, out: &mut Output) -> Result<(), crate::game::Error> {
        // Time spent from the bank comes off it.
        if let Phase::Playing(p) = &mut self.phase
            && p.using_bank
            && let Some(m) = self.meta.get_mut(&seat)
        {
            m.bank_ms = p.deadline.saturating_sub(now);
        }
        let events = self.table.act(seat, action)?;
        self.deliver(events, now, out);
        Ok(())
    }

    /// Sends a hand's events on (and keeps them), sets the clock for the next turn, and ends
    /// the hand when it's over.
    fn deliver(&mut self, events: Vec<(To, Event)>, now: u64, out: &mut Output) {
        let Phase::Playing(p) = &mut self.phase else { return };
        let hand = p.hand;
        let mut ended = false;
        let mut showdown = false;
        let mut turn = None;
        for (to, e) in events {
            match &e {
                Event::Turn { seat, .. } => turn = Some(*seat),
                Event::HandEnded { .. } => ended = true,
                Event::Shown { .. } => showdown = true,
                _ => {}
            }
            p.log.push((to, e.clone()));
            match to {
                To::Everyone => out.all(ToApp::Event { table: self.id, hand, event: e }),
                To::Seat(s) => {
                    if let Some(uid) = self.table.seats.get(s).and_then(|x| x.as_ref()).map(|x| x.player.clone())
                        && !uid.starts_with("bot:")
                    {
                        out.to(&uid, ToApp::Event { table: self.id, hand, event: e });
                    }
                }
            }
        }
        if ended {
            self.end_hand(showdown, now, out);
            return;
        }
        if let Some(seat) = turn {
            let bot = self.is_bot(seat);
            let Phase::Playing(p) = &mut self.phase else { return };
            p.using_bank = false;
            if self.bomb {
                // (All in, for everyone: a moment to see each push.)
                p.deadline = now + BOMB_PUSH_MS;
            } else if bot {
                // Bots think for a moment (longer on bigger decisions), like people.
                p.deadline = now + 900 + (seat as u64 * 377 + hand * 131) % 1600;
            } else if self.tourney.is_some() && self.meta.get(&seat).is_some_and(|m| m.misses >= MISSES_TO_SIT_OUT) {
                // Away in a tournament: folded for (or checked) after a moment.
                p.deadline = now + AWAY_MS;
            } else {
                p.deadline = now + TURN_MS;
                let bank = self.meta.get(&seat).map(|m| m.bank_ms).unwrap_or(0);
                out.all(ToApp::Clock { table: self.id, seat, ms: TURN_MS, bank_ms: bank });
            }
        }
    }

    fn end_hand(&mut self, showdown: bool, now: u64, out: &mut Output) {
        let Phase::Playing(p) = std::mem::replace(&mut self.phase, Phase::Idle { next: Some(now + if showdown { AFTER_SHOWDOWN_MS } else { BETWEEN_MS }) }) else { return };
        // The shuffle, for anyone to check: every card everyone saw come off the deck.
        let mut seen: Vec<Option<Card>> = vec![None; p.deck.len()];
        let shown: Vec<Card> = p.log.iter().filter_map(|(_, e)| if let Event::Shown { cards, .. } = e { Some(cards.clone()) } else { None }).flatten().collect();
        let board: Vec<Card> = p.log.iter().filter_map(|(_, e)| if let Event::Street { cards, .. } = e { Some(cards.clone()) } else { None }).flatten().collect();
        for (i, c) in p.deck.iter().enumerate() {
            if shown.contains(c) || board.contains(c) {
                seen[i] = Some(*c);
            }
        }
        let last = seen.iter().rposition(|c| c.is_some()).map(|i| i + 1).unwrap_or(0);
        seen.truncate(last);
        out.all(ToApp::Reveal { table: self.id, hand: p.hand, dealer: p.dealer, dealt: seen });
        // Records: what the hand meant for each person in it.
        let rules = self.table.rules.clone();
        let events: Vec<Event> = p.log.iter().map(|(_, e)| e.clone()).collect();
        let board_final = board.clone();
        // Everyone's cards (as dealt; what's shown is the same), for the hands they made.
        let holes: HashMap<usize, Vec<Card>> = p
            .log
            .iter()
            .filter_map(|(_, e)| match e {
                Event::Dealt { seat, cards } | Event::Shown { seat, cards, .. } => Some((*seat, cards.clone())),
                _ => None,
            })
            .collect();
        let shown: BTreeSet<usize> = p.log.iter().filter_map(|(_, e)| if let Event::Shown { seat, .. } = e { Some(*seat) } else { None }).collect();
        let values = |seat: usize| {
            let h = holes.get(&seat)?;
            let (v, _) = match rules.variant {
                Variant::Holdem => eval::best_five(&[h.as_slice(), &board_final].concat()),
                Variant::Omaha => eval::best_omaha(h, &board_final),
            };
            Some((v.0, v.category(), v.describe()))
        };
        let shark_in_showdown = shown.iter().any(|s| self.meta.get(s).is_some_and(|m| m.bot == Some(Level::Shark)));
        for (seat, s) in p.seats_at_start.iter().enumerate() {
            let Some((uid, _)) = s else { continue };
            let result = progress::read_hand(&events, seat, &values);
            if result.played {
                self.note_hand(uid, seat, &result, &events, holes.get(&seat), shown.contains(&seat), &board_final, p.hand, now);
            }
            if uid.starts_with("bot:") {
                continue;
            }
            out.effects.push(Effect::Hand { uid: uid.clone(), result, big_blind: rules.big_blind, beat_shark: shark_in_showdown });
        }
        let pot: u64 = events.iter().filter_map(|e| if let Event::Won { amounts, .. } = e { Some(amounts.iter().sum::<u64>()) } else { None }).sum();
        let rake: u64 = events.iter().map(|e| if let Event::HandEnded { rake, .. } = e { *rake } else { 0 }).sum();
        if rake > 0 {
            out.effects.push(Effect::Raked { chips: rake });
        }
        // A prize box: to whoever won the hand (the main pot's first winner), a person; a bot's
        // win leaves it for the next hand.
        let winner = events.iter().find_map(|e| if let Event::Won { pot: 0, seats, .. } = e { seats.first().copied() } else { None });
        if let Some(seat) = winner
            && let Some(Some((uid, _))) = p.seats_at_start.get(seat)
            && !uid.starts_with("bot:")
            && let Some(prize) = self.prize.take()
        {
            let name = self.table.seats.get(seat).cloned().flatten().map(|s| s.name).unwrap_or_default();
            out.effects.push(Effect::Prize { uid: uid.clone(), chips: prize.chips, item: prize.item.clone() });
            out.all(ToApp::PrizeWon { table: self.id, seat, name, prize });
        }
        self.pots.push_back(pot);
        if self.pots.len() > 50 {
            self.pots.pop_front();
        }
        self.hand_times.push_back(now);
        if self.hand_times.len() > 50 {
            self.hand_times.pop_front();
        }
        let p_start = p.seats_at_start.clone();
        let hist = HandHistory {
            number: p.hand,
            rules: rules.clone(),
            seats: p.seats_at_start.clone(),
            button: p.button,
            events: vec![],
            commit: p.commit,
            dealer: p.dealer,
            seeds: p.seeds.clone(),
            ended_ms: now,
        };
        self.history.push_back((hist, p.log));
        if self.history.len() > HISTORY {
            self.history.pop_front();
        }
        if let Some(r) = self.next_rules.take() {
            self.table.rules = r;
        }
        // Time banks grow; leavers leave; the broke and the absent sit out.
        let seats: Vec<usize> = self.meta.keys().copied().collect();
        for seat in seats {
            let m = self.meta.get_mut(&seat).unwrap();
            if m.bot.is_none() {
                m.bank_ms = (m.bank_ms + BANK_PER_HAND_MS).min(BANK_MAX_MS);
            }
            if m.leaving {
                self.leave(seat, out);
                continue;
            }
            let bot = m.bot.is_some();
            let misses = m.misses;
            if self.tourney.is_some() {
                // Out of chips: out of the tournament (away players stay, and are blinded).
                if self.table.seats[seat].as_ref().is_some_and(|s| s.stack == 0)
                    && let Some(s) = self.table.stand(seat)
                {
                    let started = p_start.get(seat).cloned().flatten().map(|(_, c)| c).unwrap_or(0);
                    out.busted.push((s.player, started));
                    self.meta.remove(&seat);
                }
                continue;
            }
            if let Some(s) = self.table.seats[seat].as_mut() {
                if bot && s.stack == 0 {
                    // A bot that's out of chips buys in again.
                    s.stack = self.buy_in.max_bb * self.table.rules.big_blind;
                }
                if misses >= MISSES_TO_SIT_OUT {
                    s.sitting_out = true;
                }
            }
        }
        self.changed(out);
    }

    /// Notes a finished hand for whoever played it from `seat`.
    #[allow(clippy::too_many_arguments)]
    fn note_hand(&mut self, uid: &str, seat: usize, r: &progress::HandResult, events: &[Event], hole: Option<&Vec<Card>>, shown: bool, board: &[Card], number: u64, now: u64) {
        let net = r.won as i64 - r.put_in as i64;
        let pot: u64 = events.iter().filter_map(|e| if let Event::Won { amounts, .. } = e { Some(amounts.iter().sum::<u64>()) } else { None }).sum();
        let t = self.tallies.entry(uid.to_string()).or_default();
        t.hands += 1;
        t.won += (r.won > 0) as u64;
        t.showdowns += r.showdown as u64;
        t.showdowns_won += (r.showdown && r.won > 0) as u64;
        t.vpip += r.vpip as u64;
        t.pfr += r.pfr as u64;
        t.net += net;
        if r.won > 0 {
            t.biggest_pot = t.biggest_pot.max(pot);
        }
        if let Some((v, _, name)) = &r.shown
            && *v > t.best_value
        {
            t.best_value = *v;
            t.best_hand = name.clone();
        }
        // What happened, in words.
        let mut street = Street::Preflop;
        let mut folded_on = None;
        for e in events {
            match e {
                Event::Street { street: s, .. } => street = *s,
                Event::Acted { seat: s, action: Action::Fold, .. } if *s == seat && folded_on.is_none() => folded_on = Some(street),
                _ => {}
            }
        }
        let made = r.shown.as_ref().map(|(_, _, n)| n.clone());
        let how = match (folded_on, r.won > 0) {
            (Some(st), _) => format!("Folded {}", match st {
                Street::Preflop => "before the flop",
                Street::Flop => "on the flop",
                Street::Turn => "on the turn",
                _ => "on the river",
            }),
            (None, true) if r.showdown => format!("Won at the showdown with {}", made.unwrap_or_default()),
            (None, true) => "Won (everyone else folded)".to_string(),
            (None, false) => match made {
                Some(m) if r.showdown => format!("Lost at the showdown with {m}"),
                _ => "Lost".to_string(),
            },
        };
        let h = PlayerHand { number, ended_ms: now, cards: vec![], board: board.to_vec(), net, how, kept: true };
        let q = self.hands_of.entry(uid.to_string()).or_default();
        q.push_front((h, hole.cloned().unwrap_or_default(), shown));
        q.truncate(HISTORY);
    }

    /// Whoever sits at `seat`, as `asker` may see them (their own cards only when shown, or
    /// when they're the asker's). `overall`: their record across the server, for people.
    pub fn player(&self, seat: usize, asker: &str, overall: Option<Tally>) -> Option<PlayerView> {
        let s = self.table.seats.get(seat)?.as_ref()?;
        let bot = self.meta.get(&seat).and_then(|m| m.bot);
        let hands = self
            .hands_of
            .get(&s.player)
            .map(|q| {
                q.iter()
                    .map(|(h, cards, shown)| {
                        let mut h = h.clone();
                        if *shown || s.player == asker {
                            h.cards = cards.clone();
                        }
                        h.kept = self.history.iter().any(|(hh, _)| hh.number == h.number);
                        h
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(PlayerView {
            seat,
            name: s.name.clone(),
            bot,
            here: self.tallies.get(&s.player).cloned().unwrap_or_default(),
            overall: if bot.is_some() { None } else { overall },
            hands,
        })
    }

    /// Puts a prize box on the table (one at a time).
    pub fn drop_prize(&mut self, prize: PrizeView, out: &mut Output) -> Result<(), String> {
        if self.prize.is_some() {
            return Err("there's a prize box on this table already: it goes to the next hand's winner".into());
        }
        if self.tourney.is_some() {
            return Err("not at a tournament's table".into());
        }
        self.prize = Some(prize);
        self.changed(out);
        Ok(())
    }

    /// The prize box back (the table closed with it unopened).
    pub fn take_prize(&mut self) -> Option<PrizeView> {
        self.prize.take()
    }

    /// Who sits at `seat` (their id).
    pub fn player_at(&self, seat: usize) -> Option<&str> {
        self.table.seats.get(seat)?.as_ref().map(|s| s.player.as_str())
    }

    /// A finished hand as `uid` may see it: everything shown to everyone, and their own cards.
    pub fn history(&self, uid: &str, hand: u64) -> Option<HandHistory> {
        let (h, log) = self.history.iter().find(|(h, _)| h.number == hand)?;
        let my_seat = h.seats.iter().position(|s| s.as_ref().is_some_and(|(u, _)| u == uid));
        let mut h = h.clone();
        h.events = log.iter().filter(|(to, _)| *to == To::Everyone || matches!(to, To::Seat(s) if Some(*s) == my_seat)).map(|(_, e)| e.clone()).collect();
        Some(h)
    }

    /// Tells everyone the table changed (seats, stacks), as a snapshot each.
    fn changed(&self, out: &mut Output) {
        for uid in self.audience() {
            out.to(&uid, ToApp::Snapshot { table: self.view(Some(&uid)) });
        }
    }

    fn maybe_start(&mut self, now: u64, out: &mut Output) {
        let _ = out;
        if let Phase::Idle { next } = &mut self.phase
            && next.is_none()
            && self.table.ready().len() >= 2
        {
            *next = Some(now + BETWEEN_MS);
        }
    }

    /// The clock: deals the next hand, takes bots' turns, and acts for people whose time ran out.
    pub fn tick(&mut self, now: u64, rng: &mut dyn Random) -> Output {
        let mut out = Output::default();
        match &mut self.phase {
            Phase::Idle { next } => {
                if self.hold || self.table.ready().len() < 2 {
                    *next = None;
                    return out;
                }
                match next {
                    None => *next = Some(now + BETWEEN_MS),
                    Some(t) if now >= *t => {
                        let hand = self.next_hand;
                        self.next_hand += 1;
                        let dealer = rng.seed();
                        let commit = fair::commit(&dealer);
                        out.all(ToApp::Shuffle { table: self.id, hand, commit, ms: SHUFFLE_MS });
                        self.phase = Phase::Shuffle { hand, dealer, commit, seeds: BTreeMap::new(), deadline: now + SHUFFLE_MS };
                    }
                    _ => {}
                }
            }
            Phase::Shuffle { deadline, seeds, .. } => {
                // Everyone's seeds in (bots have none), or time's up: deal.
                let (deadline, have) = (*deadline, seeds.len());
                let people_in = self.table.ready().iter().filter(|&&s| !self.is_bot(s)).count();
                if now >= deadline || have >= people_in {
                    self.deal(now, &mut out);
                }
            }
            Phase::Playing(p) => {
                if now < p.deadline {
                    return out;
                }
                let Some(seat) = self.table.hand.as_ref().and_then(|h| h.to_act) else { return out };
                if self.bomb {
                    // The bomb's gone off: all in (calling, when that's all it can be).
                    for a in [Action::AllIn, Action::Call, Action::Check, Action::Fold] {
                        if self.apply(seat, a, now, &mut out).is_ok() {
                            break;
                        }
                    }
                    return out;
                }
                let bot = self.meta.get(&seat).and_then(|m| m.bot);
                if let Some(level) = bot {
                    let view = self.table.bot_view(seat);
                    let mut s = Stream::new(rng.seed());
                    let action = view.map(|v| bots::decide(&v, level, &mut s)).unwrap_or(Action::Fold);
                    if self.apply(seat, action, now, &mut out).is_err() {
                        let _ = self.apply(seat, Action::Fold, now, &mut out);
                    }
                    return out;
                }
                let leaving = self.meta.get(&seat).is_some_and(|m| m.leaving);
                let away = self.tourney.is_some() && self.meta.get(&seat).is_some_and(|m| m.misses >= MISSES_TO_SIT_OUT);
                let bank = self.meta.get(&seat).map(|m| m.bank_ms).unwrap_or(0);
                if !leaving && !away && !p.using_bank && bank > 0 {
                    // Into the time bank.
                    p.using_bank = true;
                    p.deadline = now + bank;
                    out.all(ToApp::Clock { table: self.id, seat, ms: bank, bank_ms: 0 });
                    return out;
                }
                // Out of time: check if they can, fold if not.
                if let Some(m) = self.meta.get_mut(&seat) {
                    if !leaving && !away {
                        m.misses += 1;
                    }
                    if !away {
                        m.bank_ms = 0;
                    }
                }
                let can_check = self.table.legal(seat).is_some_and(|l| l.can_check);
                let a = if can_check && !leaving { Action::Check } else { Action::Fold };
                let _ = self.apply(seat, a, now, &mut out);
            }
        }
        // A leaver whose turn it is folds at once.
        if let Some(seat) = self.table.hand.as_ref().and_then(|h| h.to_act)
            && self.meta.get(&seat).is_some_and(|m| m.leaving)
            && matches!(self.phase, Phase::Playing(_))
        {
            let _ = self.apply(seat, Action::Fold, now, &mut out);
        }
        out
    }

    fn deal(&mut self, now: u64, out: &mut Output) {
        let Phase::Shuffle { hand, dealer, commit, seeds, .. } = std::mem::replace(&mut self.phase, Phase::Idle { next: None }) else { return };
        if self.table.ready().len() < 2 {
            return;
        }
        let seeds: Vec<[u8; 32]> = seeds.into_values().collect();
        let deck = fair::shuffle(fair::deck_seed(&dealer, &seeds, hand));
        out.all(ToApp::Seeds { table: self.id, hand, fingerprint: fair::seeds_fingerprint(&seeds), seeds: seeds.clone() });
        let seats_at_start = self.table.seats.iter().map(|s| s.as_ref().map(|s| (s.player.clone(), s.stack))).collect();
        // Everyone's view as the hand starts, before the blinds go in: the hand's events (the
        // blinds among them) are played onto it.
        let views: Vec<(String, TableView)> = self
            .audience()
            .into_iter()
            .map(|uid| {
                let mut v = self.view(Some(&uid));
                v.hand = Some(hand);
                (uid, v)
            })
            .collect();
        // Raked when at least two people (not bots) are dealt in, outside tournaments.
        let people = self.table.ready().iter().filter(|&&i| self.table.seats[i].as_ref().is_some_and(|s| !s.player.starts_with("bot:"))).count();
        self.table.raked = self.tourney.is_none() && people >= 2;
        let events = match self.table.start_hand(hand, deck.clone()) {
            Ok(e) => e,
            Err(_) => return,
        };
        let button = self.table.button.unwrap_or(0);
        self.phase = Phase::Playing(Box::new(Playing { hand, dealer, commit, seeds, deck, log: Vec::new(), seats_at_start, button, deadline: now, using_bank: false }));
        for (uid, table) in views {
            out.to(&uid, ToApp::Snapshot { table });
        }
        self.deliver(events, now, out);
    }

    /// Everyone's chips back (the table is closing).
    pub fn close(&mut self) -> Output {
        let mut out = Output::default();
        // A hand in progress is called off: everyone gets back what they started it with.
        let start_stacks: Option<Vec<Option<(String, u64)>>> = match &self.phase {
            Phase::Playing(p) => Some(p.seats_at_start.clone()),
            _ => None,
        };
        for (i, s) in self.table.seats.iter().enumerate() {
            let Some(s) = s else { continue };
            if self.is_bot(i) {
                continue;
            }
            let chips = start_stacks.as_ref().and_then(|st| st.get(i).cloned().flatten()).filter(|(u, _)| *u == s.player).map(|(_, c)| c).unwrap_or(s.stack);
            out.effects.push(Effect::Credit { uid: s.player.clone(), chips });
        }
        out.all(ToApp::Closed { table: self.id });
        out
    }

    /// Tournaments: seats someone (a person or a bot) with their tournament chips.
    pub fn place(&mut self, uid: &str, name: &str, seat: usize, chips: u64, look: Look, bot: Option<Level>, out: &mut Output) -> Result<(), String> {
        self.table.sit(seat, uid, name, chips)?;
        let bank = if bot.is_some() { 0 } else { BANK_START_MS };
        self.meta.insert(seat, SeatMeta { bot, look, bank_ms: bank, misses: 0, leaving: false });
        if bot.is_none() {
            self.watchers.insert(uid.into());
        }
        self.changed(out);
        Ok(())
    }

    /// Tournaments: takes someone off the table between hands (to move them), with their
    /// chips, looks, bot level and time bank.
    pub fn take(&mut self, seat: usize, out: &mut Output) -> Option<Taken> {
        if !self.between_hands() {
            return None;
        }
        let s = self.table.stand(seat)?;
        let m = self.meta.remove(&seat);
        self.watchers.remove(&s.player);
        // They're told the table's gone from under them (the tournament sends the new one).
        out.to(&s.player, ToApp::Closed { table: self.id });
        self.changed(out);
        Some(Taken { uid: s.player, name: s.name, chips: s.stack, look: m.as_ref().map(|m| m.look.clone()).unwrap_or_default(), bot: m.as_ref().and_then(|m| m.bot), misses: m.map(|m| m.misses).unwrap_or(0) })
    }

    /// Tournaments: someone who was away is back (or went away), kept when they move.
    pub fn set_misses(&mut self, seat: usize, misses: u32) {
        if let Some(m) = self.meta.get_mut(&seat) {
            m.misses = misses;
        }
    }

    /// No hand being dealt or played.
    pub fn between_hands(&self) -> bool {
        matches!(self.phase, Phase::Idle { .. })
    }

    /// New blinds: now if between hands, else from the next hand.
    pub fn set_blinds(&mut self, small: u64, big: u64, ante: u64) {
        let mut r = self.next_rules.clone().unwrap_or_else(|| self.table.rules.clone());
        r.small_blind = small;
        r.big_blind = big;
        r.ante = ante;
        if self.between_hands() && self.table.hand.is_none() {
            self.table.rules = r;
            self.next_rules = None;
        } else {
            self.next_rules = Some(r);
        }
    }

    /// Who sits where: (seat, uid) for everyone at the table.
    pub fn seated(&self) -> Vec<(usize, String)> {
        self.table.seats.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, s.player.clone()))).collect()
    }

    /// Empty seats.
    pub fn free_seats(&self) -> Vec<usize> {
        self.table.seats.iter().enumerate().filter(|(_, s)| s.is_none()).map(|(i, _)| i).collect()
    }

    /// Chips each person has at this table now (what they'd get back if it closed).
    pub fn chips_of(&self, uid: &str) -> u64 {
        let Some(seat) = self.table.seat_of(uid) else { return 0 };
        match &self.phase {
            Phase::Playing(p) => p.seats_at_start.get(seat).cloned().flatten().filter(|(u, _)| u == uid).map(|(_, c)| c).unwrap_or(0),
            _ => self.table.seats[seat].as_ref().map(|s| s.stack).unwrap_or(0),
        }
    }
}

#[cfg(test)]
mod tests;
