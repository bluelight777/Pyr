//! A craps table in Pyr Casino: five seats for people and bots around one layout, the dice going
//! round the table from shooter to shooter. Everyone bets on the same rolls: the pass line and
//! its odds, don't pass, place bets, hardways, the field, any seven and any craps (the bets and
//! what they pay are `casino::craps`'s, as at the single-player table).
//!
//! It's fair the way the blackjack tables are: before each roll the house shows its seed's
//! fingerprint; each person's Pyr adds a seed of its own with their bets; the dice come from all
//! of them together (`roll_seed`). Once they've landed the house shows its seed, and everyone's
//! Pyr rolls them again to be sure (`check`). Bots bet with the house's chips: they're company,
//! and nobody's chips go to or from them.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::bots::persona::{self, BjBets, BjPlay, Persona};
use crate::casino::craps::{self, Bets, HARD_NUMBERS, PLACE_NUMBERS, Table};
use crate::fair::{self, Stream};
use crate::net::{CrapsReveal, CrapsSeatView, CrapsStage, CrapsSummary, CrapsView, ToApp};
use crate::room::{Effect, Look, Output, Random};

pub const SEATS: usize = 5;
/// How long bets stay open before each roll (the shooter can throw sooner), and how soon the
/// dice fly once every person at the table has said they're ready.
pub const BET_MS: u64 = 15_000;
pub const QUICK_MS: u64 = 1_200;
/// The dice in the air and landing, then the roll's results on the table.
pub const SHOW_MS: u64 = 5_500;
/// Rolls a person can sit through with nothing on the layout before they're stood up (their
/// seat's for someone playing).
pub const IDLE_ROLLS: u32 = 12;

const BOT_NAMES: [&str; 14] = ["Ace", "Blaze", "Cinder", "Dusk", "Ember", "Flint", "Gale", "Ivy", "Lux", "Moss", "Nova", "Sage", "Tarn", "Wren"];

struct Seat {
    uid: String,
    name: String,
    bot: bool,
    look: Look,
    /// What's on the layout for them now (a person's chips taken from them already), whether
    /// they've said they're ready for the roll, and their Pyr's seed for it.
    bets: Bets,
    ready: bool,
    seed: Option<[u8; 32]>,
    /// Rolls in a row with nothing on the layout.
    idle: u32,
    /// A bot: when its bets go down for the coming roll; who it is (how it bets, its pace); the
    /// units it's betting (pressing after a win).
    bot_bet_at: u64,
    who: &'static Persona,
    units: u64,
    /// What the last roll did for them (shown till the next).
    last: Option<craps::Outcome>,
}

enum Stage {
    /// Nobody (no person) sitting: the dice wait.
    Waiting,
    /// Bets go down (and come off, where they may) till `until`, when the dice fly (or sooner,
    /// when the shooter throws).
    Betting { until: u64 },
    /// The dice have landed and everyone's been paid: the results stay on the table till
    /// `until` (bets for the next roll can go down meanwhile).
    Shown { until: u64 },
}

pub struct CrapsRoom {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    seats: Vec<Option<Seat>>,
    watchers: BTreeSet<String>,
    stage: Stage,
    /// The coming roll's number and the house's seed for it (its fingerprint shown).
    roll_no: u64,
    house: [u8; 32],
    commit: [u8; 32],
    point: Option<u8>,
    /// The shooter's points made in a row.
    streak: u32,
    /// Whose dice they are (a seat), and when a bot shooter throws.
    shooter: Option<u8>,
    throw_at: Option<u64>,
    /// The last roll, with its seeds for checking.
    last: Option<CrapsReveal>,
    /// Bets still working for people who've stood up (a pass line stays up once there's a point,
    /// and its odds with it): settled with the rolls as they come, and paid to them.
    left: Vec<(String, Bets)>,
    /// When a person was last here (a table nobody's at closes after a while).
    pub seen_ms: u64,
    /// Bots' whims: their bets and their pace.
    whim: Stream,
}

