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
use pyr_proto::control::Event;
use tracing::info;

use crate::hub::Hub;

mod casino;

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
const STAFF_LOG_KEEP: usize = 300;
/// The least a Game Master / Ambassador may give a day, however little has been raked.
const GM_FLOOR: u64 = 25_000;
const AMBASSADOR_FLOOR: u64 = 5_000;
/// A finished tournament's results are kept this long.
pub const TOURNEY_KEEP_MS: u64 = 2 * 60 * 60 * 1000;
/// Time Bombs: how many at once (they don't count toward `MAX_TOURNEYS`), and how long a
/// finished one stays to be seen.
pub const MAX_BOMBS: usize = 40;
pub const BOMB_KEEP_MS: u64 = 10 * 60 * 1000;

#[derive(Default)]
pub struct State {
    pub rooms: BTreeMap<TableId, Room>,
    pub tourneys: BTreeMap<TourneyId, Tourney>,
    pub casino: casino::CasinoState,
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

    /// Play chips a person has riding: at cash tables, and buy-ins in tournaments not over.
    fn held(&self, uid: &str) -> u64 {
        self.rooms.values().map(|r| r.chips_of(uid)).sum::<u64>() + self.tourneys.values().map(|t| t.at_stake(uid)).sum::<u64>()
    }
}

pub struct Poker {
    state: Mutex<State>,
    next_tourney: AtomicU64,
    next_id: AtomicU64,
    start: Instant,
    /// When each table last had a person sitting at it.
    last_people: Mutex<HashMap<TableId, u64>>,
}

