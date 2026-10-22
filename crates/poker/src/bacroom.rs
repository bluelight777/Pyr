//! A baccarat table in Pyr Casino: five seats for people and bots around one shoe. Everyone bets
//! on the same coup: Player, Banker, Tie or either pair (the bets and what they pay are
//! `casino::baccarat`'s, as at the single-player table).
//!
//! It's fair the way the other tables are: before the bets go down the house shows its seed's
//! fingerprint; each person's Pyr adds a seed of its own with their bets; the coup's cards come
//! from all of them together (`coup_seed`), out of a fresh eight-deck shoe. Once it's dealt the
//! house shows its seed, and everyone's Pyr deals it again to be sure (`check`). Bots bet with the
//! house's chips: they're company, and nobody's chips go to or from them.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::bots::persona::{self, BjBets, BjPlay, Persona};
use crate::casino::baccarat::{self, Bets, Side};
use crate::fair::{self, Stream};
use crate::net::{BacMark, BacReveal, BacSeatView, BacStage, BacSummary, BacView, ToApp};
use crate::room::{Effect, Look, Output, Random};

pub const SEATS: usize = 5;
/// How long bets stay open before each coup, and how soon the cards come once every person at
/// the table has said they're ready.
pub const BET_MS: u64 = 15_000;
pub const QUICK_MS: u64 = 1_500;
/// The coup dealt and turned over card by card, then its result on the table.
pub const SHOW_MS: u64 = 9_000;
/// Coups a person can sit through without a bet before they're stood up.
pub const IDLE_COUPS: u32 = 12;
/// The most on Tie or a pair (as at the single-player table).
pub const SIDE_MAX: u64 = 5_000;
/// Coups kept for the scoreboard.
pub const HISTORY: usize = 72;

const BOT_NAMES: [&str; 14] = ["Ace", "Blaze", "Cinder", "Dusk", "Ember", "Flint", "Gale", "Ivy", "Lux", "Moss", "Nova", "Sage", "Tarn", "Wren"];

struct Seat {
    uid: String,
    name: String,
    bot: bool,
    look: Look,
    /// Their bets for the coming coup (a person's chips taken from them already), whether they're
    /// ready for the cards, and their Pyr's seed for it.
    bets: Bets,
    ready: bool,
    seed: Option<[u8; 32]>,
    /// Coups in a row without a bet.
    idle: u32,
    /// A bot: when its bet goes down this coup; who it is (how it bets, its pace); its units
    /// (pressing after a win).
    bot_bet_at: u64,
    who: &'static Persona,
    units: u64,
    /// The last coup: what they had on it and what came back.
    last: Option<(Bets, u64)>,
}

enum Stage {
    /// Nobody (no person) sitting.
    Waiting,
    /// Bets go down (and come off) till `until`, when the cards come.
    Betting { until: u64 },
    /// The coup's been dealt and everyone paid: it's turned over and its result stays till
    /// `until` (bets for the next can go down meanwhile).
    Shown { until: u64 },
}

pub struct BacRoom {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    seats: Vec<Option<Seat>>,
    watchers: BTreeSet<String>,
    stage: Stage,
    /// The coming coup's number and the house's seed for it (its fingerprint shown).
    coup_no: u64,
    house: [u8; 32],
    commit: [u8; 32],
    last: Option<BacReveal>,
    /// The coups so far (for the scoreboard), oldest first.
    history: Vec<BacMark>,
    /// When a person was last here.
    pub seen_ms: u64,
    whim: Stream,
}

