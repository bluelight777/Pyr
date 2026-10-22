//! Poker tables on this server (see `pyr_poker`): the rooms, the clock that runs them, each
//! player's record (chips, experience, achievements) in the database, and telling everyone
//! what they may see.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use parking_lot::Mutex;
use pyr_poker::net::{GainsView, LeaderRow, PrizeView, RecordView, Role, StaffLog, StaffMember, StaffView, TableId, ToApp, ToServer, TourneyId};
use pyr_poker::progress::{self, COSMETICS, Kind, Record};
use pyr_poker::room::{Effect, Look, Output, Random, Rcpt, Room};
use pyr_poker::net::TourneyPhase;
use pyr_poker::tourney::{self as tourney, Tourney};
use tracing::info;

use crate::hub::Hub;

mod bac_tables;
mod rl_tables;
mod bj;
mod bonfire;
mod casino;
mod chat;
mod gm;
mod craps_tables;
mod emotes;
mod ratings;

/// The most tables a server keeps, and that one person can have made at once.
pub const MAX_TABLES: usize = 40;
pub const MAX_TABLES_EACH: usize = 3;
/// A table nobody (no person) has sat at or watched for this long is closed.
pub const EMPTY_CLOSE_MS: u64 = 30 * 60 * 1000;

/// The system's random numbers.
struct SysRandom;

impl Random for SysRandom {
    fn seed(&mut self) -> [u8; 32] {
        let mut s = [0u8; 32];
        getrandom::getrandom(&mut s).expect("the system's random numbers");
        s
    }

    fn below(&mut self, n: u32) -> u32 {
        let zone = u32::MAX - (u32::MAX % n.max(1));
        loop {
            let mut b = [0u8; 4];
            getrandom::getrandom(&mut b).expect("the system's random numbers");
            let v = u32::from_le_bytes(b);
            if v < zone {
                return v % n.max(1);
            }
        }
    }
}

/// The most tournaments kept at once (running, waiting or just over).
pub const MAX_TOURNEYS: usize = 20;
/// The database's notes: the Friday Night pool (chips), and the start of the last one begun.
const FRIDAY_POOL: &str = "poker_friday_pool";
const FRIDAY_STARTED: &str = "poker_friday_started";
/// Staff: everyone's role, what each gave away today, the week's rake by day, and the log.
const ROLES: &str = "poker_roles";
const STAFF_LOG: &str = "poker_staff_log";
const STAFF_LOG_KEEP: usize = 2_000;
/// How much of it the owner's sent at once.
const STAFF_LOG_SHOWN: usize = 300;
/// The least a Game Master / Ambassador may give a day, however little has been raked.
const GM_FLOOR: u64 = 25_000;
const AMBASSADOR_FLOOR: u64 = 5_000;
/// A finished tournament's results are kept this long.
pub const TOURNEY_KEEP_MS: u64 = 2 * 60 * 60 * 1000;
/// Flash Pots: how many at once (they don't count toward `MAX_TOURNEYS`), and how long a
/// finished one stays to be seen.
pub const MAX_BOMBS: usize = 40;
pub const BOMB_KEEP_MS: u64 = 10 * 60 * 1000;

#[derive(Default)]
pub struct State {
    pub rooms: BTreeMap<TableId, Room>,
    pub tourneys: BTreeMap<TourneyId, Tourney>,
    pub casino: casino::CasinoState,
    pub bj: bj::BjState,
    pub craps: craps_tables::CrapsState,
    pub bac: bac_tables::BacState,
    pub rl: rl_tables::RlState,
    pub bonfire: bonfire::BonfireState,
    pub emotes: emotes::EmoteState,
    pub chat: chat::ChatState,
}

impl State {
    /// A table: a cash table, or one of a tournament's.
    fn room_mut(&mut self, id: TableId) -> Option<&mut Room> {
        if self.rooms.contains_key(&id) {
            return self.rooms.get_mut(&id);
        }
        self.tourneys.values_mut().find_map(|t| t.room_mut(id))
    }

    fn room(&self, id: TableId) -> Option<&Room> {
        self.rooms.get(&id).or_else(|| self.tourneys.values().find_map(|t| t.tables.get(&id)))
    }

    /// Play chips a person has riding: at cash tables, buy-ins in tournaments not over, and on
    /// blackjack tables.
    fn held(&self, uid: &str) -> u64 {
        self.rooms.values().map(|r| r.chips_of(uid)).sum::<u64>() + self.tourneys.values().map(|t| t.at_stake(uid)).sum::<u64>() + self.bj.at_stake(uid) + self.craps.at_stake(uid) + self.bac.at_stake(uid) + self.rl.at_stake(uid)
    }
}

pub struct Poker {
    state: Mutex<State>,
    next_tourney: AtomicU64,
    next_id: AtomicU64,
    start: Instant,
    /// When each table last had a person sitting at it.
    last_people: Mutex<HashMap<TableId, u64>>,
    /// Game Masters (and the game's owner) by their home server's certificates, shown here:
    /// whether they're its owner, until when (wall ms).
    gm_proofs: Mutex<HashMap<String, (bool, Option<Role>, u64)>>,
    /// (Tests: the home server's fingerprint in place of `HOME_SERVER`.)
    pub(crate) home_for_tests: std::sync::OnceLock<[u8; 32]>,
}

impl Default for Poker {
    fn default() -> Self {
        Poker { state: Mutex::new(State::default()), next_tourney: AtomicU64::new(1), next_id: AtomicU64::new(1), start: Instant::now(), last_people: Mutex::new(HashMap::new()), gm_proofs: Mutex::new(HashMap::new()), home_for_tests: std::sync::OnceLock::new() }
    }
}

