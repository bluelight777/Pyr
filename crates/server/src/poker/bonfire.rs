//! The Bonfire on this server (see `pyr_poker::casino::bonfire`): the pool and everyone's
//! embers, kept in the database; lit when its time comes (or when the server's owner lights it),
//! the winners paid, everyone online shown it, and a new fire laid.

use std::collections::BTreeMap;

use pyr_poker::casino::bonfire;
use pyr_poker::net::{BonfireDraw, BonfireEntry, BonfireView, ToApp};
use pyr_poker::room::Random;
use pyr_proto::control::Event;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{Poker, State};
use crate::hub::Hub;

const FIRE: &str = "casino_bonfire";
const DRAWS: &str = "casino_bonfire_draws";
const DRAWS_KEEP: usize = 10;
/// How often the fire's saved while it changes.
const SAVE_MS: u64 = 5_000;

/// A fire burning: its number, the house's seed for its draw (secret till it's lit), its pool,
/// when it was laid and when it's lit, and everyone's embers (and names, for the draw).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Fire {
    pub round: u64,
    #[serde(with = "pyr_poker::net::hex32")]
    pub seed: [u8; 32],
    pub pool: u64,
    pub laid_ms: u64,
    pub lit_ms: u64,
    pub embers: BTreeMap<String, u64>,
    pub names: BTreeMap<String, String>,
    /// The pool's cut not yet a whole chip (in ten-thousandths): small bets add up too.
    #[serde(default)]
    pub frac: u64,
}

#[derive(Default)]
pub struct BonfireState {
    fire: Option<Fire>,
    dirty: bool,
    saved_ms: u64,
}

fn wall() -> u64 {
    pyr_proto::now_ms().max(0) as u64
}

