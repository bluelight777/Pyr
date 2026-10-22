//! Multi-table tournaments: people register (the server takes the buy-in), it starts on time
//! with everyone at tables of `seats`, the blinds go up level by level, the broke are out (in
//! order, which decides the places), tables are kept even and broken as the field shrinks, and
//! the last one standing wins. The prize pool is the buy-ins, paid to the top places.
//!
//! Each table is a [`Room`] in tournament mode (it deals, keeps the clock and the shuffle);
//! this moves people between them between hands. Like a room, it has no clock or network of
//! its own: the server passes it the time and delivers what it returns.

use std::collections::{BTreeMap, BTreeSet};

use crate::bots::Level;
use crate::game::Rules;
use crate::net::{BlindLevel, Bomb, BombView, BuyIn, EntrantView, TableId, ToApp, TourneyId, TourneyPhase, TourneyRules, TourneySummary, TourneyView};
use crate::room::{Effect, Look, Output, Random, Rcpt, Room};

/// The most people in one tournament.
pub const MAX_ENTRANTS: usize = 180;
/// How long the final table stays up after the last hand (to see how it ended: an all-in's
/// board comes a street at a time first).
pub const FINAL_MS: u64 = 12_000;
/// The shortest and longest levels.
pub const MIN_LEVEL_MS: u64 = 60_000;
pub const MAX_LEVEL_MS: u64 = 60 * 60_000;

#[derive(Clone, Debug)]
struct Entrant {
    uid: String,
    name: String,
    look: Look,
    bot: Option<Level>,
    /// What they paid (given back if it's called off).
    paid: u64,
    place: Option<usize>,
    prize: u64,
}

pub struct Tourney {
    pub id: TourneyId,
    pub rules: TourneyRules,
    /// Who made it (they may call it off before it starts).
    pub host: String,
    pub host_name: String,
    pub phase: TourneyPhase,
    entrants: Vec<Entrant>,
    pub tables: BTreeMap<TableId, Room>,
    watchers: BTreeSet<String>,
    started: Option<u64>,
    level: usize,
    /// When it finished (the server forgets it a while after).
    pub ended: Option<u64>,
    bot_names: usize,
    /// A Flash Pot: the wheel's multiplier and when it was spun, and whether it's gone off.
    multiplier: Option<u32>,
    spun: Option<u64>,
    exploded: bool,
    /// Over: when the last table closes (it stays a moment, for its final hand to be seen).
    close_at: Option<u64>,
}

impl Tourney {
    pub fn new(id: TourneyId, rules: TourneyRules, host: &str, host_name: &str) -> Result<Tourney, String> {
        check(&rules)?;
        Ok(Tourney {
            id,
            rules,
            host: host.into(),
            host_name: host_name.into(),
            phase: TourneyPhase::Registering,
            entrants: Vec::new(),
            tables: BTreeMap::new(),
            watchers: BTreeSet::new(),
            started: None,
            level: 0,
            ended: None,
            bot_names: 0,
            multiplier: None,
            spun: None,
            exploded: false,
            close_at: None,
        })
    }

    /// Play chips someone has riding on it until it's over (their buy-in), for the server's
    /// books: given back if the server stops before the end.
    pub fn at_stake(&self, uid: &str) -> u64 {
        match self.phase {
            TourneyPhase::Registering | TourneyPhase::Running => self.entrants.iter().filter(|e| e.uid == uid && e.place.is_none()).map(|e| e.paid).sum(),
            _ => 0,
        }
    }

    pub fn prize_pool(&self) -> u64 {
        if self.rules.bomb.is_some() {
            // A Flash Pot's prize is the buy-in times what the wheel picks (the house stakes
            // the bots' seats); before the spin, the least it can be.
            return self.rules.buy_in * self.multiplier.unwrap_or(MULTIPLIERS[0].0) as u64;
        }
        self.entrants.iter().map(|e| e.paid).sum::<u64>() + self.rules.added
    }

    /// What each place wins, first first (a Flash Pot: the winner takes it all).
    fn pay_table(&self) -> Vec<u64> {
        if self.rules.bomb.is_some() { vec![self.prize_pool()] } else { payouts(self.prize_pool(), self.entrants.len()) }
    }

