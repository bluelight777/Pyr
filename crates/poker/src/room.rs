//! A table as the server runs it: who sits where (people and bots), who's watching, the clock,
//! the shuffle (with every player's Pyr adding to it), bots taking their turns, and what each
//! person is told. It has no network and no clock of its own: the server passes it messages and
//! the time, gives it random numbers, and delivers what it returns.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use crate::bots::talk::{self, Moment};
use crate::bots::{self, Level, Reads};
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
/// A Flash Pot gone off: each push, a moment apart.
/// After a Flash Pot's gone off, each push all in comes this long after the last (and up to
/// `BOMB_PUSH_SPREAD_MS` more, a little different each time, as people push their chips in).
/// An all-in's board comes a street at a time (apps show it so, and the next hand waits for
/// it): the hands turn over, the first street comes, then each next, then the pot's pushed.
pub const RUNOUT_SHOW_MS: u64 = 300;
pub const RUNOUT_FIRST_MS: u64 = 1_200;
pub const RUNOUT_STREET_MS: u64 = 1_400;
pub const RUNOUT_WIN_MS: u64 = 1_000;

/// How long an all-in's board takes to show, `streets` of it still to come.
pub fn runout_ms(streets: usize) -> u64 {
    match streets {
        0 => 0,
        n => RUNOUT_FIRST_MS + RUNOUT_STREET_MS * (n as u64 - 1) + RUNOUT_WIN_MS,
    }
}

pub const BOMB_PUSH_MS: u64 = 1_500;
pub const BOMB_PUSH_SPREAD_MS: u64 = 1_100;

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
    /// A cash hand, for MMR: everyone dealt in (their rating id, name and a bot's level) and
    /// what they won or lost, in big blinds.
    RateHand { players: Vec<(String, String, Option<Level>, f64)> },
    /// A tournament (or Flash Pot) over, for MMR: everyone in it and where they finished.
    RatePlaces { players: Vec<(String, String, Option<Level>, usize)> },
    /// A round at a blackjack table played out for a person (their bets, their hands and what
    /// came back, for their records; the chips themselves come as a `Credit`).
    BjRound { uid: String, outcome: crate::casino::blackjack::Outcome },
    /// A roll at a craps table decided some of a person's bets (for their records; the chips
    /// themselves come as a `Credit`).
    CrapsRoll { uid: String, outcome: crate::casino::craps::Outcome },
    /// A coup at a baccarat table played out for a person (for their records; the chips come as
    /// a `Credit`).
    BacCoup { uid: String, outcome: crate::casino::baccarat::Outcome },
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
    /// Who a bot is (its way of playing, its pace and its voice: see `bots::persona`), and
    /// when it last said something.
    who: &'static bots::Persona,
    said_ms: u64,
}

impl SeatMeta {
    fn new(bot: Option<Level>, look: Look, bank_ms: u64, name: &str) -> SeatMeta {
        SeatMeta { bot, look, bank_ms, misses: 0, leaving: false, who: bots::persona::of(name), said_ms: 0 }
    }
}

/// Bots' table talk: never two lines closer than this at a table, nor from one bot.
const TALK_GAP_MS: u64 = 9_000;
const BOT_TALK_GAP_MS: u64 = 40_000;

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
    /// Bots' choices, think times and table talk this hand (from the dealer's secret seed: as
    /// unpredictable as the cards, and telling nothing about them).
    whim: Stream,
    /// A bot's move, chosen as its turn came, made when its think time is up.
    plan: Option<Plan>,
}

/// What a bot's going to do when its think time's up, and whether it's worth a word.
#[derive(Clone, Copy, Debug)]
struct Plan {
    seat: usize,
    action: Action,
    moment: Option<Moment>,
}

/// Whether a bot's move is worth a word: shoving, or calling someone's shove.
fn moment_of(v: &bots::Seen, action: Action) -> Option<Moment> {
    let me = v.spot.me();
    let shove = matches!(action, Action::AllIn) || matches!(action, Action::Raise { to } if to >= v.spot.legal.max_to && v.spot.limit == crate::game::Limit::NoLimit);
    let calls_shove = action == Action::Call && v.spot.opponents().any(|o| o.all_in && o.street_bet > me.street_bet);
    if shove && v.spot.legal.can_raise {
        Some(Moment::AllIn)
    } else if calls_shove || shove {
        Some(Moment::CalledAllIn)
    } else {
        None
    }
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
    /// A Flash Pot gone off: everyone is all in, every hand (a moment for each to see it); and
    /// gone off in the middle of a hand, from the very next one.
    pub bomb: bool,
    bomb_next: bool,
    /// Blinds for the next hand (a tournament's level went up mid-hand).
    next_rules: Option<Rules>,
    /// What everyone's actions here have shown of how they play (by player): what bots read
    /// their opponents by.
    reads: HashMap<String, Reads>,
    /// When a bot last spoke here, and its lines lately (none said twice in a row).
    talk_ms: u64,
    said: VecDeque<&'static str>,
    /// Tournaments: who's gone out of chips, for the tournament to place (at the next tick).
    busted: Vec<(String, u64)>,
}

