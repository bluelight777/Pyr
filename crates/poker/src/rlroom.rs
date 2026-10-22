//! A roulette table in Pyr Casino: five seats for people and bots round one wheel. Everyone bets
//! on the same spin, anywhere on the layout (the bets and what they pay are
//! `casino::roulette`'s, as at the single-player wheel), each seat's chips in its own colour.
//!
//! It's fair the way the other tables are: before the bets go down the house shows its seed's
//! fingerprint; each person's Pyr adds a seed of its own with their bets; the spin comes from all
//! of them together (`spin_seed`). Once the ball's in its pocket the house shows its seed, and
//! everyone's Pyr spins it again to be sure (`check`). Bots bet with the house's chips: they're
//! company, and nobody's chips go to or from them.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::bots::persona::{self, BjBets, BjPlay, Persona};
use crate::casino::roulette::{self, Chip, Spot};
use crate::fair::{self, Stream};
use crate::net::{RlReveal, RlSeatView, RlStage, RlSummary, RlView, ToApp};
use crate::room::{Effect, Look, Output, Random};

pub const SEATS: usize = 5;
/// How long bets stay open before each spin, and how soon the ball goes once every person at the
/// table has said they're ready.
pub const BET_MS: u64 = 18_000;
pub const QUICK_MS: u64 = 1_500;
/// The ball round the wheel and into its pocket (the app's spin takes about 8 seconds), then the
/// number on the table for a moment.
pub const SHOW_MS: u64 = 12_500;
/// Spins a person can sit through without a bet before they're stood up.
pub const IDLE_SPINS: u32 = 12;
/// The most places one seat can have chips on at once.
pub const MAX_SPOTS: usize = 60;
/// Numbers kept for the board.
pub const HISTORY: usize = 60;

const BOT_NAMES: [&str; 14] = ["Ace", "Blaze", "Cinder", "Dusk", "Ember", "Flint", "Gale", "Ivy", "Lux", "Moss", "Nova", "Sage", "Tarn", "Wren"];

/// The chips on all of a seat's spots.
pub fn total(chips: &[Chip]) -> u64 {
    chips.iter().map(|c| c.chips).sum()
}

/// Chips merged by spot (in the order first placed), empty spots left out.
pub fn merged(chips: &[Chip]) -> Vec<Chip> {
    let mut out: Vec<Chip> = Vec::new();
    for c in chips.iter().filter(|c| c.chips > 0) {
        match out.iter_mut().find(|o| o.spot == c.spot) {
            Some(o) => o.chips += c.chips,
            None => out.push(c.clone()),
        }
    }
    out
}

struct Seat {
    uid: String,
    name: String,
    bot: bool,
    look: Look,
    /// Their chips for the coming spin (a person's taken from them already), whether they're
    /// ready for the ball, and their Pyr's seed for it.
    chips: Vec<Chip>,
    ready: bool,
    seed: Option<[u8; 32]>,
    /// Spins in a row without a bet.
    idle: u32,
    /// A bot: when its chips go down this spin; who it is (how it bets, its pace); its units
    /// (pressing after a win); its lucky number.
    bot_bet_at: u64,
    who: &'static Persona,
    units: u64,
    lucky: u8,
    /// The last spin: what they had on it and what came back.
    last: Option<(Vec<Chip>, u64)>,
}

enum Stage {
    /// Nobody (no person) sitting.
    Waiting,
    /// Bets go down (and come off) till `until`, when the ball goes.
    Betting { until: u64 },
    /// The spin's been decided and everyone paid: the ball's going round, and the number stays
    /// till `until` (bets for the next can go down once it's called).
    Shown { until: u64 },
}

pub struct RlRoom {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    seats: Vec<Option<Seat>>,
    watchers: BTreeSet<String>,
    stage: Stage,
    /// The coming spin's number and the house's seed for it (its fingerprint shown).
    spin_no: u64,
    house: [u8; 32],
    commit: [u8; 32],
    last: Option<RlReveal>,
    /// The numbers so far, oldest first.
    history: Vec<u8>,
    /// When a person was last here.
    pub seen_ms: u64,
    whim: Stream,
}