    /// A Flash Pot's wheel and fuse, as they stand.
    pub fn bomb_view(&self) -> Option<BombView> {
        let b = self.rules.bomb?;
        Some(BombView {
            fuse_ms: b.fuse_ms,
            multiplier: self.multiplier,
            spun_ms: self.spun,
            goes_off_ms: self.started.filter(|_| self.phase == TourneyPhase::Running).map(|t| t + b.fuse_ms),
            exploded: self.exploded,
        })
    }

    /// Still playing.
    fn alive(&self) -> impl Iterator<Item = &Entrant> {
        self.entrants.iter().filter(|e| e.place.is_none())
    }

    pub fn is_entrant(&self, uid: &str) -> bool {
        self.entrants.iter().any(|e| e.uid == uid)
    }

    fn late_open(&self) -> bool {
        self.phase == TourneyPhase::Running && self.level < self.rules.late_levels as usize && self.alive().count() > 1
    }

    pub fn can_register(&self) -> bool {
        (self.phase == TourneyPhase::Registering || self.late_open()) && self.entrants.len() < self.rules.max_players
    }

    /// Registers someone who's paid `paid` (the caller gives it back if this fails).
    pub fn register(&mut self, uid: &str, name: &str, look: Look, paid: u64, now: u64, out: &mut Output) -> Result<(), String> {
        if self.is_entrant(uid) {
            return Err("you're already registered".into());
        }
        if !self.can_register() {
            return Err(if self.entrants.len() >= self.rules.max_players { "it's full".into() } else { "registration is closed".into() });
        }
        self.entrants.push(Entrant { uid: uid.into(), name: name.into(), look, bot: None, paid, place: None, prize: 0 });
        self.watchers.insert(uid.into());
        if self.phase == TourneyPhase::Running {
            // Late: straight to the table with the fewest players.
            let e = self.entrants.last().unwrap().clone();
            self.seat_somewhere(&e.uid, &e.name, self.rules.stack, e.look.clone(), None, 0, now, out);
        }
        self.changed(out);
        Ok(())
    }

    /// A bot joins (free: it adds nothing to the prize pool).
    pub fn add_bot(&mut self, level: Level, out: &mut Output) -> Result<(), String> {
        if self.phase != TourneyPhase::Registering || self.entrants.len() >= self.rules.max_players {
            return Err("bots can join only while it's open, and not past its most players".into());
        }
        let names = ["Ace", "Blaze", "Cinder", "Dusk", "Ember", "Flint", "Gale", "Hex", "Ivy", "Jinx", "Koda", "Lux", "Moss", "Nova", "Onyx", "Pike", "Quill", "Rook", "Sage", "Tarn"];
        let k = self.bot_names;
        self.bot_names += 1;
        let name = if k < names.len() { names[k].to_string() } else { format!("{} {}", names[k % names.len()], k / names.len() + 1) };
        let lvl = level.look_level();
        self.entrants.push(Entrant { uid: format!("bot:t{}:{k}", self.id), name, look: Look { level: lvl, ..Look::default() }, bot: Some(level), paid: 0, place: None, prize: 0 });
        self.changed(out);
        Ok(())
    }

    /// Before it starts: unregisters, giving back what they paid.
    pub fn unregister(&mut self, uid: &str, out: &mut Output) -> Result<(), String> {
        if self.phase != TourneyPhase::Registering {
            return Err("it's started: you play until you're out".into());
        }
        let i = self.entrants.iter().position(|e| e.uid == uid).ok_or("you aren't registered")?;
        let e = self.entrants.remove(i);
        if e.paid > 0 {
            out.effects.push(Effect::Credit { uid: e.uid, chips: e.paid });
        }
        self.changed(out);
        Ok(())
    }

    /// Calls it off: everyone's buy-in back.
    pub fn cancel(&mut self, now: u64, out: &mut Output) -> Result<(), String> {
        if self.phase != TourneyPhase::Registering {
            return Err("only before it starts".into());
        }
        for e in &self.entrants {
            if e.paid > 0 {
                out.effects.push(Effect::Credit { uid: e.uid.clone(), chips: e.paid });
            }
        }
        self.phase = TourneyPhase::Cancelled;
        self.ended = Some(now);
        self.changed(out);
        Ok(())
    }

    pub fn watch(&mut self, uid: &str) -> ToApp {
        self.watchers.insert(uid.into());
        ToApp::Tourney { view: self.view(uid) }
    }