/// A roll's seed: the house's, every person's seat seed, and the roll's number.
pub fn roll_seed(house: &[u8; 32], seeds: &[(u8, [u8; 32])], roll: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr craps table v1");
    h.update(house);
    let mut seeds = seeds.to_vec();
    seeds.sort_by_key(|s| s.0);
    for (seat, s) in &seeds {
        h.update([*seat]);
        h.update(s);
    }
    h.update(roll.to_le_bytes());
    h.finalize().into()
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A seed as a roll's reveal shows it.
pub fn seed_hex(b: &[u8; 32]) -> String {
    hex(b)
}

/// Checks a roll: the house's seed matches the fingerprint it showed, and the dice rolled again
/// from everyone's seeds come up as they did at the table.
pub fn check(r: &CrapsReveal) -> Result<(), String> {
    if fair::commit(&r.house) != r.commit {
        return Err(format!("roll {}: the house's seed doesn't match the fingerprint it showed first", r.roll));
    }
    let seeds: Vec<(u8, [u8; 32])> = r.seeds.iter().filter_map(|(s, h)| crate::net::hex32::parse(h).map(|b| (*s, b))).collect();
    if seeds.len() != r.seeds.len() {
        return Err(format!("roll {}: a player's seed isn't a seed", r.roll));
    }
    let dice = craps::roll(&mut Stream::new(roll_seed(&r.house, &seeds, r.roll)));
    if dice != r.dice {
        return Err(format!("roll {}: rolled again, the dice came up {} and {}, not {} and {}", r.roll, dice.0, dice.1, r.dice.0, r.dice.1));
    }
    Ok(())
}

/// The bets that may go on the layout: each on its own at least the table's least and at most
/// its most (the odds go up to their multiple of the pass line instead).
fn within(b: &Bets, lo: u64, hi: u64) -> Result<(), String> {
    let mut each: Vec<(String, u64)> = vec![("The pass line".into(), b.pass), ("Don't pass".into(), b.dont_pass), ("The field".into(), b.field), ("Any seven".into(), b.any_seven), ("Any craps".into(), b.any_craps)];
    each.extend(b.place.iter().enumerate().map(|(i, x)| (craps::place_name(i), *x)));
    each.extend(b.hard.iter().enumerate().map(|(i, x)| (craps::hard_name(i), *x)));
    for (name, x) in each {
        if x > 0 && (x < lo || x > hi) {
            return Err(format!("{name} here is {lo} to {hi}"));
        }
    }
    Ok(())
}

/// Brings each layout bet into the table's least to its most (under goes up, over comes down),
/// so what's sent always passes [`within`]. Says whether anything moved.
pub fn fit(b: &mut Bets, lo: u64, hi: u64) -> bool {
    let before = b.clone();
    let hi = hi.max(lo);
    let mut each: Vec<&mut u64> = vec![&mut b.pass, &mut b.dont_pass, &mut b.field, &mut b.any_seven, &mut b.any_craps];
    each.extend(b.place.iter_mut());
    each.extend(b.hard.iter_mut());
    for x in each {
        if *x > 0 {
            *x = (*x).clamp(lo, hi);
        }
    }
    *b != before
}

/// How long a bot takes to get its chips down, or to throw, the way a person would: at its own
/// pace, never the same twice, with the odd long pause.
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

impl CrapsRoom {
    pub fn new(id: u64, name: &str, min_bet: u64, max_bet: u64, now: u64, rng: &mut dyn Random) -> CrapsRoom {
        let house = rng.seed();
        CrapsRoom {
            id,
            name: name.chars().take(40).collect(),
            min_bet: min_bet.max(1),
            max_bet: max_bet.max(min_bet.max(1)),
            seats: (0..SEATS).map(|_| None).collect(),
            watchers: BTreeSet::new(),
            stage: Stage::Waiting,
            roll_no: 1,
            commit: fair::commit(&house),
            house,
            point: None,
            streak: 0,
            shooter: None,
            throw_at: None,
            last: None,
            left: Vec::new(),
            seen_ms: now,
            whim: Stream::new(rng.seed()),
        }
    }

    fn people(&self) -> impl Iterator<Item = &Seat> {
        self.seats.iter().flatten().filter(|s| !s.bot)
    }

    /// The coming roll's number.
    pub fn roll_no(&self) -> u64 {
        self.roll_no
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

    pub fn summary(&self) -> CrapsSummary {
        CrapsSummary {
            id: self.id,
            name: self.name.clone(),
            min_bet: self.min_bet,
            max_bet: self.max_bet,
            people: self.people().count(),
            bots: self.seats.iter().flatten().filter(|s| s.bot).count(),
            point: self.point,
        }
    }

    /// Chips a person has on the layout (and still working from when they stood up): what the
    /// server counts as theirs while they're here.
    pub fn at_stake(&self, uid: &str) -> u64 {
        let seat = self.seat_of(uid).and_then(|s| self.seats[s as usize].as_ref()).map(|s| s.bets.total()).unwrap_or(0);
        seat + self.left.iter().filter(|(u, _)| u == uid).map(|(_, b)| b.total()).sum::<u64>()
    }

    /// Whether anyone (a person) is sitting or watching: a table nobody's at can close.
    pub fn in_use(&self) -> bool {
        !self.watchers.is_empty() || self.people().next().is_some()
    }

    /// Everyone sitting (people) or watching, to stand up or let go when they've gone.
    pub fn knows(&self, uid: &str) -> bool {
        self.watchers.contains(uid) || self.seat_of(uid).is_some()
    }

    pub fn view(&self, uid: Option<&str>, now: u64) -> CrapsView {
        let (stage, ms_left) = match &self.stage {
            Stage::Waiting => (CrapsStage::Waiting, 0),
            Stage::Betting { until } => (CrapsStage::Betting, until.saturating_sub(now)),
            Stage::Shown { until } => (CrapsStage::Shown, until.saturating_sub(now)),
        };
        CrapsView {
            id: self.id,
            name: self.name.clone(),
            min_bet: self.min_bet,
            max_bet: self.max_bet,
            seats: self
                .seats
                .iter()
                .map(|s| {
                    s.as_ref().map(|s| CrapsSeatView {
                        player: s.uid.clone(),
                        name: s.name.clone(),
                        bot: s.bot,
                        level: s.look.level,
                        frame: s.look.frame.clone(),
                        bets: s.bets.clone(),
                        ready: s.ready,
                        decided: s.last.as_ref().map(|o| o.decided.clone()).unwrap_or_default(),
                        paid: s.last.as_ref().map(|o| o.paid).unwrap_or(0),
                        style: if s.bot { s.who.craps_style().to_string() } else { String::new() },
                    })
                })
                .collect(),
            stage,
            ms_left,
            roll_no: self.roll_no,
            commit: self.commit,
            point: self.point,
            streak: self.streak,
            shooter: self.shooter,
            last: self.last.clone(),
            my_seat: uid.and_then(|u| self.seat_of(u)),
            left: uid.map(|u| self.left.iter().filter(|(x, _)| x == u).fold(Bets::default(), |mut a, (_, b)| {
                a.pass += b.pass;
                a.pass_odds += b.pass_odds;
                a
            })).unwrap_or_default(),
        }
    }

    /// Everyone's told how the table is now.
    fn changed(&self, now: u64, out: &mut Output) {
        for uid in self.audience() {
            let view = self.view(Some(&uid), now);
            out.to(&uid, ToApp::Craps { view });
        }
    }

    pub fn watch(&mut self, uid: &str, now: u64) -> ToApp {
        self.watchers.insert(uid.to_string());
        self.seen_ms = now;
        ToApp::Craps { view: self.view(Some(uid), now) }
    }

    pub fn unwatch(&mut self, uid: &str) {
        self.watchers.remove(uid);
    }

    /// The next seat round from `from` with someone in it (the dice go round the table).
    fn next_shooter(&self, from: Option<u8>) -> Option<u8> {
        let start = from.map(|f| f as usize + 1).unwrap_or(0);
        (0..SEATS).map(|k| ((start + k) % SEATS) as u8).find(|i| self.seats[*i as usize].is_some())
    }

    /// The dice go to the next shooter (a seven-out, or the shooter's gone).
    fn pass_dice(&mut self, now: u64) {
        let from = self.shooter;
        self.shooter = self.next_shooter(from);
        self.streak = 0;
        self.plan_throw(now);
    }

    /// A bot shooter throws a while after the betting opens (sooner once everyone's ready).
    fn plan_throw(&mut self, now: u64) {
        self.throw_at = None;
        let Some(s) = self.shooter.and_then(|i| self.seats[i as usize].as_ref()) else { return };
        if s.bot && let Stage::Betting { until } = self.stage {
            let who = s.who;
            let ms = bot_ms(BET_MS as f64 * 0.62, who, &mut self.whim);
            self.throw_at = Some((now + ms).min(until));
        }
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
        if self.shooter.is_none() {
            self.shooter = Some(seat);
        }
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// Stands up: what may come off the layout comes back; a pass line with a point on (and its
    /// odds) stays up till it's decided, and is paid to them then.
    pub fn leave(&mut self, uid: &str, now: u64) -> Output {
        let mut out = Output::default();
        let Some(seat) = self.seat_of(uid) else { return out };
        let s = self.seats[seat as usize].take().unwrap();
        let mut back = s.bets.clone();
        if self.point.is_some() && s.bets.pass > 0 {
            let stays = Bets { pass: s.bets.pass, pass_odds: s.bets.pass_odds, ..Bets::default() };
            back.pass = 0;
            back.pass_odds = 0;
            self.left.push((uid.to_string(), stays));
        }
        if back.total() > 0 {
            out.effects.push(Effect::Credit { uid: uid.into(), chips: back.total() });
        }
        if self.shooter == Some(seat) {
            self.pass_dice(now);
        }
        if self.people().next().is_none() {
            self.everyone_gone();
        }
        self.changed(now, &mut out);
        out
    }

    /// No people left: the bots' bets come down (they don't play alone), and the dice wait.
    fn everyone_gone(&mut self) {
        for s in self.seats.iter_mut().flatten() {
            s.bets = Bets::default();
            s.ready = false;
        }
        if self.left.is_empty() {
            self.point = None;
            self.streak = 0;
        }
        self.stage = Stage::Waiting;
        self.throw_at = None;
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
            uid: format!("bot:craps{}:{seat}", self.id),
            name,
            bot: true,
            look: Look { level: 12, ..Look::default() },
            bets: Bets::default(),
            ready: false,
            seed: None,
            idle: 0,
            bot_bet_at: now + bot_ms(2_200.0, who, &mut self.whim),
            who,
            units: 1,
            last: None,
        });
        if self.shooter.is_none() {
            self.shooter = Some(seat);
            self.plan_throw(now);
        }
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
        if self.shooter == Some(seat) {
            self.pass_dice(now);
        }
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// What going from my bets to `new` puts down (positive) or takes off (negative), if it's
    /// allowed now: the bets as the game has them (a pass line stays up once there's a point,
    /// odds up to their multiple, don't pass only comes down), each within the table's limits.
    pub fn cost(&self, uid: &str, new: &Bets) -> Result<i64, String> {
        let seat = self.seat_of(uid).ok_or("sit down first")?;
        let s = self.seats[seat as usize].as_ref().unwrap();
        craps::check_change(&Table { point: self.point, bets: s.bets.clone(), streak: self.streak }, new)?;
        within(new, self.min_bet, self.max_bet)?;
        Ok(new.total() as i64 - s.bets.total() as i64)
    }

    /// A person's bets for the coming roll (more chips taken by the server already; chips coming
    /// off go back as a credit), their Pyr's seed for it, and whether they're ready for the dice.
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

    /// Every person's ready: the dice fly soon (a bot shooter takes a moment over it).
    fn everyone_ready(&mut self, now: u64) {
        let all = self.people().next().is_some() && self.people().all(|s| s.ready);
        if let Stage::Betting { until } = &mut self.stage
            && all
        {
            *until = (*until).min(now + QUICK_MS + 1_500);
            let soon = now + QUICK_MS;
            self.throw_at = self.throw_at.map(|t| t.min(soon));
            let mut at = now + 200;
            for s in self.seats.iter_mut().flatten().filter(|s| s.bot && !s.ready) {
                s.bot_bet_at = s.bot_bet_at.min(at);
                at += 200;
            }
        }
    }

    /// The shooter (a person) throws.
    pub fn throw(&mut self, uid: &str, now: u64, rng: &mut dyn Random) -> Result<Output, String> {
        let seat = self.seat_of(uid).ok_or("sit down first")?;
        if self.shooter != Some(seat) {
            return Err("it's someone else's turn with the dice".into());
        }
        if !matches!(self.stage, Stage::Betting { .. }) {
            return Err("the dice are on their way back to you".into());
        }
        self.seen_ms = now;
        let mut out = Output::default();
        self.roll(now, rng, &mut out);
        Ok(out)
    }

    fn open_betting(&mut self, now: u64) {
        self.stage = Stage::Betting { until: now + BET_MS };
        let mut whim = std::mem::replace(&mut self.whim, Stream::new([0; 32]));
        for s in self.seats.iter_mut().flatten() {
            s.ready = false;
            if s.bot {
                s.bot_bet_at = now + bot_ms(2_000.0, s.who, &mut whim);
            }
        }
        self.whim = whim;
        if self.shooter.is_none_or(|i| self.seats[i as usize].is_none()) {
            self.shooter = self.next_shooter(self.shooter);
        }
        self.plan_throw(now);
    }

    /// A bot's bets for the coming roll, its own way, from the table as it is (its bets riding):
    /// the pass line and full odds; the pass line, then the 6 and 8 placed; flat on the pass line;
    /// the field, the hardways and the long shots; or the dark side, don't pass.
    fn bot_bets(&mut self, seat: usize) -> Bets {
        let (lo, hi, point) = (self.min_bet, self.max_bet, self.point);
        let s = self.seats[seat].as_ref().unwrap();
        let (who, old, units) = (s.who, s.bets.clone(), s.units);
        let r = &mut self.whim;
        let unit = match who.bj_bets {
            BjBets::Minimum => 1,
            BjBets::Steady => [1u64, 1, 2, 2, 3][r.below(5) as usize],
            BjBets::Press => units,
            BjBets::Big => [5u64, 10, 10, 20, 25][r.below(5) as usize],
        };
        let chips = |n: u64| (lo * n).clamp(lo, hi);
        let mut b = old.clone();
        // (Place 6 and 8 in sixes, so the 7 to 6 pays whole.)
        let sixes = |n: u64| (((lo * n).div_ceil(6)) * 6).clamp(lo, hi);
        let place = |b: &mut Bets, n: u8, x: u64| {
            if let Some(i) = PLACE_NUMBERS.iter().position(|p| *p == n) {
                b.place[i] = x;
            }
        };
        let hard = |b: &mut Bets, n: u8, x: u64| {
            if let Some(i) = HARD_NUMBERS.iter().position(|p| *p == n) {
                b.hard[i] = x;
            }
        };
        match (who.bj, point) {
            (BjPlay::Book, None) => b.pass = chips(unit),
            (BjPlay::Book, Some(p)) => {
                if b.pass > 0 {
                    b.pass_odds = b.pass * craps::max_odds(p);
                }
            }
            (BjPlay::Careful, None) => b.pass = chips(1),
            (BjPlay::Careful, Some(_)) => {
                place(&mut b, 6, sixes(unit));
                place(&mut b, 8, sixes(unit));
            }
            (BjPlay::Dealer, None) => b.pass = chips(unit),
            (BjPlay::Dealer, Some(p)) => {
                if b.pass > 0 && r.below(3) == 0 {
                    b.pass_odds = b.pass;
                }
                let _ = p;
            }
            (BjPlay::Daredevil, None) => {
                b.pass = chips(unit);
                b.field = if r.below(2) == 0 { chips(unit) } else { 0 };
                b.any_craps = if r.below(4) == 0 { chips(1) } else { 0 };
            }
            (BjPlay::Daredevil, Some(p)) => {
                b.field = if r.below(3) > 0 { chips(unit) } else { 0 };
                if HARD_NUMBERS.contains(&p) && r.below(2) == 0 {
                    hard(&mut b, p, chips(1));
                }
                b.any_seven = if r.below(8) == 0 { chips(1) } else { 0 };
            }
            (BjPlay::Superstitious, None) => {
                b.dont_pass = chips(unit);
                // (Its lucky number, the hard way, every so often.)
                if r.below(3) == 0 {
                    hard(&mut b, [4u8, 6, 8, 10][who.name.len() % 4], chips(1));
                }
            }
            (BjPlay::Superstitious, Some(_)) => {}
        }
        // (Whatever it can't do here, it leaves as it was.)
        let t = Table { point, bets: old.clone(), streak: self.streak };
        if craps::check_change(&t, &b).is_ok() && within(&b, lo, hi).is_ok() { b } else { old }
    }

    /// Bots whose moment has come put their chips down.
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

    /// The dice fly: everyone's bets settled, people paid (and their records told), the seeds
    /// shown, the dice passed on a seven-out, and the next roll's fingerprint.
    fn roll(&mut self, now: u64, rng: &mut dyn Random, out: &mut Output) {
        // (A bot still thinking it over gets its chips down now: bots play every roll.)
        self.bots_bet(now, true);
        // People with nothing on the layout count a roll idle; too many, and they're stood up.
        for s in self.seats.iter_mut().flatten().filter(|s| !s.bot) {
            if s.bets.is_empty() {
                s.idle += 1;
            }
        }
        let idle: Vec<String> = self.seats.iter().flatten().filter(|s| !s.bot && s.idle >= IDLE_ROLLS).map(|s| s.uid.clone()).collect();
        for uid in idle {
            out.extend(self.leave(&uid, now));
        }
        if self.people().next().is_none() {
            self.everyone_gone();
            self.changed(now, out);
            return;
        }
        let seeds: Vec<(u8, [u8; 32])> = self.seats.iter().enumerate().filter_map(|(i, s)| s.as_ref().filter(|s| !s.bot).and_then(|s| s.seed).map(|seed| (i as u8, seed))).collect();
        let dice = craps::roll(&mut Stream::new(roll_seed(&self.house, &seeds, self.roll_no)));
        let before = self.point;
        let table = |bets: &Bets, point, streak| Table { point, bets: bets.clone(), streak };
        let (after, _) = craps::settle(&table(&Bets::default(), self.point, self.streak), dice);
        for s in self.seats.iter_mut().flatten() {
            if s.bets.is_empty() {
                s.last = None;
                continue;
            }
            let (t, o) = craps::settle(&table(&s.bets, before, self.streak), dice);
            s.bets = t.bets;
            if s.bot {
                // (A bot that presses: up after a win, back to one after a loss.)
                if !o.decided.is_empty() {
                    s.units = if o.paid > o.wagered { (s.units * 2).min(8) } else { 1 };
                }
            } else {
                if o.paid > 0 {
                    out.effects.push(Effect::Credit { uid: s.uid.clone(), chips: o.paid });
                }
                if !o.decided.is_empty() {
                    out.effects.push(Effect::CrapsRoll { uid: s.uid.clone(), outcome: o.clone() });
                }
            }
            s.last = Some(o);
        }
        // Bets left working by people who stood up.
        let mut still = Vec::new();
        for (uid, bets) in std::mem::take(&mut self.left) {
            let (t, o) = craps::settle(&table(&bets, before, self.streak), dice);
            if o.paid > 0 {
                out.effects.push(Effect::Credit { uid: uid.clone(), chips: o.paid });
            }
            if !o.decided.is_empty() {
                out.effects.push(Effect::CrapsRoll { uid: uid.clone(), outcome: o });
            }
            if !t.bets.is_empty() {
                still.push((uid, t.bets));
            }
        }
        self.left = still;
        let seven_out = before.is_some() && dice.0 + dice.1 == 7;
        self.point = after.point;
        self.streak = after.streak;
        self.last = Some(CrapsReveal {
            roll: self.roll_no,
            commit: self.commit,
            house: self.house,
            seeds: seeds.iter().map(|(s, b)| (*s, hex(b))).collect(),
            dice,
            point_before: before,
            point_after: self.point,
            shooter: self.shooter,
        });
        // The next roll's seed, its fingerprint shown before anyone bets on it.
        self.roll_no += 1;
        self.house = rng.seed();
        self.commit = fair::commit(&self.house);
        for s in self.seats.iter_mut().flatten() {
            s.seed = None;
            s.ready = false;
        }
        self.stage = Stage::Shown { until: now + SHOW_MS };
        self.throw_at = None;
        if seven_out {
            self.pass_dice(now);
        }
        self.changed(now, out);
    }

    pub fn tick(&mut self, now: u64, rng: &mut dyn Random) -> Output {
        let mut out = Output::default();
        let (stage, until) = match &self.stage {
            Stage::Waiting => (CrapsStage::Waiting, 0),
            Stage::Betting { until } => (CrapsStage::Betting, *until),
            Stage::Shown { until } => (CrapsStage::Shown, *until),
        };
        match stage {
            CrapsStage::Waiting => {
                if self.people().next().is_some() {
                    self.open_betting(now);
                    self.changed(now, &mut out);
                }
            }
            CrapsStage::Betting => {
                if self.people().next().is_none() {
                    self.everyone_gone();
                    self.changed(now, &mut out);
                    return out;
                }
                if self.bots_bet(now, false) {
                    self.changed(now, &mut out);
                }
                let bot_throws = self.throw_at.is_some_and(|t| now >= t);
                if now >= until || bot_throws {
                    self.roll(now, rng, &mut out);
                }
            }
            CrapsStage::Shown => {
                if now >= until {
                    // (Fresh whims for the bots now and then, from the house.)
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

    /// Everyone's chips on the layout back (it's closing, or the server's stopping).
    pub fn close(&mut self) -> Output {
        let mut out = Output::default();
        for s in self.seats.iter_mut().flatten().filter(|s| !s.bot) {
            if s.bets.total() > 0 {
                out.effects.push(Effect::Credit { uid: s.uid.clone(), chips: s.bets.total() });
            }
            s.bets = Bets::default();
        }
        for (uid, b) in std::mem::take(&mut self.left) {
            out.effects.push(Effect::Credit { uid, chips: b.total() });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bets_are_fitted_to_the_table_before_they_go() {
        let mut b = Bets { pass: 5, field: 2_000, ..Default::default() };
        b.place[0] = 1;
        b.hard[1] = 1_001;
        assert!(fit(&mut b, 10, 1_000));
        assert_eq!((b.pass, b.field, b.place[0], b.hard[1], b.dont_pass), (10, 1_000, 10, 1_000, 0));
        assert!(within(&b, 10, 1_000).is_ok());
        assert!(!fit(&mut b, 10, 1_000));
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

    /// People and bots play rolls; every roll checks out from its seeds; the dice go round the
    /// table on seven-outs; the chips people put down all come back as wins or go as losses
    /// (nothing made, nothing lost), and the pass line pays about what it should.
    #[test]
    fn a_table_plays_rolls_that_check_out() {
        let mut rng = Rng(Stream::new(seed_from(5)));
        let mut t = CrapsRoom::new(1, "Test", 10, 1_000, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 1, 0).unwrap();
        t.sit("bob", "Bob", Look::default(), 3, 0).unwrap();
        for seat in [0, 2, 4] {
            t.add_bot(seat, 0, &mut rng).unwrap();
        }
        let (mut taken, mut back) = (0u64, 0u64);
        let (mut rolls, mut seen) = (0, 0);
        let mut shooters = BTreeSet::new();
        let mut now = 0u64;
        while now < 3_000_000 {
            let out = t.tick(now, &mut rng);
            back += credits(&out);
            if let Some(r) = &t.last
                && r.roll != seen
            {
                seen = r.roll;
                check(r).unwrap();
                rolls += 1;
                shooters.extend(r.shooter);
            }
            if matches!(t.stage, Stage::Betting { .. }) {
                // Alice bets the pass line and takes odds; Bob places the 6 and 8, and a field
                // bet now and then. Then they're ready (Alice throws when it's hers).
                for uid in ["alice", "bob"] {
                    let seat = t.seat_of(uid).unwrap() as usize;
                    if t.seats[seat].as_ref().unwrap().ready {
                        continue;
                    }
                    let mut b = t.seats[seat].as_ref().unwrap().bets.clone();
                    if uid == "alice" {
                        match t.point {
                            None => b.pass = 20,
                            Some(p) => b.pass_odds = b.pass * craps::max_odds(p),
                        }
                    } else if t.point.is_some() {
                        b.place[2] = 30;
                        b.place[3] = 30;
                        b.field = if rolls % 3 == 0 { 10 } else { 0 };
                    }
                    let cost = t.cost(uid, &b).unwrap();
                    if cost > 0 {
                        taken += cost as u64;
                    }
                    let out = t.place(uid, b, rng.seed(), true, now).unwrap();
                    back += credits(&out);
                    if t.shooter == Some(seat as u8) && uid == "alice" {
                        let out = t.throw("alice", now, &mut rng).unwrap();
                        back += credits(&out);
                    }
                }
            }
            now += 250;
        }
        assert!(rolls > 300, "{rolls} rolls");
        assert_eq!(shooters.len(), 5, "everyone gets the dice: {shooters:?}");
        let on_table: u64 = ["alice", "bob"].iter().map(|u| t.at_stake(u)).sum();
        let rtp = (back + on_table) as f64 / taken as f64;
        assert!((0.75..1.25).contains(&rtp), "{rtp} over {rolls} rolls");
        // The bots bet every roll, each its own way.
        assert!(t.seats.iter().flatten().filter(|s| s.bot).all(|s| s.last.is_some() || !s.bets.is_empty()));
    }

    /// The pass line can't come off once there's a point; standing up then leaves it working,
    /// and it's paid when it's decided. Everything else comes back.
    #[test]
    fn a_pass_line_stays_up_after_its_player_goes() {
        let mut rng = Rng(Stream::new(seed_from(11)));
        let mut t = CrapsRoom::new(1, "Test", 10, 1_000, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 0, 0).unwrap();
        t.sit("bob", "Bob", Look::default(), 1, 0).unwrap();
        let mut now = 0;
        // Come-out rolls till there's a point, Alice on the pass line.
        while t.point.is_none() {
            t.tick(now, &mut rng);
            if matches!(t.stage, Stage::Betting { .. }) && t.seats[0].as_ref().unwrap().bets.pass == 0 {
                t.place("alice", Bets { pass: 50, ..Bets::default() }, rng.seed(), true, now).unwrap();
                t.place("bob", Bets::default(), rng.seed(), true, now).unwrap();
            }
            now += 250;
            assert!(now < 1_000_000);
        }
        let p = t.point.unwrap();
        // Taking it down is refused; odds and a place bet go on.
        assert!(t.cost("alice", &Bets::default()).is_err());
        let i = PLACE_NUMBERS.iter().position(|x| *x != p).unwrap();
        let mut b = Bets { pass: 50, pass_odds: 50 * craps::max_odds(p), ..Bets::default() };
        b.place[i] = 60;
        assert_eq!(t.cost("alice", &b).unwrap(), (50 * craps::max_odds(p) + 60) as i64);
        while !matches!(t.stage, Stage::Betting { .. }) {
            t.tick(now, &mut rng);
            now += 250;
        }
        t.place("alice", b, rng.seed(), false, now).unwrap();
        // Alice stands up: the place bet comes back, the pass line and odds stay.
        let out = t.leave("alice", now);
        assert_eq!(credits(&out), 60);
        assert_eq!(t.at_stake("alice"), 50 + 50 * craps::max_odds(p));
        // Rolls till it's decided: paid to her (or lost), and nothing's held for her after.
        let mut paid = 0;
        while t.at_stake("alice") > 0 {
            let out = t.tick(now, &mut rng);
            paid += out.effects.iter().filter_map(|e| if let Effect::Credit { uid, chips } = e { (uid == "alice").then_some(*chips) } else { None }).sum::<u64>();
            if matches!(t.stage, Stage::Betting { .. }) && !t.seats[1].as_ref().unwrap().ready {
                t.place("bob", Bets::default(), rng.seed(), true, now).unwrap();
            }
            now += 250;
            assert!(now < 3_000_000);
        }
        let r = t.last.clone().unwrap();
        let sum = r.dice.0 + r.dice.1;
        if sum == p {
            assert_eq!(paid, 100 + 50 * craps::max_odds(p) + match p {
                4 | 10 => 2 * 50 * craps::max_odds(p),
                5 | 9 => 3 * 50 * craps::max_odds(p) / 2,
                _ => 6 * 50 * craps::max_odds(p) / 5,
            });
        } else {
            assert_eq!((sum, paid), (7, 0));
        }
    }

    /// A roll that's been tampered with doesn't check out.
    #[test]
    fn a_wrong_roll_is_caught() {
        let mut rng = Rng(Stream::new(seed_from(9)));
        let mut t = CrapsRoom::new(1, "Test", 10, 500, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 2, 0).unwrap();
        t.place("alice", Bets { pass: 10, ..Bets::default() }, rng.seed(), true, 0).unwrap();
        let mut now = 0;
        while t.last.is_none() && now < 120_000 {
            t.tick(now, &mut rng);
            if t.shooter == Some(2) && matches!(t.stage, Stage::Betting { .. }) {
                t.throw("alice", now, &mut rng).unwrap();
            }
            now += 250;
        }
        let r = t.last.clone().unwrap();
        check(&r).unwrap();
        assert_eq!(r.seeds.len(), 1, "her seed went into it");
        let mut bad = r.clone();
        bad.dice = if r.dice.0 == 6 { (1, r.dice.1) } else { (r.dice.0 + 1, r.dice.1) };
        assert!(check(&bad).is_err());
        let mut bad = r.clone();
        bad.house[0] ^= 1;
        assert!(check(&bad).is_err());
        // Bets outside the limits, or the game's rules, are refused.
        assert!(t.cost("alice", &Bets { pass: 5, ..Bets::default() }).is_err());
        assert!(t.cost("alice", &Bets { field: 501, ..Bets::default() }).is_err());
        assert!(t.cost("nobody", &Bets::default()).is_err());
    }
}
