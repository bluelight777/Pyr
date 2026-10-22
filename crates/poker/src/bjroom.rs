//! A blackjack table in Pyr Casino: seven seats for people and bots against one dealer, a round
//! every few seconds (betting, the deal, each seat in turn, the dealer, the results).
//!
//! It's as fair as the game alone, and checked the same way: before each round the house shows
//! its seed's fingerprint; each person's Pyr adds a seed of its own with their bet; the round's
//! cards come from all of them together (see `round_seed`). When it's over the house shows its
//! seed, and everyone's Pyr plays the whole round again (`bjtable::replay`) to be sure every card
//! came out as shown. Bots play basic strategy and bet with the house's chips: they're company,
//! and nobody's chips go to or from them.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

use crate::bots::persona::{self, BjBets, Persona};
use crate::casino::bjtable::{self, Round, SEATS};
use crate::casino::blackjack::{Action, Phase};
use crate::fair::{self, Stream};
use crate::net::{BjReveal, BjSeatView, BjStage, BjSummary, BjView, ToApp};
use crate::room::{Effect, Look, Output, Random};

/// How long betting stays open, and how soon the deal comes once everyone's bet.
pub const BET_MS: u64 = 12_000;
pub const QUICK_MS: u64 = 1_200;
/// A person's time to decide (then they stand, or decline insurance).
pub const TURN_MS: u64 = 20_000;
/// The round's over: the dealer's hand turns up and the results stay on the table this long.
pub const SETTLE_MS: u64 = 6_500;
/// Rounds a person can sit out before they're stood up (their seat's for someone playing).
pub const IDLE_ROUNDS: u32 = 5;

struct Seat {
    uid: String,
    name: String,
    bot: bool,
    look: Look,
    /// The bet placed for the next round (taken from their chips already), and whether they've
    /// bet or sat this one out; their Pyr's seed for it.
    bet: u64,
    decided: bool,
    seed: Option<[u8; 32]>,
    /// Rounds in a row without a bet.
    idle: u32,
    /// A bot's bet comes in at this time.
    bot_bet_at: u64,
    /// A bot: who it is (how it bets and plays, and its pace: see `bots::persona`), and the
    /// units it's betting (pressing after a win).
    who: &'static Persona,
    units: u64,
}

enum Stage {
    Waiting,
    Betting { until: u64 },
    /// `plan`: a bot's move, chosen as its turn came, made at `deadline`; `whim`: bots' choices
    /// and think times this round.
    Playing { round: Box<Round>, deadline: u64, seeds: Vec<(u8, [u8; 32])>, uids: Vec<(u8, String, bool)>, plan: Option<(u8, Action)>, whim: Stream },
    Settled { until: u64 },
}

pub struct BjRoom {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    seats: Vec<Option<Seat>>,
    watchers: BTreeSet<String>,
    stage: Stage,
    round_no: u64,
    house: [u8; 32],
    commit: [u8; 32],
    /// The last round over (shown until the next is dealt), and its seeds for checking.
    shown: Option<bjtable::RoundView>,
    last: Option<BjReveal>,
    /// When a person was last here (an empty table closes after a while).
    pub seen_ms: u64,
}

/// A round's seed: the house's, and every person's seat seed, and the round's number.
pub fn round_seed(house: &[u8; 32], seeds: &[(u8, [u8; 32])], round: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr blackjack table v1");
    h.update(house);
    let mut seeds = seeds.to_vec();
    seeds.sort_by_key(|s| s.0);
    for (seat, s) in &seeds {
        h.update([*seat]);
        h.update(s);
    }
    h.update(round.to_le_bytes());
    h.finalize().into()
}

