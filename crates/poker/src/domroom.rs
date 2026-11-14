//! A dominó table in Pyr Casino (0.13.11): four seats for people and bots, a hand every minute
//! or two, as 0.13.28 runs them (its timings, filmed: `rebuild/measured/domino.md`).
//!
//! Before each hand everyone in puts up chips (the ante to eight times it: what covers doubles;
//! what the hand doesn't use comes back) with their Pyr's seed; the hand's tiles come from the
//! house's seed (its fingerprint shown first), theirs and the hand's number. When it's over the
//! house shows its seed and everyone's Pyr plays the hand again (`domino::Hand::replay`). Bots
//! play with the house's chips: in Pairs the house fills empty seats, in Solo it sits one down
//! with someone playing alone; nobody's chips go to or from them.

use std::collections::BTreeSet;

use crate::casino::domino::{self, Hand, Kind, Move, Over};
use crate::fair::{self, Stream};
use crate::net::{DomHandView, DomRecord, DomResult, DomSeatView, DomStage, DomSummary, DomView, ToApp};
use crate::room::{Effect, Look, Output, Random};

pub const SEATS: usize = 4;
/// Once everyone's in, the hand's dealt this soon (the first at a table); after a hand, this
/// soon if everyone's in again, this long while someone isn't.
pub const GATHER_MS: u64 = 1_500;
pub const AGAIN_MS: u64 = 6_000;
pub const WAIT_MS: u64 = 15_000;
/// A person's time for a move; once it's run out, their turns are played for them this
/// quickly for the rest of the hand.
pub const TURN_MS: u64 = 25_000;
pub const AWAY_MS: u64 = 3_000;
/// The hand's over: everything turned up, its result on the table this long.
pub const SETTLE_MS: u64 = 8_000;
/// Hands in a row a person sits out before they're stood up (their seat's for someone playing).
pub const IDLE_HANDS: u32 = 5;
/// What a new table may be opened at.
pub const ANTES: [u64; 19] = [10, 25, 50, 100, 250, 500, 1_000, 2_500, 5_000, 10_000, 25_000, 50_000, 100_000, 250_000, 500_000, 1_000_000, 2_500_000, 5_000_000, 10_000_000];

/// The house's players (0.13.28's: the fourteen it seated, a hundred bots sat down at an empty
/// table and stood up again).
const BOT_NAMES: [&str; 14] = ["Papito", "Yoandri", "Tía Cuca", "Rigo", "Mayelín", "Yuneisy", "Yunior", "Osniel", "Chucho", "Lázaro", "Caridad", "Marlenis", "El Profe", "Tata"];

struct Seat {
    uid: String,
    name: String,
    bot: bool,
    look: Look,
    /// In for the next hand, and what they put up (their Pyr's seed with it).
    ready: bool,
    escrow: u64,
    seed: Option<[u8; 32]>,
    /// Hands in a row not in.
    idle: u32,
    /// Let a turn run out this hand: played for them, quickly.
    away: bool,
    /// Stood up during a hand: their seat's free once it's over.
    leaving: bool,
}

enum Stage {
    Waiting,
    Gathering { until: u64 },
    /// `plan`: a bot's (or an away person's) move, chosen as its turn came, made at `deadline`;
    /// `whim`: bots' choices and think times this hand; `uids` everyone in it (seat, uid, bot);
    /// `cap`: how far the stake can go.
    Playing { hand: Box<Hand>, deadline: u64, seeds: Vec<(u8, [u8; 32])>, uids: Vec<(u8, String, bool)>, plan: Option<(u8, Move)>, whim: Stream, cap: u32 },
    Settled { until: u64 },
}

pub struct DomRoom {
    pub id: u64,
    pub name: String,
    pub kind: Kind,
    pub ante: u64,
    seats: Vec<Option<Seat>>,
    watchers: BTreeSet<String>,
    stage: Stage,
    round_no: u64,
    house: [u8; 32],
    commit: [u8; 32],
    /// The last hand over (on the table until the next is dealt), its result and record.
    finished: Option<Box<Hand>>,
    result: Option<DomResult>,
    last: Option<DomRecord>,
    /// When a person was last here (an empty table closes after a while).
    pub seen_ms: u64,
}

