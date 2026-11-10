//! Prop bets in chat (see `pyr_poker::props`), kept here: every prop, and a book of every bet,
//! cash-out, payout and refund, in the database (`props`, `prop_bets`), so a restart loses
//! nothing. The odds, bets, cash-outs, settling and payouts are all done here, under the
//! casino's lock, with Pyr Casino's chips; the apps only show them and send what's clicked.

use std::collections::BTreeMap;

use pyr_poker::net::{ToApp, ToServer};
use pyr_poker::props::{self as p, BetLine, Holding, PropState, PropView};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{Poker, progress_chips};
use crate::hub::Hub;

/// How many of the latest bets a prop shows.
const LATEST: usize = 8;

/// A prop as the server keeps it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prop {
    pub id: u64,
    pub channel_id: i64,
    pub author: String,
    pub question: String,
    pub outcomes: Vec<String>,
    pub b: f64,
    pub q: Vec<f64>,
    pub state: PropState,
    pub winner: Option<usize>,
    pub created_at: i64,
    pub closes_at: i64,
    pub settled_at: i64,
    pub settled_by: String,
    pub pool: u64,
    pub bets: u32,
    /// The chips its author put up to back its odds (`pyr_poker::props::backing`): with the
    /// pool, all it has to pay out from.
    pub backing: u64,
    /// Everyone's shares and what they paid for them, per outcome.
    pub holdings: BTreeMap<String, Vec<(f64, u64)>>,
    pub latest: Vec<BetLine>,
    /// Once settled: the biggest winners (user, chips paid).
    pub winners: Vec<(String, u64)>,
}

fn state_name(s: PropState) -> &'static str {
    match s {
        PropState::Open => "open",
        PropState::Closed => "closed",
        PropState::Settled => "settled",
        PropState::Void => "void",
    }
}

/// Who's asking, as their connection sees it: which channels they can see, and which of
/// them they moderate (Manage messages in a server's channel).
pub struct Asker<'a> {
    pub uid: &'a str,
    pub sees: &'a dyn Fn(i64) -> bool,
    pub moderates: &'a dyn Fn(i64) -> bool,
}

impl Prop {
    fn load(hub: &Hub, id: u64) -> Result<Prop, String> {
        hub.db.prop(id).and_then(|j| serde_json::from_str(&j).ok()).ok_or_else(|| "That prop is gone.".to_string())
    }

    fn save(&self, hub: &Hub) -> Result<(), String> {
        let j = serde_json::to_string(self).map_err(|e| e.to_string())?;
        hub.db.prop_save(self.id, state_name(self.state), &j).map_err(|e| e.to_string())
    }

    /// Bets are taken now (and the prop closes, if its time came).
    fn taking_bets(&mut self, now: i64) -> bool {
        if self.state == PropState::Open && self.closes_at > 0 && now >= self.closes_at {
            self.state = PropState::Closed;
        }
        self.state == PropState::Open
    }

    fn holding(&mut self, uid: &str) -> &mut Vec<(f64, u64)> {
        let n = self.outcomes.len();
        let h = self.holdings.entry(uid.to_string()).or_default();
        h.resize(n, (0.0, 0));
        h
    }

    fn note_bet(&mut self, hub: &Hub, uid: &str, outcome: usize, chips: i64, now: i64) {
        self.latest.insert(0, BetLine { uid: uid.to_string(), name: Poker::name_of(hub, uid), outcome, chips, at: now });
        self.latest.truncate(LATEST);
    }

