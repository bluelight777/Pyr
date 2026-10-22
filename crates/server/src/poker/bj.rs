//! Blackjack tables in Pyr Casino on this server (see `pyr_poker::bjroom`): seven seats each for
//! people and bots, a round every few seconds. Bets and moves (doubles, splits, insurance) are
//! taken from people's play chips as they're made, and what comes back is paid when the round's
//! over. Chips on a table are held, like chips at a poker table: if the server stops they're
//! given back when it starts again.

use std::collections::BTreeMap;

use pyr_poker::bjroom::BjRoom;
use pyr_poker::casino::{Game, Outcome, blackjack};
use pyr_poker::net::{GainsView, ToApp, ToServer};
use pyr_poker::progress::{self, Record};
use pyr_poker::room::{Effect, Output, Random};
use tracing::info;

use super::casino::friday_share;
use super::{Poker, State, progress_chips};
use crate::hub::Hub;

/// The tables every server has (name, least and most bet). They never close.
pub const STANDING: [(&str, u64, u64); 2] = [("Hearth", 10, 1_000), ("High Limit", 500, 25_000)];
/// The most tables at once.
pub const MAX_BJ_TABLES: usize = 10;
/// A table someone opened closes when nobody's sat at it or watched it for this long.
const EMPTY_CLOSE_MS: u64 = 30 * 60 * 1000;
const CLOSED: &str = "that table has closed";

#[derive(Default)]
pub struct BjState {
    pub tables: BTreeMap<u64, BjRoom>,
    next: u64,
    /// When each table last had someone at it (or watching).
    used: BTreeMap<u64, u64>,
}

impl BjState {
    /// Chips someone has on the tables (bets placed, hands in play).
    pub fn at_stake(&self, uid: &str) -> u64 {
        self.tables.values().map(|t| t.at_stake(uid)).sum()
    }
}

impl Poker {
    /// The standing tables, set out the first time anyone comes to blackjack.
    fn bj_standing(st: &mut State, now: u64, rng: &mut dyn Random) {
        if st.bj.next > 0 {
            return;
        }
        for (name, lo, hi) in STANDING {
            st.bj.next += 1;
            let id = st.bj.next;
            st.bj.tables.insert(id, BjRoom::new(id, name, lo, hi, now, rng));
            st.bj.used.insert(id, now);
        }
    }

    fn bj_list(st: &State) -> ToApp {
        ToApp::BjTables { tables: st.bj.tables.values().map(|t| t.summary()).collect() }
    }

