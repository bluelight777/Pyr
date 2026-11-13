//! Dominó Cubano's tables in Pyr Casino on this server (0.13.11; see `pyr_poker::domroom`):
//! four seats each for people and bots, in Pairs or Solo, at an ante. What someone puts up for a
//! hand (the ante to eight times it, covering doubles) is taken from their play chips when
//! they're in, and what the hand doesn't use (and their share, if they won) comes back when
//! it's over. Chips on a table are held, like chips at a poker table: if the server stops
//! they're given back when it starts again. As 0.13.28's server runs them, to its words.

use std::collections::BTreeMap;

use pyr_poker::casino::domino::Kind;
use pyr_poker::casino::{Game, Outcome};
use pyr_poker::domroom::{ANTES, DomRoom};
use pyr_poker::net::{ToApp, ToServer};
use pyr_poker::progress::{self, Record};
use pyr_poker::room::{Effect, Output, Random};
use tracing::info;

use super::{Poker, State, progress_chips};
use crate::hub::Hub;

/// The tables always open (0.13.28's).
pub const STANDING: [(&str, Kind, u64); 5] =
    [("Café Habana", Kind::Pairs, 100), ("El Malecón", Kind::Solo, 500), ("Tropicana", Kind::Pairs, 5_000), ("Crown Room", Kind::Pairs, 100_000), ("El Presidente", Kind::Pairs, 1_000_000)];
/// The most tables at once.
pub const MAX_DOM_TABLES: usize = 12;
/// A table someone opened closes when nobody's sat at it or watched it for this long.
const EMPTY_CLOSE_MS: u64 = 30 * 60 * 1000;
const CLOSED: &str = "that table has closed";

#[derive(Default)]
pub struct DomState {
    pub tables: BTreeMap<u64, DomRoom>,
    next: u64,
    /// When each table last had someone at it (or watching).
    used: BTreeMap<u64, u64>,
}

impl DomState {
    /// Chips someone has put up on the tables.
    pub fn at_stake(&self, uid: &str) -> u64 {
        self.tables.values().map(|t| t.at_stake(uid)).sum()
    }
}

impl Poker {
    /// The standing tables, set out the first time anyone comes to dominoes.
    fn dom_standing(st: &mut State, now: u64, rng: &mut dyn Random) {
        if st.dom.next > 0 {
            return;
        }
        for (name, kind, ante) in STANDING {
            st.dom.next += 1;
            let id = st.dom.next;
            st.dom.tables.insert(id, DomRoom::new(id, name, kind, ante, now, rng));
            st.dom.used.insert(id, now);
        }
    }

    fn dom_list(st: &State) -> ToApp {
        ToApp::DomTables { tables: st.dom.tables.values().map(|t| t.summary()).collect() }
    }