    /// The prop as `uid` sees it (`moderates`: they may settle it).
    pub fn view(&self, hub: &Hub, uid: &str, moderates: bool) -> PropView {
        let mine = self
            .holdings
            .get(uid)
            .map(|h| h.iter().enumerate().filter(|(_, (s, c))| *s > 1e-9 || *c > 0).map(|(i, (s, c))| Holding { outcome: i, shares: *s, cost: *c }).collect())
            .unwrap_or_default();
        PropView {
            id: self.id,
            channel_id: self.channel_id,
            author: self.author.clone(),
            author_name: Poker::name_of(hub, &self.author),
            question: self.question.clone(),
            outcomes: self.outcomes.clone(),
            b: self.b,
            q: self.q.clone(),
            state: self.state,
            winner: self.winner,
            created_at: self.created_at,
            closes_at: self.closes_at,
            pool: self.pool,
            bets: self.bets,
            latest: self.latest.clone(),
            mine,
            can_settle: uid == self.author || moderates,
            backing: self.backing,
            settled_by: if self.settled_by.is_empty() { String::new() } else { Poker::name_of(hub, &self.settled_by) },
            winners: self.winners.iter().map(|(u, c)| (Poker::name_of(hub, u), *c)).collect(),
        }
    }

    /// Everyone who can see its channel gets their own view of it.
    fn tell_channel(&self, hub: &Hub) {
        hub.send_props_in(self.channel_id, &|uid| Some(ToApp::Props { props: vec![self.view(hub, uid, hub.moderates(uid, self.channel_id))] }));
    }
}

/// Chips to (or from: negative) someone's casino record, said to them.
fn move_chips(hub: &Hub, st: &super::State, uid: &str, chips: i64) -> Result<(), String> {
    let (mut rec, _) = hub.db.poker_record(uid);
    if chips < 0 && rec.chips < chips.unsigned_abs() {
        return Err(format!("that's {} chips, and you have {}", progress_chips(chips.unsigned_abs()), progress_chips(rec.chips)));
    }
    rec.chips = (rec.chips as i64 + chips).max(0) as u64;
    let held = st.held(uid);
    hub.db.set_poker_record(uid, &rec, held).map_err(|e| e.to_string())?;
    hub.send_poker(uid, ToApp::Record { me: Poker::view(hub, uid, &rec, held), gains: None });
    Ok(())
}

fn outcome_name(prop: &Prop, i: usize) -> String {
    prop.outcomes.get(i).cloned().unwrap_or_default()
}