    /// A request about blackjack tables. What comes back goes to whoever asked; news goes out to
    /// everyone at the table.
    pub(super) fn bj(&self, hub: &Hub, st: &mut State, uid: &str, msg: ToServer, now: u64, rng: &mut dyn Random) -> Result<Option<ToApp>, String> {
        Self::bj_standing(st, now, rng);
        match msg {
            ToServer::BjTables => Ok(Some(Self::bj_list(st))),
            ToServer::BjWatch { table } => {
                if !st.bj.tables.contains_key(&table) {
                    return Err(CLOSED.into());
                }
                // (One table at a time.)
                for t in st.bj.tables.values_mut().filter(|t| t.id != table) {
                    t.unwatch(uid);
                }
                st.bj.used.insert(table, now);
                Ok(Some(st.bj.tables.get_mut(&table).unwrap().watch(uid, now)))
            }
            ToServer::BjUnwatch { table } => {
                if let Some(t) = st.bj.tables.get_mut(&table) {
                    t.unwatch(uid);
                }
                Ok(None)
            }
            ToServer::BjSit { table, seat } => {
                if let Some(t) = st.bj.tables.values().find(|t| t.seat_of(uid).is_some()) {
                    return Err(if t.id == table { "you're sitting here already".into() } else { format!("you're sitting at {} already: stand up there first", t.name) });
                }
                let (rec, _) = hub.db.poker_record(uid);
                let name = Self::name_of(hub, uid);
                let t = st.bj.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.sit(uid, &name, Self::look(&rec), seat, now)?;
                self.flush_bj(hub, st, out);
                Ok(None)
            }
            ToServer::BjLeave { table } => {
                let t = st.bj.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.leave(uid, now);
                self.flush_bj(hub, st, out);
                Ok(None)
            }
            ToServer::BjBet { table, chips, seed } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                if chips > rec.chips {
                    return Err(format!("that's {} chips, and you have {}", progress_chips(chips), progress_chips(rec.chips)));
                }
                let t = st.bj.tables.get_mut(&table).ok_or(CLOSED)?;
                let out = t.bet(uid, chips, seed, now)?;
                Self::bj_take(hub, st, uid, &mut rec, chips)?;
                self.flush_bj(hub, st, out);
                Ok(None)
            }
            ToServer::BjAct { table, action } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                let t = st.bj.tables.get_mut(&table).ok_or(CLOSED)?;
                let cost = t.cost(uid, action).ok_or("it isn't your turn, or that move isn't open to you now")?;
                if cost > rec.chips {
                    return Err(format!("{} takes {} more chips, and you have {}", action.name(), progress_chips(cost), progress_chips(rec.chips)));
                }
                let out = t.act(uid, action, now)?;
                Self::bj_take(hub, st, uid, &mut rec, cost)?;
                self.flush_bj(hub, st, out);
                Ok(None)
            }
            ToServer::BjBot { table, seat, add } => {
                let t = st.bj.tables.get_mut(&table).ok_or(CLOSED)?;
                if !t.knows(uid) {
                    return Err("sit at the table (or watch it) first".into());
                }
                let out = if add { t.add_bot(seat, now, rng)? } else { t.remove_bot(seat, now)? };
                self.flush_bj(hub, st, out);
                Ok(None)
            }
            ToServer::BjNew => {
                if st.bj.tables.len() >= MAX_BJ_TABLES {
                    return Err("this server has as many blackjack tables as it keeps: join one".into());
                }
                st.bj.next += 1;
                let id = st.bj.next;
                let (_, lo, hi) = STANDING[0];
                let mut t = BjRoom::new(id, &format!("Table {id}"), lo, hi, now, rng);
                for other in st.bj.tables.values_mut() {
                    other.unwatch(uid);
                }
                let view = t.watch(uid, now);
                st.bj.tables.insert(id, t);
                st.bj.used.insert(id, now);
                info!(table = id, "blackjack: a table was opened");
                Ok(Some(view))
            }
            _ => Err("that isn't about a blackjack table".into()),
        }
    }

    /// Chips a person put down (a bet; a double, split or insurance), taken from them.
    fn bj_take(hub: &Hub, st: &State, uid: &str, rec: &mut Record, chips: u64) -> Result<(), String> {
        if chips == 0 {
            return Ok(());
        }
        rec.chips -= chips;
        Self::add_rake(hub, friday_share(Game::Blackjack, chips));
        let held = st.held(uid);
        hub.db.set_poker_record(uid, rec, held).map_err(|e| e.to_string())?;
        hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, rec, held), gains: None });
        Ok(())
    }

    /// Sends a blackjack table's news on, pays what came back, and counts the rounds.
    fn flush_bj(&self, hub: &Hub, st: &mut State, mut out: Output) {
        let effects = std::mem::take(&mut out.effects);
        self.flush(hub, st, 0, out);
        for e in effects {
            match e {
                Effect::BjRound { uid, outcome } => Self::bj_counted(hub, st, &uid, outcome),
                e => self.flush(hub, st, 0, Output { effects: vec![e], ..Output::default() }),
            }
        }
    }

    /// A round over for someone (paid already): their records (experience, achievements, their
    /// statistics) and the casino's (the game's statistics; a big win told to everyone there).
    fn bj_counted(hub: &Hub, st: &mut State, uid: &str, outcome: blackjack::Outcome) {
        let o = Outcome::Blackjack(outcome);
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

    /// The tables' clock: rounds move on, and tables someone opened that nobody's used close.
    pub(super) fn bj_tick(&self, hub: &Hub, st: &mut State, now: u64, rng: &mut dyn Random) {
        let ids: Vec<u64> = st.bj.tables.keys().copied().collect();
        let mut close = Vec::new();
        for id in ids {
            let Some(t) = st.bj.tables.get_mut(&id) else { continue };
            let out = t.tick(now, rng);
            let used = t.in_use();
            if !out.msgs.is_empty() || !out.effects.is_empty() {
                self.flush_bj(hub, st, out);
            }
            if used {
                st.bj.used.insert(id, now);
            } else if id > STANDING.len() as u64 && now.saturating_sub(*st.bj.used.get(&id).unwrap_or(&now)) > EMPTY_CLOSE_MS {
                close.push(id);
            }
        }
        for id in close {
            if let Some(mut t) = st.bj.tables.remove(&id) {
                let out = t.close();
                self.flush_bj(hub, st, out);
                st.bj.used.remove(&id);
                info!(table = id, "blackjack: a table nobody used for half an hour closed");
            }
        }
    }

    /// Someone's gone: they stand up from the tables (a bet placed comes back; hands in play
    /// stand, and are paid as they come out) and stop watching.
    pub(super) fn bj_gone(&self, hub: &Hub, st: &mut State, uid: &str, now: u64) {
        let ids: Vec<u64> = st.bj.tables.iter().filter(|(_, t)| t.knows(uid)).map(|(id, _)| *id).collect();
        for id in ids {
            let Some(t) = st.bj.tables.get_mut(&id) else { continue };
            t.unwatch(uid);
            if t.seat_of(uid).is_some() {
                let out = t.leave(uid, now);
                self.flush_bj(hub, st, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::casino::blackjack::Action;
    use pyr_poker::net::{BjStage, BjView};

    use super::*;
    use crate::db::Db;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-bj-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ask(hub: &Hub, uid: &str, msg: ToServer) -> Result<Option<ToApp>, String> {
        hub.poker.handle(hub, uid, false, msg)
    }

    fn view(hub: &Hub, uid: &str, table: u64) -> BjView {
        match ask(hub, uid, ToServer::BjWatch { table }).unwrap() {
            Some(ToApp::Bj { view }) => view,
            other => panic!("{other:?}"),
        }
    }

    /// Bets are taken from people's chips and held while they ride; when the round's over,
    /// what came back is paid and nothing is held.
    #[test]
    fn bets_are_taken_held_and_paid() {
        let hub = hub();
        let start = hub.db.poker_record("ann").0.chips;
        let tables = match ask(&hub, "ann", ToServer::BjTables).unwrap() {
            Some(ToApp::BjTables { tables }) => tables,
            other => panic!("{other:?}"),
        };
        assert_eq!(tables.len(), STANDING.len());
        let id = tables[0].id;
        ask(&hub, "ann", ToServer::BjSit { table: id, seat: 3 }).unwrap();
        ask(&hub, "ann", ToServer::BjBot { table: id, seat: 0, add: true }).unwrap();
        // Too much is refused, and nothing's taken.
        assert!(ask(&hub, "ann", ToServer::BjBet { table: id, chips: start + 1, seed: [7; 32] }).is_err());
        ask(&hub, "ann", ToServer::BjBet { table: id, chips: 100, seed: [7; 32] }).unwrap();
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!((rec.chips, held), (start - 100, 100));
        // Play it out: stand whenever it's my turn (or decline insurance).
        let mut done = false;
        for _ in 0..1_000 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.poker.tick(&hub);
            let v = view(&hub, "ann", id);
            if let (BjStage::Playing, Some(r)) = (v.stage, &v.round)
                && r.to_act == Some(3)
            {
                let a = if r.allowed.contains(&Action::NoInsurance) { Action::NoInsurance } else { Action::Stand };
                ask(&hub, "ann", ToServer::BjAct { table: id, action: a }).unwrap();
            }
            if v.last.is_some() {
                pyr_poker::bjroom::check(v.last.as_ref().unwrap()).unwrap();
                done = true;
                break;
            }
        }
        assert!(done, "a round was played");
        let (rec, held) = hub.db.poker_record("ann");
        assert_eq!(held, 0);
        let last = view(&hub, "ann", id).last.unwrap();
        let o = pyr_poker::casino::bjtable::replay(&last.bets, pyr_poker::fair::Stream::new(pyr_poker::bjroom::round_seed(&last.house, &last.seeds.iter().map(|(s, h)| (*s, pyr_poker::net::hex32::parse(h).unwrap())).collect::<Vec<_>>(), last.round)), &last.actions).unwrap();
        assert_eq!(rec.chips, start - 100 + o.paid(3));
        assert_eq!(rec.casino.get("blackjack").map(|c| c.rounds), Some(1));
    }

    /// Standing up gives a bet placed back; someone gone is stood up.
    #[test]
    fn standing_up_gives_the_bet_back() {
        let hub = hub();
        let start = hub.db.poker_record("bo").0.chips;
        ask(&hub, "bo", ToServer::BjTables).unwrap();
        ask(&hub, "bo", ToServer::BjSit { table: 1, seat: 0 }).unwrap();
        assert!(ask(&hub, "bo", ToServer::BjSit { table: 2, seat: 0 }).is_err(), "one table at a time");
        ask(&hub, "bo", ToServer::BjBet { table: 1, chips: 50, seed: [1; 32] }).unwrap();
        let (rec, held) = hub.db.poker_record("bo");
        assert_eq!((rec.chips, held), (start - 50, 50));
        ask(&hub, "bo", ToServer::BjLeave { table: 1 }).unwrap();
        assert_eq!(hub.db.poker_record("bo").0.chips, start);
        assert_eq!(hub.db.poker_record("bo").1, 0);
        ask(&hub, "bo", ToServer::BjSit { table: 2, seat: 6 }).unwrap();
        ask(&hub, "bo", ToServer::BjBet { table: 2, chips: 500, seed: [1; 32] }).unwrap();
        hub.poker.gone(&hub, "bo");
        assert_eq!(hub.db.poker_record("bo").0.chips, start);
        assert!(view(&hub, "cy", 2).seats.iter().all(|s| s.is_none()));
    }
}