/// What a finished hand's called (0.13.28's words).
pub fn how(over: &Over) -> &'static str {
    match over {
        Over::Domino { capicua: true, .. } => "¡Capicúa!",
        Over::Domino { chiva: true, .. } => "¡Chiva!",
        Over::Domino { .. } => "¡Dominó!",
        Over::Blocked { .. } => "Tranque",
        Over::Fold { .. } => "Folded to a double",
    }
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Checks a finished hand: the house's seed matches its fingerprint, and the hand dealt from
/// everyone's seeds, with every move the record says, ends as it says. What it paid, if so.
pub fn check(r: &DomRecord, ante: u64) -> Result<domino::Payout, String> {
    if fair::commit(&r.house) != r.commit {
        return Err(format!("hand {}: the house's seed doesn't match the fingerprint it showed first", r.round));
    }
    let seeds: Vec<(u8, [u8; 32])> = r.seeds.iter().filter_map(|(s, h)| crate::net::hex32::parse(h).map(|b| (*s, b))).collect();
    if seeds.len() != r.seeds.len() {
        return Err(format!("hand {}: a player's seed isn't a seed", r.round));
    }
    let h = Hand::replay(r.kind, &r.seats, domino::round_seed(&r.house, &seeds, r.round), &r.moves).map_err(|e| format!("hand {}: {e}", r.round))?;
    if h.over.as_ref() != Some(&r.over) {
        return Err(format!("hand {}: played again, it ended differently", r.round));
    }
    Ok(domino::payout(&h, ante))
}

impl DomRoom {
    pub fn new(id: u64, name: &str, kind: Kind, ante: u64, now: u64, rng: &mut dyn Random) -> DomRoom {
        let house = rng.seed();
        DomRoom {
            id,
            name: name.chars().take(40).collect(),
            kind,
            ante: ante.max(1),
            seats: (0..SEATS).map(|_| None).collect(),
            watchers: BTreeSet::new(),
            stage: Stage::Waiting,
            round_no: 1,
            commit: fair::commit(&house),
            house,
            finished: None,
            result: None,
            last: None,
            seen_ms: now,
        }
    }

    fn people(&self) -> impl Iterator<Item = &Seat> {
        self.seats.iter().flatten().filter(|s| !s.bot)
    }

    pub fn seat_of(&self, uid: &str) -> Option<u8> {
        self.seats.iter().position(|s| s.as_ref().is_some_and(|s| s.uid == uid && !s.bot)).map(|i| i as u8)
    }

    /// Everyone to tell: who's watching and who's sitting.
    pub fn audience(&self) -> BTreeSet<String> {
        let mut a = self.watchers.clone();
        a.extend(self.people().map(|s| s.uid.clone()));
        a
    }

    pub fn playing(&self) -> bool {
        matches!(self.stage, Stage::Playing { .. })
    }

    pub fn summary(&self) -> DomSummary {
        DomSummary {
            id: self.id,
            name: self.name.clone(),
            kind: self.kind,
            ante: self.ante,
            people: self.people().count(),
            bots: self.seats.iter().flatten().filter(|s| s.bot).count(),
            playing: self.playing(),
        }
    }

    /// Chips a person has on the table now (what they put up): what the server counts as theirs
    /// while they're here.
    pub fn at_stake(&self, uid: &str) -> u64 {
        self.seat_of(uid).and_then(|s| self.seats[s as usize].as_ref()).map(|s| s.escrow).unwrap_or(0)
    }

    /// People playing a hand here now (for the casino's "playing" count).
    pub fn playing_people(&self) -> usize {
        match &self.stage {
            Stage::Playing { uids, .. } => uids.iter().filter(|(_, _, bot)| !bot).count(),
            _ => 0,
        }
    }

    /// Whether anyone (a person) is sitting or watching: a table nobody's at can close.
    pub fn in_use(&self) -> bool {
        !self.watchers.is_empty() || self.people().next().is_some()
    }

    pub fn knows(&self, uid: &str) -> bool {
        self.watchers.contains(uid) || self.seat_of(uid).is_some()
    }

    /// The least anyone in the hand covered (in antes; 8 between hands).
    fn cap(&self) -> u32 {
        match &self.stage {
            Stage::Playing { cap, .. } => *cap,
            _ => domino::MAX_MULT,
        }
    }

