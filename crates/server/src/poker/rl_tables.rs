//! Roulette tables in Pyr Casino on this server (see `pyr_poker::rlroom`): five seats each for
//! people and bots round one wheel, a spin every few seconds. Chips are taken from people's play
//! chips as they go down (and given back as they come off), and what each spin pays is paid as
//! the ball lands. Chips on a table are held, like chips at a poker table: if the server stops
//! they're given back when it starts again.

use std::collections::BTreeMap;

use pyr_poker::rlroom::RlRoom;
use pyr_poker::casino::{Game, Outcome, roulette};
use pyr_poker::net::{GainsView, ToApp, ToServer};
use pyr_poker::progress::{self, Record};
use pyr_poker::room::{Effect, Output, Random};
use tracing::info;

use super::casino::friday_share;
use super::{Poker, State, progress_chips};
use crate::hub::Hub;

/// The tables every server has (name, least and most bet). They never close.
pub const STANDING: [(&str, u64, u64); 2] = [("Monte Carlo", 10, 1_000), ("Riviera High Rollers", 100, 25_000)];
const CLOSED: &str = "that table has closed";

#[derive(Default)]
pub struct RlState {
    pub tables: BTreeMap<u64, RlRoom>,
    next: u64,
}

impl RlState {
    /// Chips someone has on the tables' coming spins.
    pub fn at_stake(&self, uid: &str) -> u64 {
        self.tables.values().map(|t| t.at_stake(uid)).sum()
    }
}

impl Poker {
    /// The standing tables, set out the first time anyone comes to roulette.
    fn rl_standing(st: &mut State, now: u64, rng: &mut dyn Random) {
        if st.rl.next > 0 {
            return;
        }
        for (name, lo, hi) in STANDING {
            st.rl.next += 1;
            let id = st.rl.next;
            st.rl.tables.insert(id, RlRoom::new(id, name, lo, hi, now, rng));
        }
    }