/// A spin's seed: the house's, every person's seat seed, and the spin's number.
pub fn spin_seed(house: &[u8; 32], seeds: &[(u8, [u8; 32])], spin: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr roulette table v1");
    h.update(house);
    let mut seeds = seeds.to_vec();
    seeds.sort_by_key(|s| s.0);
    for (seat, s) in &seeds {
        h.update([*seat]);
        h.update(s);
    }
    h.update(spin.to_le_bytes());
    h.finalize().into()
}

/// A seed as a spin's reveal shows it.
pub fn seed_hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Checks a spin: the house's seed matches the fingerprint it showed, and the spin made again
/// from everyone's seeds lands in the same pocket.
pub fn check(r: &RlReveal) -> Result<(), String> {
    if fair::commit(&r.house) != r.commit {
        return Err(format!("spin {}: the house's seed doesn't match the fingerprint it showed first", r.spin));
    }
    let seeds: Vec<(u8, [u8; 32])> = r.seeds.iter().filter_map(|(s, h)| crate::net::hex32::parse(h).map(|b| (*s, b))).collect();
    if seeds.len() != r.seeds.len() {
        return Err(format!("spin {}: a player's seed isn't a seed", r.spin));
    }
    let (number, pocket) = roulette::spin(&mut Stream::new(spin_seed(&r.house, &seeds, r.spin)));
    if number != r.number || pocket != r.pocket {
        return Err(format!("spin {}: spun again, the ball landed somewhere else", r.spin));
    }
    Ok(())
}

/// A seat's chips: places on the layout that are there, each the table's least to its most (or
/// the spot's own most, if that's less), and not too many of them.
pub fn within(chips: &[Chip], lo: u64, hi: u64) -> Result<(), String> {
    if chips.len() > MAX_SPOTS {
        return Err(format!("that's chips on {} places: {MAX_SPOTS} at most", chips.len()));
    }
    for (i, c) in chips.iter().enumerate() {
        c.spot.check()?;
        if chips[..i].iter().any(|o| o.spot == c.spot) {
            return Err("the same place twice".into());
        }
        let top = hi.min(roulette::max_on(&c.spot));
        if c.chips < lo || c.chips > top {
            return Err(format!("{} here is {lo} to {top}", c.spot.name()));
        }
    }
    Ok(())
}

/// Brings each chip into the table's least to its most for its place (under goes up, over comes
/// down), merging places twice over. Says whether anything moved.
pub fn fit(chips: &mut Vec<Chip>, lo: u64, hi: u64) -> bool {
    let before = chips.clone();
    let mut out: Vec<Chip> = Vec::with_capacity(chips.len());
    for c in chips.drain(..) {
        match out.iter_mut().find(|o| o.spot == c.spot) {
            Some(o) => o.chips = o.chips.saturating_add(c.chips),
            None => out.push(c),
        }
    }
    out.retain(|c| c.chips > 0);
    for c in out.iter_mut() {
        let top = hi.min(roulette::max_on(&c.spot)).max(lo);
        c.chips = c.chips.clamp(lo, top);
    }
    *chips = out;
    *chips != before
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
    (ms as u64).clamp(600, 11_000)
}

