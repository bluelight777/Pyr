//! Pyr Casino on the server: it takes bets from people's play chips, plays the rounds (see
//! `pyr_poker::casino`), pays out, and keeps each person's next seed, their blackjack hand and
//! their craps bets between requests.
//!
//! Chips in a hand of blackjack or on the craps table are written down, so if the server stops
//! they're given back when it starts again.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use pyr_poker::casino::{self, Bet, Game, Outcome, blackjack, craps, dragon, reelrow, slots};
use pyr_poker::net::{BigWin, CasinoReq, CasinoRes, CasinoView, DragonView, GameStats, PeriodStats, RoundView, ToApp};
use pyr_poker::progress::{self, Record};
use pyr_poker::room::Random;
use pyr_poker::tourney;
use pyr_proto::control::Event;
use tracing::info;

use super::{Poker, State, SysRandom};
use crate::hub::Hub;

/// The database's notes: the jackpot's pool, the big wins, and chips at stake in unfinished
/// blackjack hands and on craps tables (by person).
const JACKPOT: &str = "casino_jackpot";
const WINS: &str = "casino_wins";
const AT_RISK: &str = "casino_at_risk";
const WINS_KEEP: usize = 30;
/// Each game's statistics (by its statistics key): its rounds by the hour (the last 24) and by
/// the day (the last 28), all time, and its biggest and luckiest wins.
const HOURS: &str = "casino_hours:";
const DAYS: &str = "casino_days:";
const ALL_TIME: &str = "casino_all:";
const BIG: &str = "casino_big:";
const LUCKY: &str = "casino_lucky:";
const LIST_KEEP: usize = 10;
/// Ember Dragon: each person's dragons (growth by bet), and the jackpots' pools.
const DRAGON: &str = "casino_dragon:";
const DRAGON_POOLS: &str = "casino_dragon_pools";
/// Someone "is playing" a game for this long after their last round of it.
const PLAYING_MS: u64 = 5 * 60_000;
/// The jackpot's pool is told to everyone in the casino at most this often.
const JACKPOT_TELL_MS: u64 = 3_000;

#[derive(Default)]
pub struct CasinoState {
    players: HashMap<String, Player>,
    /// Who has the casino open (they hear big wins and the jackpot).
    pub watchers: BTreeSet<String>,
    refunded: bool,
    jackpot_told: u64,
    /// Who played each game (by statistics key) and when they last did.
    playing: HashMap<String, HashMap<String, u64>>,
}

struct Player {
    /// The house's seed for this person's next round, and its number.
    seed: [u8; 32],
    nonce: u64,
    bj: Option<Hand>,
    craps: craps::Table,
}

/// A blackjack hand being played, and the round's seeds (shown when it's over).
struct Hand {
    round: blackjack::Round,
    seed: [u8; 32],
    client: [u8; 32],
    nonce: u64,
}

fn new_player() -> Player {
    Player { seed: SysRandom.seed(), nonce: 1, bj: None, craps: craps::Table::default() }
}

/// A share of every bet goes to the Friday Night pool: half the house's edge, 1% at most (so
/// the house still keeps something of every game).
pub(super) fn friday_share(game: Game, wagered: u64) -> u64 {
    let bp = (game.house_edge_bp() as u64 / 2).min(100);
    (wagered * bp).div_ceil(10_000)
}

fn wall() -> u64 {
    pyr_proto::now_ms().max(0) as u64
}

impl Poker {
    fn jackpot(hub: &Hub) -> u64 {
        hub.db.get_meta(JACKPOT).and_then(|v| v.parse().ok()).unwrap_or(slots::JACKPOT_SEED)
    }

    fn set_jackpot(hub: &Hub, pool: u64) {
        let _ = hub.db.set_meta(JACKPOT, &pool.to_string());
    }

    /// A progressive Reel Row machine's jackpot pool (its seed until it's grown).
    fn row_pool(hub: &Hub, machine: u8) -> u64 {
        hub.db.get_meta(&format!("row_pool:{machine}")).and_then(|v| v.parse().ok()).unwrap_or(reelrow::POOL_SEED)
    }

    fn set_row_pool(hub: &Hub, machine: u8, pool: u64) {
        let _ = hub.db.set_meta(&format!("row_pool:{machine}"), &pool.to_string());
    }