impl Default for Poker {
    fn default() -> Self {
        Poker { state: Mutex::new(State::default()), next_tourney: AtomicU64::new(1), next_id: AtomicU64::new(1), start: Instant::now(), last_people: Mutex::new(HashMap::new()) }
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

    pub fn role(hub: &Hub, uid: &str) -> Option<Role> {
        Self::roles(hub).get(uid).copied()
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

    fn staff_view(hub: &Hub, uid: &str, can_manage: bool) -> StaffView {
        let me = Self::role(hub, uid);
        let allowance = me.map(|r| Self::allowance(hub, r)).unwrap_or(0);
        StaffView {
            me,
            owner: can_manage,
            allowance,
            left: allowance.saturating_sub(Self::spent_today(hub, uid)),
            rake_week: Self::rake_week(hub),
            staff: Self::roles(hub).into_iter().map(|(u, role)| StaffMember { name: Self::name_of(hub, &u), uid: u, role }).collect(),
            log: if can_manage { hub.db.get_meta(STAFF_LOG).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default() } else { vec![] },
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
            match to {
                Rcpt::All => {
                    for uid in &audience {
                        hub.send_to_user(uid, Event::Poker { msg: msg.clone() });
                    }
                }
                Rcpt::User(uid) => hub.send_to_user(&uid, Event::Poker { msg }),
            }
        }
        for e in out.effects {
            match e {
                Effect::Credit { uid, chips } => {
                    let (mut rec, _) = hub.db.poker_record(&uid);
                    rec.chips += chips;
                    let held = st.held(&uid);
                    let _ = hub.db.set_poker_record(&uid, &rec, held);
                    hub.send_to_user(&uid, Event::Poker { msg: ToApp::Record { me: Self::view(hub, &uid, &rec, held), gains: None } });
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
                    hub.send_to_user(&uid, Event::Poker { msg: ToApp::Record { me: Self::view(hub, &uid, &rec, held), gains: Some(gains) } });
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
                    hub.send_to_user(&uid, Event::Poker { msg: ToApp::Record { me: Self::view(hub, &uid, &rec, held), gains: None } });
                }
                Effect::Placed { uid, place, entrants, prize, tourney } => {
                    let (mut rec, _) = hub.db.poker_record(&uid);
                    let g = progress::apply_placed(&mut rec, place, entrants);
                    let held = st.held(&uid);
                    let _ = hub.db.set_poker_record(&uid, &rec, held);
                    info!(tourney, place, entrants, prize, "poker tournament: a player finished");
                    let gains = GainsView { xp: g.xp, achievements: vec![], level_up: g.level_up, net: prize as i64 };
                    hub.send_to_user(&uid, Event::Poker { msg: ToApp::Record { me: Self::view(hub, &uid, &rec, held), gains: Some(gains) } });
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
            ToServer::Watch { table } => Ok(Some(room!(table).watch(uid))),
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
                self.flush(hub, st, table, out);
                Ok(None)
            }
            ToServer::Daily => {
                let (mut rec, _) = hub.db.poker_record(uid);
                let got = rec.claim_daily(today());
                let g = progress::after_daily(&mut rec);
                let held = st.held(uid);
                hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
                let gains = (!g.new_achievements.is_empty()).then(|| GainsView { xp: g.xp, achievements: g.new_achievements.iter().map(|s| s.to_string()).collect(), level_up: g.level_up, net: 0 });
                hub.send_to_user(uid, Event::Poker { msg: ToApp::Record { me: Self::view(hub, uid, &rec, held), gains } });
                Ok(Some(ToApp::Daily { chips: got, streak: rec.daily_streak }))
            }
            ToServer::Equip { id } => {
                let (mut rec, held) = hub.db.poker_record(uid);
                let c = COSMETICS.iter().find(|c| c.id == id).ok_or("there's no such look")?;
                if !c.unlock.met(&rec) {
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
                let player = room.player(seat, uid, overall).ok_or("nobody sits there now")?;
                Ok(Some(ToApp::Player { table, player }))
            }
            ToServer::Casino { req } => self.casino(hub, st, uid, req),
            ToServer::Staff => Ok(Some(ToApp::Staff { view: Self::staff_view(hub, uid, can_manage) })),
            ToServer::SetRole { uid: them, role } => {
                if !can_manage {
                    return Err("only people who can manage the server name Game Masters and Ambassadors".into());
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
                hub.send_to_user(&them, Event::Poker { msg: ToApp::Record { me: Self::view(hub, &them, &rec, st.held(&them)), gains: None } });
                Ok(Some(ToApp::Staff { view: Self::staff_view(hub, uid, can_manage) }))
            }
            ToServer::Gift { uid: them, chips, item, note } => {
                if Self::role(hub, uid) != Some(Role::GameMaster) {
                    return Err("only Game Masters give gifts".into());
                }
                if them == uid {
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
                Self::spend(hub, uid, Role::GameMaster, chips)?;
                let (mut rec, _) = hub.db.poker_record(&them);
                rec.chips += chips;
                if let Some(i) = &item {
                    rec.gifts.insert(i.clone());
                }
                let held = st.held(&them);
                hub.db.set_poker_record(&them, &rec, held).map_err(|e| e.to_string())?;
                hub.send_to_user(&them, Event::Poker { msg: ToApp::Record { me: Self::view(hub, &them, &rec, held), gains: None } });
                hub.send_to_user(&them, Event::Poker { msg: ToApp::Gifted { from: Self::name_of(hub, uid), chips, item: item.clone(), note: note.clone() } });
                let what = format!(
                    "gave {} {}{}{}",
                    Self::name_of(hub, &them),
                    if chips > 0 { format!("{} chips", progress_chips(chips)) } else { String::new() },
                    look.map(|l| format!("{}the {l} look", if chips > 0 { " and " } else { "" })).unwrap_or_default(),
                    if note.is_empty() { String::new() } else { format!(" (\u{201c}{note}\u{201d})") }
                );
                self.log_staff(hub, uid, what);
                Ok(Some(ToApp::Staff { view: Self::staff_view(hub, uid, can_manage) }))
            }
            ToServer::DropPrize { table, chips, item } => {
                let Some(role) = Self::role(hub, uid) else {
                    return Err("only Game Masters and Ambassadors drop prize boxes".into());
                };
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
                Self::spend(hub, uid, role, chips)?;
                let mut out = Output::default();
                room.drop_prize(PrizeView { chips, item, from: name }, &mut out)?;
                self.flush(hub, st, table, out);
                self.log_staff(
                    hub,
                    uid,
                    format!(
                        "dropped a prize box on {table_name}: {}{}",
                        if chips > 0 { format!("{} chips", progress_chips(chips)) } else { String::new() },
                        look.map(|l| format!("{}the {l} look", if chips > 0 { " and " } else { "" })).unwrap_or_default()
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
                // (Time Bombs are only started by `TimeBomb`, by the house.)
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
                    return Err("Time Bombs are 100, 1,000 or 10,000 chips to play".into());
                }
                let (mut rec, _) = hub.db.poker_record(uid);
                if rec.chips < buy_in {
                    return Err(format!("this Time Bomb is {buy_in} chips to play and you have {}", rec.chips));
                }
                if st.tourneys.values().any(|t| t.rules.bomb.is_some() && t.phase == TourneyPhase::Registering && t.is_entrant(uid)) {
                    return Err("you're waiting for a Time Bomb already".into());
                }
                // A seat in one that's filling up at this buy-in, or a new one.
                let open = st.tourneys.iter().find(|(_, t)| t.rules.bomb.is_some() && t.rules.buy_in == buy_in && t.can_register() && !t.is_entrant(uid)).map(|(id, _)| *id);
                let id = match open {
                    Some(id) => id,
                    None => {
                        if st.tourneys.values().filter(|t| t.rules.bomb.is_some() && t.ended.is_none()).count() >= MAX_BOMBS {
                            return Err("every Time Bomb table is busy: try again in a minute".into());
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
                    return Err("a Time Bomb starts by itself (leave it with Unregister before it does)".into());
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
        // Bob joins Alice's; Carol fills it, and it starts at the next tick.
        let Some(ToApp::Tourney { view: b }) = p.handle(&hub, "bob", false, ToServer::TimeBomb { buy_in: 1_000 }).unwrap() else { panic!() };
        assert_eq!(b.id, a.id);
        let Some(ToApp::Tourney { view: c }) = p.handle(&hub, "carol", false, ToServer::TimeBomb { buy_in: 1_000 }).unwrap() else { panic!() };
        assert_eq!(c.id, a.id);
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
}
