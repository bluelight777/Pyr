//! Craps tables in Pyr Casino on this server (see `pyr_poker::crapsroom`): five seats each for
//! people and bots, the dice going round the table. Bets are taken from people's play chips as
//! they go down (and given back as they come off), and what the rolls pay is paid as they land.
//! Chips on a table are held, like chips at a poker table: if the server stops they're given
//! back when it starts again.

use std::collections::BTreeMap;

use pyr_poker::casino::{Game, Outcome, craps};
use pyr_poker::crapsroom::CrapsRoom;
use pyr_poker::net::{GainsView, ToApp, ToServer};
use pyr_poker::progress::{self, Record};
use pyr_poker::room::{Effect, Output, Random};
use tracing::info;

use super::casino::friday_share;
use super::{Poker, State, progress_chips};
use crate::hub::Hub;

/// The tables every server has (name, least and most bet). They never close.
pub const STANDING: [(&str, u64, u64); 2] = [("Lucky Sevens", 10, 1_000), ("High Rollers", 500, 25_000)];
const CLOSED: &str = "that table has closed";

#[derive(Default)]
pub struct CrapsState {
    pub tables: BTreeMap<u64, CrapsRoom>,
    next: u64,
}

impl CrapsState {
    /// Chips someone has on the tables (on the layout, or still working since they stood up).
    pub fn at_stake(&self, uid: &str) -> u64 {
        self.tables.values().map(|t| t.at_stake(uid)).sum()
    }
}

impl Poker {
    /// The standing tables, set out the first time anyone comes to craps.
    fn craps_standing(st: &mut State, now: u64, rng: &mut dyn Random) {
        if st.craps.next > 0 {
            return;
        }
        for (name, lo, hi) in STANDING {
            st.craps.next += 1;
            let id = st.craps.next;
            st.craps.tables.insert(id, CrapsRoom::new(id, name, lo, hi, now, rng));
        }
    }