    pub fn unwatch(&mut self, uid: &str) {
        if !self.entrants.iter().any(|e| e.uid == uid && e.place.is_none()) {
            self.watchers.remove(uid);
        }
    }

    /// Everyone told about the tournament: its watchers and everyone still in.
    pub fn audience(&self) -> BTreeSet<String> {
        let mut a = self.watchers.clone();
        a.extend(self.alive().filter(|e| e.bot.is_none()).map(|e| e.uid.clone()));
        a
    }

    fn changed(&self, out: &mut Output) {
        for uid in self.audience() {
            out.to(&uid, ToApp::Tourney { view: self.view(&uid) });
        }
    }

    fn table_of(&self, uid: &str) -> Option<TableId> {
        self.tables.iter().find(|(_, r)| r.seat_of(uid).is_some()).map(|(id, _)| *id)
    }

    pub fn summary(&self, uid: &str) -> TourneySummary {
        TourneySummary {
            id: self.id,
            rules: self.rules.clone(),
            phase: self.phase,
            entrants: self.entrants.len(),
            players_left: self.alive().count(),
            prize_pool: self.prize_pool(),
            level: self.level,
            registered: self.is_entrant(uid),
            host: self.host_name.clone(),
            multiplier: self.multiplier,
        }
    }

    pub fn view(&self, uid: &str) -> TourneyView {
        let chips_of = |u: &str| self.tables.values().map(|r| r.chips_of(u)).sum::<u64>();
        let mut entrants: Vec<EntrantView> = self
            .entrants
            .iter()
            .map(|e| EntrantView {
                uid: e.uid.clone(),
                name: e.name.clone(),
                bot: e.bot,
                chips: if e.place.is_some() && e.place != Some(1) { 0 } else if self.phase == TourneyPhase::Registering { self.rules.stack } else { chips_of(&e.uid) },
                place: e.place,
                prize: e.prize,
                table: self.table_of(&e.uid),
            })
            .collect();
        // Standings: those still in by chips, then the rest by place.
        entrants.sort_by(|a, b| match (a.place, b.place) {
            (None, None) => b.chips.cmp(&a.chips),
            (None, Some(_)) => std::cmp::Ordering::Less,
            (Some(_), None) => std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => x.cmp(&y),
        });
        let next_level_ms = match (self.phase, self.started) {
            (TourneyPhase::Running, Some(t)) if self.level + 1 < self.rules.levels.len() => Some(t + (self.level as u64 + 1) * self.rules.level_ms),
            (TourneyPhase::Registering, _) => Some(self.rules.start_ms),
            _ => None,
        };
        TourneyView {
            id: self.id,
            rules: self.rules.clone(),
            phase: self.phase,
            host: self.host_name.clone(),
            entrants,
            level: self.level,
            next_level_ms,
            prize_pool: self.prize_pool(),
            payouts: self.pay_table(),
            my_table: self.table_of(uid),
            tables: self.tables.keys().copied().collect(),
            bomb: self.bomb_view(),
        }
    }

    fn blinds(&self) -> BlindLevel {
        self.rules.levels[self.level.min(self.rules.levels.len() - 1)]
    }

    fn table_rules(&self) -> Rules {
        let b = self.blinds();
        Rules { variant: self.rules.variant, limit: self.rules.limit, small_blind: b.small, big_blind: b.big, ante: b.ante, seats: self.rules.seats }
    }

    fn new_table(&mut self, next_table: &mut TableId) -> TableId {
        let id = *next_table;
        *next_table += 1;
        let n = self.tables.len() + 1;
        let mut room = Room::new(id, &format!("{} · table {n}", short(&self.rules.name)), &self.host, self.table_rules(), BuyIn { min_bb: 1, max_bb: u64::MAX / 4 }).expect("checked rules");
        room.tourney = Some(self.id);
        self.tables.insert(id, room);
        id
    }