    fn hand_view(&self, h: &Hand, seat: Option<u8>) -> DomHandView {
        let over = h.over.is_some();
        let place = seat.and_then(|s| h.place(s));
        DomHandView {
            kind: h.kind,
            seats: h.seats.clone(),
            counts: (0..h.n()).map(|p| h.tiles(p).len() as u8).collect(),
            pool: h.pool_left(),
            draws: h.draws(),
            chain: h.chain.clone(),
            to_act: h.to_act(),
            mult: h.mult,
            cube: h.cube,
            offer: h.offer,
            laid: h.laid.clone(),
            last: h.moves.last().copied(),
            moves: h.moves.len(),
            mine: if over { Vec::new() } else { place.map(|p| h.tiles(p).to_vec()).unwrap_or_default() },
            shown: if over { (0..h.n()).map(|p| h.tiles(p).to_vec()).collect() } else { Vec::new() },
            over: h.over.clone(),
            allowed: seat.map(|s| h.allowed(s)).unwrap_or_default(),
            lead: h.lead,
        }
    }

    pub fn view(&self, uid: Option<&str>, now: u64) -> DomView {
        let my_seat = uid.and_then(|u| self.seat_of(u));
        let (stage, ms_left) = match &self.stage {
            Stage::Waiting => (DomStage::Waiting, 0),
            Stage::Gathering { until } => (DomStage::Gathering, until.saturating_sub(now)),
            Stage::Playing { deadline, .. } => (DomStage::Playing, deadline.saturating_sub(now)),
            Stage::Settled { until } => (DomStage::Settled, until.saturating_sub(now)),
        };
        let (hand, result) = match &self.stage {
            Stage::Playing { hand, .. } => (Some(self.hand_view(hand, my_seat)), None),
            _ => (self.finished.as_ref().map(|h| self.hand_view(h, my_seat)), self.result.clone()),
        };
        DomView {
            id: self.id,
            name: self.name.clone(),
            kind: self.kind,
            ante: self.ante,
            seats: self
                .seats
                .iter()
                .map(|s| s.as_ref().map(|s| DomSeatView { player: s.uid.clone(), name: s.name.clone(), bot: s.bot, level: s.look.level, frame: s.look.frame.clone(), ready: s.ready, escrow: s.escrow }))
                .collect(),
            stage,
            ms_left,
            round_no: self.round_no,
            commit: self.commit,
            hand,
            last: self.last.clone(),
            result,
            my_seat,
            cap: self.cap(),
        }
    }

    fn changed(&self, now: u64, out: &mut Output) {
        for uid in self.audience() {
            out.to(&uid, ToApp::Dom { view: self.view(Some(&uid), now) });
        }
    }

    pub fn watch(&mut self, uid: &str, now: u64) -> ToApp {
        self.watchers.insert(uid.to_string());
        self.seen_ms = now;
        ToApp::Dom { view: self.view(Some(uid), now) }
    }

    pub fn unwatch(&mut self, uid: &str) {
        self.watchers.remove(uid);
    }

    /// Someone picked another look (a frame) while sitting here: their seat shows it at once.
    pub fn relook(&mut self, uid: &str, look: &Look, now: u64) -> Option<Output> {
        let seat = self.seat_of(uid)?;
        let s = self.seats.get_mut(seat as usize)?.as_mut()?;
        if s.bot || s.look == *look {
            return None;
        }
        s.look = look.clone();
        let mut out = Output::default();
        self.changed(now, &mut out);
        Some(out)
    }

    fn in_hand(&self, seat: u8) -> bool {
        matches!(&self.stage, Stage::Playing { hand, .. } if hand.seats.contains(&seat))
    }