impl Poker {
    /// The fire burning now: as saved, or a new one (the first, or after the last was lit).
    fn bonfire_fire<'a>(hub: &Hub, st: &'a mut State, rng: &mut dyn Random) -> &'a mut Fire {
        if st.bonfire.fire.is_none() {
            let saved: Option<Fire> = hub.db.get_meta(FIRE).and_then(|v| serde_json::from_str(&v).ok());
            st.bonfire.fire = Some(match saved {
                Some(f) => f,
                None => {
                    st.bonfire.dirty = true;
                    let now = wall();
                    Fire { round: 1, seed: rng.seed(), pool: bonfire::SEED_POOL, laid_ms: now, lit_ms: now + bonfire::BURN_MS, ..Fire::default() }
                }
            });
        }
        st.bonfire.fire.as_mut().unwrap()
    }

    /// A round played in the casino: its embers on the fire, and the pool's cut of its bet.
    pub(super) fn bonfire_add(hub: &Hub, st: &mut State, uid: &str, wagered: u64, rng: &mut dyn Random) {
        if uid.starts_with("bot:") {
            return;
        }
        let name = Self::name_of(hub, uid);
        let f = Self::bonfire_fire(hub, st, rng);
        *f.embers.entry(uid.to_string()).or_default() += bonfire::embers_for(wagered);
        f.names.insert(uid.to_string(), name);
        let total = wagered as u128 * bonfire::CUT_BP as u128 + f.frac as u128;
        f.pool = f.pool.saturating_add((total / 10_000) as u64);
        f.frac = (total % 10_000) as u64;
        st.bonfire.dirty = true;
    }

    fn bonfire_draws(hub: &Hub) -> Vec<BonfireDraw> {
        hub.db.get_meta(DRAWS).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    /// The Bonfire as someone sees it.
    pub(super) fn bonfire_view(hub: &Hub, st: &mut State, uid: &str, rng: &mut dyn Random) -> BonfireView {
        let can_light = Self::is_owner(hub, uid);
        let f = Self::bonfire_fire(hub, st, rng).clone();
        let mut top: Vec<(String, u64)> = f.embers.iter().map(|(u, e)| (f.names.get(u).cloned().unwrap_or_else(|| "Someone".into()), *e)).collect();
        top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        top.truncate(10);
        BonfireView {
            round: f.round,
            pool: f.pool,
            ms_left: f.lit_ms.saturating_sub(wall()),
            commit: pyr_poker::fair::commit(&f.seed),
            my_embers: f.embers.get(uid).copied().unwrap_or(0),
            total_embers: f.embers.values().sum(),
            entrants: f.embers.len(),
            top,
            last: Self::bonfire_draws(hub).into_iter().next(),
            can_light,
        }
    }

    /// The Bonfire's own requests: how it stands, or (the owner) light it now.
    pub(super) fn bonfire_req(&self, hub: &Hub, st: &mut State, uid: &str, light: bool, rng: &mut dyn Random) -> Result<Option<ToApp>, String> {
        if light {
            if !Self::is_owner(hub, uid) {
                return Err("only the server's owner can light the Bonfire early".into());
            }
            self.bonfire_light(hub, st, rng);
        }
        Ok(Some(ToApp::Bonfire { view: Self::bonfire_view(hub, st, uid, rng) }))
    }

    /// The clock: the fire saved now and then as it grows, and lit when its time comes.
    pub(super) fn bonfire_tick(&self, hub: &Hub, st: &mut State, rng: &mut dyn Random) {
        let now = wall();
        let due = Self::bonfire_fire(hub, st, rng).lit_ms <= now;
        if due {
            self.bonfire_light(hub, st, rng);
            return;
        }
        if st.bonfire.dirty && now.saturating_sub(st.bonfire.saved_ms) >= SAVE_MS {
            Self::bonfire_save(hub, st);
        }
    }

    fn bonfire_save(hub: &Hub, st: &mut State) {
        if let Some(f) = &st.bonfire.fire
            && let Ok(j) = serde_json::to_string(f)
        {
            let _ = hub.db.set_meta(FIRE, &j);
        }
        st.bonfire.dirty = false;
        st.bonfire.saved_ms = wall();
    }

    /// The Bonfire's lit: the draw, the winners paid, everyone online shown it, a new fire laid
    /// (with what nobody won on it).
    fn bonfire_light(&self, hub: &Hub, st: &mut State, rng: &mut dyn Random) {
        let f = Self::bonfire_fire(hub, st, rng).clone();
        let now = wall();
        let entries: Vec<BonfireEntry> = f.embers.iter().map(|(u, e)| BonfireEntry { uid: u.clone(), name: f.names.get(u).cloned().unwrap_or_else(|| "Someone".into()), embers: *e }).collect();
        let draw = bonfire::lit(f.round, now, f.seed, f.pool, entries);
        for w in &draw.winners {
            let (mut rec, _) = hub.db.poker_record(&w.uid);
            rec.chips = rec.chips.saturating_add(w.chips);
            let held = st.held(&w.uid);
            let _ = hub.db.set_poker_record(&w.uid, &rec, held);
            hub.send_poker(&w.uid, ToApp::Record { me: Self::view(hub, &w.uid, &rec, held), gains: None });
        }
        let mut draws = Self::bonfire_draws(hub);
        draws.insert(0, draw.clone());
        draws.truncate(DRAWS_KEEP);
        if let Ok(j) = serde_json::to_string(&draws) {
            let _ = hub.db.set_meta(DRAWS, &j);
        }
        info!(round = draw.round, pool = draw.pool, entrants = draw.entries.len(), "the Bonfire was lit");
        st.bonfire.fire = Some(Fire {
            round: f.round + 1,
            seed: rng.seed(),
            pool: bonfire::SEED_POOL + draw.rollover,
            laid_ms: now,
            lit_ms: now + bonfire::BURN_MS,
            ..Fire::default()
        });
        Self::bonfire_save(hub, st);
        // Everyone online with a Pyr that knows it sees it lit.
        hub.broadcast_casino_next(Event::Poker { msg: ToApp::BonfireLit { draw } });
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::net::ToServer;

    use crate::db::Db;
    use crate::hub::Hub;
    use pyr_poker::net::ToApp;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-bonfire-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    /// Playing throws embers on the fire and grows the pool; lit, it pays its winners (and only
    /// the owner can light it early); the draw checks out; a new fire's laid with what's left.
    #[test]
    fn playing_throws_embers_and_the_fire_pays() {
        use pyr_poker::net::{CasinoReq, CasinoRes};
        let hub = hub();
        let view = |uid: &str| match hub.poker.handle(&hub, uid, false, ToServer::Bonfire).unwrap() {
            Some(ToApp::Bonfire { view }) => view,
            other => panic!("{other:?}"),
        };
        let v = view("ann");
        assert_eq!((v.round, v.pool, v.my_embers), (1, pyr_poker::casino::bonfire::SEED_POOL, 0));
        assert!(v.ms_left > 0);
        // A few rounds of dice: embers and the pool's cut.
        let start = hub.db.poker_record("ann").0.chips;
        let mut commit = match hub.poker.handle(&hub, "ann", false, ToServer::Casino { req: CasinoReq::Open }).unwrap() {
            Some(ToApp::Casino { res: CasinoRes::State { view } }) => view.commit,
            other => panic!("{other:?}"),
        };
        for i in 0..5u8 {
            let bet = pyr_poker::casino::Bet::Dice { chips: 200, chance: 5_000, over: true };
            match hub.poker.handle(&hub, "ann", false, ToServer::Casino { req: CasinoReq::Play { commit, seed: [i; 32], bet } }).unwrap() {
                Some(ToApp::Casino { res: CasinoRes::Round { round } }) => commit = round.next_commit,
                other => panic!("{other:?}"),
            }
        }
        let v = view("ann");
        assert_eq!(v.my_embers, 5 * 3, "a round's ember and two for its 200 chips, five times");
        assert_eq!(v.pool, pyr_poker::casino::bonfire::SEED_POOL + 5 * 2);
        assert_eq!(v.top.len(), 1);
        // Only the owner lights it early.
        assert!(hub.poker.handle(&hub, "ann", false, ToServer::BonfireLight).is_err());
        let before = hub.db.poker_record("ann").0.chips;
        let owner = "owner-of-it";
        hub.db.insert_user(owner, "Owner", pyr_proto::control::Role::Owner).unwrap();
        let v2 = match hub.poker.handle(&hub, owner, false, ToServer::BonfireLight).unwrap() {
            Some(ToApp::Bonfire { view }) => view,
            other => panic!("{other:?}"),
        };
        let d = v2.last.expect("it was lit");
        pyr_poker::casino::bonfire::check(&d, Some(v.commit)).unwrap();
        assert_eq!(d.winners.len(), 1, "one person, one prize");
        assert_eq!(d.winners[0].uid, "ann");
        assert_eq!(hub.db.poker_record("ann").0.chips, before + d.winners[0].chips);
        assert_eq!(v2.round, 2);
        assert_eq!(v2.pool, pyr_poker::casino::bonfire::SEED_POOL + d.rollover);
        assert_eq!(v2.my_embers, 0);
        let _ = start;
    }
}