    fn row_pools(hub: &Hub) -> Vec<(u8, u64)> {
        (0..reelrow::TOPS.len() as u8).filter(|m| reelrow::progressive(*m)).map(|m| (m, Self::row_pool(hub, m))).collect()
    }

    fn big_wins(hub: &Hub) -> Vec<BigWin> {
        hub.db.get_meta(WINS).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    fn meta_json<T: serde::de::DeserializeOwned + Default>(hub: &Hub, key: &str) -> T {
        hub.db.get_meta(key).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    fn set_meta_json<T: serde::Serialize>(hub: &Hub, key: &str, v: &T) {
        if let Ok(j) = serde_json::to_string(v) {
            let _ = hub.db.set_meta(key, &j);
        }
    }

    /// A round, counted in its game's statistics; and its win on the game's lists of biggest and
    /// luckiest, if it's one of them.
    fn count_round(hub: &Hub, st: &mut State, uid: &str, key: &str, win: &BigWin, now: u64) {
        let add = |p: &mut PeriodStats| {
            p.rounds += 1;
            p.wagered += win.wagered;
            p.paid += win.paid;
        };
        let (hour, day) = (now / 3_600_000, now / 86_400_000);
        let mut hours: BTreeMap<u64, PeriodStats> = Self::meta_json(hub, &format!("{HOURS}{key}"));
        add(hours.entry(hour).or_default());
        hours.retain(|h, _| h + 24 > hour);
        Self::set_meta_json(hub, &format!("{HOURS}{key}"), &hours);
        let mut days: BTreeMap<u64, PeriodStats> = Self::meta_json(hub, &format!("{DAYS}{key}"));
        add(days.entry(day).or_default());
        days.retain(|d, _| d + 28 > day);
        Self::set_meta_json(hub, &format!("{DAYS}{key}"), &days);
        let mut all: PeriodStats = Self::meta_json(hub, &format!("{ALL_TIME}{key}"));
        add(&mut all);
        Self::set_meta_json(hub, &format!("{ALL_TIME}{key}"), &all);
        st.casino.playing.entry(key.to_string()).or_default().insert(uid.to_string(), now);
        if win.paid == 0 {
            return;
        }
        // The biggest (chips back) and the luckiest (times the bet, winners only).
        let mult = |w: &BigWin| casino::multiple_x100(w.paid, w.wagered);
        for (list, better) in [(BIG, &(|a: &BigWin, b: &BigWin| a.paid > b.paid) as &dyn Fn(&BigWin, &BigWin) -> bool), (LUCKY, &|a: &BigWin, b: &BigWin| mult(a) > mult(b))] {
            if list == LUCKY && win.paid <= win.wagered {
                continue;
            }
            let k = format!("{list}{key}");
            let mut wins: Vec<BigWin> = Self::meta_json(hub, &k);
            if wins.len() >= LIST_KEEP && !wins.iter().any(|w| better(win, w)) {
                continue;
            }
            let at = wins.iter().position(|w| better(win, w)).unwrap_or(wins.len());
            wins.insert(at, win.clone());
            wins.truncate(LIST_KEEP);
            Self::set_meta_json(hub, &k, &wins);
        }
    }

    /// A game's statistics, for its Info panel.
    fn game_stats(hub: &Hub, st: &mut State, key: &str, now: u64) -> GameStats {
        let (hour, day) = (now / 3_600_000, now / 86_400_000);
        let sum = |m: &BTreeMap<u64, PeriodStats>, keep: &dyn Fn(u64) -> bool| {
            m.iter().filter(|(k, _)| keep(**k)).fold(PeriodStats::default(), |mut a, (_, p)| {
                a.rounds += p.rounds;
                a.wagered += p.wagered;
                a.paid += p.paid;
                a
            })
        };
        let hours: BTreeMap<u64, PeriodStats> = Self::meta_json(hub, &format!("{HOURS}{key}"));
        let days: BTreeMap<u64, PeriodStats> = Self::meta_json(hub, &format!("{DAYS}{key}"));
        let periods = vec![
            sum(&hours, &|h| h + 24 > hour),
            sum(&days, &|d| d + 7 > day),
            sum(&days, &|d| d + 28 > day),
            Self::meta_json(hub, &format!("{ALL_TIME}{key}")),
        ];
        let who = st.casino.playing.entry(key.to_string()).or_default();
        who.retain(|_, at| now.saturating_sub(*at) < PLAYING_MS);
        GameStats {
            key: key.to_string(),
            periods,
            playing: who.len() as u32,
            big: Self::meta_json(hub, &format!("{BIG}{key}")),
            lucky: Self::meta_json(hub, &format!("{LUCKY}{key}")),
        }
    }

    fn at_risk(hub: &Hub) -> BTreeMap<String, u64> {
        hub.db.get_meta(AT_RISK).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    /// Writes down what someone has riding in the casino (a blackjack hand, the craps table).
    fn note_at_risk(hub: &Hub, uid: &str, p: &Player) {
        let chips = p.bj.as_ref().map(|h| h.round.wagered()).unwrap_or(0) + p.craps.bets.total();
        let mut all = Self::at_risk(hub);
        if chips == 0 {
            if all.remove(uid).is_none() {
                return;
            }
        } else {
            all.insert(uid.to_string(), chips);
        }
        if let Ok(j) = serde_json::to_string(&all) {
            let _ = hub.db.set_meta(AT_RISK, &j);
        }
    }

    /// Once, after the server starts: chips left riding when it stopped go back to their owners.
    pub(super) fn casino_after_restart(hub: &Hub, st: &mut State) {
        if st.casino.refunded {
            return;
        }
        st.casino.refunded = true;
        for (uid, chips) in Self::at_risk(hub) {
            let (mut rec, _) = hub.db.poker_record(&uid);
            rec.chips += chips;
            let held = st.held(&uid);
            let _ = hub.db.set_poker_record(&uid, &rec, held);
            info!(chips, "casino: chips riding when the server stopped were given back");
        }
        let _ = hub.db.set_meta(AT_RISK, "{}");
    }

    fn tell_casino(hub: &Hub, st: &State, res: CasinoRes) {
        // (A big win at a game apps before 0.12.0 don't know: only to the apps that do.)
        let new = matches!(&res, CasinoRes::BigWin { win } if win.game.is_new());
        for uid in &st.casino.watchers {
            let ev = Event::Poker { msg: ToApp::Casino { res: res.clone() } };
            if new { hub.send_to_user_casino_next(uid, ev) } else { hub.send_to_user(uid, ev) }
        }
    }

    fn casino_view(hub: &Hub, st: &mut State, uid: &str, next: bool) -> CasinoView {
        let (rec, _) = hub.db.poker_record(uid);
        let p = st.casino.players.entry(uid.to_string()).or_insert_with(new_player);
        let commit = casino::commit(&p.seed);
        let (nonce, blackjack, craps_table) = (p.nonce, p.bj.as_ref().map(|h| h.round.view()), p.craps.clone());
        let friday = st.tourneys.values().find(|t| t.host == tourney::FRIDAY_HOST && t.phase == pyr_poker::net::TourneyPhase::Registering);
        let (friday_ms, friday_prize) = match friday {
            Some(t) => (t.rules.start_ms, t.prize_pool()),
            None => (tourney::next_friday_night(wall()), tourney::FRIDAY_FLOOR + Self::friday_pool(hub)),
        };
        CasinoView {
            chips: rec.chips,
            commit,
            nonce,
            jackpot: Self::jackpot(hub),
            friday_ms,
            friday_prize,
            big_wins: Self::big_wins(hub).into_iter().filter(|w| next || !w.game.is_new()).collect(),
            blackjack,
            craps: craps_table,
            stats: rec.casino.clone(),
            dragon: next.then(|| DragonView { growth: Self::dragon_growth(hub, uid), pools: Self::dragon_pools(hub) }),
            row_pools: Self::row_pools(hub),
        }
    }

    fn dragon_growth(hub: &Hub, uid: &str) -> BTreeMap<u64, u32> {
        Self::meta_json(hub, &format!("{DRAGON}{uid}"))
    }

    fn dragon_pools(hub: &Hub) -> dragon::Pools {
        hub.db.get_meta(DRAGON_POOLS).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_else(dragon::seed_pools)
    }

    /// A spin of Ember Dragon: from the dragon's growth at its bet (as the app was last told),
    /// the jackpots' pools with this spin's shares in; then the dragon's new growth kept, and a
    /// pool won started again from its seed (the house puts that in).
    #[allow(clippy::too_many_arguments)]
    fn casino_dragon(&self, hub: &Hub, st: &mut State, uid: &str, commit: [u8; 32], seed: [u8; 32], bet: u64, growth: u32, feature: pyr_poker::casino::feature::Feature) -> Result<CasinoRes, String> {
        dragon::check(bet, growth)?;
        let cost = Bet::DragonPlus { bet, growth, feature }.total();
        let (mut rec, _) = hub.db.poker_record(uid);
        let mut grown = Self::dragon_growth(hub, uid);
        if grown.get(&bet).copied().unwrap_or(0) != growth {
            return Err("out of step with the house: your dragon's grown since; try again".into());
        }
        let p = st.casino.players.entry(uid.to_string()).or_insert_with(new_player);
        if casino::commit(&p.seed) != commit {
            return Err("out of step with the house: the casino is catching up, try again".into());
        }
        if cost > rec.chips {
            return Err(format!("that's {} chips, and you have {}", super::progress_chips(cost), super::progress_chips(rec.chips)));
        }
        rec.chips -= cost;
        let (server, nonce) = (p.seed, p.nonce);
        p.seed = SysRandom.seed();
        p.nonce += 1;
        let mut s = casino::stream(&server, &seed, Game::Dragon, nonce);
        Self::add_rake(hub, friday_share(Game::Dragon, cost));
        let mut pools = Self::dragon_pools(hub);
        for (pool, add) in pools.iter_mut().zip(dragon::pool_shares(cost)) {
            *pool += add;
        }
        let o = dragon::play_feature(bet, growth, pools, feature, &mut s);
        if let Some(r) = &o.respins {
            for j in dragon::Jackpot::ALL {
                let won = r.pooled[j.index()];
                if won > 0 {
                    pools[j.index()] = pools[j.index()].saturating_sub(won).max(j.seed());
                    info!(chips = won, jackpot = j.name(), "casino: an Ember Dragon jackpot was won");
                }
            }
        }
        if let Ok(j) = serde_json::to_string(&pools) {
            let _ = hub.db.set_meta(DRAGON_POOLS, &j);
        }
        grown.insert(bet, o.growth_after);
        Self::set_meta_json(hub, &format!("{DRAGON}{uid}"), &grown);
        let r = self.finish_round(hub, st, uid, rec, Outcome::Dragon(o), server, seed, nonce, None)?;
        Ok(CasinoRes::Round { round: Box::new(r) })
    }

    pub(super) fn casino(&self, hub: &Hub, st: &mut State, uid: &str, req: CasinoReq, casino: u8) -> Result<Option<ToApp>, String> {
        let next = casino >= pyr_poker::net::CASINO_012;
        let res = match req {
            CasinoReq::Open => {
                st.casino.watchers.insert(uid.to_string());
                CasinoRes::State { view: Self::casino_view(hub, st, uid, next) }
            }
            CasinoReq::Close => {
                st.casino.watchers.remove(uid);
                return Ok(None);
            }
            CasinoReq::Stats { key } => {
                let known = key.len() <= 40 && (Game::from_id(&key).is_some() || casino::slot_rtp_bp(&key).is_some());
                if !known {
                    return Err("there's no such game".into());
                }
                CasinoRes::Stats { view: Self::game_stats(hub, st, &key, wall()) }
            }
            CasinoReq::Play { commit, seed, bet } => {
                if (bet.game().is_new() && !next) || (bet.is_013() && casino < pyr_poker::net::CASINO_013) {
                    return Err("this game needs a newer Pyr".into());
                }
                match bet {
                    Bet::Dragon { bet, growth } => self.casino_dragon(hub, st, uid, commit, seed, bet, growth, pyr_poker::casino::feature::Feature::Spin)?,
                    Bet::DragonPlus { bet, growth, feature } => {
                        Bet::DragonPlus { bet, growth, feature }.check()?;
                        self.casino_dragon(hub, st, uid, commit, seed, bet, growth, feature)?
                    }
                    bet => self.casino_play(hub, st, uid, commit, seed, bet)?,
                }
            }
            CasinoReq::Blackjack { action } => self.casino_blackjack(hub, st, uid, action)?,
            CasinoReq::CrapsBets { bets } => {
                let (mut rec, _) = hub.db.poker_record(uid);
                let p = st.casino.players.entry(uid.to_string()).or_insert_with(new_player);
                craps::check_change(&p.craps, &bets)?;
                let (old, new) = (p.craps.bets.total(), bets.total());
                if new > old {
                    let more = new - old;
                    if more > rec.chips {
                        return Err("you don't have the chips for that".into());
                    }
                    rec.chips -= more;
                } else {
                    rec.chips += old - new;
                }
                p.craps.bets = bets;
                let table = p.craps.clone();
                Self::note_at_risk(hub, uid, p);
                let held = st.held(uid);
                hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
                CasinoRes::Craps { table, chips: rec.chips }
            }
            CasinoReq::CrapsRoll { commit, seed } => {
                let p = st.casino.players.entry(uid.to_string()).or_insert_with(new_player);
                if casino::commit(&p.seed) != commit {
                    return Err("out of step with the house: the casino is catching up, try again".into());
                }
                if p.craps.bets.is_empty() {
                    return Err("put a bet down first".into());
                }
                let (server, nonce) = (p.seed, p.nonce);
                let mut s = casino::stream(&server, &seed, Game::Craps, nonce);
                let (table, o) = craps::play(&p.craps, &mut s);
                p.craps = table.clone();
                p.seed = SysRandom.seed();
                p.nonce += 1;
                Self::note_at_risk(hub, uid, p);
                Self::add_rake(hub, friday_share(Game::Craps, o.wagered));
                let (rec, _) = hub.db.poker_record(uid);
                let round = self.finish_round(hub, st, uid, rec, Outcome::Craps(o), server, seed, nonce, Some(table))?;
                CasinoRes::Round { round: Box::new(round) }
            }
        };
        Ok(Some(ToApp::Casino { res }))
    }

    #[allow(clippy::too_many_arguments)]
    fn casino_play(&self, hub: &Hub, st: &mut State, uid: &str, commit: [u8; 32], seed: [u8; 32], bet: Bet) -> Result<CasinoRes, String> {
        bet.check()?;
        let total = bet.total();
        let (mut rec, _) = hub.db.poker_record(uid);
        let p = st.casino.players.entry(uid.to_string()).or_insert_with(new_player);
        if casino::commit(&p.seed) != commit {
            return Err("out of step with the house: the casino is catching up, try again".into());
        }
        if matches!(bet, Bet::Blackjack { .. }) && p.bj.is_some() {
            return Err("finish your blackjack hand first".into());
        }
        if total > rec.chips {
            return Err(format!("that's {} chips, and you have {}", super::progress_chips(total), super::progress_chips(rec.chips)));
        }
        rec.chips -= total;
        let game = bet.game();
        let (server, nonce) = (p.seed, p.nonce);
        p.seed = SysRandom.seed();
        p.nonce += 1;
        let mut s = casino::stream(&server, &seed, game, nonce);
        Self::add_rake(hub, friday_share(game, total));
        if let Bet::Blackjack { chips } = bet {
            let round = blackjack::Round::deal(chips, s);
            if round.phase == blackjack::Phase::Over {
                // A blackjack (the player's or the dealer's) is over as it's dealt.
                let o = Outcome::Blackjack(round.outcome());
                let r = self.finish_round(hub, st, uid, rec, o, server, seed, nonce, None)?;
                return Ok(CasinoRes::Round { round: Box::new(r) });
            }
            let view = round.view();
            p.bj = Some(Hand { round, seed: server, client: seed, nonce });
            let next_commit = casino::commit(&p.seed);
            let next_nonce = p.nonce;
            Self::note_at_risk(hub, uid, p);
            let held = st.held(uid);
            hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
            return Ok(CasinoRes::Blackjack { view, chips: rec.chips, next_commit, next_nonce });
        }
        // The jackpot grows by its share of every spin (before the spin, so it's in this one).
        let mut pool = Self::jackpot(hub);
        // (A feature that spins the reels feeds it as a spin does; a bought bonus doesn't.)
        match bet {
            Bet::Slots { line_bet } => pool += slots::jackpot_share(line_bet),
            Bet::SlotsPlus { line_bet, feature } if feature.spins() => pool += slots::jackpot_share(line_bet),
            _ => {}
        }
        // A progressive Reel Row machine's pool grows by its share of every round there (in
        // this one already), and is what the round plays with.
        let row = match &bet {
            Bet::Row { machine, .. } if reelrow::progressive(*machine) => Some((*machine, Self::row_pool(hub, *machine) + reelrow::pool_share(*machine, bet.total()))),
            _ => None,
        };
        let o = casino::play(&bet, &mut s, row.map(|r| r.1).unwrap_or(pool))?;
        if let (Some((machine, mut rp)), Outcome::Row(ro)) = (row, &o) {
            let won = ro.bonus.as_ref().map(|b| b.top).unwrap_or(0);
            rp = rp.saturating_sub(won).max(reelrow::POOL_SEED);
            Self::set_row_pool(hub, machine, rp);
            if won > 0 {
                info!(chips = won, machine, "casino: a Reel Row progressive jackpot was won");
            }
        }
        if let Outcome::Slots(so) = &o {
            pool = pool.saturating_sub(so.jackpot_won);
            if pool < slots::JACKPOT_SEED {
                // (The house puts the seed back in once it's won.)
                pool = slots::JACKPOT_SEED;
            }
            Self::set_jackpot(hub, pool);
            if so.jackpot_won > 0 {
                info!(chips = so.jackpot_won, "casino: the Phoenix Jackpot was won");
            }
        }
        let r = self.finish_round(hub, st, uid, rec, o, server, seed, nonce, None)?;
        Ok(CasinoRes::Round { round: Box::new(r) })
    }

    fn casino_blackjack(&self, hub: &Hub, st: &mut State, uid: &str, action: blackjack::Action) -> Result<CasinoRes, String> {
        let (mut rec, _) = hub.db.poker_record(uid);
        let p = st.casino.players.entry(uid.to_string()).or_insert_with(new_player);
        let Some(hand) = p.bj.as_mut() else {
            return Err("there's no blackjack hand to play: deal one first".into());
        };
        let cost = hand.round.cost(action);
        if cost > rec.chips {
            return Err(format!("{} takes {} more chips, and you have {}", action.name(), super::progress_chips(cost), super::progress_chips(rec.chips)));
        }
        hand.round.act(action)?;
        rec.chips -= cost;
        Self::add_rake(hub, friday_share(Game::Blackjack, cost));
        if hand.round.phase == blackjack::Phase::Over {
            let Hand { round, seed, client, nonce } = p.bj.take().unwrap();
            Self::note_at_risk(hub, uid, p);
            let r = self.finish_round(hub, st, uid, rec, Outcome::Blackjack(round.outcome()), seed, client, nonce, None)?;
            return Ok(CasinoRes::Round { round: Box::new(r) });
        }
        let view = hand.round.view();
        let (next_commit, next_nonce) = (casino::commit(&p.seed), p.nonce);
        Self::note_at_risk(hub, uid, p);
        let held = st.held(uid);
        hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
        Ok(CasinoRes::Blackjack { view, chips: rec.chips, next_commit, next_nonce })
    }

    /// A round, counted in its game's statistics; a big win goes on the floor's ticker, for
    /// everyone in the casino.
    pub(super) fn round_counted(hub: &Hub, st: &mut State, uid: &str, o: &Outcome) {
        let (wagered, paid) = (o.wagered(), match o {
            Outcome::Craps(c) => c.paid + c.kept,
            _ => o.paid(),
        });
        let jackpot = matches!(o, Outcome::Slots(s) if s.jackpot_won > 0) || matches!(o, Outcome::Dragon(d) if !d.jackpots().is_empty());
        let key = o.stats_key();
        // Embers on the Bonfire, and its cut of the bet.
        Self::bonfire_add(hub, st, uid, wagered, &mut SysRandom);
        let win = BigWin { name: Self::name_of(hub, uid), game: o.game(), wagered, paid, what: o.headline(), at_ms: wall(), key: key.clone() };
        Self::count_round(hub, st, uid, &key, &win, wall());
        if jackpot || (paid >= 1_000 && paid >= wagered.saturating_mul(20)) || paid.saturating_sub(wagered) >= 100_000 {
            let mut wins = Self::big_wins(hub);
            wins.insert(0, win.clone());
            wins.truncate(WINS_KEEP);
            if let Ok(j) = serde_json::to_string(&wins) {
                let _ = hub.db.set_meta(WINS, &j);
            }
            Self::tell_casino(hub, st, CasinoRes::BigWin { win });
        }
    }

    /// Pays a round out, keeps the record, tells the casino of a big win, and shows the house's
    /// seed.
    #[allow(clippy::too_many_arguments)]
    fn finish_round(&self, hub: &Hub, st: &mut State, uid: &str, mut rec: Record, o: Outcome, server: [u8; 32], client: [u8; 32], nonce: u64, craps_after: Option<craps::Table>) -> Result<RoundView, String> {
        rec.chips += o.paid();
        let g = progress::apply_casino(&mut rec, &o);
        let held = st.held(uid);
        hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
        if !g.new_achievements.is_empty() || g.level_up.is_some() {
            let gains = pyr_poker::net::GainsView { xp: g.xp, achievements: g.new_achievements.iter().map(|s| s.to_string()).collect(), level_up: g.level_up, net: o.paid() as i64 - o.wagered() as i64 };
            hub.send_poker(uid, ToApp::Record { me: Self::view(hub, uid, &rec, held), gains: Some(gains) });
        }
        Self::round_counted(hub, st, uid, &o);
        let jackpot_pool = Self::jackpot(hub);
        if o.game() == Game::Slots && wall().saturating_sub(st.casino.jackpot_told) >= JACKPOT_TELL_MS {
            st.casino.jackpot_told = wall();
            Self::tell_casino(hub, st, CasinoRes::Jackpot { pool: jackpot_pool });
        }
        let p = st.casino.players.entry(uid.to_string()).or_insert_with(new_player);
        Ok(RoundView {
            nonce,
            commit: casino::commit(&server),
            server_seed: server,
            client_seed: client,
            outcome: o,
            next_commit: casino::commit(&p.seed),
            next_nonce: p.nonce,
            chips: rec.chips,
            jackpot: jackpot_pool,
            craps: craps_after,
        })
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::casino::{baccarat, roulette};
    use pyr_poker::net::{CasinoView, ToServer};
    use pyr_poker::progress::STARTING_CHIPS;

    use super::*;
    use crate::db::Db;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-casino-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ask(hub: &Hub, uid: &str, req: CasinoReq) -> Result<CasinoRes, String> {
        match hub.poker.handle(hub, uid, false, ToServer::Casino { req })? {
            Some(ToApp::Casino { res }) => Ok(res),
            other => Err(format!("unexpected {other:?}")),
        }
    }

    fn open(hub: &Hub, uid: &str) -> CasinoView {
        match ask(hub, uid, CasinoReq::Open).unwrap() {
            CasinoRes::State { view } => view,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rounds_take_and_pay_chips_and_check_out() {
        let hub = hub();
        let v = open(&hub, "alice");
        assert_eq!(v.chips, STARTING_CHIPS);
        assert_eq!(v.jackpot, slots::JACKPOT_SEED);
        // A fingerprint that isn't the one shown is refused (and nothing is taken).
        assert!(ask(&hub, "alice", CasinoReq::Play { commit: [0; 32], seed: [1; 32], bet: Bet::Slots { line_bet: 1 } }).is_err());
        // More than I have, or outside the limits: refused.
        assert!(ask(&hub, "alice", CasinoReq::Play { commit: v.commit, seed: [1; 32], bet: Bet::Dice { chips: STARTING_CHIPS + 1, chance: 5_000, over: false } }).is_err());
        assert!(ask(&hub, "alice", CasinoReq::Play { commit: v.commit, seed: [1; 32], bet: Bet::Slots { line_bet: 501 } }).is_err());
        let (mut chips, mut commit, mut nonce) = (v.chips, v.commit, v.nonce);
        let bets = [
            Bet::Slots { line_bet: 5 },
            Bet::Roulette { chips: vec![roulette::Chip { spot: roulette::Spot::Red, chips: 100 }, roulette::Chip { spot: roulette::Spot::Straight { n: 17 }, chips: 10 }] },
            Bet::Baccarat(baccarat::Bets { banker: 200, tie: 20, ..Default::default() }),
            Bet::Dice { chips: 50, chance: 2_500, over: true },
            Bet::Keno { chips: 10, picks: vec![4, 8, 15, 16, 23, 42] },
        ];
        for round_no in 0..3u8 {
            for (i, bet) in bets.iter().enumerate() {
                let CasinoRes::Round { round } = ask(&hub, "alice", CasinoReq::Play { commit, seed: [round_no * 10 + i as u8; 32], bet: bet.clone() }).unwrap() else { panic!() };
                assert_eq!(round.nonce, nonce);
                casino::verify(&commit, &round.server_seed, &round.client_seed, round.nonce, &round.outcome).unwrap();
                assert_eq!(round.chips, chips - bet.total() + round.outcome.paid(), "{bet:?}");
                assert_ne!(round.next_commit, commit, "a new seed every round");
                chips = round.chips;
                commit = round.next_commit;
                nonce = round.next_nonce;
            }
        }
        assert_eq!(hub.db.poker_record("alice").0.chips, chips);
        // The jackpot grew by its share of each spin (unless it was won).
        assert!(Poker::jackpot(&hub) >= slots::JACKPOT_SEED);
        // The casino's share went to the Friday Night pool.
        assert!(Poker::friday_pool(&hub) > 0);
        // Someone else's rounds come from their own seeds.
        let b = open(&hub, "bob");
        assert_ne!(b.commit, commit);
    }

    #[test]
    fn blackjack_and_craps_go_a_step_at_a_time() {
        let hub = hub();
        let v = open(&hub, "carol");
        let (mut chips, mut commit) = (v.chips, v.commit);
        // Blackjack: dealt (unless it's over at once), then stood on until it's over.
        let mut hands = 0;
        while hands < 6 {
            let start = chips;
            match ask(&hub, "carol", CasinoReq::Play { commit, seed: [hands as u8; 32], bet: Bet::Blackjack { chips: 100 } }).unwrap() {
                CasinoRes::Round { round } => {
                    chips = round.chips;
                    commit = round.next_commit;
                }
                CasinoRes::Blackjack { view, chips: c, next_commit, .. } => {
                    assert_eq!(c, start - 100);
                    assert!(view.hole_hidden, "the hole card stays hidden");
                    // Another hand can't start meanwhile; other games can.
                    assert!(ask(&hub, "carol", CasinoReq::Play { commit: next_commit, seed: [9; 32], bet: Bet::Blackjack { chips: 100 } }).is_err());
                    let deal_commit = commit;
                    let mut view = view;
                    loop {
                        let a = if view.phase == blackjack::Phase::Insurance { blackjack::Action::NoInsurance } else { blackjack::Action::Stand };
                        match ask(&hub, "carol", CasinoReq::Blackjack { action: a }).unwrap() {
                            CasinoRes::Blackjack { view: v2, .. } => view = v2,
                            CasinoRes::Round { round } => {
                                casino::verify(&deal_commit, &round.server_seed, &round.client_seed, round.nonce, &round.outcome).unwrap();
                                assert_eq!(round.chips, start - 100 + round.outcome.paid());
                                chips = round.chips;
                                commit = round.next_commit;
                                break;
                            }
                            other => panic!("{other:?}"),
                        }
                    }
                }
                other => panic!("{other:?}"),
            }
            hands += 1;
        }
        assert_eq!(hub.db.poker_record("carol").0.chips, chips);
        // Craps: bets onto the table, then rolls.
        let bets = craps::Bets { pass: 100, field: 20, ..Default::default() };
        let CasinoRes::Craps { table, chips: c } = ask(&hub, "carol", CasinoReq::CrapsBets { bets: bets.clone() }).unwrap() else { panic!() };
        assert_eq!((table.bets.clone(), c), (bets, chips - 120));
        chips = c;
        for k in 0..20 {
            let before = open(&hub, "carol");
            if before.craps.bets.is_empty() {
                let CasinoRes::Craps { chips: c, .. } = ask(&hub, "carol", CasinoReq::CrapsBets { bets: craps::Bets { field: 10, ..Default::default() } }).unwrap() else { panic!() };
                chips = c;
            }
            let commit = open(&hub, "carol").commit;
            let CasinoRes::Round { round } = ask(&hub, "carol", CasinoReq::CrapsRoll { commit, seed: [k; 32] }).unwrap() else { panic!() };
            casino::verify(&commit, &round.server_seed, &round.client_seed, round.nonce, &round.outcome).unwrap();
            assert_eq!(round.chips, chips + round.outcome.paid());
            chips = round.chips;
        }
        // Chips riding (a hand of blackjack, craps bets) when the server stops come back when it
        // starts again.
        let riding = open(&hub, "carol").craps.bets.total();
        let mut extra = 0;
        if let CasinoRes::Blackjack { .. } = ask(&hub, "carol", CasinoReq::Play { commit: open(&hub, "carol").commit, seed: [77; 32], bet: Bet::Blackjack { chips: 50 } }).unwrap() {
            extra = 50;
        }
        let have = hub.db.poker_record("carol").0.chips;
        Poker::casino_after_restart(&hub, &mut State::default());
        assert_eq!(hub.db.poker_record("carol").0.chips, have + riding + extra);
        assert!(Poker::at_risk(&hub).is_empty());
    }
}
