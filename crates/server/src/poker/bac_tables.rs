//! Baccarat tables in Pyr Casino on this server (see `pyr_poker::bacroom`): five seats each for
//! people and bots, a coup every few seconds. Bets are taken from people's play chips as they go
//! down (and given back as they come off), and what each coup pays is paid as it's dealt. Chips on
//! a table are held, like chips at a poker table: if the server stops they're given back when it
//! starts again.

use std::collections::BTreeMap;

use pyr_poker::bacroom::BacRoom;
use pyr_poker::casino::{Game, Outcome, baccarat};
use pyr_poker::net::{GainsView, ToApp, ToServer};
use pyr_poker::progress::{self, Record};
use pyr_poker::room::{Effect, Output, Random};
use tracing::info;

use super::casino::friday_share;
use super::{Poker, State, progress_chips};
use crate::hub::Hub;

/// The tables every server has (name, least and most bet). They never close.
pub const STANDING: [(&str, u64, u64); 2] = [("Salon Privé", 10, 1_000), ("Black Pearl", 500, 25_000)];
const CLOSED: &str = "that table has closed";

#[derive(Default)]
pub struct BacState {
    pub tables: BTreeMap<u64, BacRoom>,
    next: u64,
}

impl BacState {
    /// Chips someone has bet on the tables' coming coups.
    pub fn at_stake(&self, uid: &str) -> u64 {
        self.tables.values().map(|t| t.at_stake(uid)).sum()
    }
}

impl Poker {
    /// The standing tables, set out the first time anyone comes to baccarat.
    fn bac_standing(st: &mut State, now: u64, rng: &mut dyn Random) {
        if st.bac.next > 0 {
            return;
        }
        for (name, lo, hi) in STANDING {
            st.bac.next += 1;
            let id = st.bac.next;
            st.bac.tables.insert(id, BacRoom::new(id, name, lo, hi, now, rng));
        }
    }