    /// A request about dominó tables. What comes back goes to whoever asked; news goes out to
    /// everyone at the table.
    pub(super) fn dom(&self, hub: &Hub, st: &mut State, uid: &str, msg: ToServer, now: u64, rng: &mut dyn Random) -> Result<Option<ToApp>, String> {
        Self::dom_standing(st, now, rng);
        match msg {
            ToServer::DomTables => Ok(Some(Self::dom_list(st))),
            ToServer::DomWatch { table } => {
                if !st.dom.tables.contains_key(&table) {
                    return Err(CLOSED.into());
                }
                // (One table at a time.)
                for t in st.dom.tables.values_mut().filter(|t| t.id != table) {
                    t.unwatch(uid);
                }
                st.dom.used.insert(table, now);
                Ok(Some(st.dom.tables.get_mut(&table).unwrap().watch(uid, now)))
            }
            ToServer::DomUnwatch { table } => {
                if let Some(t) = st.dom.tables.get_mut(&table) {
                    t.unwatch(uid);
                }
                Ok(None)
            }
            ToServer::DomSit { table, seat } => {
                if let Some(t) = st.dom.tables.values().find(|t| t.seat_of(uid).is_some()) {
                    return Err(if t.id == table { "you're sitting here already".into() } else { format!("you're sitting at {} already: stand up there first", t.name) });
                }
                let (rec, _) = hub.db.poker_record(uid);
                let name = Self::name_of(hub, uid);
                let t = st.dom.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.sit(uid, &name, Self::look(&rec), seat, now)?;
                self.flush_dom(hub, st, out);
                Ok(None)
            }
            ToServer::DomLeave { table } => {
                let t = st.dom.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.leave(uid, now);
                self.flush_dom(hub, st, out);
                Ok(None)
            }
            ToServer::DomReady { table, chips, seed } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                if chips > rec.chips {
                    return Err(format!("that's {} chips, and you have {}", progress_chips(chips), progress_chips(rec.chips)));
                }
                let t = st.dom.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.ready(uid, chips, seed, now)?;
                Self::dom_take(hub, st, uid, &mut rec, chips)?;
                self.flush_dom(hub, st, out);
                Ok(None)
            }
            ToServer::DomMove { table, mv } => {
                let t = st.dom.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.act(uid, mv, now)?;
                self.flush_dom(hub, st, out);
                Ok(None)
            }
            ToServer::DomBot { table, seat, add } => {
                let t = st.dom.tables.get_mut(&table).ok_or(CLOSED)?;
                if !t.knows(uid) {
                    return Err("sit at the table (or watch it) first".into());
                }
                let out = if add { t.add_bot(seat, now, rng)? } else { t.remove_bot(seat, now)? };
                self.flush_dom(hub, st, out);
                Ok(None)
            }
            ToServer::DomNew { kind, ante } => {
                if !ANTES.contains(&ante) {
                    return Err("pick one of the table antes".into());
                }
                if st.dom.tables.len() >= MAX_DOM_TABLES {
                    return Err("this server has as many dominó tables as it keeps: join one".into());
                }
                st.dom.next += 1;
                let id = st.dom.next;
                let mut t = DomRoom::new(id, &format!("Mesa {id}"), kind, ante, now, rng);
                for other in st.dom.tables.values_mut() {
                    other.unwatch(uid);
                }
                let view = t.watch(uid, now);
                st.dom.tables.insert(id, t);
                st.dom.used.insert(id, now);
                info!(table = id, "dominó: a table was opened");
                Ok(Some(view))
            }
            _ => Err("that isn't about a dominó table".into()),
        }
    }

    /// Chips a person put up for a hand, taken from them (held while they're on the table).
    fn dom_take(hub: &Hub, st: &State, uid: &str, rec: &mut Record, chips: u64) -> Result<(), String> {
        if chips == 0 {
            return Ok(());
        }
        rec.chips -= chips;
        let held = st.held(uid);
        hub.db.set_poker_record(uid, rec, held).map_err(|e| e.to_string())?;
        hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, rec, held), gains: None });
        Ok(())
    }

    /// Sends a dominó table's news on, pays what came back, and counts the hands.
    pub(crate) fn flush_dom(&self, hub: &Hub, st: &mut State, mut out: Output) {
        let effects = std::mem::take(&mut out.effects);
        self.flush(hub, st, 0, out);
        for e in effects {
            match e {
                Effect::DomHand { uid, outcome } => Self::dom_counted(hub, st, &uid, outcome),
                e => self.flush(hub, st, 0, Output { effects: vec![e], ..Output::default() }),
            }
        }
    }

    /// A hand over for someone (paid already): their records (experience, achievements, their
    /// statistics) and the casino's (the game's statistics; a big win told to everyone there).
    fn dom_counted(hub: &Hub, st: &mut State, uid: &str, outcome: pyr_poker::casino::domino::Outcome) {
        let o = Outcome::Domino(outcome);
        let (mut rec, _) = hub.db.poker_record(uid);
        let g = progress::apply_casino(&mut rec, &o, super::today());
        let held = st.held(uid);
        let _ = hub.db.set_poker_record(uid, &rec, held);
        let gains = g.any().then(|| g.view(o.paid() as i64 - o.wagered() as i64));
        hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, &rec, held), gains });
        Self::round_counted(hub, st, uid, &o);
        let _ = Game::Domino;
    }

    /// The tables' clock: hands move on, and tables someone opened that nobody's used close.
    pub(super) fn dom_tick(&self, hub: &Hub, st: &mut State, now: u64, rng: &mut dyn Random) {
        let ids: Vec<u64> = st.dom.tables.keys().copied().collect();
        let mut close = Vec::new();
        for id in ids {
            let Some(t) = st.dom.tables.get_mut(&id) else { continue };
            let out = t.tick(now, rng);
            let used = t.in_use();
            if !out.msgs.is_empty() || !out.effects.is_empty() {
                self.flush_dom(hub, st, out);
            }
            if used {
                st.dom.used.insert(id, now);
            } else if id > STANDING.len() as u64 && now.saturating_sub(*st.dom.used.get(&id).unwrap_or(&now)) > EMPTY_CLOSE_MS {
                close.push(id);
            }
        }
        for id in close {
            if let Some(mut t) = st.dom.tables.remove(&id) {
                let out = t.close();
                self.flush_dom(hub, st, out);
                st.dom.used.remove(&id);
                info!(table = id, "dominó: a table nobody used for half an hour closed");
            }
        }
    }

    /// Someone's gone: they stand up from the tables (what they put up comes back; a hand
    /// they're in is played for them to its end) and stop watching.
    pub(super) fn dom_gone(&self, hub: &Hub, st: &mut State, uid: &str, now: u64) {
        let ids: Vec<u64> = st.dom.tables.iter().filter(|(_, t)| t.knows(uid)).map(|(id, _)| *id).collect();
        for id in ids {
            let Some(t) = st.dom.tables.get_mut(&id) else { continue };
            t.unwatch(uid);
            if t.seat_of(uid).is_some() {
                let out = t.leave(uid, now);
                self.flush_dom(hub, st, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::casino::domino::Move;
    use pyr_poker::net::{DomStage, DomView};

    use super::*;
    use crate::db::Db;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-dom-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ask(hub: &Hub, uid: &str, msg: ToServer) -> Result<Option<ToApp>, String> {
        hub.poker.handle_from(hub, uid, false, pyr_poker::net::CASINO_01311, msg)
    }

    fn view(hub: &Hub, uid: &str, table: u64) -> DomView {
        match ask(hub, uid, ToServer::DomWatch { table }).unwrap() {
            Some(ToApp::Dom { view }) => view,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_standing_tables_and_what_older_apps_are_told() {
        let hub = hub();
        let Some(ToApp::DomTables { tables }) = ask(&hub, "olive", ToServer::DomTables).unwrap() else { panic!() };
        let names: Vec<&str> = tables.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Café Habana", "El Malecón", "Tropicana", "Crown Room", "El Presidente"]);
        assert_eq!(hub.poker.handle_from(&hub, "olive", false, pyr_poker::net::CASINO_0137, ToServer::DomTables).unwrap_err(), "dominó tables need a newer Pyr");
        assert_eq!(ask(&hub, "olive", ToServer::DomNew { kind: Kind::Solo, ante: 300 }).unwrap_err(), "pick one of the table antes");
        let Some(ToApp::Dom { view }) = ask(&hub, "olive", ToServer::DomNew { kind: Kind::Solo, ante: 250 }).unwrap() else { panic!() };
        assert_eq!(view.name, format!("Mesa {}", view.id));
    }

    #[test]
    fn putting_up_chips_takes_and_holds_them_and_standing_up_gives_them_back() {
        let hub = hub();
        let start = hub.db.poker_record("olive").0.chips;
        view(&hub, "olive", 2);
        ask(&hub, "olive", ToServer::DomSit { table: 2, seat: 0 }).unwrap();
        assert!(ask(&hub, "olive", ToServer::DomSit { table: 1, seat: 0 }).is_err(), "one table at a time");
        assert_eq!(ask(&hub, "olive", ToServer::DomReady { table: 2, chips: 4_001, seed: [1; 32] }).unwrap_err(), "this table's ante is 500; put up 500 to 4000");
        assert!(ask(&hub, "olive", ToServer::DomReady { table: 2, chips: start + 1, seed: [1; 32] }).is_err());
        ask(&hub, "olive", ToServer::DomReady { table: 2, chips: 1_000, seed: [1; 32] }).unwrap();
        assert_eq!(hub.db.poker_record("olive"), (hub.db.poker_record("olive").0, 1_000));
        assert_eq!(hub.db.poker_record("olive").0.chips, start - 1_000);
        let v = view(&hub, "olive", 2);
        assert_eq!(v.stage, DomStage::Gathering);
        assert_eq!(v.seats[0].as_ref().map(|s| (s.ready, s.escrow)), Some((true, 1_000)));
        // (Before the hand's dealt: up at once, everything back.)
        ask(&hub, "olive", ToServer::DomLeave { table: 2 }).unwrap();
        assert_eq!(hub.db.poker_record("olive").0.chips, start);
        assert_eq!(hub.db.poker_record("olive").1, 0);
        let _ = Move::Pass;
    }
}