    /// Seats someone at the table with the fewest players (with a free seat).
    #[allow(clippy::too_many_arguments)]
    fn seat_somewhere(&mut self, uid: &str, name: &str, chips: u64, look: Look, bot: Option<Level>, misses: u32, _now: u64, out: &mut Output) -> bool {
        let target = self.tables.iter().filter(|(_, r)| !r.free_seats().is_empty()).min_by_key(|(_, r)| r.seated().len()).map(|(id, _)| *id);
        let Some(t) = target else { return false };
        let room = self.tables.get_mut(&t).unwrap();
        // A seat away from the others, more or less at random.
        let free = room.free_seats();
        let seat = free[(uid.bytes().map(|b| b as usize).sum::<usize>()) % free.len()];
        let ok = room.place(uid, name, seat, chips, look, bot, out).is_ok();
        if ok && misses > 0 {
            room.set_misses(seat, misses);
        }
        if ok && bot.is_none() {
            // Their Pyr opens the table they've been sat at.
            out.to(uid, ToApp::Snapshot { table: room.view(Some(uid)) });
        }
        ok
    }

    fn start(&mut self, now: u64, next_table: &mut TableId, rng: &mut dyn Random, out: &mut Output) {
        if self.entrants.len() < self.rules.min_players.max(2) {
            // Not enough: called off, buy-ins back.
            for e in &self.entrants {
                if e.paid > 0 {
                    out.effects.push(Effect::Credit { uid: e.uid.clone(), chips: e.paid });
                }
            }
            self.phase = TourneyPhase::Cancelled;
            self.ended = Some(now);
            self.changed(out);
            return;
        }
        self.phase = TourneyPhase::Running;
        self.started = Some(now);
        self.level = 0;
        let n = self.entrants.len();
        let tables = n.div_ceil(self.rules.seats);
        for _ in 0..tables {
            self.new_table(next_table);
        }
        // Everyone in a random order round the tables.
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = rng.below(i as u32 + 1) as usize;
            order.swap(i, j);
        }
        let ids: Vec<TableId> = self.tables.keys().copied().collect();
        for (k, i) in order.into_iter().enumerate() {
            let e = self.entrants[i].clone();
            let t = ids[k % ids.len()];
            let room = self.tables.get_mut(&t).unwrap();
            let seat = room.free_seats()[0];
            let _ = room.place(&e.uid, &e.name, seat, self.rules.stack, e.look.clone(), e.bot, out);
        }
        if self.rules.bomb.is_some() {
            // The prize wheel spins; the first hand is dealt once it's stopped.
            self.multiplier = Some(spin(rng));
            self.spun = Some(now);
            self.started = Some(now + WHEEL_MS);
            for r in self.tables.values_mut() {
                r.hold = true;
            }
        }
        for (_, room) in self.tables.iter() {
            for (_, uid) in room.seated() {
                if !uid.starts_with("bot:") {
                    out.to(&uid, ToApp::Snapshot { table: room.view(Some(&uid)) });
                }
            }
        }
        self.changed(out);
    }

    /// The clock: starts it, raises the blinds, runs the tables, knocks out the broke, keeps
    /// the tables even, and pays out at the end. `next_table` hands out table ids.
    pub fn tick(&mut self, now: u64, next_table: &mut TableId, rng: &mut dyn Random) -> Output {
        let mut out = Output::default();
        match self.phase {
            TourneyPhase::Registering if self.rules.bomb.is_some() => {
                // A Flash Pot starts the moment it's full; when the wait is over, bots take the
                // empty seats (nobody waits long), unless nobody's left in it.
                if self.entrants.len() < self.rules.max_players && now >= self.rules.start_ms {
                    if self.entrants.iter().all(|e| e.bot.is_some()) {
                        let _ = self.cancel(now, &mut out);
                        return out;
                    }
                    while self.entrants.len() < self.rules.max_players {
                        if self.add_bot(Level::Regular, &mut out).is_err() {
                            break;
                        }
                    }
                }
                if self.entrants.len() >= self.rules.max_players {
                    self.start(now, next_table, rng, &mut out);
                }
                return out;
            }
            TourneyPhase::Registering => {
                if now >= self.rules.start_ms {
                    self.start(now, next_table, rng, &mut out);
                }
                return out;
            }
            TourneyPhase::Running => {}
            TourneyPhase::Finished => {
                // The final table goes once its last hand has been seen.
                if self.close_at.is_some_and(|t| now >= t) {
                    self.close_at = None;
                    for (id, r) in std::mem::take(&mut self.tables) {
                        for uid in r.audience() {
                            out.to(&uid, ToApp::Closed { table: id });
                        }
                    }
                }
                return out;
            }
            _ => return out,
        }
        let mut news = false;
        if let Some(b) = self.rules.bomb {
            let first_hand = self.started.unwrap_or(now);
            if now < first_hand {
                // (The wheel is still spinning.)
                return out;
            }
            for r in self.tables.values_mut() {
                r.hold = false;
            }
            if !self.exploded && now >= first_hand + b.fuse_ms {
                self.exploded = true;
                news = true;
            }
        }
        // The blinds.
        let started = self.started.unwrap_or(now);
        let level = (((now.saturating_sub(started)) / self.rules.level_ms) as usize).min(self.rules.levels.len() - 1);
        if level != self.level {
            self.level = level;
            let b = self.blinds();
            for r in self.tables.values_mut() {
                r.set_blinds(b.small, b.big, b.ante);
            }
            news = true;
        }
        // The tables.
        let mut busted: Vec<(String, u64)> = Vec::new();
        let ids: Vec<TableId> = self.tables.keys().copied().collect();
        for id in ids {
            let r = self.tables.get_mut(&id).unwrap();
            // The bomb's gone off: everyone's all in from the very next hand.
            let mut o = Output::default();
            if self.exploded {
                r.bomb_goes_off(&mut o);
            }
            o.extend(r.tick(now, rng));
            // "Everyone at the table", spelled out (the tables are many; the server sees one
            // tournament).
            let aud = r.audience();
            o.msgs = std::mem::take(&mut o.msgs)
                .into_iter()
                .flat_map(|(to, m)| match to {
                    Rcpt::All => aud.iter().map(|u| (Rcpt::User(u.clone()), m.clone())).collect::<Vec<_>>(),
                    x => vec![(x, m)],
                })
                .collect();
            busted.append(&mut o.busted);
            out.extend(o);
        }
        if !busted.is_empty() {
            self.knock_out(busted, now, &mut out);
            news = true;
        }
        // (A Flash Pot is one table.)
        if self.phase == TourneyPhase::Running && self.rules.bomb.is_none() && self.balance(now, &mut out) {
            news = true;
        }
        if news {
            self.changed(&mut out);
        }
        out
    }

    /// People out of chips: their places (those who started the hand with more finish
    /// higher), and the end when one is left.
    fn knock_out(&mut self, mut busted: Vec<(String, u64)>, now: u64, out: &mut Output) {
        busted.sort_by_key(|(_, started)| *started);
        let n = self.entrants.len();
        let pay = self.pay_table();
        for (uid, _) in busted {
            let left = self.alive().count();
            let Some(e) = self.entrants.iter_mut().find(|e| e.uid == uid && e.place.is_none()) else { continue };
            e.place = Some(left);
            e.prize = pay.get(left - 1).copied().unwrap_or(0);
            if e.bot.is_none() {
                out.effects.push(Effect::Placed { uid: e.uid.clone(), place: left, entrants: n, prize: e.prize, tourney: self.id });
                if e.prize > 0 {
                    out.effects.push(Effect::Credit { uid: e.uid.clone(), chips: e.prize });
                }
            }
        }
        let alive: Vec<String> = self.alive().map(|e| e.uid.clone()).collect();
        if alive.len() <= 1 {
            if let Some(w) = alive.first() {
                let e = self.entrants.iter_mut().find(|e| e.uid == *w).unwrap();
                e.place = Some(1);
                e.prize = pay.first().copied().unwrap_or(0);
                if e.bot.is_none() {
                    out.effects.push(Effect::Placed { uid: e.uid.clone(), place: 1, entrants: n, prize: e.prize, tourney: self.id });
                    if e.prize > 0 {
                        out.effects.push(Effect::Credit { uid: e.uid.clone(), chips: e.prize });
                    }
                }
            }
            // MMR: everyone in it, by where they finished.
            let players = self.entrants.iter().filter_map(|e| Some((crate::rating::id_of(&e.uid, &e.name, e.bot), e.name.clone(), e.bot, e.place?))).collect::<Vec<_>>();
            if players.len() >= 2 {
                out.effects.push(Effect::RatePlaces { players });
            }
            self.phase = TourneyPhase::Finished;
            self.ended = Some(now);
            // The tables go (in a moment: the last hand's cards stay up to be seen), and
            // everyone sees the results.
            self.close_at = Some(now + FINAL_MS);
        }
    }

    /// Keeps the tables even, and breaks one when the rest can take its players. Moves
    /// happen between hands (a table to move from is held until its hand is over). Whether
    /// anything moved.
    fn balance(&mut self, now: u64, out: &mut Output) -> bool {
        let seats = self.rules.seats;
        let players = self.alive().count();
        let needed = players.div_ceil(seats).max(1);
        let counts: Vec<(TableId, usize)> = self.tables.iter().map(|(id, r)| (*id, r.seated().len())).collect();
        if counts.is_empty() {
            return false;
        }
        let mut moved = false;
        if self.tables.len() > needed {
            // Break the smallest table.
            let (t, _) = *counts.iter().min_by_key(|(id, n)| (*n, std::cmp::Reverse(*id))).unwrap();
            let room = self.tables.get_mut(&t).unwrap();
            room.hold = true;
            if room.between_hands() {
                let seated: Vec<usize> = room.seated().into_iter().map(|(s, _)| s).collect();
                let mut taken = Vec::new();
                for s in seated {
                    if let Some(p) = self.tables.get_mut(&t).unwrap().take(s, out) {
                        taken.push(p);
                    }
                }
                let room = self.tables.remove(&t).unwrap();
                for uid in room.audience() {
                    out.to(&uid, ToApp::Closed { table: t });
                }
                for p in taken {
                    if !self.seat_somewhere(&p.uid, &p.name, p.chips, p.look.clone(), p.bot, p.misses, now, out) {
                        // (Can't happen: the others have room by count. Kept safe anyway.)
                        let id = self.new_table(&mut (t + 1_000_000));
                        let r = self.tables.get_mut(&id).unwrap();
                        let seat = r.free_seats()[0];
                        let _ = r.place(&p.uid, &p.name, seat, p.chips, p.look, p.bot, out);
                    }
                }
                moved = true;
            }
            return moved;
        }
        // Even them out: from the biggest to the smallest, one at a time.
        let (big, nb) = *counts.iter().max_by_key(|(_, n)| *n).unwrap();
        let (small, ns) = *counts.iter().min_by_key(|(_, n)| *n).unwrap();
        if nb >= ns + 2 {
            let room = self.tables.get_mut(&big).unwrap();
            room.hold = true;
            if room.between_hands() {
                // Whoever is in the seat after the button's (they'd be big blind soonest
                // anyway) — or the first seated.
                let seated = room.seated();
                let pick = seated.first().map(|(s, _)| *s);
                if let Some(s) = pick
                    && let Some(p) = room.take(s, out)
                {
                    room.hold = false;
                    let r = self.tables.get_mut(&small).unwrap();
                    let free = r.free_seats();
                    if let Some(&seat) = free.first() {
                        let _ = r.place(&p.uid, &p.name, seat, p.chips, p.look.clone(), p.bot, out);
                        if p.misses > 0 {
                            r.set_misses(seat, p.misses);
                        }
                        if p.bot.is_none() {
                            out.to(&p.uid, ToApp::Snapshot { table: r.view(Some(&p.uid)) });
                        }
                    }
                    moved = true;
                }
            }
        } else {
            for r in self.tables.values_mut() {
                r.hold = false;
            }
        }
        moved
    }

    /// The table someone may act at (theirs), or watch.
    pub fn room_mut(&mut self, table: TableId) -> Option<&mut Room> {
        self.tables.get_mut(&table)
    }

    /// Chips still at stake for someone (for "chips at tables": none, tournament chips aren't
    /// play chips).
    pub fn entrants_count(&self) -> usize {
        self.entrants.len()
    }
}