    /// A request about baccarat tables. What comes back goes to whoever asked; news goes out to
    /// everyone at the table.
    pub(super) fn bac_tables(&self, hub: &Hub, st: &mut State, uid: &str, msg: ToServer, now: u64, rng: &mut dyn Random) -> Result<Option<ToApp>, String> {
        Self::bac_standing(st, now, rng);
        match msg {
            ToServer::BacTables => Ok(Some(ToApp::BacTables { tables: st.bac.tables.values().map(|t| t.summary()).collect() })),
            ToServer::BacWatch { table } => {
                if !st.bac.tables.contains_key(&table) {
                    return Err(CLOSED.into());
                }
                // (One table at a time.)
                for t in st.bac.tables.values_mut().filter(|t| t.id != table) {
                    t.unwatch(uid);
                }
                Ok(Some(st.bac.tables.get_mut(&table).unwrap().watch(uid, now)))
            }
            ToServer::BacUnwatch { table } => {
                if let Some(t) = st.bac.tables.get_mut(&table) {
                    t.unwatch(uid);
                }
                Ok(None)
            }
            ToServer::BacSit { table, seat } => {
                if let Some(t) = st.bac.tables.values().find(|t| t.seat_of(uid).is_some()) {
                    return Err(if t.id == table { "you're sitting here already".into() } else { format!("you're sitting at {} already: stand up there first", t.name) });
                }
                let (rec, _) = hub.db.poker_record(uid);
                let name = Self::name_of(hub, uid);
                let t = st.bac.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.sit(uid, &name, Self::look(&rec), seat, now)?;
                self.flush_bac(hub, st, out);
                Ok(None)
            }
            ToServer::BacLeave { table } => {
                let t = st.bac.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.leave(uid, now);
                self.flush_bac(hub, st, out);
                Ok(None)
            }
            ToServer::BacBets { table, coup, bets, seed, ready } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                let t = st.bac.tables.get_mut(&table).ok_or(CLOSED)?;
                if coup != t.coup_no() {
                    return Err("the cards came just then: your bets are as the table shows them".into());
                }
                let cost = t.cost(uid, &bets)?;
                if cost > 0 && cost as u64 > rec.chips {
                    return Err(format!("that's {} more chips on the table, and you have {}", progress_chips(cost as u64), progress_chips(rec.chips)));
                }
                let out = t.place(uid, bets, seed, ready, now)?;
                if cost > 0 {
                    Self::bac_take(hub, st, uid, &mut rec, cost as u64)?;
                }
                self.flush_bac(hub, st, out);
                Ok(None)
            }
            ToServer::BacBot { table, seat, add } => {
                let t = st.bac.tables.get_mut(&table).ok_or(CLOSED)?;
                if !t.knows(uid) {
                    return Err("sit at the table (or watch it) first".into());
                }
                let out = if add { t.add_bot(seat, now, rng)? } else { t.remove_bot(seat, now)? };
                self.flush_bac(hub, st, out);
                Ok(None)
            }
            _ => Err("that isn't about a baccarat table".into()),
        }
    }

    /// Chips a person bet, taken from them.
    fn bac_take(hub: &Hub, st: &State, uid: &str, rec: &mut Record, chips: u64) -> Result<(), String> {
        rec.chips -= chips;
        Self::add_rake(hub, friday_share(Game::Baccarat, chips));
        let held = st.held(uid);
        hub.db.set_poker_record(uid, rec, held).map_err(|e| e.to_string())?;
        hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, rec, held), gains: None });
        Ok(())
    }

    /// Sends a baccarat table's news on, pays what the coups paid, and counts them.
    fn flush_bac(&self, hub: &Hub, st: &mut State, mut out: Output) {
        let effects = std::mem::take(&mut out.effects);
        self.flush(hub, st, 0, out);
        for e in effects {
            match e {
                Effect::BacCoup { uid, outcome } => Self::bac_counted(hub, st, &uid, outcome),
                e => self.flush(hub, st, 0, Output { effects: vec![e], ..Output::default() }),
            }
        }
    }

    /// A coup over for someone (paid already): their records (experience, achievements,
    /// statistics) and the casino's (the game's statistics; a big win told).
    fn bac_counted(hub: &Hub, st: &mut State, uid: &str, outcome: baccarat::Outcome) {
        let o = Outcome::Baccarat(outcome);
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

    /// The tables' clock: the coups go on.
    pub(super) fn bac_tick(&self, hub: &Hub, st: &mut State, now: u64, rng: &mut dyn Random) {
        let ids: Vec<u64> = st.bac.tables.keys().copied().collect();
        for id in ids {
            let Some(t) = st.bac.tables.get_mut(&id) else { continue };
            let out = t.tick(now, rng);
            if !out.msgs.is_empty() || !out.effects.is_empty() {
                self.flush_bac(hub, st, out);
            }
        }
    }

    /// Someone's gone: they stand up from the tables (their bets come back) and stop watching.
    pub(super) fn bac_gone(&self, hub: &Hub, st: &mut State, uid: &str, now: u64) {
        let ids: Vec<u64> = st.bac.tables.iter().filter(|(_, t)| t.knows(uid)).map(|(id, _)| *id).collect();
        for id in ids {
            let Some(t) = st.bac.tables.get_mut(&id) else { continue };
            t.unwatch(uid);
            if t.seat_of(uid).is_some() {
                let out = t.leave(uid, now);
                self.flush_bac(hub, st, out);
                info!(table = id, "baccarat: someone who went was stood up");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::casino::baccarat::Bets;
    use pyr_poker::net::{BacStage, BacView};

    use super::*;
    use crate::db::Db;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-bac-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ask(hub: &Hub, uid: &str, msg: ToServer) -> Result<Option<ToApp>, String> {
        hub.poker.handle(hub, uid, false, msg)
    }

    fn view(hub: &Hub, uid: &str, table: u64) -> BacView {
        match ask(hub, uid, ToServer::BacWatch { table }).unwrap() {
            Some(ToApp::Bac { view }) => view,
            other => panic!("{other:?}"),
        }
    }

    /// Bets are taken from people's chips and held till the coup; coming off, they come back;
    /// the coup pays as the game does; standing up gives back what's bet; someone gone is stood
    /// up.
    #[test]
    fn bets_are_taken_held_and_paid() {
        let hub = hub();
        let start = hub.db.poker_record("ann").0.chips;
        let tables = match ask(&hub, "ann", ToServer::BacTables).unwrap() {
            Some(ToApp::BacTables { tables }) => tables,
            other => panic!("{other:?}"),
        };
        assert_eq!(tables.len(), STANDING.len());
        let id = tables[0].id;
        ask(&hub, "ann", ToServer::BacSit { table: id, seat: 2 }).unwrap();
        assert!(ask(&hub, "ann", ToServer::BacSit { table: 2, seat: 0 }).is_err(), "one table at a time");
        ask(&hub, "ann", ToServer::BacBot { table: id, seat: 4, add: true }).unwrap();
        assert!(ask(&hub, "ann", ToServer::BacBets { table: id, coup: view(&hub, "ann", id).coup_no, bets: Bets { banker: start + 1, ..Bets::default() }, seed: [7; 32], ready: false }).is_err());
        ask(&hub, "ann", ToServer::BacBets { table: id, coup: view(&hub, "ann", id).coup_no, bets: Bets { banker: 100, tie: 20, ..Bets::default() }, seed: [7; 32], ready: false }).unwrap();
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 120, 120));
        ask(&hub, "ann", ToServer::BacBets { table: id, coup: view(&hub, "ann", id).coup_no, bets: Bets { banker: 100, ..Bets::default() }, seed: [8; 32], ready: true }).unwrap();
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 100, 100));
        let mut dealt = None;
        for _ in 0..1_000 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.poker.tick(&hub);
            let v = view(&hub, "ann", id);
            if let Some(r) = &v.last {
                pyr_poker::bacroom::check(r).unwrap();
                dealt = Some(r.clone());
                break;
            }
        }
        let r = dealt.expect("the coup was dealt");
        let (_, _, _, paid) = baccarat::settle(&Bets { banker: 100, ..Bets::default() }, &r.player, &r.banker);
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 100 + paid, 0));
        assert_eq!(rec.casino.get("baccarat").map(|c| c.rounds), Some(1));
        // A bet down, then gone: it comes back.
        while view(&hub, "ann", id).stage != BacStage::Betting {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.poker.tick(&hub);
        }
        ask(&hub, "ann", ToServer::BacBets { table: id, coup: view(&hub, "ann", id).coup_no, bets: Bets { player: 50, ..Bets::default() }, seed: [9; 32], ready: false }).unwrap();
        hub.poker.gone(&hub, "ann");
        let (after, held) = hub.db.poker_record("ann");
        assert_eq!((after.chips, held), (rec.chips, 0));
    }
}