    /// A request about roulette tables. What comes back goes to whoever asked; news goes out to
    /// everyone at the table.
    pub(super) fn rl_tables(&self, hub: &Hub, st: &mut State, uid: &str, msg: ToServer, now: u64, rng: &mut dyn Random) -> Result<Option<ToApp>, String> {
        Self::rl_standing(st, now, rng);
        match msg {
            ToServer::RlTables => Ok(Some(ToApp::RlTables { tables: st.rl.tables.values().map(|t| t.summary()).collect() })),
            ToServer::RlWatch { table } => {
                if !st.rl.tables.contains_key(&table) {
                    return Err(CLOSED.into());
                }
                // (One table at a time.)
                for t in st.rl.tables.values_mut().filter(|t| t.id != table) {
                    t.unwatch(uid);
                }
                Ok(Some(st.rl.tables.get_mut(&table).unwrap().watch(uid, now)))
            }
            ToServer::RlUnwatch { table } => {
                if let Some(t) = st.rl.tables.get_mut(&table) {
                    t.unwatch(uid);
                }
                Ok(None)
            }
            ToServer::RlSit { table, seat } => {
                if let Some(t) = st.rl.tables.values().find(|t| t.seat_of(uid).is_some()) {
                    return Err(if t.id == table { "you're sitting here already".into() } else { format!("you're sitting at {} already: stand up there first", t.name) });
                }
                let (rec, _) = hub.db.poker_record(uid);
                let name = Self::name_of(hub, uid);
                let t = st.rl.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.sit(uid, &name, Self::look(&rec), seat, now)?;
                self.flush_rl(hub, st, out);
                Ok(None)
            }
            ToServer::RlLeave { table } => {
                let t = st.rl.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.leave(uid, now);
                self.flush_rl(hub, st, out);
                Ok(None)
            }
            ToServer::RlBets { table, spin, chips, seed, ready } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                let t = st.rl.tables.get_mut(&table).ok_or(CLOSED)?;
                if spin != t.spin_no() {
                    return Err("the ball went just then: your chips are as the table shows them".into());
                }
                let cost = t.cost(uid, &chips)?;
                if cost > 0 && cost as u64 > rec.chips {
                    return Err(format!("that's {} more chips on the table, and you have {}", progress_chips(cost as u64), progress_chips(rec.chips)));
                }
                let out = t.place(uid, chips, seed, ready, now)?;
                if cost > 0 {
                    Self::rl_take(hub, st, uid, &mut rec, cost as u64)?;
                }
                self.flush_rl(hub, st, out);
                Ok(None)
            }
            ToServer::RlBot { table, seat, add } => {
                let t = st.rl.tables.get_mut(&table).ok_or(CLOSED)?;
                if !t.knows(uid) {
                    return Err("sit at the table (or watch it) first".into());
                }
                let out = if add { t.add_bot(seat, now, rng)? } else { t.remove_bot(seat, now)? };
                self.flush_rl(hub, st, out);
                Ok(None)
            }
            _ => Err("that isn't about a roulette table".into()),
        }
    }

    /// Chips a person bet, taken from them.
    fn rl_take(hub: &Hub, st: &State, uid: &str, rec: &mut Record, chips: u64) -> Result<(), String> {
        rec.chips -= chips;
        Self::add_rake(hub, friday_share(Game::Roulette, chips));
        let held = st.held(uid);
        hub.db.set_poker_record(uid, rec, held).map_err(|e| e.to_string())?;
        hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, rec, held), gains: None });
        Ok(())
    }

    /// Sends a roulette table's news on, pays what the spins paid, and counts them.
    fn flush_rl(&self, hub: &Hub, st: &mut State, mut out: Output) {
        let effects = std::mem::take(&mut out.effects);
        self.flush(hub, st, 0, out);
        for e in effects {
            match e {
                Effect::RlSpin { uid, outcome } => Self::rl_counted(hub, st, &uid, outcome),
                e => self.flush(hub, st, 0, Output { effects: vec![e], ..Output::default() }),
            }
        }
    }

    /// A spin over for someone (paid already): their records (experience, achievements,
    /// statistics) and the casino's (the game's statistics; a big win told).
    fn rl_counted(hub: &Hub, st: &mut State, uid: &str, outcome: roulette::Outcome) {
        let o = Outcome::Roulette(outcome);
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

    /// The tables' clock: the spins go on.
    pub(super) fn rl_tick(&self, hub: &Hub, st: &mut State, now: u64, rng: &mut dyn Random) {
        let ids: Vec<u64> = st.rl.tables.keys().copied().collect();
        for id in ids {
            let Some(t) = st.rl.tables.get_mut(&id) else { continue };
            let out = t.tick(now, rng);
            if !out.msgs.is_empty() || !out.effects.is_empty() {
                self.flush_rl(hub, st, out);
            }
        }
    }

    /// Someone's gone: they stand up from the tables (their bets come back) and stop watching.
    pub(super) fn rl_gone(&self, hub: &Hub, st: &mut State, uid: &str, now: u64) {
        let ids: Vec<u64> = st.rl.tables.iter().filter(|(_, t)| t.knows(uid)).map(|(id, _)| *id).collect();
        for id in ids {
            let Some(t) = st.rl.tables.get_mut(&id) else { continue };
            t.unwatch(uid);
            if t.seat_of(uid).is_some() {
                let out = t.leave(uid, now);
                self.flush_rl(hub, st, out);
                info!(table = id, "roulette: someone who went was stood up");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::casino::roulette::{Chip, Spot};
    use pyr_poker::net::{RlStage, RlView};

    use super::*;
    use crate::db::Db;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-rl-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ask(hub: &Hub, uid: &str, msg: ToServer) -> Result<Option<ToApp>, String> {
        hub.poker.handle(hub, uid, false, msg)
    }

    fn view(hub: &Hub, uid: &str, table: u64) -> RlView {
        match ask(hub, uid, ToServer::RlWatch { table }).unwrap() {
            Some(ToApp::Rl { view }) => view,
            other => panic!("{other:?}"),
        }
    }

    fn chips(v: &[(Spot, u64)]) -> Vec<Chip> {
        v.iter().map(|(spot, chips)| Chip { spot: *spot, chips: *chips }).collect()
    }

    /// Chips are taken from people's and held till the spin; coming off, they come back; the
    /// spin pays as the wheel does; standing up gives back what's down; someone gone is stood
    /// up; a 0.12 app isn't let near the tables.
    #[test]
    fn chips_are_taken_held_and_paid() {
        let hub = hub();
        let start = hub.db.poker_record("ann").0.chips;
        assert!(hub.poker.handle_from(&hub, "ann", false, pyr_poker::net::CASINO_012, ToServer::RlTables).is_err());
        let tables = match ask(&hub, "ann", ToServer::RlTables).unwrap() {
            Some(ToApp::RlTables { tables }) => tables,
            other => panic!("{other:?}"),
        };
        assert_eq!(tables.len(), STANDING.len());
        let id = tables[0].id;
        ask(&hub, "ann", ToServer::RlSit { table: id, seat: 2 }).unwrap();
        assert!(ask(&hub, "ann", ToServer::RlSit { table: 2, seat: 0 }).is_err(), "one table at a time");
        ask(&hub, "ann", ToServer::RlBot { table: id, seat: 4, add: true }).unwrap();
        assert!(ask(&hub, "ann", ToServer::RlBets { table: id, spin: view(&hub, "ann", id).spin_no, chips: chips(&[(Spot::Red, start + 1)]), seed: [7; 32], ready: false }).is_err());
        ask(&hub, "ann", ToServer::RlBets { table: id, spin: view(&hub, "ann", id).spin_no, chips: chips(&[(Spot::Red, 100), (Spot::Straight { n: 7 }, 20)]), seed: [7; 32], ready: false }).unwrap();
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 120, 120));
        let mine = chips(&[(Spot::Red, 100)]);
        ask(&hub, "ann", ToServer::RlBets { table: id, spin: view(&hub, "ann", id).spin_no, chips: mine.clone(), seed: [8; 32], ready: true }).unwrap();
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 100, 100));
        let mut spun = None;
        for _ in 0..1_000 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.poker.tick(&hub);
            let v = view(&hub, "ann", id);
            if let Some(r) = &v.last {
                pyr_poker::rlroom::check(r).unwrap();
                spun = Some(r.clone());
                break;
            }
        }
        let r = spun.expect("the ball went");
        let paid: u64 = roulette::settle(&mine, r.number).iter().sum();
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 100 + paid, 0));
        assert_eq!(rec.casino.get("roulette").map(|c| c.rounds), Some(1));
        // Chips down, then gone: they come back.
        while view(&hub, "ann", id).stage != RlStage::Betting {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.poker.tick(&hub);
        }
        ask(&hub, "ann", ToServer::RlBets { table: id, spin: view(&hub, "ann", id).spin_no, chips: chips(&[(Spot::Odd, 50)]), seed: [9; 32], ready: false }).unwrap();
        hub.poker.gone(&hub, "ann");
        let (after, held) = hub.db.poker_record("ann");
        assert_eq!((after.chips, held), (rec.chips, 0));
    }
}