fn short(s: &str) -> String {
    s.chars().take(28).collect()
}

/// Checks a tournament's rules.
/// Pyr Friday Night: a free tournament every Friday at 10 pm (US Eastern time), its prize
/// money a guaranteed `FRIDAY_FLOOR` plus every chip raked at the cash tables that week.
pub const FRIDAY_HOST: &str = "pyr:friday";
pub const FRIDAY_FLOOR: u64 = 100_000;
pub const FRIDAY_STACK: u64 = 10_000;

/// The next Friday Night after `now_ms` (Unix ms): Friday 22:00 in New York, summer time or not.
pub fn next_friday_night(now_ms: u64) -> u64 {
    let day_ms = 86_400_000u64;
    let today = (now_ms / day_ms) as i64;
    (today - 1..today + 9)
        .filter(|&d| (d + 4).rem_euclid(7) == 5) // 1970-01-01 was a Thursday
        .map(|d| {
            let (y, m, dd) = civil(d);
            let offset_h = if eastern_summer(y, m, dd) { 4 } else { 5 };
            d as u64 * day_ms + (22 + offset_h) * 3_600_000
        })
        .find(|&t| t > now_ms)
        .unwrap_or(now_ms + 7 * day_ms)
}

/// Days since 1970-01-01 to (year, month, day).
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + (m <= 2) as i64, m, d)
}