/// A coup's seed: the house's, every person's seat seed, and the coup's number.
pub fn coup_seed(house: &[u8; 32], seeds: &[(u8, [u8; 32])], coup: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr baccarat table v1");
    h.update(house);
    let mut seeds = seeds.to_vec();
    seeds.sort_by_key(|s| s.0);
    for (seat, s) in &seeds {
        h.update([*seat]);
        h.update(s);
    }
    h.update(coup.to_le_bytes());
    h.finalize().into()
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A seed as a coup's reveal shows it.
pub fn seed_hex(b: &[u8; 32]) -> String {
    hex(b)
}

/// Checks a coup: the house's seed matches the fingerprint it showed, and the coup dealt again
/// from everyone's seeds comes out card for card as it did at the table.
pub fn check(r: &BacReveal) -> Result<(), String> {
    if fair::commit(&r.house) != r.commit {
        return Err(format!("coup {}: the house's seed doesn't match the fingerprint it showed first", r.coup));
    }
    let seeds: Vec<(u8, [u8; 32])> = r.seeds.iter().filter_map(|(s, h)| crate::net::hex32::parse(h).map(|b| (*s, b))).collect();
    if seeds.len() != r.seeds.len() {
        return Err(format!("coup {}: a player's seed isn't a seed", r.coup));
    }
    let (player, banker) = baccarat::deal(&mut Stream::new(coup_seed(&r.house, &seeds, r.coup)));
    if player != r.player || banker != r.banker {
        return Err(format!("coup {}: dealt again, the cards came out differently", r.coup));
    }
    Ok(())
}

/// Each bet on its own: Player and Banker the table's least to its most, Tie and the pairs the
/// least to 5,000 (or the table's most, if that's less).
fn within(b: &Bets, lo: u64, hi: u64) -> Result<(), String> {
    for (name, x, top) in [("Player", b.player, hi), ("Banker", b.banker, hi), ("Tie", b.tie, hi.min(SIDE_MAX)), ("Player pair", b.player_pair, hi.min(SIDE_MAX)), ("Banker pair", b.banker_pair, hi.min(SIDE_MAX))] {
        if x > 0 && (x < lo || x > top) {
            return Err(format!("{name} here is {lo} to {top}"));
        }
    }
    Ok(())
}

/// Brings each bet into what the table takes (a bet under its least goes up to it, one over its
/// most comes down to it), so what's sent always passes [`within`]. Says whether anything moved.
pub fn fit(b: &mut Bets, lo: u64, hi: u64) -> bool {
    let side = hi.min(SIDE_MAX).max(lo);
    let before = b.clone();
    for (x, top) in [(&mut b.player, hi), (&mut b.banker, hi), (&mut b.tie, side), (&mut b.player_pair, side), (&mut b.banker_pair, side)] {
        if *x > 0 {
            *x = (*x).clamp(lo, top.max(lo));
        }
    }
    *b != before
}

/// How long a bot takes to get its chips down, the way a person would: at its own pace, never
/// the same twice, with the odd long pause.
fn bot_ms(base: f64, who: &Persona, rng: &mut Stream) -> u64 {
    let roll = |rng: &mut Stream| (rng.below(1_000_000) as f64 + 0.5) / 1_000_000.0;
    let (u1, u2) = (roll(rng), roll(rng));
    let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
    let mut ms = base * who.pace * (0.35 * z).exp();
    if roll(rng) > 1.0 - who.tanks * 0.5 {
        ms += 1_200.0 + roll(rng) * 2_200.0;
    }
    (ms as u64).clamp(500, 9_000)
}

impl BacRoom {
    pub fn new(id: u64, name: &str, min_bet: u64, max_bet: u64, now: u64, rng: &mut dyn Random) -> BacRoom {
        let house = rng.seed();
        BacRoom {
            id,
            name: name.chars().take(40).collect(),
            min_bet: min_bet.max(1),
            max_bet: max_bet.max(min_bet.max(1)),
            seats: (0..SEATS).map(|_| None).collect(),
            watchers: BTreeSet::new(),
            stage: Stage::Waiting,
            coup_no: 1,
            commit: fair::commit(&house),
            house,
            last: None,
            history: Vec::new(),
            seen_ms: now,
            whim: Stream::new(rng.seed()),
        }
    }

    fn people(&self) -> impl Iterator<Item = &Seat> {
        self.seats.iter().flatten().filter(|s| !s.bot)
    }

    /// The coming coup's number.
    pub fn coup_no(&self) -> u64 {
        self.coup_no
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

    pub fn summary(&self) -> BacSummary {
        BacSummary {
            id: self.id,
            name: self.name.clone(),
            min_bet: self.min_bet,
            max_bet: self.max_bet,
            people: self.people().count(),
            bots: self.seats.iter().flatten().filter(|s| s.bot).count(),
        }
    }

    /// Chips a person has bet on the coming coup: what the server counts as theirs while
    /// they're here.
    pub fn at_stake(&self, uid: &str) -> u64 {
        self.seat_of(uid).and_then(|s| self.seats[s as usize].as_ref()).map(|s| s.bets.total()).unwrap_or(0)
    }

    pub fn in_use(&self) -> bool {
        !self.watchers.is_empty() || self.people().next().is_some()
    }

    pub fn knows(&self, uid: &str) -> bool {
        self.watchers.contains(uid) || self.seat_of(uid).is_some()
    }

    pub fn view(&self, uid: Option<&str>, now: u64) -> BacView {
        let (stage, ms_left) = match &self.stage {
            Stage::Waiting => (BacStage::Waiting, 0),
            Stage::Betting { until } => (BacStage::Betting, until.saturating_sub(now)),
            Stage::Shown { until } => (BacStage::Shown, until.saturating_sub(now)),
        };
        BacView {
            id: self.id,
            name: self.name.clone(),
            min_bet: self.min_bet,
            max_bet: self.max_bet,
            seats: self
                .seats
                .iter()
                .map(|s| {
                    s.as_ref().map(|s| BacSeatView {
                        player: s.uid.clone(),
                        name: s.name.clone(),
                        bot: s.bot,
                        level: s.look.level,
                        frame: s.look.frame.clone(),
                        bets: s.bets.clone(),
                        ready: s.ready,
                        last_bets: s.last.as_ref().map(|l| l.0.clone()).unwrap_or_default(),
                        paid: s.last.as_ref().map(|l| l.1).unwrap_or(0),
                        style: if s.bot { s.who.bac_style().to_string() } else { String::new() },
                    })
                })
                .collect(),
            stage,
            ms_left,
            coup_no: self.coup_no,
            commit: self.commit,
            last: self.last.clone(),
            history: self.history.clone(),
            my_seat: uid.and_then(|u| self.seat_of(u)),
        }
    }

    fn changed(&self, now: u64, out: &mut Output) {
        for uid in self.audience() {
            let view = self.view(Some(&uid), now);
            out.to(&uid, ToApp::Bac { view });
        }
    }

    pub fn watch(&mut self, uid: &str, now: u64) -> ToApp {
        self.watchers.insert(uid.to_string());
        self.seen_ms = now;
        ToApp::Bac { view: self.view(Some(uid), now) }
    }

    pub fn unwatch(&mut self, uid: &str) {
        self.watchers.remove(uid);
    }

    pub fn sit(&mut self, uid: &str, name: &str, look: Look, seat: u8, now: u64) -> Result<Output, String> {
        if self.seat_of(uid).is_some() {
            return Err("you're sitting here already".into());
        }
        let s = self.seats.get_mut(seat as usize).ok_or("there's no such seat")?;
        if s.is_some() {
            return Err("that seat's taken".into());
        }
        *s = Some(Seat { uid: uid.into(), name: name.into(), bot: false, look, bets: Bets::default(), ready: false, seed: None, idle: 0, bot_bet_at: 0, who: &persona::PLAIN, units: 1, last: None });
        self.watchers.insert(uid.into());
        self.seen_ms = now;
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// Stands up: their bets for the coming coup come back.
    pub fn leave(&mut self, uid: &str, now: u64) -> Output {
        let mut out = Output::default();
        let Some(seat) = self.seat_of(uid) else { return out };
        let s = self.seats[seat as usize].take().unwrap();
        if s.bets.total() > 0 {
            out.effects.push(Effect::Credit { uid: uid.into(), chips: s.bets.total() });
        }
        if self.people().next().is_none() {
            self.everyone_gone();
        }
        self.changed(now, &mut out);
        out
    }

    /// No people left: the bots' bets come down (they don't play alone).
    fn everyone_gone(&mut self) {
        for s in self.seats.iter_mut().flatten() {
            s.bets = Bets::default();
            s.ready = false;
        }
        self.stage = Stage::Waiting;
    }

    pub fn add_bot(&mut self, seat: u8, now: u64, rng: &mut dyn Random) -> Result<Output, String> {
        let taken: Vec<String> = self.seats.iter().flatten().map(|s| s.name.clone()).collect();
        let s = self.seats.get_mut(seat as usize).ok_or("there's no such seat")?;
        if s.is_some() {
            return Err("that seat's taken".into());
        }
        let free: Vec<&&str> = BOT_NAMES.iter().filter(|n| !taken.iter().any(|t| t == **n)).collect();
        let name = free.get(rng.below(free.len().max(1) as u32) as usize).map(|n| n.to_string()).unwrap_or_else(|| "Bot".into());
        let who = persona::of(&name);
        *s = Some(Seat {
            uid: format!("bot:bac{}:{seat}", self.id),
            name,
            bot: true,
            look: Look { level: 12, ..Look::default() },
            bets: Bets::default(),
            ready: false,
            seed: None,
            idle: 0,
            bot_bet_at: now + bot_ms(2_400.0, who, &mut self.whim),
            who,
            units: 1,
            last: None,
        });
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    pub fn remove_bot(&mut self, seat: u8, now: u64) -> Result<Output, String> {
        let s = self.seats.get_mut(seat as usize).ok_or("there's no such seat")?;
        if !s.as_ref().is_some_and(|s| s.bot) {
            return Err("there's no bot in that seat".into());
        }
        *s = None;
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// What going from my bets to `new` puts down (positive) or takes off (negative), if it's
    /// allowed now (before the cards come, each bet within the table's limits).
    pub fn cost(&self, uid: &str, new: &Bets) -> Result<i64, String> {
        let seat = self.seat_of(uid).ok_or("sit down first")?;
        let s = self.seats[seat as usize].as_ref().unwrap();
        within(new, self.min_bet, self.max_bet)?;
        Ok(new.total() as i64 - s.bets.total() as i64)
    }

    /// A person's bets for the coming coup (more chips taken by the server already; chips coming
    /// off go back as a credit), their Pyr's seed for it, and whether they're ready for the cards.
    pub fn place(&mut self, uid: &str, new: Bets, seed: [u8; 32], ready: bool, now: u64) -> Result<Output, String> {
        let cost = self.cost(uid, &new)?;
        let seat = self.seat_of(uid).unwrap();
        let mut out = Output::default();
        if cost < 0 {
            out.effects.push(Effect::Credit { uid: uid.into(), chips: cost.unsigned_abs() });
        }
        if matches!(self.stage, Stage::Waiting) {
            self.open_betting(now);
        }
        let s = self.seats[seat as usize].as_mut().unwrap();
        s.bets = new;
        s.seed = Some(seed);
        s.ready = ready;
        if s.bets.total() > 0 {
            s.idle = 0;
        }
        self.seen_ms = now;
        self.everyone_ready(now);
        self.changed(now, &mut out);
        Ok(out)
    }

    fn everyone_ready(&mut self, now: u64) {
        let all = self.people().next().is_some() && self.people().all(|s| s.ready);
        if let Stage::Betting { until } = &mut self.stage
            && all
        {
            *until = (*until).min(now + QUICK_MS);
            let mut at = now + 200;
            for s in self.seats.iter_mut().flatten().filter(|s| s.bot && !s.ready) {
                s.bot_bet_at = s.bot_bet_at.min(at);
                at += 200;
            }
        }
    }

    fn open_betting(&mut self, now: u64) {
        self.stage = Stage::Betting { until: now + BET_MS };
        let mut whim = std::mem::replace(&mut self.whim, Stream::new([0; 32]));
        for s in self.seats.iter_mut().flatten() {
            s.ready = false;
            if s.bot {
                s.bot_bet_at = now + bot_ms(2_200.0, s.who, &mut whim);
            }
        }
        self.whim = whim;
    }

    /// A bot's bet on the coming coup, its own way: Banker every time (the best odds there are),
    /// Banker for the least, Player flat, following the shoe (with a tie or a pair on a whim), or
    /// against it (and the tie when it's due, it says).
    fn bot_bets(&mut self, seat: usize) -> Bets {
        let (lo, hi) = (self.min_bet, self.max_bet);
        let s = self.seats[seat].as_ref().unwrap();
        let (who, units) = (s.who, s.units);
        let last = self.history.last().map(|m| m.winner);
        let r = &mut self.whim;
        let unit = match who.bj_bets {
            BjBets::Minimum => 1,
            BjBets::Steady => [1u64, 1, 2, 2, 3][r.below(5) as usize],
            BjBets::Press => units,
            BjBets::Big => [5u64, 10, 10, 20, 25][r.below(5) as usize],
        };
        let chips = |n: u64| (lo * n).clamp(lo, hi);
        let side = |n: u64| (lo * n).clamp(lo, hi.min(SIDE_MAX));
        let mut b = Bets::default();
        match who.bj {
            BjPlay::Book => b.banker = chips(unit),
            BjPlay::Careful => b.banker = chips(1),
            BjPlay::Dealer => b.player = chips(unit),
            BjPlay::Daredevil => {
                match last {
                    Some(Side::Player) => b.player = chips(unit),
                    _ => b.banker = chips(unit),
                }
                if r.below(5) == 0 {
                    b.tie = side(1);
                }
                if r.below(6) == 0 {
                    b.player_pair = side(1);
                }
            }
            BjPlay::Superstitious => {
                match last {
                    Some(Side::Banker) => b.player = chips(unit),
                    Some(Side::Player) => b.banker = chips(unit),
                    _ => b.tie = side(unit),
                }
                if r.below(4) == 0 {
                    b.banker_pair = side(1);
                }
            }
        }
        if within(&b, lo, hi).is_ok() { b } else { Bets { banker: lo, ..Bets::default() } }
    }

    fn bots_bet(&mut self, now: u64, all: bool) -> bool {
        let mut any = false;
        for i in 0..SEATS {
            let due = self.seats[i].as_ref().is_some_and(|s| s.bot && !s.ready && (all || now >= s.bot_bet_at));
            if due {
                let b = self.bot_bets(i);
                let s = self.seats[i].as_mut().unwrap();
                s.bets = b;
                s.ready = true;
                any = true;
            }
        }
        any
    }

    /// The coup: dealt from everyone's seeds, everyone paid (and their records told), the seeds
    /// shown, and the next coup's fingerprint.
    fn deal(&mut self, now: u64, rng: &mut dyn Random, out: &mut Output) {
        self.bots_bet(now, true);
        for s in self.seats.iter_mut().flatten().filter(|s| !s.bot) {
            if s.bets.total() == 0 {
                s.idle += 1;
            }
        }
        let idle: Vec<String> = self.seats.iter().flatten().filter(|s| !s.bot && s.idle >= IDLE_COUPS).map(|s| s.uid.clone()).collect();
        for uid in idle {
            out.extend(self.leave(&uid, now));
        }
        if self.people().next().is_none() {
            self.everyone_gone();
            self.changed(now, out);
            return;
        }
        let seeds: Vec<(u8, [u8; 32])> = self.seats.iter().enumerate().filter_map(|(i, s)| s.as_ref().filter(|s| !s.bot).and_then(|s| s.seed).map(|seed| (i as u8, seed))).collect();
        let (player, banker) = baccarat::deal(&mut Stream::new(coup_seed(&self.house, &seeds, self.coup_no)));
        let (winner, player_pair, banker_pair, _) = baccarat::settle(&Bets::default(), &player, &banker);
        for s in self.seats.iter_mut().flatten() {
            if s.bets.total() == 0 {
                s.last = None;
                continue;
            }
            let bets = std::mem::take(&mut s.bets);
            let (_, _, _, paid) = baccarat::settle(&bets, &player, &banker);
            if s.bot {
                s.units = if paid > bets.total() { (s.units * 2).min(8) } else { 1 };
            } else {
                if paid > 0 {
                    out.effects.push(Effect::Credit { uid: s.uid.clone(), chips: paid });
                }
                let outcome = baccarat::Outcome {
                    bets: bets.clone(),
                    player: player.clone(),
                    banker: banker.clone(),
                    player_total: baccarat::total(&player),
                    banker_total: baccarat::total(&banker),
                    winner,
                    player_pair,
                    banker_pair,
                    paid,
                };
                out.effects.push(Effect::BacCoup { uid: s.uid.clone(), outcome });
            }
            s.last = Some((bets, paid));
        }
        let (pt, bt) = (baccarat::total(&player), baccarat::total(&banker));
        self.history.push(BacMark { winner, player_pair, banker_pair, natural: player.len() == 2 && banker.len() == 2 && (pt >= 8 || bt >= 8), total: pt.max(bt) });
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        self.last = Some(BacReveal { coup: self.coup_no, commit: self.commit, house: self.house, seeds: seeds.iter().map(|(s, b)| (*s, hex(b))).collect(), player, banker, winner, player_pair, banker_pair });
        // The next coup's seed, its fingerprint shown before anyone bets on it.
        self.coup_no += 1;
        self.house = rng.seed();
        self.commit = fair::commit(&self.house);
        for s in self.seats.iter_mut().flatten() {
            s.seed = None;
            s.ready = false;
        }
        self.stage = Stage::Shown { until: now + SHOW_MS };
        self.changed(now, out);
    }

    pub fn tick(&mut self, now: u64, rng: &mut dyn Random) -> Output {
        let mut out = Output::default();
        let (stage, until) = match &self.stage {
            Stage::Waiting => (BacStage::Waiting, 0),
            Stage::Betting { until } => (BacStage::Betting, *until),
            Stage::Shown { until } => (BacStage::Shown, *until),
        };
        match stage {
            BacStage::Waiting => {
                if self.people().next().is_some() {
                    self.open_betting(now);
                    self.changed(now, &mut out);
                }
            }
            BacStage::Betting => {
                if self.people().next().is_none() {
                    self.everyone_gone();
                    self.changed(now, &mut out);
                    return out;
                }
                if self.bots_bet(now, false) {
                    self.changed(now, &mut out);
                }
                if now >= until {
                    self.deal(now, rng, &mut out);
                }
            }
            BacStage::Shown => {
                if now >= until {
                    self.whim = Stream::new(rng.seed());
                    if self.people().next().is_some() {
                        self.open_betting(now);
                    } else {
                        self.everyone_gone();
                    }
                    self.changed(now, &mut out);
                }
            }
        }
        out
    }

    /// Everyone's bets back (it's closing, or the server's stopping).
    pub fn close(&mut self) -> Output {
        let mut out = Output::default();
        for s in self.seats.iter_mut().flatten().filter(|s| !s.bot) {
            if s.bets.total() > 0 {
                out.effects.push(Effect::Credit { uid: s.uid.clone(), chips: s.bets.total() });
            }
            s.bets = Bets::default();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bets_are_fitted_to_the_table_before_they_go() {
        let mut b = Bets { player: 5, banker: 40_000, tie: 9_000, ..Default::default() };
        assert!(fit(&mut b, 10, 1_000));
        assert_eq!((b.player, b.banker, b.tie, b.player_pair), (10, 1_000, 1_000, 0));
        assert!(within(&b, 10, 1_000).is_ok());
        let mut b = Bets { player: 25, tie: 6_000, ..Default::default() };
        assert!(fit(&mut b, 500, 25_000));
        assert_eq!((b.player, b.tie), (500, SIDE_MAX));
        assert!(within(&b, 500, 25_000).is_ok());
        assert!(!fit(&mut b, 500, 25_000));
    }
    use crate::fair::seed_from;

    struct Rng(Stream);
    impl Random for Rng {
        fn seed(&mut self) -> [u8; 32] {
            let mut s = [0u8; 32];
            for b in &mut s {
                *b = self.0.below(256) as u8;
            }
            s
        }
        fn below(&mut self, n: u32) -> u32 {
            self.0.below(n)
        }
    }

    fn credits(out: &Output) -> u64 {
        out.effects.iter().map(|e| if let Effect::Credit { chips, .. } = e { *chips } else { 0 }).sum()
    }

    /// People and bots play coups; every coup checks out from its seeds; what people bet comes
    /// back as the game pays it (Banker about 98.9%, Player about 98.8%), nothing made or lost
    /// on the way; the bots bet every coup, each its own way.
    #[test]
    fn a_table_plays_coups_that_check_out() {
        let mut rng = Rng(Stream::new(seed_from(5)));
        let mut t = BacRoom::new(1, "Test", 10, 1_000, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 1, 0).unwrap();
        t.sit("bob", "Bob", Look::default(), 3, 0).unwrap();
        for seat in [0, 2, 4] {
            t.add_bot(seat, 0, &mut rng).unwrap();
        }
        let (mut taken, mut back) = (0u64, 0u64);
        let (mut coups, mut seen) = (0, 0);
        let mut bots_bet = 0;
        let mut now = 0u64;
        while now < 4_000_000 {
            let out = t.tick(now, &mut rng);
            back += credits(&out);
            if let Some(r) = &t.last
                && r.coup != seen
            {
                seen = r.coup;
                check(r).unwrap();
                coups += 1;
                bots_bet += t.seats.iter().flatten().filter(|s| s.bot && s.last.is_some()).count();
            }
            if matches!(t.stage, Stage::Betting { .. }) {
                for (uid, bets) in [("alice", Bets { banker: 100, ..Bets::default() }), ("bob", Bets { player: 50, tie: 10, ..Bets::default() })] {
                    let seat = t.seat_of(uid).unwrap() as usize;
                    if t.seats[seat].as_ref().unwrap().ready {
                        continue;
                    }
                    let cost = t.cost(uid, &bets).unwrap();
                    taken += cost.max(0) as u64;
                    back += credits(&t.place(uid, bets, rng.seed(), true, now).unwrap());
                }
            }
            now += 250;
        }
        assert!(coups > 300, "{coups} coups");
        assert_eq!(bots_bet, coups * 3, "the bots bet every coup");
        let rtp = back as f64 / taken as f64;
        assert!((0.85..1.15).contains(&rtp), "{rtp} over {coups} coups");
        assert_eq!(t.history.len(), HISTORY.min(coups));
    }

    /// Bets come off before the cards come (the chips back); standing up gives back what's bet;
    /// a coup that's been tampered with doesn't check out; bets outside the limits are refused.
    #[test]
    fn bets_come_off_and_a_wrong_card_is_caught() {
        let mut rng = Rng(Stream::new(seed_from(9)));
        let mut t = BacRoom::new(1, "Test", 10, 500, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 2, 0).unwrap();
        t.place("alice", Bets { player: 100, tie: 20, ..Bets::default() }, rng.seed(), false, 0).unwrap();
        assert_eq!(t.at_stake("alice"), 120);
        let out = t.place("alice", Bets { player: 100, ..Bets::default() }, rng.seed(), true, 0).unwrap();
        assert_eq!(credits(&out), 20);
        assert!(t.cost("alice", &Bets { banker: 5, ..Bets::default() }).is_err());
        assert!(t.cost("alice", &Bets { player: 501, ..Bets::default() }).is_err());
        assert!(t.cost("nobody", &Bets::default()).is_err());
        let mut now = 0;
        while t.last.is_none() && now < 60_000 {
            t.tick(now, &mut rng);
            now += 250;
        }
        let r = t.last.clone().unwrap();
        check(&r).unwrap();
        assert_eq!(r.seeds.len(), 1, "her seed went into it");
        let mut bad = r.clone();
        bad.player[0] = if bad.player[0].index() == 0 { crate::cards::Card::from_index(1) } else { crate::cards::Card::from_index(0) };
        assert!(check(&bad).is_err());
        let mut bad = r.clone();
        bad.house[0] ^= 1;
        assert!(check(&bad).is_err());
        // Standing up with a bet down gives it back.
        while !matches!(t.stage, Stage::Betting { .. }) {
            t.tick(now, &mut rng);
            now += 250;
        }
        t.place("alice", Bets { banker: 200, ..Bets::default() }, rng.seed(), false, now).unwrap();
        assert_eq!(credits(&t.leave("alice", now)), 200);
        assert_eq!(t.at_stake("alice"), 0);
    }
}