/// 12,345 chips, written as people write it.
fn progress_chips(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn today() -> u64 {
    (pyr_proto::now_ms().max(0) as u64) / 86_400_000
}

impl Poker {
    fn now(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    /// The time on the wall (Unix ms), for tournaments' start times.
    fn wall(&self) -> u64 {
        pyr_proto::now_ms().max(0) as u64
    }

    fn name_of(hub: &Hub, uid: &str) -> String {
        hub.db.get_user(uid).map(|u| u.name).unwrap_or_else(|| "Someone".into())
    }

    fn view(hub: &Hub, uid: &str, rec: &Record, at_tables: u64) -> RecordView {
        let level = rec.level();
        RecordView {
            uid: uid.into(),
            name: Self::name_of(hub, uid),
            chips: rec.chips,
            at_tables,
            xp: rec.xp,
            level,
            level_from: progress::xp_for_level(level),
            level_to: progress::xp_for_level(level + 1),
            hands: rec.hands,
            hands_won: rec.hands_won,
            showdowns: rec.showdowns,
            showdowns_won: rec.showdowns_won,
            vpip: rec.vpip,
            pfr: rec.pfr,
            biggest_pot: rec.biggest_pot,
            best_hand: rec.best_hand_name.clone(),
            achievements: rec.achievements.iter().cloned().collect(),
            daily_ready: !(rec.daily_day == today() && rec.daily_streak > 0),
            daily_streak: rec.daily_streak,
            card_back: rec.card_back.clone(),
            felt: rec.felt.clone(),
            chip_set: rec.chip_set.clone(),
            frame: rec.frame.clone(),
            unlocked: rec.unlocked().iter().map(|c| c.id.to_string()).collect(),
            role: Self::role(hub, uid),
            mmr: rec.rating.mmr.round() as u32,
            mmr_peak: rec.rating.peak.round() as u32,
            mmr_placing: rec.rating.placing(),
        }
    }

    fn look(rec: &Record) -> Look {
        Look { level: rec.level(), card_back: rec.card_back.clone(), frame: rec.frame.clone() }
    }

    fn me(&self, hub: &Hub, st: &State, uid: &str) -> RecordView {
        let (rec, _) = hub.db.poker_record(uid);
        Self::view(hub, uid, &rec, st.held(uid))
    }

    /// Everyone with a role.
    fn roles(hub: &Hub) -> BTreeMap<String, Role> {
        hub.db.get_meta(ROLES).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    /// Someone's role here. Game Masters are the home server's (see `gm`): named there, and
    /// anywhere else only by its certificate; Ambassadors are each server's own.
    pub fn role(hub: &Hub, uid: &str) -> Option<Role> {
        if Self::is_home(hub) {
            return Self::roles(hub).get(uid).copied();
        }
        // (Elsewhere, only by the home's certificate: the game's owner counts as a Game Master.)
        Self::proof(hub, uid).map(|(_, role)| role.unwrap_or(Role::GameMaster))
    }

    /// The home server's fingerprint (Brethren of Blood's, or a test's).
    fn home(hub: &Hub) -> Option<[u8; 32]> {
        hub.poker.home_for_tests.get().copied().or_else(gm::home)
    }

    /// Whether this is the Game Masters' home server.
    pub fn is_home(hub: &Hub) -> bool {
        Self::home(hub) == Some(hub.fingerprint)
    }

    /// Someone's home certificate shown here and still good: whether they're the home's owner.
    /// What it says: whether they're the home's owner, and their role (none for the owner).
    fn proof(hub: &Hub, uid: &str) -> Option<(bool, Option<Role>)> {
        let now = pyr_proto::now_ms().max(0) as u64;
        hub.poker.gm_proofs.lock().get(uid).filter(|p| p.2 > now).map(|p| (p.0, p.1))
    }

    /// The Owner of the Game: the home server's owner (there, or here by their certificate).
    pub fn is_game_owner(hub: &Hub, uid: &str) -> bool {
        if Self::is_home(hub) { Self::is_owner(hub, uid) } else { Self::proof(hub, uid).is_some_and(|p| p.0) }
    }

    /// Who may light the Bonfire early: the game's owner and Game Masters (never just a
    /// server's owner).
    pub fn can_light(hub: &Hub, uid: &str) -> bool {
        Self::is_game_owner(hub, uid) || Self::role(hub, uid) == Some(Role::GameMaster)
    }

    /// The server's owner: the only one who names Game Masters and Ambassadors (not admins,
    /// and not Game Masters, whatever else they may do).
    pub fn is_owner(hub: &Hub, uid: &str) -> bool {
        hub.db.get_user(uid).is_some_and(|u| u.role == pyr_proto::control::Role::Owner)
    }

    /// Chips raked over the last 7 days.
    fn rake_week(hub: &Hub) -> u64 {
        let d = today();
        (d.saturating_sub(6)..=d).map(|d| hub.db.get_meta(&format!("poker_rake:{d}")).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0)).sum()
    }

    /// What a role may give away a day: a Game Master, a day's rake (the week's average); an
    /// Ambassador, a quarter of that; never less than their floor.
    fn allowance(hub: &Hub, role: Role) -> u64 {
        let day = Self::rake_week(hub) / 7;
        match role {
            Role::GameMaster => day.max(GM_FLOOR),
            Role::Ambassador => (day / 4).max(AMBASSADOR_FLOOR),
        }
    }

    fn spent_today(hub: &Hub, uid: &str) -> u64 {
        hub.db.get_meta(&format!("poker_spent:{}:{uid}", today())).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// Takes `chips` from `uid`'s allowance today (refused when it's not there).
    fn spend(hub: &Hub, uid: &str, role: Role, chips: u64) -> Result<(), String> {
        let spent = Self::spent_today(hub, uid);
        let left = Self::allowance(hub, role).saturating_sub(spent);
        if chips > left {
            return Err(format!("that's more than you have left to give today ({})", progress_chips(left)));
        }
        hub.db.set_meta(&format!("poker_spent:{}:{uid}", today()), &(spent + chips).to_string()).map_err(|e| e.to_string())
    }

    fn log_staff(&self, hub: &Hub, by: &str, what: String) {
        let mut log: Vec<StaffLog> = hub.db.get_meta(STAFF_LOG).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default();
        info!("poker staff: {by}: {what}");
        log.insert(0, StaffLog { at_ms: self.wall(), by: Self::name_of(hub, by), what });
        log.truncate(STAFF_LOG_KEEP);
        if let Ok(j) = serde_json::to_string(&log) {
            let _ = hub.db.set_meta(STAFF_LOG, &j);
        }
    }

    fn staff_view(hub: &Hub, uid: &str, _can_manage: bool) -> StaffView {
        let me = Self::role(hub, uid);
        let allowance = me.map(|r| Self::allowance(hub, r)).unwrap_or(0);
        // ("owner": the Owner of the Game, not a server's owner.)
        let owner = Self::is_game_owner(hub, uid);
        StaffView {
            me,
            owner,
            allowance,
            left: allowance.saturating_sub(Self::spent_today(hub, uid)),
            rake_week: Self::rake_week(hub),
            staff: if Self::is_home(hub) { Self::roles(hub).into_iter().map(|(u, role)| StaffMember { name: Self::name_of(hub, &u), uid: u, role }).collect() } else { Vec::new() },
            // (The newest few hundred: all of it's kept, and in the server's own log.)
            log: if owner { hub.db.get_meta(STAFF_LOG).and_then(|v| serde_json::from_str::<Vec<StaffLog>>(&v).ok()).unwrap_or_default().into_iter().take(STAFF_LOG_SHOWN).collect() } else { vec![] },
        }
    }

    /// A gift's or prize box's look, checked: one of Pyr Poker's.
    fn item_name(item: &Option<String>) -> Result<Option<&'static str>, String> {
        match item {
            None => Ok(None),
            Some(id) => progress::cosmetic(id).map(|c| Some(c.name)).ok_or_else(|| "there's no such look".to_string()),
        }
    }

    /// Chips into the Friday Night pool (kept in the database, so a restart keeps them), counted
    /// as the day's rake too (staff allowances follow it).
    pub(crate) fn add_rake(hub: &Hub, chips: u64) {
        if chips == 0 {
            return;
        }
        let pool = Self::friday_pool(hub) + chips;
        let _ = hub.db.set_meta(FRIDAY_POOL, &pool.to_string());
        let key = format!("poker_rake:{}", today());
        let day = hub.db.get_meta(&key).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0) + chips;
        let _ = hub.db.set_meta(&key, &day.to_string());
    }

    /// The chips raked at the cash tables since the last Friday Night started.
    pub fn friday_pool(hub: &Hub) -> u64 {
        hub.db.get_meta(FRIDAY_POOL).and_then(|v| v.parse().ok()).unwrap_or(0)
    }

    /// Pyr Friday Night: there's always the next one on the list (its prize money growing with
    /// the week's rake until it starts, when the pool starts over for the next week).
    fn friday_night(&self, hub: &Hub, st: &mut State, wall: u64) {
        let pool = Self::friday_pool(hub);
        let mut have = false;
        for t in st.tourneys.values_mut().filter(|t| t.host == tourney::FRIDAY_HOST) {
            match t.phase {
                TourneyPhase::Registering => {
                    t.rules.added = tourney::FRIDAY_FLOOR + pool;
                    have = true;
                }
                TourneyPhase::Running => {
                    have = true;
                    let started = t.rules.start_ms.to_string();
                    if hub.db.get_meta(FRIDAY_STARTED).as_deref() != Some(&started) {
                        // Its prize money is fixed now: the pool starts over.
                        let _ = hub.db.set_meta(FRIDAY_STARTED, &started);
                        let _ = hub.db.set_meta(FRIDAY_POOL, &pool.saturating_sub(t.rules.added.saturating_sub(tourney::FRIDAY_FLOOR)).to_string());
                        info!(tourney = t.id, prize = t.rules.added, "Pyr Friday Night started");
                    }
                }
                _ => {}
            }
        }
        if have || st.tourneys.values().filter(|t| t.rules.bomb.is_none()).count() >= MAX_TOURNEYS {
            return;
        }
        let start = tourney::next_friday_night(wall);
        let id = self.next_tourney.fetch_add(1, Ordering::Relaxed);
        if let Ok(t) = Tourney::new(id, tourney::friday_rules(start, tourney::FRIDAY_FLOOR + pool), tourney::FRIDAY_HOST, "Pyr") {
            info!(tourney = id, start, "Pyr Friday Night listed");
            st.tourneys.insert(id, t);
        }
    }

    /// Sends a table's news on, and records what hands and leaving meant.
    fn flush(&self, hub: &Hub, st: &State, table: TableId, out: Output) {
        let audience: Vec<String> = st.room(table).map(|r| r.audience().into_iter().collect()).unwrap_or_default();
        for (to, msg) in out.msgs {
            let msg = Self::dress(hub, msg);
            match to {
                Rcpt::All => {
                    for uid in &audience {
                        hub.send_poker(uid, msg.clone());
                    }
                }
                Rcpt::User(uid) => hub.send_poker(&uid, msg),
            }
        }
        for e in out.effects {
            match e {
                Effect::Credit { uid, chips } => {
                    let (mut rec, _) = hub.db.poker_record(&uid);
                    rec.chips += chips;
                    let held = st.held(&uid);
                    let _ = hub.db.set_poker_record(&uid, &rec, held);
                    hub.send_poker(&uid, ToApp::Record { me: Self::view(hub, &uid, &rec, held), gains: None });
                }
                Effect::Hand { uid, result, big_blind, beat_shark } => {
                    let (mut rec, _) = hub.db.poker_record(&uid);
                    let g = progress::apply(&mut rec, &result, big_blind, beat_shark);
                    let held = st.held(&uid);
                    let _ = hub.db.set_poker_record(&uid, &rec, held);
                    let gains = GainsView {
                        xp: g.xp,
                        achievements: g.new_achievements.iter().map(|s| s.to_string()).collect(),
                        level_up: g.level_up,
                        net: result.won as i64 - result.put_in as i64,
                    };
                    hub.send_poker(&uid, ToApp::Record { me: Self::view(hub, &uid, &rec, held), gains: Some(gains) });
                }
                Effect::Raked { chips } => Self::add_rake(hub, chips),
                Effect::Prize { uid, chips, item } => {
                    let (mut rec, _) = hub.db.poker_record(&uid);
                    rec.chips += chips;
                    if let Some(i) = item {
                        rec.gifts.insert(i);
                    }
                    let held = st.held(&uid);
                    let _ = hub.db.set_poker_record(&uid, &rec, held);
                    hub.send_poker(&uid, ToApp::Record { me: Self::view(hub, &uid, &rec, held), gains: None });
                }
                Effect::RateHand { players } => {
                    let nets: Vec<f64> = players.iter().map(|p| p.3).collect();
                    let who: Vec<ratings::Rated> = players.into_iter().map(|(id, name, bot, _)| (id, name, bot)).collect();
                    Self::rate(hub, st, &who, |r| pyr_poker::rating::rate_hand(r, &nets), false);
                }
                Effect::RatePlaces { players } => {
                    let places: Vec<usize> = players.iter().map(|p| p.3).collect();
                    let who: Vec<ratings::Rated> = players.into_iter().map(|(id, name, bot, _)| (id, name, bot)).collect();
                    Self::rate(hub, st, &who, |r| pyr_poker::rating::rate_places(r, &places), true);
                }
                // (Counted as the tables' news goes out: see `flush_bj` and the others.)
                Effect::BjRound { .. } | Effect::CrapsRoll { .. } | Effect::BacCoup { .. } | Effect::RlSpin { .. } => {}
                Effect::Placed { uid, place, entrants, prize, tourney } => {
                    let (mut rec, _) = hub.db.poker_record(&uid);
                    let g = progress::apply_placed(&mut rec, place, entrants);
                    let held = st.held(&uid);
                    let _ = hub.db.set_poker_record(&uid, &rec, held);
                    info!(tourney, place, entrants, prize, "poker tournament: a player finished");
                    let gains = GainsView { xp: g.xp, achievements: vec![], level_up: g.level_up, net: prize as i64 };
                    hub.send_poker(&uid, ToApp::Record { me: Self::view(hub, &uid, &rec, held), gains: Some(gains) });
                }
            }
        }
    }

    /// Sends a tournament's news on (every message already addressed to someone).
    fn flush_tourney(&self, hub: &Hub, st: &State, out: Output) {
        debug_assert!(out.msgs.iter().all(|(to, _)| *to != Rcpt::All));
        self.flush(hub, st, 0, out);
    }

    /// A request from someone (`can_manage`: they may manage the server, so may close any
    /// table). What comes back goes to them alone; news goes out to the tables.
    pub fn handle(&self, hub: &Hub, uid: &str, can_manage: bool, msg: ToServer) -> Result<Option<ToApp>, String> {
        self.handle_from(hub, uid, can_manage, pyr_poker::net::CASINO_013, msg)
    }

    /// `handle`, for an app that knows Pyr Casino up to `casino` (`pyr_poker::net::CASINO_011`
    /// to `CASINO_013`): it's asked for nothing it doesn't know, and told nothing it couldn't
    /// read (see `CASINO_012_CAP` and `CASINO_013_CAP`).
    pub fn handle_from(&self, hub: &Hub, uid: &str, can_manage: bool, casino: u8, msg: ToServer) -> Result<Option<ToApp>, String> {
        let next = casino >= pyr_poker::net::CASINO_012;
        let now = self.now();
        let mut guard = self.state.lock();
        let st: &mut State = &mut guard;
        let mut rng = SysRandom;
        macro_rules! room {
            ($t:expr) => {
                st.room_mut($t).ok_or_else(|| "that table has closed".to_string())?
            };
        }
        match msg {
            ToServer::Lobby => {
                let tables = st.rooms.values().map(|r| r.summary()).collect();
                Ok(Some(ToApp::Lobby { tables, me: self.me(hub, st, uid) }))
            }
            ToServer::Create { name, rules, bots, buy_in } => {
                if st.rooms.len() >= MAX_TABLES {
                    return Err("this server has as many tables as it can keep; join one".into());
                }
                if st.rooms.values().filter(|r| r.owner == uid).count() >= MAX_TABLES_EACH {
                    return Err(format!("you can have {MAX_TABLES_EACH} tables open at once"));
                }
                let id = self.next_id.fetch_add(1, Ordering::Relaxed);
                let mut room = Room::new(id, &name, uid, rules, buy_in)?;
                for (seat, level) in bots {
                    room.add_bot(seat, level, now, &mut rng)?;
                }
                let snap = room.watch(uid);
                info!(table = id, "poker table made: {}", room.name);
                st.rooms.insert(id, room);
                self.last_people.lock().insert(id, now);
                Ok(Some(snap))
            }
            ToServer::Watch { table } => Ok(Some(Self::dress(hub, room!(table).watch(uid)))),
            ToServer::Ratings => Ok(Some(ToApp::Ratings { rows: Self::ladder(hub) })),
            ToServer::Unwatch { table } => {
                if let Some(r) = st.room_mut(table) {
                    r.unwatch(uid);
                }
                Ok(None)
            }
            ToServer::Sit { table, seat, chips } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                if rec.chips < chips {
                    return Err(format!("you have {} chips (take the daily chips in the lobby)", rec.chips));
                }
                let name = Self::name_of(hub, uid);
                let look = Self::look(&rec);
                let room = room!(table);
                // Taken from the record first; given back if they can't sit.
                rec.chips -= chips;
                let out = room.sit(uid, &name, seat, chips, look, now)?;
                let held = st.held(uid);
                hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
                self.flush(hub, st, table, out);
                Ok(Some(ToApp::Record { me: Self::view(hub, uid, &rec, held), gains: None }))
            }
            ToServer::Stand { table } => {
                let out = room!(table).stand(uid, now)?;
                self.flush(hub, st, table, out);
                Ok(None)
            }
            ToServer::Muck { table, on } => {
                room!(table).set_muck(uid, on)?;
                Ok(None)
            }
            ToServer::ShowCards { table, hand } => {
                let o = room!(table).show_cards(uid, hand)?;
                self.flush(hub, st, table, o);
                Ok(None)
            }
            ToServer::SitOut { table, out } => {
                let o = room!(table).sit_out(uid, out)?;
                self.flush(hub, st, table, o);
                Ok(None)
            }
            ToServer::TopUp { table, chips } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                if rec.chips < chips {
                    return Err(format!("you have {} chips", rec.chips));
                }
                let out = room!(table).top_up(uid, chips)?;
                rec.chips -= chips;
                let held = st.held(uid);
                hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
                self.flush(hub, st, table, out);
                Ok(Some(ToApp::Record { me: Self::view(hub, uid, &rec, held), gains: None }))
            }
            ToServer::Act { table, hand, action } => {
                let out = room!(table).act(uid, hand, action, now)?;
                self.flush(hub, st, table, out);
                Ok(None)
            }
            ToServer::Seed { table, hand, seed } => {
                if let Some(r) = st.room_mut(table) {
                    r.seed(uid, hand, seed);
                }
                Ok(None)
            }
            ToServer::TimeBank { table } => {
                let out = room!(table).time_bank(uid, now)?;
                self.flush(hub, st, table, out);
                Ok(None)
            }
            ToServer::AddBot { table, seat, level } => {
                let room = room!(table);
                if room.owner != uid && !can_manage {
                    return Err("only whoever made the table can add bots".into());
                }
                let out = room.add_bot(seat, level, now, &mut rng)?;
                self.flush(hub, st, table, out);
                Ok(None)
            }
            ToServer::RemoveBot { table, seat } => {
                let room = room!(table);
                if room.owner != uid && !can_manage {
                    return Err("only whoever made the table can take bots away".into());
                }
                let out = room.remove_bot(seat)?;
                self.flush(hub, st, table, out);
                Ok(None)
            }
            ToServer::Emote { table, emote } => {
                let out = room!(table).emote(uid, &emote)?;
                // (One too soon after the last is dropped.)
                if st.emotes.allow(uid, now) {
                    self.flush(hub, st, table, out);
                }
                Ok(None)
            }
            ToServer::Daily => {
                let (mut rec, _) = hub.db.poker_record(uid);
                let got = rec.claim_daily(today());
                let g = progress::after_daily(&mut rec);
                let held = st.held(uid);
                hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
                let gains = (!g.new_achievements.is_empty()).then(|| GainsView { xp: g.xp, achievements: g.new_achievements.iter().map(|s| s.to_string()).collect(), level_up: g.level_up, net: 0 });
                hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, &rec, held), gains });
                Ok(Some(ToApp::Daily { chips: got, streak: rec.daily_streak }))
            }
            ToServer::Equip { id } => {
                let (mut rec, held) = hub.db.poker_record(uid);
                let c = COSMETICS.iter().find(|c| c.id == id).ok_or("there's no such look")?;
                // (A prize box's or a Game Master's look is theirs once given: bug report 110,
                // "Inferno isn't yours yet" for someone who'd won it.)
                if !c.unlock.met(&rec) && !rec.gifts.contains(c.id) {
                    return Err(format!("{} isn't yours yet: {}", c.name, c.unlock.describe()));
                }
                match c.kind {
                    Kind::CardBack => rec.card_back = id,
                    Kind::Felt => rec.felt = id,
                    Kind::Chips => rec.chip_set = id,
                    Kind::Frame => rec.frame = id,
                }
                hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
                Ok(Some(ToApp::Record { me: Self::view(hub, uid, &rec, st.held(uid)), gains: None }))
            }
            ToServer::Leaderboard => {
                let mut rows: Vec<LeaderRow> = hub
                    .db
                    .poker_records()
                    .into_iter()
                    .map(|(u, rec, _)| LeaderRow {
                        name: Self::name_of(hub, &u),
                        chips: rec.chips + st.held(&u),
                        level: rec.level(),
                        hands: rec.hands,
                        biggest_pot: rec.biggest_pot,
                        best_hand: rec.best_hand_name.clone(),
                        uid: u,
                    })
                    .collect();
                rows.sort_by(|a, b| b.chips.cmp(&a.chips).then(b.level.cmp(&a.level)));
                rows.truncate(50);
                Ok(Some(ToApp::Leaderboard { rows }))
            }
            ToServer::History { table, hand } => {
                let h = room!(table).history(uid, hand).ok_or("that hand isn't kept any more")?;
                Ok(Some(ToApp::History { table, hand: h }))
            }
            ToServer::Player { table, seat } => {
                let room = room!(table);
                let them = room.player_at(seat).ok_or("nobody sits there now")?.to_string();
                let overall = (!them.starts_with("bot:")).then(|| hub.db.poker_record(&them)).map(|(rec, _)| pyr_poker::net::Tally {
                    hands: rec.hands,
                    won: rec.hands_won,
                    showdowns: rec.showdowns,
                    showdowns_won: rec.showdowns_won,
                    vpip: rec.vpip,
                    pfr: rec.pfr,
                    net: 0,
                    biggest_pot: rec.biggest_pot,
                    best_hand: rec.best_hand_name.clone(),
                    best_value: rec.best_hand,
                });
                let mut player = room.player(seat, uid, overall).ok_or("nobody sits there now")?;
                player.mmr = Some(Self::mmr_for(hub, &them, &player.name, player.bot));
                Ok(Some(ToApp::Player { table, player }))
            }
            ToServer::Casino { req } => self.casino(hub, st, uid, req, casino),
            msg @ (ToServer::BjTables
            | ToServer::BjWatch { .. }
            | ToServer::BjUnwatch { .. }
            | ToServer::BjSit { .. }
            | ToServer::BjLeave { .. }
            | ToServer::BjBet { .. }
            | ToServer::BjAct { .. }
            | ToServer::BjBot { .. }
            | ToServer::BjNew) => {
                if !next {
                    return Err("blackjack tables need a newer Pyr".into());
                }
                self.bj(hub, st, uid, msg, now, &mut rng)
            }
            msg @ (ToServer::CrapsTables
            | ToServer::CrapsWatch { .. }
            | ToServer::CrapsUnwatch { .. }
            | ToServer::CrapsSit { .. }
            | ToServer::CrapsLeave { .. }
            | ToServer::CrapsBets { .. }
            | ToServer::CrapsThrow { .. }
            | ToServer::CrapsBot { .. }) => {
                if !next {
                    return Err("craps tables need a newer Pyr".into());
                }
                self.craps_tables(hub, st, uid, msg, now, &mut rng)
            }
            ToServer::GameEmote { game, table, emote } => {
                if !next {
                    return Err("emotes at the casino's tables need a newer Pyr".into());
                }
                Self::game_emote(hub, st, uid, game, table, &emote, now)
            }
            ToServer::GmCertAsk => {
                // At home: a certificate for its owner and its Game Masters (none for anyone
                // else, so a Pyr no longer one forgets its own).
                if !Self::is_home(hub) {
                    return Err("this isn't the Game Masters' home server".into());
                }
                let owner = Self::is_owner(hub, uid);
                let role = Self::roles(hub).get(uid).copied();
                let signed = if owner || role.is_some() {
                    let signing = hub.signing.get().ok_or("this server can't sign certificates")?;
                    Some(gm::sign(signing, uid, owner, if owner { None } else { role }, self.wall())?)
                } else {
                    None
                };
                Ok(Some(ToApp::GmCert { signed }))
            }
            ToServer::GmProof { signed } => {
                let Some(home) = Self::home(hub) else { return Ok(None) };
                if Self::is_home(hub) {
                    return Ok(None);
                }
                let gm = gm::verify(&signed, home, uid, self.wall())?;
                hub.poker.gm_proofs.lock().insert(uid.to_string(), (gm.home_owner, if gm.home_owner { None } else { Some(gm.role) }, gm.expires_ms));
                let (rec, _) = hub.db.poker_record(uid);
                Ok(Some(ToApp::Record { me: Self::view(hub, uid, &rec, st.held(uid)), gains: None }))
            }
            ToServer::CasinoChat { .. } | ToServer::CasinoChatLog | ToServer::CasinoChatDelete { .. } => {
                if casino < pyr_poker::net::CASINO_013 {
                    return Err("the casino's chat needs a newer Pyr".into());
                }
                if !st.chat.loaded() {
                    st.chat.load(hub.db.get_meta(chat::META_KEY));
                }
                match msg {
                    ToServer::CasinoChatLog => Ok(Some(ToApp::CasinoChat { lines: st.chat.recent(), all: true })),
                    ToServer::CasinoChat { text } => {
                        let line = st.chat.say(uid, &Self::name_of(hub, uid), &text, self.wall())?;
                        let _ = hub.db.set_meta(chat::META_KEY, &st.chat.saved());
                        hub.broadcast_poker_from(pyr_poker::net::CASINO_013, ToApp::CasinoChat { lines: vec![line], all: false });
                        Ok(None)
                    }
                    ToServer::CasinoChatDelete { id } => {
                        let moderator = can_manage || Self::is_owner(hub, uid) || Self::role(hub, uid).is_some();
                        st.chat.delete(uid, moderator, id)?;
                        let _ = hub.db.set_meta(chat::META_KEY, &st.chat.saved());
                        hub.broadcast_poker_from(pyr_poker::net::CASINO_013, ToApp::CasinoChatGone { id });
                        Ok(None)
                    }
                    _ => Ok(None),
                }
            }
            ToServer::Bonfire | ToServer::BonfireLight => {
                if !next {
                    return Err("the Bonfire needs a newer Pyr".into());
                }
                let light = matches!(msg, ToServer::BonfireLight);
                self.bonfire_req(hub, st, uid, light, &mut rng)
            }
            msg @ (ToServer::BacTables
            | ToServer::BacWatch { .. }
            | ToServer::BacUnwatch { .. }
            | ToServer::BacSit { .. }
            | ToServer::BacLeave { .. }
            | ToServer::BacBets { .. }
            | ToServer::BacBot { .. }) => {
                if !next {
                    return Err("baccarat tables need a newer Pyr".into());
                }
                self.bac_tables(hub, st, uid, msg, now, &mut rng)
            }
            msg @ (ToServer::RlTables
            | ToServer::RlWatch { .. }
            | ToServer::RlUnwatch { .. }
            | ToServer::RlSit { .. }
            | ToServer::RlLeave { .. }
            | ToServer::RlBets { .. }
            | ToServer::RlBot { .. }) => {
                if casino < pyr_poker::net::CASINO_013 {
                    return Err("roulette tables need a newer Pyr".into());
                }
                self.rl_tables(hub, st, uid, msg, now, &mut rng)
            }
            ToServer::Staff => Ok(Some(ToApp::Staff { view: Self::staff_view(hub, uid, can_manage) })),
            ToServer::SetRole { uid: them, role } => {
                // (Staff are the whole game's: named by the Owner of the Game on the home server
                // only, and staff on every server. A server's owner names nobody.)
                if !Self::is_home(hub) {
                    return Err("Game Masters and Ambassadors are named on Brethren of Blood by the Owner of the Game, and count on every server".into());
                }
                if !Self::is_game_owner(hub, uid) {
                    return Err("only the Owner of the Game names Game Masters and Ambassadors".into());
                }
                if hub.db.get_user(&them).is_none() {
                    return Err("there's nobody by that id here".into());
                }
                let mut roles = Self::roles(hub);
                match role {
                    Some(r) => roles.insert(them.clone(), r),
                    None => roles.remove(&them),
                };
                hub.db.set_meta(ROLES, &serde_json::to_string(&roles).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                let what = match role {
                    Some(r) => format!("made {} a {}", Self::name_of(hub, &them), r.name()),
                    None => format!("took {}'s role away", Self::name_of(hub, &them)),
                };
                self.log_staff(hub, uid, what);
                // (Their own Pyr learns it with their record.)
                let (rec, _) = hub.db.poker_record(&them);
                hub.send_poker(&them, ToApp::Record { me: Self::view(hub, &them, &rec, st.held(&them)), gains: None });
                Ok(Some(ToApp::Staff { view: Self::staff_view(hub, uid, can_manage) }))
            }
            ToServer::Gift { uid: them, chips, item, note } => {
                // (The Owner of the Game gives as much as they like; it's all in the log. A
                // server's owner, as such, gives nothing.)
                let owner = Self::is_game_owner(hub, uid);
                if !owner && Self::role(hub, uid) != Some(Role::GameMaster) {
                    return Err("only Game Masters and the Owner of the Game give gifts".into());
                }
                // (The owner may give to themselves, as much as they like, in the log like any gift.)
                if them == uid && !owner {
                    return Err("not to yourself".into());
                }
                if hub.db.get_user(&them).is_none() {
                    return Err("there's nobody by that id here".into());
                }
                let look = Self::item_name(&item)?;
                if chips == 0 && look.is_none() {
                    return Err("give some chips, a look, or both".into());
                }
                let note: String = note.trim().chars().take(140).collect();
                if !owner {
                    Self::spend(hub, uid, Role::GameMaster, chips)?;
                }
                let (mut rec, _) = hub.db.poker_record(&them);
                rec.chips = rec.chips.saturating_add(chips);
                if let Some(i) = &item {
                    rec.gifts.insert(i.clone());
                }
                let held = st.held(&them);
                hub.db.set_poker_record(&them, &rec, held).map_err(|e| e.to_string())?;
                hub.send_poker(&them, ToApp::Record { me: Self::view(hub, &them, &rec, held), gains: None });
                hub.send_poker(&them, ToApp::Gifted { from: Self::name_of(hub, uid), chips, item: item.clone(), note: note.clone() });
                let what = format!(
                    "gave {} {}{}{}{}",
                    Self::name_of(hub, &them),
                    if chips > 0 { format!("{} chips", progress_chips(chips)) } else { String::new() },
                    look.map(|l| format!("{}the {l} look", if chips > 0 { " and " } else { "" })).unwrap_or_default(),
                    if note.is_empty() { String::new() } else { format!(" (\u{201c}{note}\u{201d})") },
                    if owner { " [the Owner of the Game: no limit]" } else { "" }
                );
                self.log_staff(hub, uid, what);
                Ok(Some(ToApp::Staff { view: Self::staff_view(hub, uid, can_manage) }))
            }
            ToServer::DropPrize { table, chips, item } => {
                let owner = Self::is_game_owner(hub, uid);
                let role = Self::role(hub, uid);
                if role.is_none() && !owner {
                    return Err("only Game Masters, Ambassadors and the Owner of the Game drop prize boxes".into());
                }
                let look = Self::item_name(&item)?;
                if chips == 0 && look.is_none() {
                    return Err("put some chips, a look, or both in it".into());
                }
                let name = Self::name_of(hub, uid);
                let room = st.rooms.get_mut(&table).ok_or("that table has closed")?;
                let table_name = room.name.clone();
                if room.prize.is_some() {
                    return Err("there's a prize box on this table already: it goes to the next hand's winner".into());
                }
                if let (false, Some(role)) = (owner, role) {
                    Self::spend(hub, uid, role, chips)?;
                }
                let mut out = Output::default();
                room.drop_prize(PrizeView { chips, item, from: name }, &mut out)?;
                self.flush(hub, st, table, out);
                self.log_staff(
                    hub,
                    uid,
                    format!(
                        "dropped a prize box on {table_name}: {}{}{}",
                        if chips > 0 { format!("{} chips", progress_chips(chips)) } else { String::new() },
                        look.map(|l| format!("{}the {l} look", if chips > 0 { " and " } else { "" })).unwrap_or_default(),
                        if owner { " [the Owner of the Game: no limit]" } else { "" }
                    ),
                );
                Ok(None)
            }
            ToServer::Close { table } => {
                let room = st.rooms.get_mut(&table).ok_or("that table has closed (a tournament's tables close when it's over)")?;
                if room.owner != uid && !can_manage {
                    return Err("only whoever made the table can close it".into());
                }
                let out = room.close();
                // Told while it's still there (its audience), then gone.
                self.flush(hub, st, table, out);
                st.rooms.remove(&table);
                Ok(None)
            }
            ToServer::Tourneys => Ok(Some(ToApp::Tourneys { list: st.tourneys.values().map(|t| t.summary(uid)).collect() })),
            ToServer::TourneyCreate { mut rules, bots } => {
                // (Only Pyr Friday Night has prize money the house adds.)
                rules.added = 0;
                if !can_manage {
                    return Err("tournaments here are set up by people who can manage the server".into());
                }
                // (Flash Pots are only started by `TimeBomb`, by the house.)
                rules.bomb = None;
                if st.tourneys.values().filter(|t| t.rules.bomb.is_none()).count() >= MAX_TOURNEYS {
                    return Err(format!("this server keeps {MAX_TOURNEYS} tournaments at most"));
                }
                if rules.start_ms > self.wall() + 30 * 86_400_000 {
                    return Err("start it within 30 days".into());
                }
                let id = self.next_tourney.fetch_add(1, Ordering::Relaxed);
                let mut t = Tourney::new(id, rules, uid, &Self::name_of(hub, uid))?;
                let mut out = Output::default();
                for level in bots {
                    t.add_bot(level, &mut out)?;
                }
                let v = t.watch(uid);
                info!(tourney = id, "poker tournament set up: {}", t.rules.name);
                st.tourneys.insert(id, t);
                Ok(Some(v))
            }
            ToServer::TourneyRegister { id } => {
                let wall = self.wall();
                let t = st.tourneys.get_mut(&id).ok_or("that tournament is gone")?;
                let (mut rec, _) = hub.db.poker_record(uid);
                let fee = t.rules.buy_in;
                if rec.chips < fee {
                    return Err(format!("the buy-in is {fee} chips and you have {}", rec.chips));
                }
                let mut out = Output::default();
                t.register(uid, &Self::name_of(hub, uid), Self::look(&rec), fee, wall, &mut out)?;
                rec.chips -= fee;
                let held = st.held(uid);
                hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
                self.flush_tourney(hub, st, out);
                Ok(Some(ToApp::Record { me: Self::view(hub, uid, &rec, held), gains: None }))
            }
            ToServer::TourneyUnregister { id } => {
                let t = st.tourneys.get_mut(&id).ok_or("that tournament is gone")?;
                let mut out = Output::default();
                t.unregister(uid, &mut out)?;
                self.flush_tourney(hub, st, out);
                Ok(None)
            }
            ToServer::TourneyWatch { id } => Ok(Some(st.tourneys.get_mut(&id).ok_or("that tournament is gone")?.watch(uid))),
            ToServer::TourneyUnwatch { id } => {
                if let Some(t) = st.tourneys.get_mut(&id) {
                    t.unwatch(uid);
                }
                Ok(None)
            }
            ToServer::TimeBomb { buy_in } => {
                let wall = self.wall();
                if !tourney::BOMB_BUY_INS.contains(&buy_in) {
                    return Err("Flash Pots are 100, 1,000 or 10,000 chips to play".into());
                }
                let (mut rec, _) = hub.db.poker_record(uid);
                if rec.chips < buy_in {
                    return Err(format!("this Flash Pot is {buy_in} chips to play and you have {}", rec.chips));
                }
                if st.tourneys.values().any(|t| t.rules.bomb.is_some() && t.phase == TourneyPhase::Registering && t.is_entrant(uid)) {
                    return Err("you're waiting for a Flash Pot already".into());
                }
                // A seat in one that's filling up at this buy-in, or a new one.
                let open = st.tourneys.iter().find(|(_, t)| t.rules.bomb.is_some() && t.rules.buy_in == buy_in && t.can_register() && !t.is_entrant(uid)).map(|(id, _)| *id);
                let id = match open {
                    Some(id) => id,
                    None => {
                        if st.tourneys.values().filter(|t| t.rules.bomb.is_some() && t.ended.is_none()).count() >= MAX_BOMBS {
                            return Err("every Flash Pot table is busy: try again in a minute".into());
                        }
                        let id = self.next_tourney.fetch_add(1, Ordering::Relaxed);
                        let t = Tourney::new(id, tourney::bomb_rules(buy_in, wall), tourney::BOMB_HOST, "Pyr Casino")?;
                        st.tourneys.insert(id, t);
                        id
                    }
                };
                let t = st.tourneys.get_mut(&id).expect("just found or made");
                let mut out = Output::default();
                t.register(uid, &Self::name_of(hub, uid), Self::look(&rec), buy_in, wall, &mut out)?;
                rec.chips -= buy_in;
                let held = st.held(uid);
                hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
                out.msgs.push((Rcpt::User(uid.to_string()), ToApp::Record { me: Self::view(hub, uid, &rec, held), gains: None }));
                let view = st.tourneys.get_mut(&id).expect("there").watch(uid);
                self.flush_tourney(hub, st, out);
                Ok(Some(view))
            }
            ToServer::TourneyCancel { id } => {
                let wall = self.wall();
                let t = st.tourneys.get_mut(&id).ok_or("that tournament is gone")?;
                if t.host == tourney::FRIDAY_HOST {
                    return Err("Pyr Friday Night happens every week; it can't be called off".into());
                }
                if t.host == tourney::BOMB_HOST {
                    return Err("a Flash Pot starts by itself (leave it with Unregister before it does)".into());
                }
                if t.host != uid && !can_manage {
                    return Err("only whoever set it up can call it off".into());
                }
                let mut out = Output::default();
                t.cancel(wall, &mut out)?;
                self.flush_tourney(hub, st, out);
                Ok(None)
            }
        }
    }

    /// Someone's last connection closed: they leave every table (folding if they're in a hand)
    /// and stop watching.
    pub fn gone(&self, hub: &Hub, uid: &str) {
        let now = self.now();
        let mut guard = self.state.lock();
        let st: &mut State = &mut guard;
        let ids: Vec<TableId> = st.rooms.keys().copied().collect();
        for id in ids {
            let Some(r) = st.rooms.get_mut(&id) else { continue };
            r.unwatch(uid);
            if r.seat_of(uid).is_some()
                && let Ok(out) = r.stand(uid, now)
            {
                self.flush(hub, st, id, out);
            }
        }
        // (In a tournament they stay in: folded for until they're back, or out.)
        st.casino.watchers.remove(uid);
        self.bj_gone(hub, st, uid, now);
        self.craps_gone(hub, st, uid, now);
        self.bac_gone(hub, st, uid, now);
        self.rl_gone(hub, st, uid, now);
    }

    /// The clock (a few times a second).
    pub fn tick(&self, hub: &Hub) {
        let now = self.now();
        let mut guard = self.state.lock();
        let st: &mut State = &mut guard;
        let mut rng = SysRandom;
        // Tournaments (on the wall clock: they start at a time).
        let wall = self.wall();
        self.friday_night(hub, st, wall);
        Self::casino_after_restart(hub, st);
        let tids: Vec<TourneyId> = st.tourneys.keys().copied().collect();
        for tid in tids {
            let mut next = self.next_id.load(Ordering::Relaxed);
            let t = st.tourneys.get_mut(&tid).unwrap();
            let out = t.tick(wall, &mut next, &mut rng);
            self.next_id.store(next, Ordering::Relaxed);
            let keep = if t.rules.bomb.is_some() { BOMB_KEEP_MS } else { TOURNEY_KEEP_MS };
            if t.ended.is_some_and(|e| wall.saturating_sub(e) > keep) {
                st.tourneys.remove(&tid);
            }
            if !out.msgs.is_empty() || !out.effects.is_empty() {
                self.flush_tourney(hub, st, out);
            }
        }
        self.bj_tick(hub, st, now, &mut rng);
        self.craps_tick(hub, st, now, &mut rng);
        self.bac_tick(hub, st, now, &mut rng);
        self.rl_tick(hub, st, now, &mut rng);
        self.bonfire_tick(hub, st, &mut rng);
        let ids: Vec<TableId> = st.rooms.keys().copied().collect();
        let mut close = Vec::new();
        for id in ids {
            let Some(r) = st.rooms.get_mut(&id) else { continue };
            let out = r.tick(now, &mut rng);
            // (Someone watching keeps it open too: a table shouldn't close under them.)
            let has_people = !r.people().is_empty() || r.watchers().next().is_some();
            if !out.msgs.is_empty() || !out.effects.is_empty() {
                self.flush(hub, st, id, out);
            }
            let mut lp = self.last_people.lock();
            if has_people {
                lp.insert(id, now);
            } else if now.saturating_sub(*lp.get(&id).unwrap_or(&now)) > EMPTY_CLOSE_MS {
                close.push(id);
            }
        }
        for id in close {
            if let Some(mut r) = st.rooms.remove(&id) {
                let out = r.close();
                st.rooms.insert(id, r);
                self.flush(hub, st, id, out);
                st.rooms.remove(&id);
                self.last_people.lock().remove(&id);
                info!(table = id, "poker table closed: nobody played or watched for half an hour");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use pyr_poker::bots::Level;
    use pyr_poker::game::Rules;
    use pyr_poker::net::BuyIn;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-poker-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    #[test]
    fn chips_go_to_the_table_and_come_back() {
        let hub = hub();
        let p = &hub.poker;
        let Some(ToApp::Lobby { tables, me }) = p.handle(&hub, "alice", false, ToServer::Lobby).unwrap() else { panic!() };
        assert!(tables.is_empty());
        assert_eq!(me.chips, progress::STARTING_CHIPS);
        let Some(ToApp::Snapshot { table }) = p
            .handle(&hub, "alice", false, ToServer::Create { name: "Friday".into(), rules: Rules::holdem(10, 20, 6), bots: vec![(1, Level::Shark), (2, Level::Rookie)], buy_in: BuyIn::default() })
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(table.seats.iter().flatten().count(), 2);
        // Too many chips, then a good amount.
        assert!(p.handle(&hub, "alice", false, ToServer::Sit { table: table.id, seat: 0, chips: 100_000 }).is_err());
        let Some(ToApp::Record { me, .. }) = p.handle(&hub, "alice", false, ToServer::Sit { table: table.id, seat: 0, chips: 2000 }).unwrap() else { panic!() };
        assert_eq!((me.chips, me.at_tables), (progress::STARTING_CHIPS - 2000, 2000));
        // Only whoever made it (or a manager) can close it or move bots.
        assert!(p.handle(&hub, "bob", false, ToServer::RemoveBot { table: table.id, seat: 1 }).is_err());
        assert!(p.handle(&hub, "bob", false, ToServer::Close { table: table.id }).is_err());
        // The server stopping with chips at the table: they come back when it starts.
        let (rec, held) = hub.db.poker_record("alice");
        assert_eq!((rec.chips, held), (progress::STARTING_CHIPS - 2000, 2000));
        // Leaving: the chips come back at once (no hand yet).
        p.gone(&hub, "alice");
        let (rec, held) = hub.db.poker_record("alice");
        assert_eq!((rec.chips, held), (progress::STARTING_CHIPS, 0));
        assert!(p.handle(&hub, "alice", false, ToServer::Close { table: table.id }).unwrap().is_none());
        let Some(ToApp::Lobby { tables, .. }) = p.handle(&hub, "alice", false, ToServer::Lobby).unwrap() else { panic!() };
        assert!(tables.is_empty());
    }

    #[test]
    fn held_chips_return_when_the_server_starts() {
        let hub = hub();
        let mut rec = Record::new();
        rec.chips = 500;
        hub.db.set_poker_record("carol", &rec, 1500).unwrap();
        assert_eq!(hub.db.poker_return_held(), 1);
        assert_eq!(hub.db.poker_record("carol"), (Record { chips: 2000, ..rec }, 0));
    }

    #[test]
    fn the_daily_chips_and_looks() {
        let hub = hub();
        let p = &hub.poker;
        let Some(ToApp::Daily { chips, streak }) = p.handle(&hub, "dee", false, ToServer::Daily).unwrap() else { panic!() };
        assert_eq!((chips, streak), (Some(progress::DAILY_BASE), 1));
        let Some(ToApp::Daily { chips, .. }) = p.handle(&hub, "dee", false, ToServer::Daily).unwrap() else { panic!() };
        assert_eq!(chips, None);
        assert!(p.handle(&hub, "dee", false, ToServer::Equip { id: "felt_crimson".into() }).unwrap_err().contains("level 12"));
        let Some(ToApp::Record { me, .. }) = p.handle(&hub, "dee", false, ToServer::Equip { id: "felt_classic".into() }).unwrap() else { panic!() };
        assert_eq!(me.felt, "felt_classic");
        // A prize look: not theirs till it's given, then theirs to wear (bug report 110).
        assert!(p.handle(&hub, "dee", false, ToServer::Equip { id: "back_inferno".into() }).unwrap_err().contains("prize box"));
        let (mut rec, held) = hub.db.poker_record("dee");
        rec.gifts.insert("back_inferno".into());
        hub.db.set_poker_record("dee", &rec, held).unwrap();
        let Some(ToApp::Record { me, .. }) = p.handle(&hub, "dee", false, ToServer::Equip { id: "back_inferno".into() }).unwrap() else { panic!() };
        assert_eq!(me.card_back, "back_inferno");
    }

    #[test]
    fn tournament_buy_ins_are_taken_and_given_back() {
        use pyr_poker::game::{Limit, Variant};
        use pyr_poker::net::TourneyRules;
        let hub = hub();
        let p = &hub.poker;
        let rules = TourneyRules {
            name: "Sunday Major".into(),
            variant: Variant::Holdem,
            limit: Limit::NoLimit,
            seats: 9,
            stack: 3000,
            buy_in: 1000,
            min_players: 2,
            max_players: 90,
            start_ms: pyr_proto::now_ms() as u64 + 3_600_000,
            level_ms: 600_000,
            levels: pyr_poker::tourney::standard_levels(3000),
            late_levels: 3,
            added: 0,
            bomb: None,
        };
        // Only managers set them up.
        assert!(p.handle(&hub, "alice", false, ToServer::TourneyCreate { rules: rules.clone(), bots: vec![] }).is_err());
        let Some(ToApp::Tourney { view }) = p.handle(&hub, "host", true, ToServer::TourneyCreate { rules, bots: vec![Level::Shark; 3] }).unwrap() else { panic!() };
        assert_eq!(view.entrants.len(), 3);
        let Some(ToApp::Record { me, .. }) = p.handle(&hub, "alice", false, ToServer::TourneyRegister { id: view.id }).unwrap() else { panic!() };
        assert_eq!((me.chips, me.at_tables), (progress::STARTING_CHIPS - 1000, 1000));
        assert!(p.handle(&hub, "alice", false, ToServer::TourneyRegister { id: view.id }).is_err());
        // Broke: can't register.
        let mut poor = Record::new();
        poor.chips = 10;
        hub.db.set_poker_record("bob", &poor, 0).unwrap();
        assert!(p.handle(&hub, "bob", false, ToServer::TourneyRegister { id: view.id }).unwrap_err().contains("buy-in"));
        // The server stopping now would give Alice her buy-in back.
        assert_eq!(hub.db.poker_record("alice").1, 1000);
        let Some(ToApp::Tourneys { list }) = p.handle(&hub, "alice", false, ToServer::Tourneys).unwrap() else { panic!() };
        assert!(list[0].registered && list[0].prize_pool == 1000 && list[0].entrants == 4);
        // Called off: the buy-in comes back.
        assert!(p.handle(&hub, "alice", false, ToServer::TourneyCancel { id: view.id }).is_err());
        p.handle(&hub, "host", true, ToServer::TourneyCancel { id: view.id }).unwrap();
        assert_eq!(hub.db.poker_record("alice"), (Record { chips: progress::STARTING_CHIPS, ..Record::new() }, 0));
    }

    #[test]
    fn time_bombs_seat_people_together_and_start_when_full() {
        let hub = hub();
        let p = &hub.poker;
        assert!(p.handle(&hub, "alice", false, ToServer::TimeBomb { buy_in: 777 }).is_err());
        let Some(ToApp::Tourney { view: a }) = p.handle(&hub, "alice", false, ToServer::TimeBomb { buy_in: 1_000 }).unwrap() else { panic!() };
        assert!(a.bomb.is_some() && a.phase == TourneyPhase::Registering);
        assert_eq!(hub.db.poker_record("alice").0.chips, progress::STARTING_CHIPS - 1_000);
        assert!(p.handle(&hub, "alice", false, ToServer::TimeBomb { buy_in: 1_000 }).unwrap_err().contains("already"));
        // Bob, Carol and Fay join Alice's (it isn't full yet: five seats); Gus fills it, and it
        // starts at the next tick.
        for who in ["bob", "carol", "fay"] {
            let Some(ToApp::Tourney { view }) = p.handle(&hub, who, false, ToServer::TimeBomb { buy_in: 1_000 }).unwrap() else { panic!() };
            assert_eq!(view.id, a.id);
        }
        p.tick(&hub);
        assert_eq!(p.state.lock().tourneys[&a.id].phase, TourneyPhase::Registering, "four of five: still waiting");
        let Some(ToApp::Tourney { view: g }) = p.handle(&hub, "gus", false, ToServer::TimeBomb { buy_in: 1_000 }).unwrap() else { panic!() };
        assert_eq!(g.id, a.id);
        p.tick(&hub);
        {
            let st = p.state.lock();
            let t = &st.tourneys[&a.id];
            assert_eq!(t.phase, TourneyPhase::Running);
            assert!(t.bomb_view().unwrap().multiplier.is_some(), "the wheel spun");
        }
        // Dee gets a new one; a different buy-in is a different one.
        let Some(ToApp::Tourney { view: d }) = p.handle(&hub, "dee", false, ToServer::TimeBomb { buy_in: 1_000 }).unwrap() else { panic!() };
        assert_ne!(d.id, a.id);
        let Some(ToApp::Tourney { view: e }) = p.handle(&hub, "erin", false, ToServer::TimeBomb { buy_in: 100 }).unwrap() else { panic!() };
        assert_ne!(e.id, d.id);
        // Nobody calls one off; leaving before it starts gives the buy-in back.
        assert!(p.handle(&hub, "dee", true, ToServer::TourneyCancel { id: d.id }).is_err());
        p.handle(&hub, "dee", false, ToServer::TourneyUnregister { id: d.id }).unwrap();
        assert_eq!(hub.db.poker_record("dee").0.chips, progress::STARTING_CHIPS);
    }

    /// The server's owner gives as many chips as they like, to themselves too (a Game Master only
    /// what's left of today's allowance, never to themselves, and nobody else any), and every
    /// gift is in the log the owner reads.
    #[test]
    fn the_owner_gives_without_a_limit_and_it_is_logged() {
        use pyr_proto::control::Role as UserRole;
        let hub = hub();
        let p = &hub.poker;
        let owner = "blue";
        hub.db.insert_user(owner, "Bluelight", UserRole::Owner).unwrap();
        hub.db.insert_user("gm", "Gee", UserRole::Member).unwrap();
        hub.db.insert_user("amy", "Amy", UserRole::Member).unwrap();
        let gift = |from: &str, chips: u64| p.handle(&hub, from, false, ToServer::Gift { uid: "amy".into(), chips, item: None, note: "for the final".into() });
        let start = hub.db.poker_record("amy").0.chips;
        assert!(gift("gm", 10).is_err(), "nobody else gives anything");
        // (Game Masters are named on the home server: this one, here.)
        let _ = p.home_for_tests.set(hub.fingerprint);
        p.handle(&hub, owner, false, ToServer::SetRole { uid: "gm".into(), role: Some(Role::GameMaster) }).unwrap();
        assert!(gift("gm", GM_FLOOR + 1).is_err(), "a Game Master: today's allowance");
        gift("gm", 1_000).unwrap();
        // The owner: as many as they like, again and again, well past any allowance.
        gift(owner, 5_000_000).unwrap();
        gift(owner, 7_000_000).unwrap();
        assert_eq!(hub.db.poker_record("amy").0.chips, start + 1_000 + 12_000_000);
        // The owner to themselves too, without a limit (a Game Master never).
        let mine = hub.db.poker_record(owner).0.chips;
        p.handle(&hub, owner, false, ToServer::Gift { uid: owner.into(), chips: 9_000_000, item: None, note: String::new() }).unwrap();
        assert_eq!(hub.db.poker_record(owner).0.chips, mine + 9_000_000);
        assert!(p.handle(&hub, "gm", false, ToServer::Gift { uid: "gm".into(), chips: 5, item: None, note: String::new() }).is_err(), "not to themselves");
        let Some(ToApp::Staff { view }) = p.handle(&hub, owner, false, ToServer::Staff).unwrap() else { panic!() };
        assert!(view.owner);
        assert!(view.log[0].what.contains("Bluelight 9,000,000 chips") && view.log[0].what.contains("no limit"), "{}", view.log[0].what);
        let top = &view.log[1];
        assert_eq!(top.by, "Bluelight");
        assert!(top.what.contains("Amy 7,000,000 chips") && top.what.contains("no limit"), "{}", top.what);
        assert!(view.log.iter().any(|l| l.by == "Bluelight" && l.what.contains("5,000,000 chips")));
        assert!(view.log.iter().any(|l| l.by == "Gee" && l.what.contains("1,000 chips") && !l.what.contains("no limit")));
        // Only the owner reads it.
        let Some(ToApp::Staff { view }) = p.handle(&hub, "gm", false, ToServer::Staff).unwrap() else { panic!() };
        assert!(view.log.is_empty() && !view.owner);
    }

    /// Game Masters are named on the home server only, and are Game Masters everywhere by its
    /// certificate; the Bonfire's lit early by them and the game's owner, not a server's owner.
    #[test]
    fn game_masters_are_the_home_servers_everywhere() {
        use pyr_proto::control::Role as UserRole;
        let dir = std::env::temp_dir().join(format!("pyr-gm-hubs-{}", std::process::id()));
        let tls_at = |name: &str| {
            let d = dir.join(name);
            std::fs::create_dir_all(&d).unwrap();
            crate::tls::load_or_create(&d).unwrap()
        };
        let (home_tls, other_tls) = (tls_at("home"), tls_at("other"));
        let hub_for = |tls: &crate::tls::TlsIdentity| {
            let h = Hub::new(Db::open_in_memory().unwrap(), tls.fingerprint, "127.0.0.1".into(), 1, dir.join("files"));
            let _ = h.signing.set(crate::directory::Signing::new(&tls.cert_der, &tls.key_der).unwrap());
            let _ = h.poker.home_for_tests.set(home_tls.fingerprint);
            h
        };
        let (home, other) = (hub_for(&home_tls), hub_for(&other_tls));
        for h in [&home, &other] {
            h.db.insert_user("gee", "Gee", UserRole::Member).unwrap();
            h.db.insert_user("amy", "Amy", UserRole::Member).unwrap();
        }
        home.db.insert_user("blue", "Bluelight", UserRole::Owner).unwrap();
        other.db.insert_user("bob", "Bob", UserRole::Owner).unwrap();
        other.db.insert_user("blue", "Bluelight", UserRole::Member).unwrap();
        // Another server's owner names nobody, gives no chips, drops no prize box, lights no
        // Bonfire.
        assert!(other.poker.handle(&other, "bob", false, ToServer::SetRole { uid: "gee".into(), role: Some(Role::GameMaster) }).is_err());
        assert!(other.poker.handle(&other, "bob", false, ToServer::SetRole { uid: "gee".into(), role: Some(Role::Ambassador) }).is_err());
        assert!(other.poker.handle(&other, "bob", false, ToServer::Gift { uid: "amy".into(), chips: 10, item: None, note: String::new() }).is_err());
        assert!(other.poker.handle(&other, "bob", false, ToServer::BonfireLight).is_err());
        let Some(ToApp::Staff { view }) = other.poker.handle(&other, "bob", false, ToServer::Staff).unwrap() else { panic!() };
        assert!(!view.owner && view.me.is_none());
        // Nor does a Game Master at home name anyone.
        home.db.insert_user("mo", "Mo", UserRole::Member).unwrap();
        // Named at home: a certificate, which counts on the other server.
        home.poker.handle(&home, "blue", false, ToServer::SetRole { uid: "gee".into(), role: Some(Role::GameMaster) }).unwrap();
        let cert = |h: &Hub, uid: &str| match h.poker.handle(h, uid, false, ToServer::GmCertAsk).unwrap() {
            Some(ToApp::GmCert { signed }) => signed,
            other => panic!("{other:?}"),
        };
        assert!(cert(&home, "amy").is_none(), "not staff: no certificate");
        assert!(other.poker.handle(&other, "gee", false, ToServer::GmCertAsk).is_err(), "only the home signs");
        let gee = cert(&home, "gee").unwrap();
        assert_eq!(Poker::role(&other, "gee"), None);
        assert!(other.poker.handle(&other, "amy", false, ToServer::GmProof { signed: gee.clone() }).is_err(), "someone else's");
        other.poker.handle(&other, "gee", false, ToServer::GmProof { signed: gee }).unwrap();
        assert_eq!(Poker::role(&other, "gee"), Some(Role::GameMaster));
        assert!(Poker::can_light(&other, "gee") && !Poker::is_game_owner(&other, "gee"));
        assert!(home.poker.handle(&home, "gee", false, ToServer::SetRole { uid: "mo".into(), role: Some(Role::Ambassador) }).is_err());
        // An Ambassador named at home is one everywhere too.
        home.poker.handle(&home, "blue", false, ToServer::SetRole { uid: "amy".into(), role: Some(Role::Ambassador) }).unwrap();
        let amy = cert(&home, "amy").unwrap();
        other.poker.handle(&other, "amy", false, ToServer::GmProof { signed: amy }).unwrap();
        assert_eq!(Poker::role(&other, "amy"), Some(Role::Ambassador));
        assert!(!Poker::can_light(&other, "amy"));
        // The game's owner: lights it anywhere.
        let blue = cert(&home, "blue").unwrap();
        other.poker.handle(&other, "blue", false, ToServer::GmProof { signed: blue }).unwrap();
        assert!(Poker::is_game_owner(&other, "blue") && Poker::can_light(&other, "blue"));
        // The Owner of the Game gives as much as they like there too.
        other.poker.handle(&other, "blue", false, ToServer::Gift { uid: "amy".into(), chips: 3_000_000, item: None, note: String::new() }).unwrap();
        let Some(ToApp::Staff { view }) = other.poker.handle(&other, "blue", false, ToServer::Staff).unwrap() else { panic!() };
        assert!(view.owner);
        assert!(!Poker::can_light(&other, "bob"));
        // Taken away at home: no certificate next time (and the old one runs out).
        home.poker.handle(&home, "blue", false, ToServer::SetRole { uid: "gee".into(), role: None }).unwrap();
        assert!(cert(&home, "gee").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