/// A hand's actions as everyone saw them, from its events.
fn public_actions(log: &[(To, Event)]) -> Vec<crate::game::Played> {
    let mut street = Street::Preflop;
    let mut out = Vec::new();
    for (_, e) in log {
        match e {
            Event::Street { street: s, .. } => street = *s,
            Event::Acted { seat, action, amount, .. } => out.push(crate::game::Played { street, seat: *seat, action: *action, put: *amount }),
            _ => {}
        }
    }
    out
}

impl Room {
    /// What the bots here have read of everyone dealt in now, by seat.
    fn reads_here(&self) -> Vec<(usize, Reads)> {
        self.table.seats.iter().enumerate().filter_map(|(i, s)| s.as_ref().map(|s| (i, self.reads.get(&s.player).copied().unwrap_or_default()))).collect()
    }

    /// A bot says something at `m`, now and then: one line at a time at the table, each bot
    /// not too often, none repeated soon. (0.12 apps show it; older ones never see it.)
    fn talk(&mut self, seat: usize, m: Moment, now: u64, whim: &mut Stream, out: &mut Output) {
        if now < self.talk_ms + TALK_GAP_MS {
            return;
        }
        let Some(meta) = self.meta.get(&seat) else { return };
        let Some(level) = meta.bot else { return };
        if meta.said_ms > 0 && now < meta.said_ms + BOT_TALK_GAP_MS {
            return;
        }
        let who = meta.who;
        let recent: Vec<&str> = self.said.iter().copied().collect();
        let Some(text) = talk::line(m, level, who, &recent, whim) else { return };
        self.talk_ms = now;
        if let Some(meta) = self.meta.get_mut(&seat) {
            meta.said_ms = now;
        }
        self.said.push_back(text);
        while self.said.len() > 12 {
            self.said.pop_front();
        }
        out.all(ToApp::Say { table: self.id, seat, text: text.to_string() });
    }