    pub fn sit(&mut self, uid: &str, name: &str, look: Look, seat: u8, now: u64) -> Result<Output, String> {
        if self.seat_of(uid).is_some() {
            return Err("you're sitting here already".into());
        }
        let s = self.seats.get(seat as usize).ok_or("there's no such seat")?;
        if s.is_some() {
            return Err(if self.in_hand(seat) { "that seat's in the hand being played: after it".into() } else { "that seat's taken".into() });
        }
        self.seats[seat as usize] = Some(Seat { uid: uid.into(), name: name.into(), bot: false, look, ready: false, escrow: 0, seed: None, idle: 0, away: false, leaving: false });
        self.watchers.insert(uid.into());
        self.seen_ms = now;
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// Stands up: between hands at once (what they put up comes back); in a hand, after it (their
    /// tiles played for them till then).
    pub fn leave(&mut self, uid: &str, now: u64) -> Output {
        let mut out = Output::default();
        let Some(seat) = self.seat_of(uid) else { return out };
        if self.in_hand(seat) {
            if let Some(s) = self.seats[seat as usize].as_mut() {
                s.leaving = true;
                s.away = true;
            }
            // (Their turn now: played for them at once.)
            if let Stage::Playing { hand, deadline, .. } = &mut self.stage
                && (hand.to_act() == Some(seat) || hand.allowed(seat).contains(&Move::Accept))
            {
                *deadline = (*deadline).min(now + 1_200);
            }
            self.changed(now, &mut out);
            return out;
        }
        if let Some(s) = self.seats[seat as usize].take()
            && s.escrow > 0
        {
            out.effects.push(Effect::Credit { uid: uid.into(), chips: s.escrow });
        }
        self.after_seats(now);
        self.changed(now, &mut out);
        out
    }

    pub fn add_bot(&mut self, seat: u8, now: u64, rng: &mut dyn Random) -> Result<Output, String> {
        let s = self.seats.get(seat as usize).ok_or("there's no such seat")?;
        if s.is_some() {
            return Err("that seat's taken".into());
        }
        if self.playing() {
            return Err("after this hand".into());
        }
        self.sit_bot(seat as usize, rng);
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    fn sit_bot(&mut self, seat: usize, rng: &mut dyn Random) {
        let taken: Vec<String> = self.seats.iter().flatten().map(|s| s.name.clone()).collect();
        let free: Vec<&&str> = BOT_NAMES.iter().filter(|n| !taken.iter().any(|t| t == **n)).collect();
        let name = free.get(rng.below(free.len().max(1) as u32) as usize).map(|n| n.to_string()).unwrap_or_else(|| "Bot".into());
        self.seats[seat] = Some(Seat {
            uid: format!("bot:dom{}:{seat}", self.id),
            name,
            bot: true,
            look: Look { level: 12, ..Look::default() },
            ready: true,
            escrow: 0,
            seed: None,
            idle: 0,
            away: false,
            leaving: false,
        });
    }

    pub fn remove_bot(&mut self, seat: u8, now: u64) -> Result<Output, String> {
        let s = self.seats.get(seat as usize).ok_or("there's no such seat")?;
        if !s.as_ref().is_some_and(|s| s.bot) {
            return Err("there's no bot in that seat".into());
        }
        if self.playing() {
            return Err("after this hand".into());
        }
        self.seats[seat as usize] = None;
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// The chips a person may put up here: the ante to eight times it.
    pub fn cover_range(&self) -> (u64, u64) {
        (self.ante, self.ante * domino::MAX_MULT as u64)
    }

    /// A person's in for the next hand, the chips (already taken from them by the server) put
    /// up, their Pyr's seed with them.
    pub fn ready(&mut self, uid: &str, chips: u64, seed: [u8; 32], now: u64) -> Result<Output, String> {
        let seat = self.seat_of(uid).ok_or("sit down first")?;
        if self.in_hand(seat) {
            return Err("you're in the hand being played".into());
        }
        let (lo, hi) = self.cover_range();
        if chips < lo || chips > hi {
            return Err(format!("this table's ante is {}; put up {} to {}", self.ante, lo, hi));
        }
        let s = self.seats[seat as usize].as_mut().unwrap();
        if s.ready {
            return Err("you're in for the next hand already".into());
        }
        s.ready = true;
        s.escrow = chips;
        s.seed = Some(seed);
        s.idle = 0;
        self.seen_ms = now;
        let all = self.people().all(|s| s.ready);
        match &mut self.stage {
            Stage::Waiting => self.stage = Stage::Gathering { until: now + GATHER_MS },
            Stage::Gathering { until } if all => *until = (*until).min(now + GATHER_MS),
            _ => {}
        }
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// Whether the stake may be doubled now (as far as everyone in the hand covered).
    pub fn can_double(&self) -> bool {
        match &self.stage {
            Stage::Playing { hand, cap, .. } => hand.mult * 2 <= *cap,
            _ => false,
        }
    }

    pub fn act(&mut self, uid: &str, m: Move, now: u64) -> Result<Output, String> {
        let seat = self.seat_of(uid).ok_or("you aren't sitting here")?;
        if m == Move::Double && !self.can_double() {
            return Err("the stake can't be doubled now (or not as far as everyone can cover)".into());
        }
        let Stage::Playing { hand, .. } = &mut self.stage else { return Err(domino::NOT_OPEN.into()) };
        hand.act(seat, m)?;
        if let Some(s) = self.seats[seat as usize].as_mut()
            && !s.leaving
        {
            // (Back: their own turns again.)
            s.away = false;
        }
        self.seen_ms = now;
        let mut out = Output::default();
        self.after_move(now, &mut out);
        if !matches!(self.stage, Stage::Settled { .. }) {
            self.changed(now, &mut out);
        }
        Ok(out)
    }

    /// After a move: whose turn, and its clock; or the hand's end.
    fn after_move(&mut self, now: u64, out: &mut Output) {
        let Stage::Playing { hand, deadline, plan, whim, cap, .. } = &mut self.stage else { return };
        if hand.over.is_some() {
            self.settle(now, out);
            return;
        }
        let Some(seat) = hand.to_act() else { return };
        *plan = None;
        // A ¡Doblo! against a side with someone in it (not away): they answer it, in their own
        // time, as 0.13.28's (its bots never answer for a person, and it's taken if they don't).
        if let Some(by) = hand.offer {
            let person = self.seats.iter().enumerate().any(|(i, s)| s.as_ref().is_some_and(|s| !s.bot && !s.away) && hand.place(i as u8).is_some_and(|p| hand.side(p) != by));
            if person {
                *deadline = now + TURN_MS;
                return;
            }
        }
        let s = self.seats[seat as usize].as_ref();
        if s.is_some_and(|s| !s.bot && !s.away) {
            *deadline = now + TURN_MS;
            return;
        }
        // A bot (or someone away) chooses as the turn comes, and takes its time over it.
        let m = bot_move(hand, seat, *cap, whim);
        let think = bot_think_ms(hand, m, whim);
        *plan = Some((seat, m));
        *deadline = now + if s.is_some_and(|s| !s.bot) { AWAY_MS } else { think };
    }

    fn deal(&mut self, now: u64, rng: &mut dyn Random, out: &mut Output) {
        // People not in: a hand idle (stood up after a few); in Pairs, up now (the house plays).
        let mut stand = Vec::new();
        for s in self.seats.iter_mut().flatten().filter(|s| !s.bot && !s.ready) {
            s.idle += 1;
            if s.idle >= IDLE_HANDS || self.kind == Kind::Pairs {
                stand.push(s.uid.clone());
            }
        }
        if !self.people().any(|s| s.ready) {
            self.stage = Stage::Waiting;
            self.changed(now, out);
            return;
        }
        for uid in stand {
            out.extend(self.leave(&uid, now));
        }
        // The house's players: every empty seat in Pairs; someone to play against in Solo.
        match self.kind {
            Kind::Pairs => {
                for seat in 0..SEATS {
                    if self.seats[seat].is_none() {
                        self.sit_bot(seat, rng);
                    }
                }
            }
            Kind::Solo => {
                if self.seats.iter().flatten().filter(|s| s.bot || s.ready).count() < 2
                    && let Some(seat) = self.seats.iter().position(|s| s.is_none())
                {
                    self.sit_bot(seat, rng);
                }
            }
        }
        let seats: Vec<u8> = (0..SEATS).filter(|i| self.seats[*i].as_ref().is_some_and(|s| s.bot || s.ready)).map(|i| i as u8).collect();
        let seeds: Vec<(u8, [u8; 32])> = seats.iter().filter_map(|i| self.seats[*i as usize].as_ref().filter(|s| !s.bot).and_then(|s| s.seed).map(|seed| (*i, seed))).collect();
        let uids: Vec<(u8, String, bool)> = seats.iter().filter_map(|i| self.seats[*i as usize].as_ref().map(|s| (*i, s.uid.clone(), s.bot))).collect();
        let cap = seats
            .iter()
            .filter_map(|i| self.seats[*i as usize].as_ref().filter(|s| !s.bot))
            .map(|s| (s.escrow / self.ante).min(domino::MAX_MULT as u64) as u32)
            .min()
            .unwrap_or(domino::MAX_MULT)
            .max(1);
        for s in self.seats.iter_mut().flatten() {
            s.away = false;
        }
        let hand = Hand::deal(self.kind, &seats, domino::round_seed(&self.house, &seeds, self.round_no));
        let whim = Stream::new(rng.seed());
        self.stage = Stage::Playing { hand: Box::new(hand), deadline: now, seeds, uids, plan: None, whim, cap };
        self.after_move(now, out);
        if matches!(self.stage, Stage::Playing { .. }) {
            self.changed(now, out);
        }
    }

    /// The hand's over: everyone in it paid (what they put up back, less their stake, and their
    /// share if they won), their records told, the house's seed shown, the next fingerprint.
    fn settle(&mut self, now: u64, out: &mut Output) {
        let Stage::Playing { hand, seeds, uids, .. } = std::mem::replace(&mut self.stage, Stage::Settled { until: now + SETTLE_MS }) else { return };
        let pay = domino::payout(&hand, self.ante);
        let over = hand.over.clone().unwrap_or(Over::Fold { winner: 0 });
        let how = how(&over).to_string();
        let winners = hand.winners();
        let stake = self.ante * hand.mult as u64;
        for (seat, uid, bot) in &uids {
            if *bot {
                continue;
            }
            let share = pay.paid.iter().find(|(s, _)| s == seat).map(|(_, c)| *c).unwrap_or(0);
            let escrow = self.seats[*seat as usize].as_ref().filter(|s| s.uid == *uid).map(|s| s.escrow).unwrap_or(0);
            let back = escrow.saturating_sub(stake) + share;
            if back > 0 {
                out.effects.push(Effect::Credit { uid: uid.clone(), chips: back });
            }
            let (capicua, chiva) = match &over {
                Over::Domino { capicua, chiva, .. } => (*capicua, *chiva),
                _ => (false, false),
            };
            out.effects.push(Effect::DomHand {
                uid: uid.clone(),
                outcome: domino::Outcome { kind: self.kind, ante: self.ante, mult: hand.mult, how: how.clone(), won: winners.contains(seat), wagered: stake, paid: share, capicua, chiva, bonus_pct: pay.bonus_pct },
            });
        }
        if pay.rake > 0 && uids.iter().any(|(_, _, bot)| !bot) {
            out.effects.push(Effect::Raked { chips: pay.rake });
        }
        self.result = Some(DomResult { how, winners, pot: pay.pot, rake: pay.rake, bonus_pct: pay.bonus_pct, mult: pay.mult, paid: pay.paid });
        self.last = Some(DomRecord {
            round: self.round_no,
            commit: self.commit,
            house: self.house,
            seeds: seeds.iter().map(|(s, b)| (*s, hex(b))).collect(),
            kind: hand.kind,
            seats: hand.seats.clone(),
            moves: hand.moves.clone(),
            over,
        });
        self.finished = Some(hand);
        for s in self.seats.iter_mut().flatten() {
            if !s.bot {
                s.ready = false;
                s.escrow = 0;
                s.seed = None;
            }
        }
        // (Whoever stood up in the hand goes now.)
        for seat in 0..SEATS {
            if self.seats[seat].as_ref().is_some_and(|s| s.leaving) {
                self.seats[seat] = None;
            }
        }
        self.changed(now, out);
    }

    fn next_round(&mut self, rng: &mut dyn Random) {
        self.round_no += 1;
        self.house = rng.seed();
        self.commit = fair::commit(&self.house);
    }

    /// Nobody left sitting: the table waits.
    fn after_seats(&mut self, _now: u64) {
        if self.people().next().is_none() && matches!(self.stage, Stage::Gathering { .. }) {
            self.stage = Stage::Waiting;
        }
    }

    pub fn tick(&mut self, now: u64, rng: &mut dyn Random) -> Output {
        let mut out = Output::default();
        match &mut self.stage {
            Stage::Waiting => {}
            Stage::Gathering { until } => {
                if now >= *until {
                    self.deal(now, rng, &mut out);
                }
            }
            Stage::Playing { hand, deadline, plan, whim, cap, .. } => {
                if now < *deadline {
                    return out;
                }
                let Some(seat) = hand.to_act() else {
                    self.after_move(now, &mut out);
                    return out;
                };
                // A bot's move (chosen as its turn came), or a person's time running out (played
                // for them, and quickly from now on this hand).
                let planned = plan.take().filter(|(s, _)| *s == seat).map(|(_, m)| m);
                let offer = hand.offer;
                let m = planned.unwrap_or_else(|| bot_move(hand, seat, *cap, whim));
                if hand.act(seat, m).is_err() {
                    let fallback = hand.allowed(seat).into_iter().find(|m| *m != Move::Double).unwrap_or(Move::Pass);
                    let _ = hand.act(seat, fallback);
                }
                // (Whoever's time ran out is away now: the one to play, or a ¡Doblo!'s answerers.)
                for (i, s) in self.seats.iter_mut().enumerate() {
                    let Some(s) = s.as_mut().filter(|s| !s.bot) else { continue };
                    let answering = offer.is_some_and(|by| hand.place(i as u8).is_some_and(|p| hand.side(p) != by));
                    if i == seat as usize || answering {
                        s.away = true;
                    }
                }
                self.after_move(now, &mut out);
                if !matches!(self.stage, Stage::Settled { .. }) {
                    self.changed(now, &mut out);
                }
            }
            Stage::Settled { until } => {
                if now >= *until {
                    self.next_round(rng);
                    let all = self.people().all(|s| s.ready);
                    self.stage = if self.people().next().is_none() { Stage::Waiting } else { Stage::Gathering { until: now + if all { AGAIN_MS } else { WAIT_MS } } };
                    self.changed(now, &mut out);
                }
            }
        }
        out
    }

    /// Everyone's chips on the table back (it's closing, or the server's stopping).
    pub fn close(&mut self) -> Output {
        let mut out = Output::default();
        for s in self.seats.iter_mut().flatten().filter(|s| !s.bot && s.escrow > 0) {
            out.effects.push(Effect::Credit { uid: s.uid.clone(), chips: s.escrow });
            s.escrow = 0;
        }
        out
    }
}

/// A bot's move (and an away person's): leads its heaviest double (or tile); plays to keep
/// what it holds most of open, heavy tiles first; draws or passes when it must. Doubles when
/// its side's well ahead and the stake can go higher; takes every double.
pub fn bot_move(h: &Hand, seat: u8, cap: u32, whim: &mut Stream) -> Move {
    let allowed = h.allowed(seat);
    let Some(place) = h.place(seat) else { return Move::Pass };
    let n = h.n();
    let me = h.side(place);
    let mine: Vec<usize> = (0..n).filter(|p| h.side(*p) == me).collect();
    let theirs: Vec<usize> = (0..n).filter(|p| h.side(*p) != me).collect();
    let least = |ps: &[usize]| ps.iter().map(|p| h.tiles(*p).len()).min().unwrap_or(0);
    // (0.13.28's bots take every ¡Doblo!: 22 of 22 in its hands.)
    if allowed.contains(&Move::Accept) {
        return Move::Accept;
    }
    if allowed.contains(&Move::Double) && h.mult * 2 <= cap {
        let ahead = least(&mine) + 3 <= least(&theirs) && least(&mine) <= 4;
        if ahead && whim.below(4) == 0 {
            return Move::Double;
        }
    }
    let plays: Vec<Move> = allowed.iter().copied().filter(|m| matches!(m, Move::Play { .. })).collect();
    if plays.is_empty() {
        return allowed.into_iter().find(|m| matches!(m, Move::Draw | Move::Pass)).unwrap_or(Move::Pass);
    }
    let held = h.tiles(place);
    let count = |n: u8| held.iter().filter(|t| domino::fits(**t, n)).count() as i64;
    let score = |m: &Move| -> i64 {
        let Move::Play { tile, end } = *m else { return 0 };
        let double = tile[0] == tile[1];
        // The number this play leaves open at that end.
        let open = match (h.ends(), end) {
            (None, _) => tile[1],
            (Some((l, _)), domino::End::Left) => if tile[0] == l { tile[1] } else { tile[0] },
            (Some((_, r)), domino::End::Right) => if tile[0] == r { tile[1] } else { tile[0] },
        };
        domino::pips(tile) as i64 * 2 + if double { 12 } else { 0 } + count(open) * 6
    };
    let best = plays.iter().map(score).max().unwrap_or(0);
    let top: Vec<Move> = plays.into_iter().filter(|m| score(m) >= best - 2).collect();
    top[whim.below(top.len() as u32) as usize]
}

/// How long a bot takes over a move: a look at the line, longer over a double; drawing quick.
fn bot_think_ms(h: &Hand, m: Move, whim: &mut Stream) -> u64 {
    let r = whim.below(1_000) as u64;
    match m {
        Move::Draw => 500 + r * 3 / 2,
        Move::Pass => 600 + r * 2,
        Move::Double | Move::Accept | Move::Fold => 1_500 + r * 2,
        Move::Play { .. } if h.chain.is_empty() => 1_500 + r * 2,
        Move::Play { .. } => 600 + r * 22 / 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fair::seed_from;

    struct Rng(Stream);
    impl Random for Rng {
        fn seed(&mut self) -> [u8; 32] {
            let mut s = [0u8; 32];
            for c in s.chunks_mut(4) {
                c.copy_from_slice(&self.0.next_u32().to_le_bytes());
            }
            s
        }
        fn below(&mut self, n: u32) -> u32 {
            self.0.below(n)
        }
    }

    fn view(r: &DomRoom, uid: &str, now: u64) -> DomView {
        r.view(Some(uid), now)
    }

    /// Plays the room's clock on till `until` (moves for `me` the first allowed, as a person
    /// would), returning the time it got to.
    fn run(r: &mut DomRoom, me: &str, mut now: u64, until: impl Fn(&DomRoom, u64) -> bool, rng: &mut Rng) -> u64 {
        for _ in 0..20_000 {
            if until(r, now) {
                return now;
            }
            let v = view(r, me, now);
            if let Some(h) = &v.hand
                && v.stage == DomStage::Playing
                && let Some(m) = h.allowed.iter().find(|m| **m != Move::Double)
            {
                let _ = r.act(me, *m, now);
            }
            now += 250;
            r.tick(now, rng);
        }
        panic!("never got there");
    }

    #[test]
    fn pairs_are_filled_by_the_house_and_a_hand_plays_out_and_checks() {
        let mut rng = Rng(Stream::new(seed_from(3)));
        let mut r = DomRoom::new(1, "Café Habana", Kind::Pairs, 100, 0, &mut rng);
        r.sit("olive", "Olive", Look::default(), 0, 0).unwrap();
        assert_eq!(view(&r, "olive", 0).stage, DomStage::Waiting);
        assert!(r.ready("olive", 50, [1; 32], 0).err().unwrap().contains("put up 100 to 800"));
        r.ready("olive", 200, [1; 32], 0).unwrap();
        assert_eq!(view(&r, "olive", 0).stage, DomStage::Gathering);
        let now = run(&mut r, "olive", 0, |r, _| r.playing(), &mut rng);
        let v = view(&r, "olive", now);
        let h = v.hand.as_ref().unwrap();
        assert_eq!(h.seats, vec![0, 1, 2, 3], "the house fills Pairs");
        assert_eq!(v.cap, 2, "Olive covered twice the ante");
        assert_eq!(h.counts.iter().map(|c| *c as usize).sum::<usize>() + h.chain.len(), 40);
        let now = run(&mut r, "olive", now, |r, now| view(r, "olive", now).stage == DomStage::Settled, &mut rng);
        let v = view(&r, "olive", now);
        let rec = v.last.as_ref().unwrap();
        let res = v.result.as_ref().unwrap();
        let pay = check(rec, 100).unwrap();
        assert_eq!(pay.paid, res.paid);
        assert_eq!(rec.seeds.len(), 1);
        assert!(v.hand.as_ref().unwrap().over.is_some());
    }

    #[test]
    fn a_double_goes_no_further_than_everyone_covered() {
        let mut rng = Rng(Stream::new(seed_from(8)));
        let mut r = DomRoom::new(2, "Mesa", Kind::Solo, 250, 0, &mut rng);
        r.sit("tess", "Tess", Look::default(), 2, 0).unwrap();
        r.ready("tess", 250, [2; 32], 0).unwrap();
        let mut now = run(&mut r, "tess", 0, |r, _| r.playing(), &mut rng);
        // (Alone in Solo: the house sits one down.)
        assert_eq!(view(&r, "tess", now).hand.unwrap().seats.len(), 2);
        // Wait for Tess's turn with a tile down.
        now = run(&mut r, "tess", now, |r, now| { let v = view(r, "tess", now); v.hand.is_some_and(|h| h.allowed.contains(&Move::Double)) }, &mut rng);
        assert!(r.act("tess", Move::Double, now).err().unwrap().contains("can't be doubled"));
    }
}