impl Poker {
    /// A prop bets request (`ToServer::Prop*`), from `who`.
    pub fn props(&self, hub: &Hub, who: &Asker, msg: ToServer) -> Result<Option<ToApp>, String> {
        let guard = self.state.lock();
        let st = &*guard;
        let now = pyr_proto::now_ms();
        let uid = who.uid;
        match msg {
            ToServer::PropOpen { channel_id, question, outcomes, depth, closes_in } => {
                if !(who.sees)(channel_id) {
                    return Err("No such channel.".into());
                }
                let (question, outcomes) = p::check_new(&question, &outcomes)?;
                let b = p::DEPTHS.iter().find(|d| d.0 == depth).map(|d| d.0 as f64).ok_or("pick how deep it is: Friendly, Big or High roller")?;
                if !p::CLOSES_IN.contains(&closes_in) {
                    return Err("pick when betting closes".into());
                }
                if hub.db.props_open_by(uid) >= p::MAX_OPEN_EACH {
                    return Err(format!("You have {} props open: settle one first.", p::MAX_OPEN_EACH));
                }
                if hub.db.props_in(channel_id, &["open", "closed"], p::MAX_OPEN_IN_CHANNEL).len() >= p::MAX_OPEN_IN_CHANNEL {
                    return Err("This channel has as many props open as it can: settle some first.".into());
                }
                let n = outcomes.len();
                // Whoever opens it backs its odds with their own chips (and gets back what's left
                // when it's settled): a prop never makes chips out of nothing.
                let backing = p::backing(n, b);
                let have = hub.db.poker_record(uid).0.chips;
                if have < backing {
                    return Err(format!(
                        "Opening it puts up {} chips to back its odds (you get back what's left when it's settled), and you have {}. Pick Friendly, or fewer outcomes.",
                        progress_chips(backing),
                        progress_chips(have)
                    ));
                }
                let mut prop = Prop {
                    channel_id,
                    author: uid.to_string(),
                    question,
                    outcomes,
                    b,
                    q: vec![0.0; n],
                    created_at: now,
                    closes_at: if closes_in == 0 { 0 } else { now + closes_in as i64 * 1000 },
                    backing,
                    ..Default::default()
                };
                let j = serde_json::to_string(&prop).map_err(|e| e.to_string())?;
                prop.id = hub.db.prop_insert(channel_id, uid, "open", now, &j).map_err(|e| e.to_string())?;
                prop.save(hub)?;
                if let Err(e) = move_chips(hub, st, uid, -(backing as i64)) {
                    // (Can't happen under the casino's lock: they had the chips a moment ago.)
                    prop.state = PropState::Void;
                    prop.save(hub)?;
                    return Err(e);
                }
                let _ = hub.db.prop_bet_record(prop.id, uid, 0, -(backing as i64), 0.0, "backing", now);
                info!(prop = prop.id, channel = channel_id, outcomes = n, depth = b, "prop bet opened");
                let view = prop.view(hub, uid, (who.moderates)(channel_id));
                prop.tell_channel(hub);
                Ok(Some(ToApp::PropOpened { prop: view }))
            }
            ToServer::PropBet { id, outcome, chips, min_shares } => {
                let mut prop = Prop::load(hub, id)?;
                if !(who.sees)(prop.channel_id) {
                    return Err("That prop is gone.".into());
                }
                if !prop.taking_bets(now) {
                    prop.save(hub)?;
                    prop.tell_channel(hub);
                    return Err("Betting on this prop has closed.".into());
                }
                if prop.author == uid {
                    return Err("You opened this prop, so you're the one who settles it: you can't bet on it.".into());
                }
                if outcome >= prop.outcomes.len() {
                    return Err("there's no such outcome".into());
                }
                if !(p::MIN_BET..=p::MAX_BET).contains(&chips) {
                    return Err(format!("A bet is {} to {} chips.", progress_chips(p::MIN_BET), progress_chips(p::MAX_BET)));
                }
                let shares = p::shares_for(&prop.q, prop.b, outcome, chips as f64);
                if !shares.is_finite() || shares <= 0.0 {
                    return Err("That bet is too big for this prop.".into());
                }
                if shares < min_shares * (1.0 - 1e-9) {
                    return Err("The odds moved: look again before you bet.".into());
                }
                move_chips(hub, st, uid, -(chips as i64))?;
                prop.q[outcome] += shares;
                let h = prop.holding(uid);
                h[outcome].0 += shares;
                h[outcome].1 += chips;
                prop.pool += chips;
                prop.bets += 1;
                prop.note_bet(hub, uid, outcome, chips as i64, now);
                prop.save(hub)?;
                let _ = hub.db.prop_bet_record(id, uid, outcome, chips as i64, shares, "bet", now);
                info!(prop = id, outcome, chips, shares, "prop bet");
                prop.tell_channel(hub);
                let text = format!("You bet {} chips on {}: it pays {} if it wins.", progress_chips(chips), outcome_name(&prop, outcome), progress_chips(shares.floor() as u64));
                Ok(Some(ToApp::PropNote { id, text, chips: -(chips as i64) }))
            }
            ToServer::PropCashOut { id, outcome } => {
                let mut prop = Prop::load(hub, id)?;
                if !(who.sees)(prop.channel_id) {
                    return Err("That prop is gone.".into());
                }
                if !prop.taking_bets(now) {
                    prop.save(hub)?;
                    return Err("Betting on this prop has closed: it pays out when it's settled.".into());
                }
                let n = prop.outcomes.len();
                if outcome >= n {
                    return Err("there's no such outcome".into());
                }
                let (shares, _cost) = prop.holding(uid)[outcome];
                if shares <= 1e-9 {
                    return Err(format!("You have nothing on {}.", outcome_name(&prop, outcome)));
                }
                let got = p::proceeds_for(&prop.q, prop.b, outcome, shares).max(0.0).floor() as u64;
                prop.q[outcome] -= shares;
                prop.holding(uid)[outcome] = (0.0, 0);
                prop.pool = prop.pool.saturating_sub(got);
                prop.note_bet(hub, uid, outcome, -(got as i64), now);
                move_chips(hub, st, uid, got as i64)?;
                prop.save(hub)?;
                let _ = hub.db.prop_bet_record(id, uid, outcome, -(got as i64), -shares, "cash_out", now);
                info!(prop = id, outcome, chips = got, "prop cashed out");
                prop.tell_channel(hub);
                let text = format!("You sold your {} back for {} chips.", outcome_name(&prop, outcome), progress_chips(got));
                Ok(Some(ToApp::PropNote { id, text, chips: got as i64 }))
            }
            ToServer::PropSettle { id, outcome } => {
                let mut prop = Prop::load(hub, id)?;
                if !(who.sees)(prop.channel_id) {
                    return Err("That prop is gone.".into());
                }
                if prop.author != uid && !(who.moderates)(prop.channel_id) {
                    return Err("Only whoever opened it (or a moderator) settles a prop.".into());
                }
                if matches!(prop.state, PropState::Settled | PropState::Void) {
                    return Err("That prop is settled already.".into());
                }
                if outcome >= prop.outcomes.len() {
                    return Err("there's no such outcome".into());
                }
                prop.state = PropState::Settled;
                prop.winner = Some(outcome);
                prop.settled_at = now;
                prop.settled_by = uid.to_string();
                let won = outcome_name(&prop, outcome);
                // A chip a winning share, from what the bets brought in and the backing (which
                // the market maker's sums keep enough for; rounding aside, never more than it has).
                let owed: Vec<(String, u64, f64, u64)> = prop
                    .holdings
                    .iter()
                    .map(|(u, h)| {
                        let shares = h.get(outcome).map(|x| x.0).unwrap_or(0.0);
                        (u.clone(), shares.max(0.0).floor() as u64, shares, h.iter().map(|x| x.1).sum::<u64>())
                    })
                    .collect();
                let funds = prop.backing + prop.pool;
                let total: u64 = owed.iter().map(|o| o.1).sum();
                let scale = if total > funds { funds as f64 / total as f64 } else { 1.0 };
                let mut paid: Vec<(String, u64)> = Vec::new();
                let mut out = 0u64;
                for (u, owe, shares, staked) in owed {
                    let pay = ((owe as f64 * scale).floor() as u64).min(funds - out);
                    if pay > 0 {
                        move_chips(hub, st, &u, pay as i64)?;
                        out += pay;
                        let _ = hub.db.prop_bet_record(id, &u, outcome, pay as i64, shares, "payout", now);
                        paid.push((u.clone(), pay));
                        let net = pay as i64 - staked as i64;
                        let text = format!("“{}” settled: {}. You won {} chips ({}{}).", prop.question, won, progress_chips(pay), if net >= 0 { "+" } else { "-" }, progress_chips(net.unsigned_abs()));
                        hub.send_poker(&u, ToApp::PropNote { id, text, chips: pay as i64 });
                    } else if staked > 0 {
                        let text = format!("“{}” settled: {}. Your bet didn't win this time.", prop.question, won);
                        hub.send_poker(&u, ToApp::PropNote { id, text, chips: 0 });
                    }
                }
                Self::back_to_author(hub, st, &prop, funds - out, now)?;
                paid.sort_by(|a, b| b.1.cmp(&a.1));
                paid.truncate(5);
                prop.winners = paid;
                prop.save(hub)?;
                info!(prop = id, outcome, "prop settled");
                prop.tell_channel(hub);
                Ok(None)
            }
            ToServer::PropVoid { id } => {
                let mut prop = Prop::load(hub, id)?;
                if !(who.sees)(prop.channel_id) {
                    return Err("That prop is gone.".into());
                }
                let moderator = (who.moderates)(prop.channel_id);
                let others_bet = prop.holdings.keys().any(|u| u != uid);
                if !(moderator || (prop.author == uid && !others_bet)) {
                    return Err(if prop.author == uid { "People have bet on it: only a moderator can call it off now. Settle it instead.".into() } else { "Only whoever opened it (or a moderator) calls a prop off.".into() });
                }
                if matches!(prop.state, PropState::Settled | PropState::Void) {
                    return Err("That prop is settled already.".into());
                }
                Self::void(hub, st, &mut prop, uid, now)?;
                prop.tell_channel(hub);
                Ok(None)
            }
            ToServer::PropGet { ids } => {
                let props = ids
                    .into_iter()
                    .take(50)
                    .filter_map(|id| Prop::load(hub, id).ok())
                    .filter(|p| (who.sees)(p.channel_id))
                    .map(|mut prop| {
                        // (One whose time came is closed as it's looked at.)
                        if prop.state == PropState::Open && !prop.taking_bets(now) {
                            let _ = prop.save(hub);
                        }
                        prop.view(hub, uid, (who.moderates)(prop.channel_id))
                    })
                    .collect();
                Ok(Some(ToApp::Props { props }))
            }
            ToServer::PropList { channel_id } => {
                if !(who.sees)(channel_id) {
                    return Err("No such channel.".into());
                }
                let props = hub
                    .db
                    .props_in(channel_id, &["open", "closed"], p::MAX_OPEN_IN_CHANNEL)
                    .into_iter()
                    .filter_map(|j| serde_json::from_str::<Prop>(&j).ok())
                    .map(|prop| prop.view(hub, uid, (who.moderates)(channel_id)))
                    .collect();
                Ok(Some(ToApp::Props { props }))
            }
            _ => Err("not a prop bets request".into()),
        }
    }