/// Whether New York is on summer time on that day at 10 pm: from the second Sunday of March
/// to the first Sunday of November.
fn eastern_summer(y: i64, m: u32, d: u32) -> bool {
    // The weekday (0 = Sunday) of the first of a month.
    let first = |m: u32| {
        let (yy, mm) = if m < 3 { (y - 1, m + 12) } else { (y, m) };
        let k = yy % 100;
        let j = yy / 100;
        let h = (1 + 13 * (mm as i64 + 1) / 5 + k + k / 4 + j / 4 + 5 * j).rem_euclid(7); // 0 = Saturday
        ((h + 6) % 7) as u32
    };
    let second_sunday_march = 1 + (7 - first(3)) % 7 + 7;
    let first_sunday_nov = 1 + (7 - first(11)) % 7;
    match m {
        4..=10 => true,
        3 => d >= second_sunday_march,
        11 => d < first_sunday_nov,
        _ => false,
    }
}

/// Pyr Friday Night's rules, starting at `start_ms`, with `added` prize money.
pub fn friday_rules(start_ms: u64, added: u64) -> TourneyRules {
    TourneyRules {
        name: "Pyr Friday Night".into(),
        variant: crate::game::Variant::Holdem,
        limit: crate::game::Limit::NoLimit,
        seats: 9,
        stack: FRIDAY_STACK,
        buy_in: 0,
        min_players: 2,
        max_players: MAX_ENTRANTS,
        start_ms,
        level_ms: 10 * 60_000,
        levels: standard_levels(FRIDAY_STACK),
        late_levels: 6,
        added,
        bomb: None,
    }
}