fn hex(b: &[u8; 32]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Checks a finished round: the house's seed matches its fingerprint, and the round played
/// again from everyone's seeds, bets and moves comes out exactly as the table showed it.
pub fn check(r: &BjReveal) -> Result<(), String> {
    checked(r).map(|_| ())
}

/// `check`, and the round as it played again (each seat's hands and what came back).
pub fn checked(r: &BjReveal) -> Result<bjtable::Outcome, String> {
    if fair::commit(&r.house) != r.commit {
        return Err(format!("round {}: the house's seed doesn't match the fingerprint it showed first", r.round));
    }
    let seeds: Vec<(u8, [u8; 32])> = r.seeds.iter().filter_map(|(s, h)| crate::net::hex32::parse(h).map(|b| (*s, b))).collect();
    if seeds.len() != r.seeds.len() {
        return Err(format!("round {}: a player's seed isn't a seed", r.round));
    }
    let o = bjtable::replay(&r.bets, Stream::new(round_seed(&r.house, &seeds, r.round)), &r.actions)?;
    if o.dealer != r.dealer || o.seats != r.seats {
        return Err(format!("round {}: played again, the cards came out differently", r.round));
    }
    Ok(o)
}

/// A seed as a round's reveal shows it.
pub fn seed_hex(b: &[u8; 32]) -> String {
    hex(b)
}

/// How long a bot takes over a move at blackjack, the way a person would: standing on 19 or
/// hitting a 9 in a moment, a think over a 16 against a 10, counting out the chips for a double
/// or a split, a look at the cards first on its first move. At its own pace, never the same
/// twice, with the odd pause.
fn bj_think_ms(r: &bjtable::RoundView, a: Action, who: &Persona, first: bool, rng: &mut Stream) -> u64 {
    let roll = |rng: &mut Stream| (rng.below(1_000_000) as f64 + 0.5) / 1_000_000.0;
    let hand = r.seats.iter().find(|s| Some(s.seat) == r.to_act).and_then(|s| s.hands.get(s.active));
    let (v, soft) = hand.map(|h| crate::casino::blackjack::value(&h.cards)).unwrap_or((0, false));
    let up = r.dealer.first().map(|c| crate::casino::blackjack::card_value(*c)).unwrap_or(10);
    // How hard a choice it is.
    let base = match a {
        Action::Insure | Action::NoInsurance => 1_300.0,
        Action::Double => 1_900.0,
        Action::Split => 2_200.0,
        // Stiff hands against a strong card, and soft 17s and 18s, take some thought.
        _ if !soft && (12..=16).contains(&v) && up >= 7 => 1_700.0,
        _ if soft && (17..=18).contains(&v) => 1_500.0,
        _ if v >= 19 || v <= 11 => 650.0,
        _ => 1_050.0,
    };
    let (u1, u2) = (roll(rng), roll(rng));
    let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
    let mut ms = base * who.pace * (0.32 * z).exp() + if first { 400.0 + roll(rng) * 600.0 } else { 0.0 };
    let r = roll(rng);
    if r < who.snaps * 0.8 {
        ms = 420.0 + roll(rng) * 380.0;
    } else if r > 1.0 - who.tanks * 0.7 {
        ms += 1_500.0 + roll(rng) * 3_000.0;
    }
    (ms as u64).clamp(400, 9_000)
}

const BOT_NAMES: [&str; 14] = ["Ace", "Blaze", "Cinder", "Dusk", "Ember", "Flint", "Gale", "Ivy", "Lux", "Moss", "Nova", "Sage", "Tarn", "Wren"];

impl BjRoom {
    pub fn new(id: u64, name: &str, min_bet: u64, max_bet: u64, now: u64, rng: &mut dyn Random) -> BjRoom {
        let house = rng.seed();
        BjRoom {
            id,
            name: name.chars().take(40).collect(),
            min_bet: min_bet.max(1),
            max_bet: max_bet.max(min_bet.max(1)),
            seats: (0..SEATS).map(|_| None).collect(),
            watchers: BTreeSet::new(),
            stage: Stage::Waiting,
            round_no: 1,
            commit: fair::commit(&house),
            house,
            shown: None,
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

    pub fn summary(&self) -> BjSummary {
        BjSummary { id: self.id, name: self.name.clone(), min_bet: self.min_bet, max_bet: self.max_bet, people: self.people().count(), bots: self.seats.iter().flatten().filter(|s| s.bot).count() }
    }

    /// Chips a person has on the table now (a bet placed, or in the round): what the server
    /// counts as theirs while they're here.
    pub fn at_stake(&self, uid: &str) -> u64 {
        match &self.stage {
            // (Their hands play on if they've stood up: still theirs until they're paid.)
            Stage::Playing { round, uids, .. } => uids
                .iter()
                .filter(|(_, u, bot)| !bot && u == uid)
                .map(|(seat, ..)| round.seats.iter().find(|s| s.seat == *seat).map(|s| s.wagered()).unwrap_or(0))
                .sum(),
            _ => self.seat_of(uid).and_then(|seat| self.seats[seat as usize].as_ref()).map(|s| s.bet).unwrap_or(0),
        }
    }

    /// Whether anyone (a person) is sitting or watching: a table nobody's at can close.
    pub fn in_use(&self) -> bool {
        !self.watchers.is_empty() || self.people().next().is_some()
    }

    /// Everyone sitting (people) or watching, to stand up or let go when they've gone.
    pub fn knows(&self, uid: &str) -> bool {
        self.watchers.contains(uid) || self.seat_of(uid).is_some()
    }

    pub fn view(&self, uid: Option<&str>, now: u64) -> BjView {
        let (stage, ms_left, round) = match &self.stage {
            Stage::Waiting => (BjStage::Waiting, 0, self.shown.clone()),
            Stage::Betting { until } => (BjStage::Betting, until.saturating_sub(now), self.shown.clone()),
            Stage::Playing { round, deadline, .. } => (BjStage::Playing, deadline.saturating_sub(now), Some(round.view())),
            Stage::Settled { until } => (BjStage::Settled, until.saturating_sub(now), self.shown.clone()),
        };
        BjView {
            id: self.id,
            name: self.name.clone(),
            min_bet: self.min_bet,
            max_bet: self.max_bet,
            seats: self
                .seats
                .iter()
                .map(|s| {
                    s.as_ref().map(|s| BjSeatView { player: s.uid.clone(), name: s.name.clone(), bot: s.bot, level: s.look.level, card_back: s.look.card_back.clone(), frame: s.look.frame.clone(), bet: s.bet, decided: s.decided })
                })
                .collect(),
            stage,
            ms_left,
            round_no: self.round_no,
            commit: self.commit,
            round,
            last: self.last.clone(),
            my_seat: uid.and_then(|u| self.seat_of(u)),
        }
    }

    /// Everyone's told how the table is now.
    fn changed(&self, now: u64, out: &mut Output) {
        for uid in self.audience() {
            let view = self.view(Some(&uid), now);
            out.to(&uid, ToApp::Bj { view });
        }
    }

    pub fn watch(&mut self, uid: &str, now: u64) -> ToApp {
        self.watchers.insert(uid.to_string());
        self.seen_ms = now;
        ToApp::Bj { view: self.view(Some(uid), now) }
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
        *s = Some(Seat { uid: uid.into(), name: name.into(), bot: false, look, bet: 0, decided: false, seed: None, idle: 0, bot_bet_at: 0, who: &persona::PLAIN, units: 1 });
        self.watchers.insert(uid.into());
        self.seen_ms = now;
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// Stands up (a bet placed for the next round comes back; hands in play stand, and are paid
    /// as they come out).
    pub fn leave(&mut self, uid: &str, now: u64) -> Output {
        let mut out = Output::default();
        let Some(seat) = self.seat_of(uid) else { return out };
        if !matches!(self.stage, Stage::Playing { .. })
            && let Some(s) = &self.seats[seat as usize]
            && s.bet > 0
        {
            out.effects.push(Effect::Credit { uid: uid.into(), chips: s.bet });
        }
        if let Stage::Playing { round, .. } = &mut self.stage {
            // (Their hands stand: the rest of the table plays on.)
            while round.to_act() == Some(seat) {
                let a = if round.phase == Phase::Insurance { Action::NoInsurance } else { Action::Stand };
                if round.act(seat, a).is_err() {
                    break;
                }
            }
        }
        self.seats[seat as usize] = None;
        self.changed(now, &mut out);
        self.after_move(now, &mut out);
        out
    }

    pub fn add_bot(&mut self, seat: u8, now: u64, rng: &mut dyn Random) -> Result<Output, String> {
        let taken: Vec<String> = self.seats.iter().flatten().map(|s| s.name.clone()).collect();
        let s = self.seats.get_mut(seat as usize).ok_or("there's no such seat")?;
        if s.is_some() {
            return Err("that seat's taken".into());
        }
        let free: Vec<&&str> = BOT_NAMES.iter().filter(|n| !taken.iter().any(|t| t == **n)).collect();
        let name = free.get(rng.below(free.len().max(1) as u32) as usize).map(|n| n.to_string()).unwrap_or_else(|| "Bot".into());
        *s = Some(Seat {
            uid: format!("bot:bj{}:{seat}", self.id),
            name: name.clone(),
            bot: true,
            look: Look { level: 12, ..Look::default() },
            bet: 0,
            decided: false,
            seed: None,
            idle: 0,
            bot_bet_at: now + 1_500 + rng.below(4_000) as u64,
            who: persona::of(&name),
            units: 1,
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
        if matches!(self.stage, Stage::Playing { .. }) {
            return Err("after this round".into());
        }
        *s = None;
        let mut out = Output::default();
        self.changed(now, &mut out);
        Ok(out)
    }

    /// A person's bet for the next round (0: they sit it out), the chips already taken from
    /// them by the server; their Pyr's seed with it.
    pub fn bet(&mut self, uid: &str, chips: u64, seed: [u8; 32], now: u64) -> Result<Output, String> {
        if !matches!(self.stage, Stage::Betting { .. } | Stage::Waiting) {
            return Err("betting's closed: next round".into());
        }
        if chips > 0 && (chips < self.min_bet || chips > self.max_bet) {
            return Err(format!("bets here are {} to {}", self.min_bet, self.max_bet));
        }
        let seat = self.seat_of(uid).ok_or("sit down first")?;
        let s = self.seats[seat as usize].as_mut().unwrap();
        if s.decided {
            return Err("you've bet this round already".into());
        }
        s.bet = chips;
        s.decided = true;
        s.seed = Some(seed);
        if chips > 0 {
            s.idle = 0;
        }
        self.seen_ms = now;
        let mut out = Output::default();
        if matches!(self.stage, Stage::Waiting) {
            self.stage = Stage::Betting { until: now + BET_MS };
        }
        self.everyone_bet(now);
        self.changed(now, &mut out);
        Ok(out)
    }

    /// Once every person's bet (or sat out), the deal comes quickly, and the bots still thinking
    /// it over get their bets in before it (a moment apart).
    fn everyone_bet(&mut self, now: u64) {
        let all = self.people().all(|s| s.decided);
        if let Stage::Betting { until } = &mut self.stage
            && all
        {
            *until = (*until).min(now + QUICK_MS);
            let mut at = now + 250;
            for s in self.seats.iter_mut().flatten().filter(|s| s.bot && !s.decided) {
                s.bot_bet_at = s.bot_bet_at.min(at);
                at += 230;
            }
        }
    }

    /// The chips a move would put down (for the server to take first), if it's this person's
    /// turn and the move's allowed.
    pub fn cost(&self, uid: &str, a: Action) -> Option<u64> {
        let seat = self.seat_of(uid)?;
        let Stage::Playing { round, .. } = &self.stage else { return None };
        (round.to_act() == Some(seat) && round.allowed().contains(&a)).then(|| round.cost(a))
    }

    pub fn act(&mut self, uid: &str, a: Action, now: u64) -> Result<Output, String> {
        let seat = self.seat_of(uid).ok_or("you aren't sitting here")?;
        let Stage::Playing { round, .. } = &mut self.stage else { return Err("there's no round being played".into()) };
        round.act(seat, a)?;
        self.seen_ms = now;
        let mut out = Output::default();
        self.after_move(now, &mut out);
        if !matches!(self.stage, Stage::Settled { .. }) {
            self.changed(now, &mut out);
        }
        Ok(out)
    }

    /// After a move: the next turn's clock, or the round's end.
    fn after_move(&mut self, now: u64, out: &mut Output) {
        let Stage::Playing { round, deadline, plan, whim, .. } = &mut self.stage else { return };
        if round.phase == Phase::Over {
            self.settle(now, out);
            return;
        }
        let Some(to_act) = round.to_act() else { return };
        let seat = self.seats[to_act as usize].as_ref();
        *plan = None;
        match seat {
            Some(s) if !s.bot => *deadline = now + TURN_MS,
            _ => {
                // A bot chooses as its turn comes, then takes about as long as a person would
                // over that choice, at its own pace (or, its seat empty, stands at once).
                let who = seat.map(|s| s.who).unwrap_or(&persona::PLAIN);
                let view = round.view();
                let a = bjtable::bot_move_as(&view, who.bj);
                let first = round.actions.iter().all(|(s, _)| *s != to_act);
                *plan = Some((to_act, a));
                *deadline = now + bj_think_ms(&view, a, who, first, whim);
            }
        }
    }

    fn deal(&mut self, now: u64, rng: &mut dyn Random, out: &mut Output) {
        // (A bot still thinking about its bet makes it now: bots play every round.)
        let (lo, hi) = (self.min_bet, self.max_bet);
        for s in self.seats.iter_mut().flatten().filter(|s| s.bot && !s.decided) {
            s.bet = (lo * Self::bot_units(s, rng)).clamp(lo, hi);
            s.decided = true;
        }
        let bets: Vec<(u8, u64)> = self.seats.iter().enumerate().filter_map(|(i, s)| s.as_ref().filter(|s| s.bet > 0).map(|s| (i as u8, s.bet))).collect();
        // Nobody in: on to the next round's betting (people who didn't bet count a round idle).
        for s in self.seats.iter_mut().flatten() {
            if !s.bot && s.bet == 0 {
                s.idle += 1;
            }
        }
        let idle: Vec<String> = self.seats.iter().flatten().filter(|s| !s.bot && s.idle >= IDLE_ROUNDS).map(|s| s.uid.clone()).collect();
        for uid in idle {
            out.extend(self.leave(&uid, now));
        }
        if !bets.iter().any(|(seat, _)| self.seats[*seat as usize].as_ref().is_some_and(|s| !s.bot)) {
            // (Bots don't play alone: their bets go back to the house.)
            self.reset_bets();
            self.stage = if self.people().count() > 0 { Stage::Betting { until: now + BET_MS } } else { Stage::Waiting };
            self.changed(now, out);
            return;
        }
        let seeds: Vec<(u8, [u8; 32])> = self.seats.iter().enumerate().filter_map(|(i, s)| s.as_ref().filter(|s| s.bet > 0 && !s.bot).and_then(|s| s.seed).map(|seed| (i as u8, seed))).collect();
        let uids: Vec<(u8, String, bool)> = bets.iter().map(|(seat, _)| (*seat, self.seats[*seat as usize].as_ref().map(|s| s.uid.clone()).unwrap_or_default(), self.seats[*seat as usize].as_ref().is_some_and(|s| s.bot))).collect();
        let round = Round::deal(&bets, Stream::new(round_seed(&self.house, &seeds, self.round_no)));
        let whim = Stream::new(rng.seed());
        self.stage = Stage::Playing { round: Box::new(round), deadline: now, seeds, uids, plan: None, whim };
        self.after_move(now, out);
        if matches!(self.stage, Stage::Playing { .. }) {
            self.changed(now, out);
        }
    }

    /// A bot's bet, in units of the least bet: its own way (the least, a few units, pressing
    /// after a win, or big).
    fn bot_units(s: &Seat, rng: &mut dyn Random) -> u64 {
        match s.who.bj_bets {
            BjBets::Minimum => 1,
            BjBets::Steady => [1u64, 1, 2, 2, 3][rng.below(5) as usize],
            BjBets::Press => s.units,
            BjBets::Big => [5u64, 10, 10, 20, 25, 50][rng.below(6) as usize],
        }
    }

    fn reset_bets(&mut self) {
        for s in self.seats.iter_mut().flatten() {
            s.bet = 0;
            s.decided = false;
            s.seed = None;
        }
    }

    /// The round's over: everyone paid, their records told, the seeds shown, and the next
    /// round's fingerprint.
    fn settle(&mut self, now: u64, out: &mut Output) {
        let Stage::Playing { round, seeds, uids, .. } = std::mem::replace(&mut self.stage, Stage::Settled { until: now + SETTLE_MS }) else { return };
        let o = round.outcome();
        for (seat, uid, bot) in &uids {
            if *bot {
                // (A bot that presses its bets: up after a win, back to one after a loss.)
                if let Some(s) = self.seats[*seat as usize].as_mut().filter(|s| s.bot) {
                    let base = o.bets.iter().find(|b| b.0 == *seat).map(|b| b.1).unwrap_or(0);
                    s.units = if o.paid(*seat) > base { (s.units * 2).min(8) } else { 1 };
                }
                continue;
            }
            let paid = o.paid(*seat);
            if paid > 0 {
                out.effects.push(Effect::Credit { uid: uid.clone(), chips: paid });
            }
            if let Some(mine) = o.for_seat(*seat) {
                out.effects.push(Effect::BjRound { uid: uid.clone(), outcome: mine });
            }
        }
        self.shown = Some(round.view());
        self.last = Some(BjReveal {
            round: self.round_no,
            commit: self.commit,
            house: self.house,
            seeds: seeds.iter().map(|(s, b)| (*s, hex(b))).collect(),
            bets: o.bets.clone(),
            actions: o.actions.clone(),
            dealer: o.dealer.clone(),
            seats: o.seats.clone(),
        });
        self.reset_bets();
        self.changed(now, out);
    }

    /// The next round's house seed (its fingerprint shown before anyone bets).
    fn next_round(&mut self, rng: &mut dyn Random) {
        self.round_no += 1;
        self.house = rng.seed();
        self.commit = fair::commit(&self.house);
    }

    pub fn tick(&mut self, now: u64, rng: &mut dyn Random) -> Output {
        let mut out = Output::default();
        match &mut self.stage {
            Stage::Waiting => {
                if self.people().count() > 0 {
                    self.stage = Stage::Betting { until: now + BET_MS };
                    self.changed(now, &mut out);
                }
            }
            Stage::Betting { until } => {
                let until = *until;
                // Bots bet when they're ready, each its own way: the least, a few units, pressing
                // after a win, or big.
                let (lo, hi) = (self.min_bet, self.max_bet);
                let mut bot_bet = false;
                for s in self.seats.iter_mut().flatten().filter(|s| s.bot && !s.decided) {
                    if now >= s.bot_bet_at {
                        s.bet = (lo * Self::bot_units(s, rng)).clamp(lo, hi);
                        s.decided = true;
                        bot_bet = true;
                    }
                }
                if bot_bet {
                    self.everyone_bet(now);
                    self.changed(now, &mut out);
                }
                if self.people().count() == 0 {
                    self.reset_bets();
                    self.stage = Stage::Waiting;
                    self.changed(now, &mut out);
                } else if now >= until {
                    self.deal(now, rng, &mut out);
                }
            }
            Stage::Playing { round, deadline, plan, .. } => {
                if now < *deadline {
                    return out;
                }
                let Some(seat) = round.to_act() else {
                    self.after_move(now, &mut out);
                    return out;
                };
                let bot = self.seats[seat as usize].as_ref().is_none_or(|s| s.bot);
                // A bot's move (chosen as its turn came), or a person's time running out (they
                // stand).
                let play = self.seats[seat as usize].as_ref().map(|s| s.who.bj).unwrap_or(persona::BjPlay::Book);
                let planned = plan.take().filter(|(s, _)| *s == seat).map(|(_, a)| a);
                let a = if bot {
                    planned.unwrap_or_else(|| bjtable::bot_move_as(&round.view(), play))
                } else if round.phase == Phase::Insurance {
                    Action::NoInsurance
                } else {
                    Action::Stand
                };
                if round.act(seat, a).is_err() {
                    let _ = round.act(seat, Action::Stand);
                }
                self.after_move(now, &mut out);
                if !matches!(self.stage, Stage::Settled { .. }) {
                    self.changed(now, &mut out);
                }
            }
            Stage::Settled { until } => {
                if now >= *until {
                    self.next_round(rng);
                    for s in self.seats.iter_mut().flatten().filter(|s| s.bot) {
                        // (Each at its own pace: some bet at once, some think it over.)
                        s.bot_bet_at = now + ((900 + rng.below(4_200)) as f64 * s.who.pace) as u64;
                    }
                    self.stage = if self.people().count() > 0 { Stage::Betting { until: now + BET_MS } } else { Stage::Waiting };
                    self.changed(now, &mut out);
                }
            }
        }
        out
    }

    /// Everyone's chips on the table back (it's closing, or the server's stopping).
    pub fn close(&mut self) -> Output {
        let mut out = Output::default();
        match &self.stage {
            Stage::Playing { round, uids, .. } => {
                for (seat, uid, bot) in uids {
                    if !bot && let Some(sp) = round.seats.iter().find(|s| s.seat == *seat) {
                        out.effects.push(Effect::Credit { uid: uid.clone(), chips: sp.wagered() });
                    }
                }
            }
            _ => {
                for s in self.seats.iter().flatten().filter(|s| !s.bot && s.bet > 0) {
                    out.effects.push(Effect::Credit { uid: s.uid.clone(), chips: s.bet });
                }
            }
        }
        self.reset_bets();
        out
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
            for b in &mut s {
                *b = self.0.below(256) as u8;
            }
            s
        }
        fn below(&mut self, n: u32) -> u32 {
            self.0.below(n)
        }
    }

    /// People and bots play rounds; every round checks out from its seeds; chips taken for
    /// bets and moves all come back as payouts or losses (nothing made, nothing lost).
    #[test]
    fn a_table_plays_rounds_that_check_out() {
        let mut rng = Rng(Stream::new(seed_from(5)));
        let mut t = BjRoom::new(1, "Test", 10, 500, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 2, 0).unwrap();
        t.sit("bob", "Bob", Look::default(), 5, 0).unwrap();
        t.add_bot(0, 0, &mut rng).unwrap();
        t.add_bot(6, 0, &mut rng).unwrap();
        let (mut taken, mut back) = (0u64, 0u64);
        let mut checked = 0;
        let mut seen_round = 0;
        let mut now = 0u64;
        while now < 600_000 {
            let out = t.tick(now, &mut rng);
            for e in &out.effects {
                if let Effect::Credit { chips, .. } = e {
                    back += chips;
                }
            }
            // People bet when betting's open, and play basic strategy (paying for doubles and
            // splits and insurance as the server would).
            if matches!(t.stage, Stage::Betting { .. }) {
                for (uid, chips) in [("alice", 50u64), ("bob", 20)] {
                    let seat = t.seat_of(uid).unwrap() as usize;
                    if !t.seats[seat].as_ref().unwrap().decided {
                        taken += chips;
                        t.bet(uid, chips, rng.seed(), now).unwrap();
                    }
                }
            }
            if let Stage::Playing { round, .. } = &t.stage
                && let Some(seat) = round.to_act()
                && let Some(s) = t.seats[seat as usize].as_ref().filter(|s| !s.bot)
            {
                let uid = s.uid.clone();
                let a = bjtable::bot_move(&round.view());
                taken += t.cost(&uid, a).unwrap();
                let out = t.act(&uid, a, now).unwrap();
                for e in &out.effects {
                    if let Effect::Credit { chips, .. } = e {
                        back += chips;
                    }
                }
            }
            if let Some(r) = &t.last
                && r.round != seen_round
            {
                seen_round = r.round;
                check(r).unwrap();
                // (The bots play every round: their bets are in before the deal.)
                assert!(r.bets.iter().any(|b| b.0 == 0) && r.bets.iter().any(|b| b.0 == 6), "round {}: {:?}", r.round, r.bets);
                checked += 1;
            }
            now += 250;
        }
        assert!(checked > 20, "{checked} rounds");
        // Basic strategy loses a little, never wildly (chips in the open round aside).
        let open: u64 = ["alice", "bob"].iter().map(|u| t.at_stake(u)).sum();
        let rtp = (back + open) as f64 / taken as f64;
        assert!((0.8..1.2).contains(&rtp), "{rtp} over {checked} rounds");
    }

    /// A round that's been tampered with doesn't check out.
    #[test]
    fn a_wrong_card_is_caught() {
        let mut rng = Rng(Stream::new(seed_from(9)));
        let mut t = BjRoom::new(1, "Test", 10, 500, 0, &mut rng);
        t.sit("alice", "Alice", Look::default(), 3, 0).unwrap();
        let mut now = 0;
        while t.last.is_none() && now < 120_000 {
            t.tick(now, &mut rng);
            if matches!(t.stage, Stage::Betting { .. }) && !t.seats[3].as_ref().unwrap().decided {
                t.bet("alice", 10, rng.seed(), now).unwrap();
            }
            now += 250;
        }
        let mut r = t.last.clone().unwrap();
        check(&r).unwrap();
        r.dealer[0] = if r.dealer[0].index() == 0 { crate::cards::Card::from_index(1) } else { crate::cards::Card::from_index(0) };
        assert!(check(&r).is_err());
        let mut r = t.last.clone().unwrap();
        r.house[0] ^= 1;
        assert!(check(&r).is_err());
    }
}