    /// Calls a prop off: everyone gets back what they paid for the shares they still hold (from
    /// what the prop has: if cashing out took more than was put in, a share of it each), and its
    /// author the rest of the backing.
    fn void(hub: &Hub, st: &super::State, prop: &mut Prop, by: &str, now: i64) -> Result<(), String> {
        let funds = prop.backing + prop.pool;
        let owed: Vec<(String, u64)> = prop.holdings.iter().map(|(u, h)| (u.clone(), h.iter().map(|x| x.1).sum::<u64>())).collect();
        let total: u64 = owed.iter().map(|o| o.1).sum();
        let scale = if total > funds { funds as f64 / total as f64 } else { 1.0 };
        let mut out = 0u64;
        for (u, owe) in owed {
            let back = ((owe as f64 * scale).floor() as u64).min(funds - out);
            if back > 0 {
                move_chips(hub, st, &u, back as i64)?;
                out += back;
                let _ = hub.db.prop_bet_record(prop.id, &u, 0, back as i64, 0.0, "refund", now);
                hub.send_poker(&u, ToApp::PropNote { id: prop.id, text: format!("“{}” was called off. Your {} chips are back.", prop.question, progress_chips(back)), chips: back as i64 });
            }
        }
        Self::back_to_author(hub, st, prop, funds - out, now)?;
        prop.state = PropState::Void;
        prop.settled_at = now;
        prop.settled_by = by.to_string();
        prop.save(hub)?;
        info!(prop = prop.id, "prop called off");
        Ok(())
    }