impl RlRoom {
    pub fn new(id: u64, name: &str, min_bet: u64, max_bet: u64, now: u64, rng: &mut dyn Random) -> RlRoom {
        let house = rng.seed();
        RlRoom {
            id,
            name: name.chars().take(40).collect(),
            min_bet: min_bet.max(1),
            max_bet: max_bet.max(min_bet.max(1)),
            seats: (0..SEATS).map(|_| None).collect(),
            watchers: BTreeSet::new(),
            stage: Stage::Waiting,
            spin_no: 1,
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

    /// The coming spin's number.
    pub fn spin_no(&self) -> u64 {
        self.spin_no
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

    pub fn summary(&self) -> RlSummary {
        RlSummary {
            id: self.id,
            name: self.name.clone(),
            min_bet: self.min_bet,
            max_bet: self.max_bet,
            people: self.people().count(),
            bots: self.seats.iter().flatten().filter(|s| s.bot).count(),
        }
    }

    /// Chips a person has bet on the coming spin: what the server counts as theirs while
    /// they're here.
    pub fn at_stake(&self, uid: &str) -> u64 {
        self.seat_of(uid).and_then(|s| self.seats[s as usize].as_ref()).map(|s| total(&s.chips)).unwrap_or(0)
    }

    pub fn in_use(&self) -> bool {
        !self.watchers.is_empty() || self.people().next().is_some()
    }

    pub fn knows(&self, uid: &str) -> bool {
        self.watchers.contains(uid) || self.seat_of(uid).is_some()
    }

    pub fn view(&self, uid: Option<&str>, now: u64) -> RlView {
        let (stage, ms_left) = match &self.stage {
            Stage::Waiting => (RlStage::Waiting, 0),
            Stage::Betting { until } => (RlStage::Betting, until.saturating_sub(now)),
            Stage::Shown { until } => (RlStage::Shown, until.saturating_sub(now)),
        };
        RlView {
            id: self.id,
            name: self.name.clone(),
            min_bet: self.min_bet,
            max_bet: self.max_bet,
            seats: self
                .seats
                .iter()
                .map(|s| {
                    s.as_ref().map(|s| RlSeatView {
                        player: s.uid.clone(),
                        name: s.name.clone(),
                        bot: s.bot,
                        level: s.look.level,
                        frame: s.look.frame.clone(),
                        chips: s.chips.clone(),
                        ready: s.ready,
                        last_chips: s.last.as_ref().map(|l| l.0.clone()).unwrap_or_default(),
                        paid: s.last.as_ref().map(|l| l.1).unwrap_or(0),
                        style: if s.bot { s.who.rl_style().to_string() } else { String::new() },
                    })
                })
                .collect(),
            stage,
            ms_left,
            spin_no: self.spin_no,
            commit: self.commit,
            last: self.last.clone(),
            history: self.history.clone(),
            my_seat: uid.and_then(|u| self.seat_of(u)),
        }
    }

    fn changed(&self, now: u64, out: &mut Output) {
        for uid in self.audience() {
            let view = self.view(Some(&uid), now);
            out.to(&uid, ToApp::Rl { view });
        }
    }

    pub fn watch(&mut self, uid: &str, now: u64) -> ToApp {
        self.watchers.insert(uid.to_string());
        self.seen_ms = now;
        ToApp::Rl { view: self.view(Some(uid), now) }
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
        *s = Some(Seat { uid: uid.into(), name: name.into(), bot: false, look, chips: Vec::new(), ready: false, seed: None, idle: 0, bot_bet_at: 0, who: &persona::PLAIN, units: 1, lucky: 0, last: None });
        self.watchers.insert(uid.into());
        self.seen_ms = now;
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// Stands up: their chips for the coming spin come back.
    pub fn leave(&mut self, uid: &str, now: u64) -> Output {
        let mut out = Output::default();
        let Some(seat) = self.seat_of(uid) else { return out };
        let s = self.seats[seat as usize].take().unwrap();
        if total(&s.chips) > 0 {
            out.effects.push(Effect::Credit { uid: uid.into(), chips: total(&s.chips) });
        }
        if self.people().next().is_none() {
            self.everyone_gone();
        }
        self.changed(now, &mut out);
        out
    }

    /// No people left: the bots' chips come off (they don't play alone).
    fn everyone_gone(&mut self) {
        for s in self.seats.iter_mut().flatten() {
            s.chips.clear();
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
        // (Its lucky number: the same every time it sits down.)
        let lucky = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32)) % 37;
        *s = Some(Seat {
            uid: format!("bot:rl{}:{seat}", self.id),
            name,
            bot: true,
            look: Look { level: 12, ..Look::default() },
            chips: Vec::new(),
            ready: false,
            seed: None,
            idle: 0,
            bot_bet_at: now + bot_ms(2_800.0, who, &mut self.whim),
            who,
            units: 1,
            lucky: lucky as u8,
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

    /// What going from my chips to `new` puts down (positive) or takes off (negative), if it's
    /// allowed now (before the ball goes, each place within the table's limits).
    pub fn cost(&self, uid: &str, new: &[Chip]) -> Result<i64, String> {
        let seat = self.seat_of(uid).ok_or("sit down first")?;
        let s = self.seats[seat as usize].as_ref().unwrap();
        within(new, self.min_bet, self.max_bet)?;
        Ok(total(new) as i64 - total(&s.chips) as i64)
    }

    /// A person's chips for the coming spin (more chips taken by the server already; chips coming
    /// off go back as a credit), their Pyr's seed for it, and whether they're ready for the ball.
    pub fn place(&mut self, uid: &str, new: Vec<Chip>, seed: [u8; 32], ready: bool, now: u64) -> Result<Output, String> {
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
        s.chips = new;
        s.seed = Some(seed);
        s.ready = ready;
        if total(&s.chips) > 0 {
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
                s.bot_bet_at = now + bot_ms(2_600.0, s.who, &mut whim);
            }
        }
        self.whim = whim;
    }

    /// A bot's chips on the coming spin, its own way: even money on red or black (pressing after
    /// a win), the least on a dozen and a column, straight up on its lucky number and its
    /// neighbours round the wheel, a spray of corners and splits, or the last number again
    /// (it's hot, it says) and the colour that's due.
    fn bot_chips(&mut self, seat: usize) -> Vec<Chip> {
        let (lo, hi) = (self.min_bet, self.max_bet);
        let s = self.seats[seat].as_ref().unwrap();
        let (who, units, lucky) = (s.who, s.units, s.lucky);
        let last = self.history.last().copied();
        let r = &mut self.whim;
        let unit = match who.bj_bets {
            BjBets::Minimum => 1,
            BjBets::Steady => [1u64, 1, 2, 2, 3][r.below(5) as usize],
            BjBets::Press => units,
            BjBets::Big => [5u64, 10, 10, 20, 25][r.below(5) as usize],
        };
        let on = |spot: Spot, n: u64| Chip { spot, chips: (lo * n).clamp(lo, hi.min(roulette::max_on(&spot))) };
        let mut c = Vec::new();
        match who.bj {
            BjPlay::Book => c.push(on(if r.below(2) == 0 { Spot::Red } else { Spot::Black }, unit)),
            BjPlay::Careful => {
                c.push(on(Spot::Dozen { d: r.below(3) as u8 }, 1));
                c.push(on(Spot::Column { c: r.below(3) as u8 }, 1));
            }
            BjPlay::Dealer => {
                // Its number and the two either side of it round the wheel.
                let at = roulette::WHEEL.iter().position(|n| *n == lucky).unwrap_or(0);
                for d in [36usize, 0, 1] {
                    c.push(on(Spot::Straight { n: roulette::WHEEL[(at + d) % 37] }, 1));
                }
            }
            BjPlay::Daredevil => {
                for _ in 0..(2 + r.below(3)) {
                    let a = 1 + r.below(32) as u8;
                    let spot = if a % 3 != 0 { Spot::Corner { a } } else { Spot::Split { a, b: a + 3 } };
                    if !c.iter().any(|x: &Chip| x.spot == spot) {
                        c.push(on(spot, unit.min(3)));
                    }
                }
                c.push(on(Spot::Straight { n: lucky }, 1));
            }
            BjPlay::Superstitious => {
                match last {
                    Some(n) => {
                        c.push(on(Spot::Straight { n }, 1));
                        let due = if n != 0 && roulette::is_red(n) { Spot::Black } else { Spot::Red };
                        c.push(on(due, unit));
                    }
                    None => c.push(on(Spot::Straight { n: 0 }, 1)),
                }
                if r.below(3) == 0 {
                    c.push(on(Spot::Straight { n: 0 }, 1));
                }
            }
        }
        let c = merged(&c);
        if within(&c, lo, hi).is_ok() { c } else { vec![Chip { spot: Spot::Red, chips: lo }] }
    }

    fn bots_bet(&mut self, now: u64, all: bool) -> bool {
        let mut any = false;
        for i in 0..SEATS {
            let due = self.seats[i].as_ref().is_some_and(|s| s.bot && !s.ready && (all || now >= s.bot_bet_at));
            if due {
                let c = self.bot_chips(i);
                let s = self.seats[i].as_mut().unwrap();
                s.chips = c;
                s.ready = true;
                any = true;
            }
        }
        any
    }

    /// The spin: decided from everyone's seeds, everyone paid (and their records told), the
    /// seeds shown, and the next spin's fingerprint.
    fn spin(&mut self, now: u64, rng: &mut dyn Random, out: &mut Output) {
        self.bots_bet(now, true);
        for s in self.seats.iter_mut().flatten().filter(|s| !s.bot) {
            if total(&s.chips) == 0 {
                s.idle += 1;
            }
        }
        let idle: Vec<String> = self.seats.iter().flatten().filter(|s| !s.bot && s.idle >= IDLE_SPINS).map(|s| s.uid.clone()).collect();
        for uid in idle {
            out.extend(self.leave(&uid, now));
        }
        if self.people().next().is_none() {
            self.everyone_gone();
            self.changed(now, out);
            return;
        }
        let seeds: Vec<(u8, [u8; 32])> = self.seats.iter().enumerate().filter_map(|(i, s)| s.as_ref().filter(|s| !s.bot).and_then(|s| s.seed).map(|seed| (i as u8, seed))).collect();
        let (number, pocket) = roulette::spin(&mut Stream::new(spin_seed(&self.house, &seeds, self.spin_no)));
        for s in self.seats.iter_mut().flatten() {
            if total(&s.chips) == 0 {
                s.last = None;
                continue;
            }
            let chips = std::mem::take(&mut s.chips);
            let returns = roulette::settle(&chips, number);
            let paid: u64 = returns.iter().sum();
            if s.bot {
                s.units = if paid > total(&chips) { (s.units * 2).min(8) } else { 1 };
            } else {
                if paid > 0 {
                    out.effects.push(Effect::Credit { uid: s.uid.clone(), chips: paid });
                }
                let outcome = roulette::Outcome { number, pocket, chips: chips.clone(), returns, paid };
                out.effects.push(Effect::RlSpin { uid: s.uid.clone(), outcome });
            }
            s.last = Some((chips, paid));
        }
        self.history.push(number);
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
        self.last = Some(RlReveal { spin: self.spin_no, commit: self.commit, house: self.house, seeds: seeds.iter().map(|(s, b)| (*s, seed_hex(b))).collect(), number, pocket });
        // The next spin's seed, its fingerprint shown before anyone bets on it.
        self.spin_no += 1;
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
            Stage::Waiting => (RlStage::Waiting, 0),
            Stage::Betting { until } => (RlStage::Betting, *until),
            Stage::Shown { until } => (RlStage::Shown, *until),
        };
        match stage {
            RlStage::Waiting => {
                if self.people().next().is_some() {
                    self.open_betting(now);
                    self.changed(now, &mut out);
                }
            }
            RlStage::Betting => {
                if self.people().next().is_none() {
                    self.everyone_gone();
                    self.changed(now, &mut out);
                    return out;
                }
                if self.bots_bet(now, false) {
                    self.changed(now, &mut out);
                }
                if now >= until {
                    self.spin(now, rng, &mut out);
                }
            }
            RlStage::Shown => {
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

    /// Everyone's chips back (it's closing, or the server's stopping).
    pub fn close(&mut self) -> Output {
        let mut out = Output::default();
        for s in self.seats.iter_mut().flatten().filter(|s| !s.bot) {
            if total(&s.chips) > 0 {
                out.effects.push(Effect::Credit { uid: s.uid.clone(), chips: total(&s.chips) });
            }
            s.chips.clear();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chips_are_fitted_to_the_table_before_they_go() {
        let n = |n: u8| Spot::Straight { n };
        let mut c = vec![Chip { spot: n(7), chips: 5 }, Chip { spot: n(7), chips: 3 }, Chip { spot: n(9), chips: 50_000 }, Chip { spot: n(12), chips: 0 }];
        assert!(fit(&mut c, 10, 1_000));
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].chips, 10);
        assert_eq!(c[1].chips, 1_000.min(roulette::max_on(&n(9))));
        assert!(within(&c, 10, 1_000).is_ok());
        assert!(!fit(&mut c, 10, 1_000));
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

    /// People and bots play spins; every spin checks out from its seeds; what people bet comes
    /// back as the wheel pays it (about 97.3%), nothing made or lost on the way; the bots bet
    /// every spin, each its own way and within the limits.
    #[test]
    fn a_table_plays_spins_that_check_out() {
        let mut rng = Rng(Stream::new(seed_from(5)));
        let mut t = RlRoom::new(1, "Test", 10, 1_000, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 1, 0).unwrap();
        t.sit("bob", "Bob", Look::default(), 3, 0).unwrap();
        for seat in [0, 2, 4] {
            t.add_bot(seat, 0, &mut rng).unwrap();
        }
        let (mut taken, mut back) = (0u64, 0u64);
        let (mut spins, mut seen) = (0, 0);
        let mut bots_bet = 0;
        let mut now = 0u64;
        let alice = vec![Chip { spot: Spot::Red, chips: 100 }, Chip { spot: Spot::Dozen { d: 1 }, chips: 50 }];
        let bob = vec![Chip { spot: Spot::Straight { n: 17 }, chips: 10 }, Chip { spot: Spot::Corner { a: 1 }, chips: 20 }, Chip { spot: Spot::Black, chips: 60 }];
        while now < 8_000_000 {
            let out = t.tick(now, &mut rng);
            back += credits(&out);
            if let Some(r) = &t.last
                && r.spin != seen
            {
                seen = r.spin;
                check(r).unwrap();
                spins += 1;
                for s in t.seats.iter().flatten().filter(|s| s.bot) {
                    let (c, _) = s.last.as_ref().expect("the bots bet every spin");
                    within(c, t.min_bet, t.max_bet).unwrap();
                    bots_bet += 1;
                }
            }
            if matches!(t.stage, Stage::Betting { .. }) {
                for (uid, chips) in [("alice", &alice), ("bob", &bob)] {
                    let seat = t.seat_of(uid).unwrap() as usize;
                    if t.seats[seat].as_ref().unwrap().ready {
                        continue;
                    }
                    let cost = t.cost(uid, chips).unwrap();
                    taken += cost.max(0) as u64;
                    back += credits(&t.place(uid, chips.clone(), rng.seed(), true, now).unwrap());
                }
            }
            now += 250;
        }
        assert!(spins > 450, "{spins} spins");
        assert_eq!(bots_bet, spins * 3);
        let rtp = back as f64 / taken as f64;
        assert!((0.75..1.25).contains(&rtp), "{rtp} over {spins} spins");
        assert_eq!(t.history.len(), HISTORY.min(spins));
    }

    /// Chips come off before the ball goes (back to their owner); standing up gives back what's
    /// down; a spin that's been tampered with doesn't check out; chips outside the limits, on a
    /// place that isn't on the layout, or the same place twice are refused.
    #[test]
    fn chips_come_off_and_a_wrong_pocket_is_caught() {
        let mut rng = Rng(Stream::new(seed_from(9)));
        let mut t = RlRoom::new(1, "Test", 10, 500, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 2, 0).unwrap();
        t.place("alice", vec![Chip { spot: Spot::Odd, chips: 100 }, Chip { spot: Spot::Straight { n: 0 }, chips: 20 }], rng.seed(), false, 0).unwrap();
        assert_eq!(t.at_stake("alice"), 120);
        let out = t.place("alice", vec![Chip { spot: Spot::Odd, chips: 100 }], rng.seed(), true, 0).unwrap();
        assert_eq!(credits(&out), 20);
        assert!(t.cost("alice", &[Chip { spot: Spot::Red, chips: 5 }]).is_err());
        assert!(t.cost("alice", &[Chip { spot: Spot::Red, chips: 501 }]).is_err());
        assert!(t.cost("alice", &[Chip { spot: Spot::Corner { a: 3 }, chips: 50 }]).is_err());
        assert!(t.cost("alice", &[Chip { spot: Spot::Red, chips: 50 }, Chip { spot: Spot::Red, chips: 50 }]).is_err());
        assert!(t.cost("nobody", &[]).is_err());
        let mut now = 0;
        while t.last.is_none() && now < 60_000 {
            t.tick(now, &mut rng);
            now += 250;
        }
        let r = t.last.clone().unwrap();
        check(&r).unwrap();
        assert_eq!(r.seeds.len(), 1, "her seed went into it");
        let mut bad = r.clone();
        bad.pocket = (bad.pocket + 1) % 37;
        bad.number = roulette::WHEEL[bad.pocket as usize];
        assert!(check(&bad).is_err());
        let mut bad = r.clone();
        bad.house[0] ^= 1;
        assert!(check(&bad).is_err());
        // Standing up with chips down gives them back.
        while !matches!(t.stage, Stage::Betting { .. }) {
            t.tick(now, &mut rng);
            now += 250;
        }
        t.place("alice", vec![Chip { spot: Spot::High, chips: 200 }], rng.seed(), false, now).unwrap();
        assert_eq!(credits(&t.leave("alice", now)), 200);
        assert_eq!(t.at_stake("alice"), 0);
    }
}