    /// A request about craps tables. What comes back goes to whoever asked; news goes out to
    /// everyone at the table.
    pub(super) fn craps_tables(&self, hub: &Hub, st: &mut State, uid: &str, msg: ToServer, now: u64, rng: &mut dyn Random) -> Result<Option<ToApp>, String> {
        Self::craps_standing(st, now, rng);
        match msg {
            ToServer::CrapsTables => Ok(Some(ToApp::CrapsTables { tables: st.craps.tables.values().map(|t| t.summary()).collect() })),
            ToServer::CrapsWatch { table } => {
                if !st.craps.tables.contains_key(&table) {
                    return Err(CLOSED.into());
                }
                // (One table at a time.)
                for t in st.craps.tables.values_mut().filter(|t| t.id != table) {
                    t.unwatch(uid);
                }
                Ok(Some(st.craps.tables.get_mut(&table).unwrap().watch(uid, now)))
            }
            ToServer::CrapsUnwatch { table } => {
                if let Some(t) = st.craps.tables.get_mut(&table) {
                    t.unwatch(uid);
                }
                Ok(None)
            }
            ToServer::CrapsSit { table, seat } => {
                if let Some(t) = st.craps.tables.values().find(|t| t.seat_of(uid).is_some()) {
                    return Err(if t.id == table { "you're sitting here already".into() } else { format!("you're sitting at {} already: stand up there first", t.name) });
                }
                let (rec, _) = hub.db.poker_record(uid);
                let name = Self::name_of(hub, uid);
                let t = st.craps.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.sit(uid, &name, Self::look(&rec), seat, now)?;
                self.flush_craps(hub, st, out);
                Ok(None)
            }
            ToServer::CrapsLeave { table } => {
                let t = st.craps.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.leave(uid, now);
                self.flush_craps(hub, st, out);
                Ok(None)
            }
            ToServer::CrapsBets { table, roll, bets, seed, ready } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                let t = st.craps.tables.get_mut(&table).ok_or(CLOSED)?;
                if roll != t.roll_no() {
                    return Err("the dice rolled just then: your bets are as the table shows them".into());
                }
                let cost = t.cost(uid, &bets)?;
                if cost > 0 && cost as u64 > rec.chips {
                    return Err(format!("that's {} more chips on the layout, and you have {}", progress_chips(cost as u64), progress_chips(rec.chips)));
                }
                let out = t.place(uid, bets, seed, ready, now)?;
                if cost > 0 {
                    Self::craps_take(hub, st, uid, &mut rec, cost as u64)?;
                }
                self.flush_craps(hub, st, out);
                Ok(None)
            }
            ToServer::CrapsThrow { table } => {
                let t = st.craps.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.throw(uid, now, rng)?;
                self.flush_craps(hub, st, out);
                Ok(None)
            }
            ToServer::CrapsBot { table, seat, add } => {
                let t = st.craps.tables.get_mut(&table).ok_or(CLOSED)?;
                if !t.knows(uid) {
                    return Err("sit at the table (or watch it) first".into());
                }
                let out = if add { t.add_bot(seat, now, rng)? } else { t.remove_bot(seat, now)? };
                self.flush_craps(hub, st, out);
                Ok(None)
            }
            _ => Err("that isn't about a craps table".into()),
        }
    }

    /// Chips a person put on the layout, taken from them.
    fn craps_take(hub: &Hub, st: &State, uid: &str, rec: &mut Record, chips: u64) -> Result<(), String> {
        rec.chips -= chips;
        Self::add_rake(hub, friday_share(Game::Craps, chips));
        let held = st.held(uid);
        hub.db.set_poker_record(uid, rec, held).map_err(|e| e.to_string())?;
        hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, rec, held), gains: None });
        Ok(())
    }

    /// Sends a craps table's news on, pays what the rolls paid, and counts them.
    fn flush_craps(&self, hub: &Hub, st: &mut State, mut out: Output) {
        let effects = std::mem::take(&mut out.effects);
        self.flush(hub, st, 0, out);
        for e in effects {
            match e {
                Effect::CrapsRoll { uid, outcome } => Self::craps_counted(hub, st, &uid, outcome),
                e => self.flush(hub, st, 0, Output { effects: vec![e], ..Output::default() }),
            }
        }
    }

    /// A roll that decided some of someone's bets (paid already): their records (experience,
    /// achievements, statistics) and the casino's (the game's statistics; a big win told).
    fn craps_counted(hub: &Hub, st: &mut State, uid: &str, outcome: craps::Outcome) {
        let o = Outcome::Craps(outcome);
        let (mut rec, _) = hub.db.poker_record(uid);
        let g = progress::apply_casino(&mut rec, &o);
        let held = st.held(uid);
        let _ = hub.db.set_poker_record(uid, &rec, held);
        let gains = (!g.new_achievements.is_empty() || g.level_up.is_some()).then(|| GainsView {
            xp: g.xp,
            achievements: g.new_achievements.iter().map(|s| s.to_string()).collect(),
            level_up: g.level_up,
            net: o.paid() as i64 - o.wagered() as i64,
        });
        hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, &rec, held), gains });
        Self::round_counted(hub, st, uid, &o);
    }

    /// The tables' clock: the dice go on rolling.
    pub(super) fn craps_tick(&self, hub: &Hub, st: &mut State, now: u64, rng: &mut dyn Random) {
        let ids: Vec<u64> = st.craps.tables.keys().copied().collect();
        for id in ids {
            let Some(t) = st.craps.tables.get_mut(&id) else { continue };
            let out = t.tick(now, rng);
            if !out.msgs.is_empty() || !out.effects.is_empty() {
                self.flush_craps(hub, st, out);
            }
        }
    }

    /// Someone's gone: they stand up from the tables (what may come off comes back; a pass line
    /// with a point on keeps working for them) and stop watching.
    pub(super) fn craps_gone(&self, hub: &Hub, st: &mut State, uid: &str, now: u64) {
        let ids: Vec<u64> = st.craps.tables.iter().filter(|(_, t)| t.knows(uid)).map(|(id, _)| *id).collect();
        for id in ids {
            let Some(t) = st.craps.tables.get_mut(&id) else { continue };
            t.unwatch(uid);
            if t.seat_of(uid).is_some() {
                let out = t.leave(uid, now);
                self.flush_craps(hub, st, out);
                info!(table = id, "craps: someone who went was stood up");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::casino::craps::Bets;
    use pyr_poker::net::{CrapsStage, CrapsView};

    use super::*;
    use crate::db::Db;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-craps-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ask(hub: &Hub, uid: &str, msg: ToServer) -> Result<Option<ToApp>, String> {
        hub.poker.handle(hub, uid, false, msg)
    }

    fn view(hub: &Hub, uid: &str, table: u64) -> CrapsView {
        match ask(hub, uid, ToServer::CrapsWatch { table }).unwrap() {
            Some(ToApp::Craps { view }) => view,
            other => panic!("{other:?}"),
        }
    }

    /// Bets are taken from people's chips as they go down and held while they're on the
    /// layout; coming off, they come back; the rolls pay; standing up gives back what may come
    /// off; someone gone is stood up.
    #[test]
    fn bets_are_taken_held_and_paid() {
        let hub = hub();
        // (Fewer chips than a full layout at the table's limits costs.)
        let (mut rec, held) = hub.db.poker_record("ann");
        rec.chips = 5_000;
        hub.db.set_poker_record("ann", &rec, held).unwrap();
        let start = hub.db.poker_record("ann").0.chips;
        let tables = match ask(&hub, "ann", ToServer::CrapsTables).unwrap() {
            Some(ToApp::CrapsTables { tables }) => tables,
            other => panic!("{other:?}"),
        };
        assert_eq!(tables.len(), STANDING.len());
        let id = tables[0].id;
        ask(&hub, "ann", ToServer::CrapsSit { table: id, seat: 2 }).unwrap();
        assert!(ask(&hub, "ann", ToServer::CrapsSit { table: 2, seat: 0 }).is_err(), "one table at a time");
        ask(&hub, "ann", ToServer::CrapsBot { table: id, seat: 0, add: true }).unwrap();
        // Too much is refused, and nothing's taken: more than she has (twelve thousand), or more
        // than the table takes on one bet.
        let too_much = Bets { pass: 1_000, field: 1_000, place: [1_000; 6], hard: [1_000; 4], ..Bets::default() };
        assert!(ask(&hub, "ann", ToServer::CrapsBets { table: id, roll: view(&hub, "ann", id).roll_no, bets: too_much, seed: [7; 32], ready: false }).is_err());
        let over_limit = Bets { pass: 1_001, ..Bets::default() };
        assert!(ask(&hub, "ann", ToServer::CrapsBets { table: id, roll: view(&hub, "ann", id).roll_no, bets: over_limit, seed: [7; 32], ready: false }).is_err());
        assert_eq!(hub.db.poker_record("ann").0.chips, start);
        let bets = Bets { pass: 100, field: 20, ..Bets::default() };
        ask(&hub, "ann", ToServer::CrapsBets { table: id, roll: view(&hub, "ann", id).roll_no, bets: bets.clone(), seed: [7; 32], ready: false }).unwrap();
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 120, 120));
        // The field bet comes off again.
        ask(&hub, "ann", ToServer::CrapsBets { table: id, roll: view(&hub, "ann", id).roll_no, bets: Bets { pass: 100, ..Bets::default() }, seed: [8; 32], ready: true }).unwrap();
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 100, 100));
        // Roll it: the shooter (Ann sat first) throws.
        let v = view(&hub, "ann", id);
        assert_eq!(v.stage, CrapsStage::Betting);
        if v.shooter == Some(2) {
            ask(&hub, "ann", ToServer::CrapsThrow { table: id }).unwrap();
        }
        let mut rolled = None;
        for _ in 0..1_000 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.poker.tick(&hub);
            let v = view(&hub, "ann", id);
            if let Some(r) = &v.last {
                pyr_poker::crapsroom::check(r).unwrap();
                rolled = Some((r.clone(), v));
                break;
            }
        }
        let (r, v) = rolled.expect("the dice were thrown");
        let (rec, held) = hub.db.poker_record("ann");
        let sum = r.dice.0 + r.dice.1;
        match sum {
            7 | 11 => assert_eq!((rec.chips, held), (start + 100, 0)),
            2 | 3 | 12 => assert_eq!((rec.chips, held), (start - 100, 0)),
            p => {
                assert_eq!((rec.chips, held, v.point), (start - 100, 100, Some(p)));
                // A point: the pass line can't come off, and standing up leaves it working.
                assert!(ask(&hub, "ann", ToServer::CrapsBets { table: id, roll: view(&hub, "ann", id).roll_no, bets: Bets::default(), seed: [9; 32], ready: false }).is_err());
                hub.poker.gone(&hub, "ann");
                assert_eq!(hub.db.poker_record("ann").1, 100, "still held while it works");
            }
        }
        assert!(rec.casino.get("craps").is_some_and(|c| c.rounds >= 1) || !matches!(sum, 2 | 3 | 7 | 11 | 12));
    }
}