/// Flash Pots: five seats, 500 chips each, the blinds up every minute, and a three-minute
/// fuse; from the hand after it goes off, everyone's all in, every hand. Buy-ins (play chips), and the prize wheel's
/// multipliers with their weights (out of 10,000): about 4.85 buy-ins come back for the five
/// paid in, so the house keeps about 3%. (Three seats until 0.10.0, with 2x the least.)
pub const BOMB_HOST: &str = "pyr:bomb";
pub const BOMB_BUY_INS: [u64; 3] = [100, 1_000, 10_000];
pub const BOMB_SEATS: usize = 5;
pub const BOMB_STACK: u64 = 500;
pub const BOMB_FUSE_MS: u64 = 180_000;
/// How long a Flash Pot waits for people before bots take the empty seats.
pub const BOMB_WAIT_MS: u64 = 30_000;
/// The wheel's spin, and the prize it lands on shown off, before the first hand.
pub const WHEEL_MS: u64 = 9_000;
pub const MULTIPLIERS: [(u32, u32); 7] = [(3, 6783), (5, 2120), (8, 700), (15, 300), (40, 80), (150, 15), (1000, 2)];

/// The prize wheel: a multiplier, by its weight.
pub fn spin(rng: &mut dyn Random) -> u32 {
    let total: u32 = MULTIPLIERS.iter().map(|m| m.1).sum();
    let mut x = rng.below(total);
    for (m, w) in MULTIPLIERS {
        if x < w {
            return m;
        }
        x -= w;
    }
    MULTIPLIERS[0].0
}