    /// A word from a bot as a hand ends, about the most notable thing everyone saw: a bot gone
    /// broke, a bad beat, a big pot, someone gone broke, a showdown, a pot nobody contested.
    #[allow(clippy::too_many_arguments)]
    fn talk_at_hand_end(&mut self, start: &[Option<(String, u64)>], events: &[Event], board: &[Card], pot: u64, now: u64, whim: &mut Stream, out: &mut Output) {
        let bb = self.table.rules.big_blind.max(1);
        let big = pot >= bb * 40;
        let stacks: HashMap<usize, u64> = events.iter().find_map(|e| if let Event::HandEnded { stacks, .. } = e { Some(stacks.iter().copied().collect()) } else { None }).unwrap_or_default();
        let net = |seat: usize| -> i64 { stacks.get(&seat).copied().unwrap_or(0) as i64 - start.get(seat).cloned().flatten().map(|(_, c)| c as i64).unwrap_or(0) };
        let winners: BTreeSet<usize> = events.iter().filter_map(|e| if let Event::Won { seats, amounts, .. } = e { Some(seats.iter().zip(amounts).filter(|(_, a)| **a > 0).map(|(s, _)| *s).collect::<Vec<_>>()) } else { None }).flatten().collect();
        let shown: HashMap<usize, Vec<Card>> = events.iter().filter_map(|e| if let Event::Shown { seat, cards, .. } = e { Some((*seat, cards.clone())) } else { None }).collect();
        let showdown = !shown.is_empty();
        let is_bot = |seat: usize| self.meta.get(&seat).is_some_and(|m| m.bot.is_some());
        let dealt: Vec<usize> = (0..start.len()).filter(|s| start[*s].is_some() && stacks.contains_key(s)).collect();
        let bots_here: Vec<usize> = self.meta.iter().filter(|(_, m)| m.bot.is_some()).map(|(s, _)| *s).collect();
        let mut tries: Vec<(usize, Moment)> = Vec::new();
        for &seat in &dealt {
            if is_bot(seat) && stacks.get(&seat) == Some(&0) {
                tries.push((seat, Moment::Busted));
            }
        }
        for &seat in &dealt {
            if is_bot(seat)
                && net(seat) < 0
                && let Some(cards) = shown.get(&seat)
                && board.len() == 5
                && bots::hand_value(self.table.rules.variant, cards, board).category() >= eval::Category::TwoPair
            {
                tries.push((seat, Moment::BadBeat));
            }
        }
        for &seat in &winners {
            if is_bot(seat) && big {
                tries.push((seat, Moment::WonBig));
            }
        }
        // A person's big moments: a bot (not one who just lost to them, mind) says so.
        let person_moment = dealt.iter().find_map(|&s| (!is_bot(s) && stacks.get(&s) == Some(&0)).then_some(Moment::OtherBusted)).or_else(|| winners.iter().find(|&&s| !is_bot(s) && big).map(|_| Moment::OtherWonBig));
        if let Some(m) = person_moment {
            let quiet: Vec<usize> = bots_here.iter().copied().filter(|b| net(*b) >= 0).collect();
            let pool = if quiet.is_empty() { &bots_here } else { &quiet };
            if !pool.is_empty() {
                tries.push((pool[whim.below(pool.len() as u32) as usize], m));
            }
        }
        for &seat in &winners {
            if is_bot(seat) {
                tries.push((seat, if showdown { Moment::WonShowdown } else { Moment::WonUncontested }));
            }
        }
        for &seat in &dealt {
            if is_bot(seat) && shown.contains_key(&seat) && net(seat) < 0 {
                tries.push((seat, Moment::LostShowdown));
            }
        }
        for (seat, m) in tries {
            let before = self.talk_ms;
            self.talk(seat, m, now, whim, out);
            if self.talk_ms != before {
                break;
            }
        }
    }

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
            bomb_next: false,
            next_rules: None,
            reads: HashMap::new(),
            talk_ms: 0,
            said: VecDeque::new(),
            busted: Vec::new(),
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
                    mmr: None,
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
            // (A Flash Pot gone off leaves nothing to choose.)
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
        self.meta.insert(seat, SeatMeta::new(None, look, BANK_START_MS, name));
        self.watchers.insert(uid.into());
        let mut out = Output::default();
        self.changed(&mut out);
        // A bot says hello, now and then.
        let mut whim = Stream::new(fair::seed_from(now ^ (seat as u64) << 48));
        let bots: Vec<usize> = self.meta.iter().filter(|(_, m)| m.bot.is_some()).map(|(s, _)| *s).collect();
        if !bots.is_empty() {
            let who = bots[whim.below(bots.len() as u32) as usize];
            self.talk(who, Moment::Welcome, now, &mut whim, &mut out);
        }
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
        let lvl = level.look_level();
        self.meta.insert(seat, SeatMeta::new(Some(level), Look { level: lvl, card_back: String::new(), frame: String::new() }, 0, &name));
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
            return Err("the Flash Pot's gone off: everyone's all in, every hand".into());
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
        let mut streets = 0;
        for (to, e) in events {
            match &e {
                Event::Turn { seat, .. } => turn = Some(*seat),
                Event::HandEnded { .. } => ended = true,
                Event::Shown { .. } => showdown = true,
                Event::Street { .. } => streets += 1,
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
            // Streets dealt with nobody left to bet (everyone all in): the board's shown a
            // street at a time, and the next hand waits for it.
            if streets > 0
                && let Phase::Idle { next: Some(t) } = &mut self.phase
            {
                *t += runout_ms(streets);
            }
            return;
        }
        if let Some(seat) = turn {
            let bot = self.meta.get(&seat).and_then(|m| m.bot.map(|l| (l, m.who)));
            let seen = if bot.is_some() && !self.bomb {
                self.table.seen(seat).map(|mut v| {
                    v.reads = self.reads_here();
                    v
                })
            } else {
                None
            };
            let first = self.table.hand.as_ref().is_some_and(|h| h.actions.is_empty());
            let bomb = self.bomb;
            let Phase::Playing(p) = &mut self.phase else { return };
            p.using_bank = false;
            p.plan = None;
            if bomb {
                // (All in, for everyone, one push after another, a moment to see each: a
                // little longer for the hand's first, with everyone's cards just dealt.)
                p.deadline = now + BOMB_PUSH_MS + p.whim.below(BOMB_PUSH_SPREAD_MS as u32) as u64 + if first { 1_000 } else { 0 };
            } else if let Some((level, who)) = bot {
                // A bot chooses as its turn comes (from what it sees: its own cards and the
                // table), then takes as long over it as a person would over that choice, at its
                // own pace: a quick fold, a think before a big call, the odd snap and the odd
                // long tank. It makes its move when the time's up.
                let action = seen.as_ref().map(|v| bots::decide_as(v, level, who, &mut p.whim)).unwrap_or(Action::Fold);
                let moment = seen.as_ref().and_then(|v| moment_of(v, action));
                let ms = seen.as_ref().map(|v| bots::think_ms(&v.spot, level, who, action, &mut p.whim)).unwrap_or(1_200);
                p.plan = Some(Plan { seat, action, moment });
                p.deadline = now + ms;
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
        let Phase::Playing(mut p) = std::mem::replace(&mut self.phase, Phase::Idle { next: Some(now + if showdown { AFTER_SHOWDOWN_MS } else { BETWEEN_MS }) }) else { return };
        // The bomb went off during the hand: the next one's all in.
        if self.bomb_next {
            self.bomb_next = false;
            self.bomb = true;
            self.changed(out);
        }
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
        let shark_in_showdown = shown.iter().any(|s| self.meta.get(s).is_some_and(|m| matches!(m.bot, Some(Level::Shark | Level::Genius))));
        // MMR: a cash hand, chips won against chips lost (tournaments are rated by place).
        if self.tourney.is_none() {
            let end: HashMap<usize, u64> = events.iter().find_map(|e| if let Event::HandEnded { stacks, .. } = e { Some(stacks.iter().copied().collect()) } else { None }).unwrap_or_default();
            let bb = rules.big_blind.max(1) as f64;
            let players: Vec<(String, String, Option<Level>, f64)> = p
                .seats_at_start
                .iter()
                .enumerate()
                .filter_map(|(seat, s)| {
                    let (uid, start) = s.as_ref()?;
                    let now_has = *end.get(&seat)?;
                    let name = self.table.seats.get(seat).cloned().flatten().map(|x| x.name).unwrap_or_default();
                    let bot = self.meta.get(&seat).and_then(|m| m.bot);
                    Some((crate::rating::id_of(uid, &name, bot), name, bot, (now_has as f64 - *start as f64) / bb))
                })
                .collect();
            if players.len() >= 2 {
                out.effects.push(Effect::RateHand { players });
            }
        }
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
        // What everyone's actions showed (for the bots' reads), and a word from a bot.
        let actions = public_actions(&p.log);
        let dealt: Vec<usize> = events.iter().find_map(|e| if let Event::HandStarted { seats, .. } = e { Some(seats.clone()) } else { None }).unwrap_or_default();
        for seat in dealt {
            if let Some(Some((uid, _))) = p.seats_at_start.get(seat) {
                self.reads.entry(uid.clone()).or_default().count(seat, &actions);
            }
        }
        let mut whim = std::mem::replace(&mut p.whim, Stream::new([0; 32]));
        self.talk_at_hand_end(&p.seats_at_start, &events, &board_final, pot, now, &mut whim, out);
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
                    // (Told to the tournament at the table's next tick, however the hand ended:
                    // a person's action ends hands too, and its output goes elsewhere.)
                    self.busted.push((s.player, started));
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
            mmr: None,
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
        out.busted.append(&mut self.busted);
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
                let bot = self.meta.get(&seat).and_then(|m| m.bot.map(|l| (l, m.who)));
                if let Some((level, who)) = bot {
                    let mut s = Stream::new(rng.seed());
                    // Its move, chosen as its turn came (or, without one, chosen now).
                    let (action, moment) = match p.plan.take().filter(|pl| pl.seat == seat) {
                        Some(pl) => (pl.action, pl.moment),
                        None => {
                            let seen = self.table.seen(seat).map(|mut v| {
                                v.reads = self.reads_here();
                                v
                            });
                            let action = seen.as_ref().map(|v| bots::decide_as(v, level, who, &mut s)).unwrap_or(Action::Fold);
                            (action, seen.as_ref().and_then(|v| moment_of(v, action)))
                        }
                    };
                    if self.apply(seat, action, now, &mut out).is_err() {
                        let _ = self.apply(seat, Action::Fold, now, &mut out);
                    } else if let Some(m) = moment {
                        self.talk(seat, m, now, &mut s, &mut out);
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
        let whim = Stream::new(fair::derive(&dealer, "bots"));
        self.phase = Phase::Playing(Box::new(Playing { hand, dealer, commit, seeds, deck, log: Vec::new(), seats_at_start, button, deadline: now, using_bank: false, whim, plan: None }));
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
        self.meta.insert(seat, SeatMeta::new(bot, look, bank, name));
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

    /// The bomb's gone off (a Flash Pot): from the very next hand everyone's all in, every hand
    /// till one is left. A hand being played when it goes off plays out as it was.
    pub fn bomb_goes_off(&mut self, out: &mut Output) {
        if self.bomb || self.bomb_next {
            return;
        }
        if matches!(self.phase, Phase::Playing(_)) {
            self.bomb_next = true;
        } else {
            self.bomb = true;
            self.changed(out);
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