    /// What's left of a prop's chips when it's settled or called off goes back to its author (who
    /// put up its backing).
    fn back_to_author(hub: &Hub, st: &super::State, prop: &Prop, left: u64, now: i64) -> Result<(), String> {
        if left == 0 {
            return Ok(());
        }
        move_chips(hub, st, &prop.author, left as i64)?;
        let _ = hub.db.prop_bet_record(prop.id, &prop.author, 0, left as i64, 0.0, "backing_back", now);
        let text = format!("“{}”: {} of the {} chips you put up to back it came back to you.", prop.question, progress_chips(left), progress_chips(prop.backing));
        hub.send_poker(&prop.author, ToApp::PropNote { id: prop.id, text, chips: left as i64 });
        Ok(())
    }

    /// Now and then (from the casino's clock): props whose time came close (everyone in the
    /// channel hears), and ones left unsettled for `MAX_OPEN_DAYS` are called off.
    pub(super) fn props_sweep(&self, hub: &Hub, st: &super::State) {
        let now = pyr_proto::now_ms();
        for j in hub.db.props_open() {
            let Ok(mut prop) = serde_json::from_str::<Prop>(&j) else { continue };
            if now - prop.created_at > p::MAX_OPEN_DAYS * 86_400_000 {
                let _ = Self::void(hub, st, &mut prop, "", now);
                prop.tell_channel(hub);
                continue;
            }
            if prop.state == PropState::Open && !prop.taking_bets(now) {
                let _ = prop.save(hub);
                prop.tell_channel(hub);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use pyr_poker::progress::{self, Record};

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-props-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn chips(hub: &Hub, uid: &str) -> u64 {
        hub.db.poker_record(uid).0.chips
    }

    fn give(hub: &Hub, uid: &str, n: u64) {
        let mut rec = Record::new();
        rec.chips = n;
        hub.db.set_poker_record(uid, &rec, 0).unwrap();
    }

    /// Everyone sees every channel; nobody moderates (unless said).
    fn ask(hub: &Hub, uid: &str, msg: ToServer) -> Result<Option<ToApp>, String> {
        let who = Asker { uid, sees: &|_| true, moderates: &|_| false };
        hub.poker.props(hub, &who, msg)
    }

    fn open(hub: &Hub, uid: &str, q: &str, outcomes: &[&str], closes_in: u64) -> PropView {
        let msg = ToServer::PropOpen { channel_id: 1, question: q.into(), outcomes: outcomes.iter().map(|o| o.to_string()).collect(), depth: 10_000, closes_in };
        match ask(hub, uid, msg).unwrap() {
            Some(ToApp::PropOpened { prop }) => prop,
            other => panic!("{other:?}"),
        }
    }

    fn get(hub: &Hub, uid: &str, id: u64) -> PropView {
        match ask(hub, uid, ToServer::PropGet { ids: vec![id] }).unwrap() {
            Some(ToApp::Props { mut props }) => props.remove(0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bets_move_the_odds_and_the_winners_are_paid_by_the_server() {
        let hub = hub();
        for u in ["bob", "carol"] {
            give(&hub, u, 100_000);
        }
        let prop = open(&hub, "alice", "Will Maanix hit a billion chips?", &[], 0);
        assert_eq!((prop.outcomes.clone(), prop.prices()), (vec!["Yes".to_string(), "No".into()], vec![0.5, 0.5]));
        // The author settles it, so doesn't bet on it.
        assert!(ask(&hub, "alice", ToServer::PropBet { id: prop.id, outcome: 0, chips: 100, min_shares: 0.0 }).unwrap_err().contains("you can't bet"));
        // Bob bets on Yes: his chips go, the odds move, and he's told what it pays.
        let shares = p::shares_for(&prop.q, prop.b, 0, 4_000.0);
        let Some(ToApp::PropNote { chips: c, text, .. }) = ask(&hub, "bob", ToServer::PropBet { id: prop.id, outcome: 0, chips: 4_000, min_shares: shares }).unwrap() else { panic!() };
        assert_eq!(c, -4_000);
        assert!(text.contains(&progress_chips(shares.floor() as u64)), "{text}");
        assert_eq!(chips(&hub, "bob"), 96_000);
        let after = get(&hub, "bob", prop.id);
        assert!(after.prices()[0] > 0.5);
        assert_eq!((after.pool, after.bets, after.mine.len()), (4_000, 1, 1));
        // Odds that moved under a bet: refused, nothing taken.
        assert!(ask(&hub, "carol", ToServer::PropBet { id: prop.id, outcome: 0, chips: 1_000, min_shares: 1e9 }).unwrap_err().contains("odds moved"));
        assert_eq!(chips(&hub, "carol"), 100_000);
        // More than she has: refused.
        assert!(ask(&hub, "carol", ToServer::PropBet { id: prop.id, outcome: 1, chips: 200_000, min_shares: 0.0 }).is_err());
        ask(&hub, "carol", ToServer::PropBet { id: prop.id, outcome: 1, chips: 6_000, min_shares: 0.0 }).unwrap();
        // Settled Yes: Bob is paid a chip a share; Carol gets nothing; every line is in the book.
        assert!(ask(&hub, "carol", ToServer::PropSettle { id: prop.id, outcome: 1 }).unwrap_err().contains("Only whoever opened it"));
        ask(&hub, "alice", ToServer::PropSettle { id: prop.id, outcome: 0 }).unwrap();
        assert_eq!(chips(&hub, "bob"), 96_000 + shares.floor() as u64);
        assert_eq!(chips(&hub, "carol"), 94_000);
        let done = get(&hub, "bob", prop.id);
        assert_eq!((done.state, done.winner), (PropState::Settled, Some(0)));
        assert_eq!(done.winners, vec![("Someone".to_string(), shares.floor() as u64)]);
        let book = hub.db.prop_bets(prop.id);
        assert_eq!(book.iter().map(|b| b.4.as_str()).collect::<Vec<_>>(), vec!["backing", "bet", "bet", "payout", "backing_back"]);
        // Settled is settled.
        assert!(ask(&hub, "alice", ToServer::PropSettle { id: prop.id, outcome: 1 }).is_err());
        assert!(ask(&hub, "bob", ToServer::PropBet { id: prop.id, outcome: 0, chips: 100, min_shares: 0.0 }).is_err());
    }

    #[test]
    fn cashing_out_and_calling_off_give_chips_back() {
        let hub = hub();
        give(&hub, "bob", 50_000);
        give(&hub, "carol", 50_000);
        let prop = open(&hub, "alice", "Who wins the Friday tourney?", &["Bluelight", "Maanix", "Someone else"], 0);
        ask(&hub, "bob", ToServer::PropBet { id: prop.id, outcome: 1, chips: 10_000, min_shares: 0.0 }).unwrap();
        // Sold straight back: about what was paid (nobody else bet in between).
        let Some(ToApp::PropNote { chips: back, .. }) = ask(&hub, "bob", ToServer::PropCashOut { id: prop.id, outcome: 1 }).unwrap() else { panic!() };
        assert!((9_998..=10_000).contains(&back), "{back}");
        assert!(ask(&hub, "bob", ToServer::PropCashOut { id: prop.id, outcome: 1 }).unwrap_err().contains("nothing on"));
        // Once someone else has bet, its author can't call it off; a moderator can, and
        // everyone gets back what they paid.
        ask(&hub, "carol", ToServer::PropBet { id: prop.id, outcome: 2, chips: 7_000, min_shares: 0.0 }).unwrap();
        assert!(ask(&hub, "alice", ToServer::PropVoid { id: prop.id }).unwrap_err().contains("moderator"));
        let modr = Asker { uid: "mod", sees: &|_| true, moderates: &|_| true };
        hub.poker.props(&hub, &modr, ToServer::PropVoid { id: prop.id }).unwrap();
        assert_eq!(chips(&hub, "carol"), 50_000);
        assert_eq!(get(&hub, "carol", prop.id).state, PropState::Void);
    }

    #[test]
    fn betting_closes_when_its_time_comes_and_props_are_checked() {
        let hub = hub();
        give(&hub, "bob", 10_000);
        let prop = open(&hub, "alice", "Rain on Saturday?", &[], 3600);
        // (Its time comes.)
        let mut kept: Prop = serde_json::from_str(&hub.db.prop(prop.id).unwrap()).unwrap();
        kept.closes_at = pyr_proto::now_ms() - 1;
        kept.save(&hub).unwrap();
        assert!(ask(&hub, "bob", ToServer::PropBet { id: prop.id, outcome: 0, chips: 100, min_shares: 0.0 }).unwrap_err().contains("closed"));
        assert_eq!(get(&hub, "bob", prop.id).state, PropState::Closed);
        // Settling a closed prop still works.
        ask(&hub, "alice", ToServer::PropSettle { id: prop.id, outcome: 1 }).unwrap();
        // A channel I can't see isn't there for me.
        let blind = Asker { uid: "eve", sees: &|_| false, moderates: &|_| false };
        assert!(hub.poker.props(&hub, &blind, ToServer::PropGet { ids: vec![prop.id] }).map(|r| matches!(r, Some(ToApp::Props { props }) if props.is_empty())).unwrap());
        // Five open at once each.
        give(&hub, "dan", 100_000);
        for i in 0..5 {
            open(&hub, "dan", &format!("Question number {i}?"), &[], 0);
        }
        let msg = ToServer::PropOpen { channel_id: 1, question: "One too many?".into(), outcomes: vec![], depth: 10_000, closes_in: 0 };
        assert!(ask(&hub, "dan", msg).unwrap_err().contains("5 props open"));
        // Only the depths and closing times offered.
        let msg = ToServer::PropOpen { channel_id: 1, question: "A strange depth?".into(), outcomes: vec![], depth: 12_345, closes_in: 0 };
        assert!(ask(&hub, "erin", msg).is_err());
        // Backing a prop takes chips its author has.
        give(&hub, "fay", 5_000);
        let msg = ToServer::PropOpen { channel_id: 1, question: "Can I afford it?".into(), outcomes: vec![], depth: 10_000, closes_in: 0 };
        assert!(ask(&hub, "fay", msg).unwrap_err().contains("puts up 6,932 chips"));
        assert_eq!(chips(&hub, "fay"), 5_000);
        assert!(progress::STARTING_CHIPS >= p::backing(2, 10_000.0), "anyone can open a Friendly yes-or-no prop");
    }

    fn everyone(hub: &Hub, who: &[&str]) -> u64 {
        who.iter().map(|u| chips(hub, u)).sum()
    }

    /// Chips are never made or lost by a prop: what winners get comes from the bets and the
    /// backing, and what's left goes back to whoever backed it. Settled, or called off after
    /// someone cashed out at a profit.
    #[test]
    fn props_never_make_or_lose_chips() {
        let hub = hub();
        let who = ["alice", "bob", "carol", "dan"];
        for u in who {
            give(&hub, u, 1_000_000);
        }
        let start = everyone(&hub, &who);
        let prop = open(&hub, "alice", "Who takes the Friday tourney?", &["Bob", "Carol", "Dan"], 0);
        assert_eq!(prop.backing, p::backing(3, 10_000.0));
        assert_eq!(chips(&hub, "alice"), 1_000_000 - prop.backing);
        for (u, o, c) in [("bob", 0, 4_000), ("carol", 1, 60_000), ("dan", 0, 900_000), ("bob", 2, 15)] {
            ask(&hub, u, ToServer::PropBet { id: prop.id, outcome: o, chips: c, min_shares: 0.0 }).unwrap();
        }
        ask(&hub, "carol", ToServer::PropCashOut { id: prop.id, outcome: 1 }).unwrap();
        assert_eq!(everyone(&hub, &who) + get(&hub, "bob", prop.id).pool + prop.backing, start);
        // Dan's huge bet wins: paid in full, from the bets and the backing.
        let dan_before = chips(&hub, "dan");
        let owed = get(&hub, "dan", prop.id).mine.iter().find(|h| h.outcome == 0).unwrap().shares.floor() as u64;
        ask(&hub, "alice", ToServer::PropSettle { id: prop.id, outcome: 0 }).unwrap();
        assert_eq!(chips(&hub, "dan"), dan_before + owed);
        assert_eq!(everyone(&hub, &who), start, "settled");

        // Bob buys in cheap, Dan pushes the price up, Bob sells at a profit; then it's called off.
        let start = everyone(&hub, &who);
        let prop = open(&hub, "alice", "Rain on Saturday?", &[], 0);
        ask(&hub, "bob", ToServer::PropBet { id: prop.id, outcome: 0, chips: 50_000, min_shares: 0.0 }).unwrap();
        ask(&hub, "dan", ToServer::PropBet { id: prop.id, outcome: 0, chips: 500_000, min_shares: 0.0 }).unwrap();
        let Some(ToApp::PropNote { chips: got, .. }) = ask(&hub, "bob", ToServer::PropCashOut { id: prop.id, outcome: 0 }).unwrap() else { panic!() };
        assert!(got > 50_000, "{got}");
        let modr = Asker { uid: "mod", sees: &|_| true, moderates: &|_| true };
        hub.poker.props(&hub, &modr, ToServer::PropVoid { id: prop.id }).unwrap();
        assert_eq!(everyone(&hub, &who), start, "called off");
        let book = hub.db.prop_bets(prop.id);
        assert_eq!(book.first().map(|b| b.4.as_str()), Some("backing"));
    }
}