/// A Flash Pot's rules, open from `now` (it waits `BOMB_WAIT_MS` for people).
pub fn bomb_rules(buy_in: u64, now: u64) -> TourneyRules {
    let levels = [(10, 20), (15, 30), (20, 40), (30, 60), (50, 100), (75, 150), (100, 200), (150, 300), (200, 400), (300, 600), (500, 1000), (750, 1500)]
        .into_iter()
        .map(|(small, big)| BlindLevel { small, big, ante: 0 })
        .collect();
    TourneyRules {
        name: format!("Flash Pot · {}", group(buy_in)),
        variant: crate::game::Variant::Holdem,
        limit: crate::game::Limit::NoLimit,
        seats: BOMB_SEATS,
        stack: BOMB_STACK,
        buy_in,
        min_players: BOMB_SEATS,
        max_players: BOMB_SEATS,
        start_ms: now + BOMB_WAIT_MS,
        level_ms: 60_000,
        levels,
        late_levels: 0,
        added: 0,
        bomb: Some(Bomb { fuse_ms: BOMB_FUSE_MS }),
    }
}

/// 12345 as "12,345".
fn group(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

pub fn check(r: &TourneyRules) -> Result<(), String> {
    let name = r.name.trim();
    if name.is_empty() || name.chars().count() > 40 {
        return Err("a tournament's name is 1 to 40 characters".into());
    }
    if !(2..=9).contains(&r.seats) {
        return Err("2 to 9 players a table".into());
    }
    if r.min_players < 2 || r.min_players > r.max_players || r.max_players > MAX_ENTRANTS {
        return Err(format!("from 2 to {MAX_ENTRANTS} players, the least no more than the most"));
    }
    if r.levels.is_empty() || r.levels.len() > 60 {
        return Err("1 to 60 levels of blinds".into());
    }
    for l in &r.levels {
        if l.small == 0 || l.big < l.small || l.ante > l.big {
            return Err("each level's blinds: small more than nothing, big at least small, ante no more than big".into());
        }
    }
    if r.stack < r.levels[0].big * 10 {
        return Err("the starting chips have to be at least 10 big blinds".into());
    }
    if !(MIN_LEVEL_MS..=MAX_LEVEL_MS).contains(&r.level_ms) {
        return Err("levels last 1 to 60 minutes".into());
    }
    Ok(())
}

/// A standard structure: the first big blind about 1/75 of the stack, climbing by a third
/// to a half each level, antes (an eighth of the big blind) from the fifth.
pub fn standard_levels(stack: u64) -> Vec<BlindLevel> {
    let base = nice((stack / 75).max(2), true).max(2);
    // Steps in tenths of the first big blind.
    let steps = [10u64, 15, 20, 30, 40, 50, 60, 80, 100, 120, 150, 200, 250, 300, 400, 500, 600, 800, 1000, 1200, 1500, 2000, 2500, 3000, 4000, 5000, 6000, 8000, 10000];
    steps
        .iter()
        .enumerate()
        .map(|(i, k)| {
            let big = (base * k / 10).max(2) / 2 * 2;
            let ante = if i >= 4 { nice(big / 8, false) } else { 0 };
            BlindLevel { small: big / 2, big, ante }
        })
        .collect()
}

/// Numbers that read well on a blinds sheet: 1, 1.5, 2, 2.5, 3, 4, 5, 6, 8 times a power of ten.
fn nice(n: u64, up: bool) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut best = 1;
    let mut p = 1u64;
    loop {
        for m in [10u64, 15, 20, 25, 30, 40, 50, 60, 80] {
            let v = m * p / 10;
            if v == 0 || (m * p) % 10 != 0 {
                continue;
            }
            if v <= n {
                best = v;
            } else {
                return if up && (v - n) < (n - best) { v } else { best };
            }
        }
        p *= 10;
    }
}

/// How many places are paid for `entrants`.
pub fn paid_places(entrants: usize) -> usize {
    match entrants {
        0..=4 => 1,
        5..=7 => 2,
        8..=10 => 3,
        n => (n * 15).div_ceil(100),
    }
}

/// The prize for each paid place (first first), adding up to the pool exactly.
pub fn payouts(pool: u64, entrants: usize) -> Vec<u64> {
    let k = paid_places(entrants).max(1);
    let w: Vec<f64> = (0..k).map(|i| 1.0 / (i as f64 + 1.6).powf(1.1)).collect();
    let sum: f64 = w.iter().sum();
    let mut out: Vec<u64> = w.iter().map(|x| (pool as f64 * x / sum).floor() as u64).collect();
    let given: u64 = out.iter().sum();
    out[0] += pool - given;
    out
}

#[cfg(test)]
mod tests;
