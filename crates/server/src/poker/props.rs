//! Prop bets (see `pyr_poker::props`), kept here: every prop, and a book of every bet,
//! cash-out, payout and refund, in the database (`props`, `prop_bets`), so a restart loses
//! nothing. The odds, bets, cash-outs, settling and payouts are all done here, under the
//! casino's lock, with Pyr Casino's chips; the apps only show them and send what's clicked.
//!
//! Two kinds. **A member's prop** lives in a channel: whoever opens it backs it with their own
//! chips and settles it (or a moderator does). **The casino's own** (0.15, `house`) belong to
//! no channel and nobody: they open, close and settle by themselves, from the props feed
//! (`pyr_proto::props_feed`, followed by `crate::props_feed`: [`Poker::props_follow`]) and
//! from the casino's own numbers, once whoever manages the server turns them on
//! ([`AutoKept`]). The casino backs those, so their winners are paid whatever was bet, and what
//! the bets brought in beyond that is the casino's.

use std::collections::{BTreeMap, HashSet};

use pyr_poker::net::{ToApp, ToServer};
use pyr_poker::props::{self as p, BetLine, Holding, PricePoint, PropAuto, PropLeader, PropState, PropStats, PropView};
use pyr_proto::props_feed::{Feed, FeedProp};
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use super::{Poker, progress_chips};
use crate::hub::{ConnId, Hub};

mod own;
pub mod parlay;

/// How many of the latest bets a prop shows.
const LATEST: usize = 8;
/// How long an app that asked for every prop hears of the casino's own as they change (Prop
/// Bets' window asks again every few minutes while it's open).
pub const WATCH_SECS: u64 = 15 * 60;
/// The casino's own props nobody bet on are forgotten this long after they're over.
const FORGET_AFTER_MS: i64 = 2 * 86_400_000;
/// A prop the feed brings is opened here only with this long left to bet on it.
const WORTH_OPENING_MS: i64 = 2 * 60_000;
/// How many props a message carries at most (a control frame holds a megabyte).
const A_MESSAGE: usize = 100;
/// The most props a list of everything gives an app that knows the casino's own: the open
/// ones, the ones I've something on, and the latest that are over.
const MAX_OPEN_LISTED: usize = 1500;
const MAX_MINE_LISTED: usize = 200;
const MAX_OVER_LISTED: usize = 120;
/// Who the casino's own props are by.
pub const HOUSE_NAME: &str = "Pyr Casino";

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
    /// pool, all it has to pay out from. The casino's own: the most the casino can lose on it
    /// (`house_backing`), which nobody put up.
    pub backing: u64,
    /// Everyone's shares and what they paid for them, per outcome.
    pub holdings: BTreeMap<String, Vec<(f64, u64)>>,
    pub latest: Vec<BetLine>,
    /// Once settled: the biggest winners (user, chips paid).
    pub winners: Vec<(String, u64)>,
    /// Its category (`pyr_poker::props::CATEGORIES`; none, from before 0.15: `community`).
    pub category: String,
    /// The casino's own: no channel, no author; it settles by itself.
    pub house: bool,
    /// The feed's name for it (`nfl:401772345:win`), or the casino's own for one it made from
    /// its numbers (`casino:20261117:jackpot`).
    pub feed_id: String,
    /// What it's about, shared by the props of one game or day; that's name; when that starts.
    pub event: String,
    pub event_name: String,
    pub starts_at: i64,
    /// A line about it as it stands; whose numbers settle it and how; what happened, once over.
    pub detail: String,
    pub source: String,
    pub rules: String,
    pub note: String,
    pub featured: u8,
    /// How its odds went (oldest first, `pyr_poker::props::HISTORY` points at most).
    pub history: Vec<PricePoint>,
    /// What parlays have taken of each outcome (0.16; see `parlay`): its parlay book.
    pub pq: Vec<f64>,
    /// The casino's own: the odds it opened at (the feed's), as shares like `q`. A parlay's
    /// leg is priced against these as well as `q` (`pyr_poker::props::leg_price`). One opened
    /// before 0.16 has none until the feed is next followed, and is no leg till then.
    pub line: Vec<f64>,
}

fn state_name(s: PropState) -> &'static str {
    match s {
        PropState::Open => "open",
        PropState::Closed => "closed",
        PropState::Settled => "settled",
        PropState::Void => "void",
    }
}

/// Who's asking, as their connection sees it: which channels they can see, which of them they
/// moderate (Manage messages in a server's channel), whether they manage the server (the
/// casino's own props are theirs to turn on, and to call off), and whether their app knows
/// the casino's own props at all (`PROPS2_CAP`: an app that doesn't is shown members' only).
pub struct Asker<'a> {
    pub uid: &'a str,
    pub sees: &'a dyn Fn(i64) -> bool,
    pub moderates: &'a dyn Fn(i64) -> bool,
    pub manages: bool,
    pub house: bool,
    /// Their connection (a long list goes to it a message at a time).
    pub conn: Option<ConnId>,
}

/// The casino's own props on this server, as kept (`meta`'s `props_auto`): off until whoever
/// manages the server turns them on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoKept {
    pub on: bool,
    /// The categories turned off.
    pub off: Vec<String>,
    /// How deep their markets are (0: `HOUSE_DEPTH`).
    pub depth: u64,
}

impl AutoKept {
    pub fn load(hub: &Hub) -> AutoKept {
        hub.db.meta_json("props_auto").unwrap_or_default()
    }

    fn depth(&self) -> u64 {
        if p::DEPTHS.iter().any(|d| d.0 == self.depth) { self.depth } else { p::HOUSE_DEPTH }
    }

    /// Whether props of a category open here.
    pub fn takes(&self, category: &str) -> bool {
        self.on && !self.off.iter().any(|c| c == category)
    }
}

/// The feed as this server last read it (see `crate::props_feed`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeedSeen {
    /// When the feed read last was made (ms; 0: none read yet), and how many props it had.
    pub at: i64,
    pub props: u32,
    /// What went wrong the last time, if something did.
    pub error: String,
}

/// What following the feed did here.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Followed {
    pub opened: u32,
    pub settled: u32,
    pub voided: u32,
    pub changed: u32,
}

impl Prop {
    fn load(hub: &Hub, id: u64) -> Result<Prop, String> {
        hub.db.prop(id).and_then(|j| serde_json::from_str(&j).ok()).ok_or_else(|| "That prop is gone.".to_string())
    }

    fn save(&self, hub: &Hub) -> Result<(), String> {
        let j = serde_json::to_string(self).map_err(|e| e.to_string())?;
        hub.db.prop_save(self.id, state_name(self.state), &j).map_err(|e| e.to_string())
    }

    /// Kept for the first time: it gets its number.
    fn insert(&mut self, hub: &Hub) -> Result<(), String> {
        let j = serde_json::to_string(self).map_err(|e| e.to_string())?;
        self.id = hub.db.prop_insert(self.channel_id, &self.author, state_name(self.state), self.created_at, &j).map_err(|e| e.to_string())?;
        self.save(hub)
    }

    /// Bets are taken now (and the prop closes, if its time came).
    fn taking_bets(&mut self, now: i64) -> bool {
        if self.state == PropState::Open && self.closes_at > 0 && now >= self.closes_at {
            self.state = PropState::Closed;
        }
        self.state == PropState::Open
    }

    fn over(&self) -> bool {
        matches!(self.state, PropState::Settled | PropState::Void)
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
        self.mark(now);
    }

    /// The odds as they are now, kept (how they went is shown as a line). Past
    /// `HISTORY` points, the older half thins out: every other one goes.
    fn mark(&mut self, now: i64) {
        let point = PricePoint { at: now, p: p::prices(&self.q, self.b).into_iter().map(|x| x as f32).collect() };
        self.history.push(point);
        if self.history.len() > p::HISTORY {
            let half = self.history.len() / 2;
            let mut i = 0;
            self.history.retain(|_| {
                i += 1;
                i > half || i % 2 == 1
            });
        }
    }

    /// Whether who's asking sees it at all: the casino's own, anyone whose app knows them; a
    /// member's, whoever sees its channel.
    fn seen_by(&self, who: &Asker) -> bool {
        if self.house { who.house } else { (who.sees)(self.channel_id) }
    }

    /// Whether who's asking may call it off (and settle it, a member's): its author or a
    /// moderator of its channel; the casino's own, whoever manages the server.
    fn theirs(&self, who: &Asker) -> bool {
        if self.house { who.manages } else { self.author == who.uid || (who.moderates)(self.channel_id) }
    }

    /// The prop as `uid` sees it (`may`: they moderate its channel, or manage the server for
    /// one of the casino's own; `history`: with how its odds went).
    pub fn view(&self, hub: &Hub, uid: &str, may: bool, history: bool) -> PropView {
        let mine = self
            .holdings
            .get(uid)
            .map(|h| h.iter().enumerate().filter(|(_, (s, c))| *s > 1e-9 || *c > 0).map(|(i, (s, c))| Holding { outcome: i, shares: *s, cost: *c }).collect())
            .unwrap_or_default();
        PropView {
            id: self.id,
            channel_id: self.channel_id,
            author: self.author.clone(),
            author_name: if self.house { HOUSE_NAME.to_string() } else { Poker::name_of(hub, &self.author) },
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
            can_settle: if self.house { may } else { uid == self.author || may },
            backing: self.backing,
            settled_by: if self.settled_by.is_empty() { String::new() } else { Poker::name_of(hub, &self.settled_by) },
            winners: self.winners.iter().map(|(u, c)| (Poker::name_of(hub, u), *c)).collect(),
            category: p::category_id(&self.category),
            house: self.house,
            event: self.event.clone(),
            event_name: self.event_name.clone(),
            starts_at: self.starts_at,
            detail: self.detail.clone(),
            source: self.source.clone(),
            rules: self.rules.clone(),
            note: self.note.clone(),
            settled_at: self.settled_at,
            featured: self.featured,
            bettors: self.holdings.values().filter(|h| h.iter().any(|(s, c)| *s > 1e-9 || *c > 0)).count() as u32,
            history: if history { self.history.clone() } else { Vec::new() },
            // (While it takes bets: what a parlay's leg on it is priced with. Its line, only
            // when bets have moved its odds from it.)
            pq: if self.over() || self.pq.iter().all(|x| *x == 0.0) { Vec::new() } else { self.pq.clone() },
            line: if self.over() || self.line == self.q { Vec::new() } else { self.line.clone() },
        }
    }

    /// The odds one of the casino's own opened at, from the feed's prop (`line`).
    fn line_from(&self, fp: &FeedProp) -> Vec<f64> {
        p::q_from_prior(&p::tidy_prior(&fp.prior, self.outcomes.len()), self.b)
    }

    fn view_for(&self, hub: &Hub, who: &Asker, history: bool) -> PropView {
        let may = if self.house { who.manages } else { (who.moderates)(self.channel_id) };
        self.view(hub, who.uid, may, history)
    }

    /// Everyone who should hear of it gets their own view of it: a member's, whoever can see
    /// its channel; the casino's own, the apps watching them and whoever has something on it.
    fn tell(&self, hub: &Hub) {
        if self.house {
            tell_house(hub, std::slice::from_ref(self));
        } else {
            hub.send_props_in(self.channel_id, &|uid| Some(ToApp::Props { props: vec![self.view(hub, uid, hub.moderates(uid, self.channel_id), false)] }));
        }
    }
}

/// The casino's own props, as they changed, to the apps watching them and to whoever has
/// something on one of them (a hundred a message at most).
fn tell_house(hub: &Hub, props: &[Prop]) {
    for chunk in props.chunks(A_MESSAGE) {
        hub.send_props_house(&|uid| chunk.iter().any(|prop| prop.holdings.contains_key(uid)), &|uid| {
            let manages = hub.manages(uid);
            Some(ToApp::Props { props: chunk.iter().map(|prop| prop.view(hub, uid, manages, false)).collect() })
        });
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

/// What of a feed's prop a server acts on once it's open here (its times, its line, how it
/// came out), as one number: the same number as last time means nothing to do.
fn what_counts(fp: &FeedProp) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (fp.closes, fp.starts, &fp.detail, fp.featured, &fp.event_name).hash(&mut h);
    fp.result.as_ref().map(|r| (r.winner, &r.note)).hash(&mut h);
    h.finish()
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
            ToServer::PropOpen { channel_id, question, outcomes, depth, closes_in, category } => {
                if channel_id == 0 || !(who.sees)(channel_id) {
                    return Err("No such channel.".into());
                }
                let (question, outcomes) = p::check_new(&question, &outcomes)?;
                let b = p::DEPTHS.iter().find(|d| d.0 == depth).map(|d| d.0 as f64).ok_or("pick how deep it is: Friendly, Big or High roller")?;
                if !p::CLOSES_IN.contains(&closes_in) {
                    return Err("pick when betting closes".into());
                }
                // (A category that isn't one of Pyr's: the member's own kind.)
                let category = Some(p::category_id(&category)).filter(|c| p::CATEGORIES.iter().any(|k| k.0 == c)).unwrap_or_else(|| p::COMMUNITY.to_string());
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
                    category,
                    ..Default::default()
                };
                prop.mark(now);
                prop.insert(hub)?;
                if let Err(e) = move_chips(hub, st, uid, -(backing as i64)) {
                    // (Can't happen under the casino's lock: they had the chips a moment ago.)
                    prop.state = PropState::Void;
                    prop.save(hub)?;
                    return Err(e);
                }
                let _ = hub.db.prop_bet_record(prop.id, uid, 0, -(backing as i64), 0.0, "backing", now);
                info!(prop = prop.id, channel = channel_id, outcomes = n, depth = b, "prop bet opened");
                let view = prop.view_for(hub, who, true);
                prop.tell(hub);
                Ok(Some(ToApp::PropOpened { prop: view }))
            }
            ToServer::PropBet { id, outcome, chips, min_shares } => {
                let mut prop = Prop::load(hub, id)?;
                if !prop.seen_by(who) {
                    return Err("That prop is gone.".into());
                }
                if !prop.taking_bets(now) {
                    prop.save(hub)?;
                    prop.tell(hub);
                    return Err("Betting on this prop has closed.".into());
                }
                if !prop.house && prop.author == uid {
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
                prop.tell(hub);
                let text = format!("You bet {} chips on {}: it pays {} if it wins.", progress_chips(chips), outcome_name(&prop, outcome), progress_chips(shares.floor() as u64));
                Ok(Some(ToApp::PropNote { id, text, chips: -(chips as i64) }))
            }
            ToServer::PropCashOut { id, outcome } => {
                let mut prop = Prop::load(hub, id)?;
                if !prop.seen_by(who) {
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
                let (shares, cost) = prop.holding(uid)[outcome];
                if shares <= 1e-9 {
                    return Err(format!("You have nothing on {}.", outcome_name(&prop, outcome)));
                }
                // (A share never sells for more than a chip, nor for less than nothing: a sum
                // that says otherwise has gone wrong, and nothing is done. 0.15.0's came out
                // infinite on a bet far past the depth, and took the bet for a chip.)
                let got = p::proceeds_for(&prop.q, prop.b, outcome, shares);
                if !got.is_finite() || got < -1e-6 || got > shares + 1.0 {
                    tracing::warn!(prop = id, outcome, shares, got, "a cash-out's sum went wrong");
                    return Err("That can't be cashed out just now: it still pays out when it's settled.".into());
                }
                let got = got.max(0.0).floor() as u64;
                prop.q[outcome] -= shares;
                prop.holding(uid)[outcome] = (0.0, 0);
                prop.pool = prop.pool.saturating_sub(got);
                prop.note_bet(hub, uid, outcome, -(got as i64), now);
                move_chips(hub, st, uid, got as i64)?;
                prop.save(hub)?;
                let _ = hub.db.prop_bet_record(id, uid, outcome, -(got as i64), -shares, "cash_out", now);
                let _ = hub.db.prop_result_record(uid, now, id, got as i64 - cost as i64, cost, false);
                info!(prop = id, outcome, chips = got, "prop cashed out");
                prop.tell(hub);
                let text = format!("You sold your {} back for {} chips.", outcome_name(&prop, outcome), progress_chips(got));
                Ok(Some(ToApp::PropNote { id, text, chips: got as i64 }))
            }
            ToServer::PropSettle { id, outcome } => {
                let mut prop = Prop::load(hub, id)?;
                if !prop.seen_by(who) {
                    return Err("That prop is gone.".into());
                }
                if prop.house {
                    return Err(format!("This one settles by itself, from {}.", if prop.source.is_empty() { "its source" } else { &prop.source }));
                }
                if !prop.theirs(who) {
                    return Err("Only whoever opened it (or a moderator) settles a prop.".into());
                }
                if prop.over() {
                    return Err("That prop is settled already.".into());
                }
                if outcome >= prop.outcomes.len() {
                    return Err("there's no such outcome".into());
                }
                Self::settle(hub, st, &mut prop, outcome, uid, "", now)?;
                prop.tell(hub);
                Ok(None)
            }
            ToServer::PropVoid { id } => {
                let mut prop = Prop::load(hub, id)?;
                if !prop.seen_by(who) {
                    return Err("That prop is gone.".into());
                }
                if prop.house {
                    if !who.manages {
                        return Err("Only someone who manages the server calls off one of the casino's own props.".into());
                    }
                } else {
                    let moderator = (who.moderates)(prop.channel_id);
                    let others_bet = prop.holdings.keys().any(|u| u != uid);
                    if !(moderator || (prop.author == uid && !others_bet)) {
                        return Err(if prop.author == uid { "People have bet on it: only a moderator can call it off now. Settle it instead.".into() } else { "Only whoever opened it (or a moderator) calls a prop off.".into() });
                    }
                }
                if prop.over() {
                    return Err("That prop is settled already.".into());
                }
                Self::void(hub, st, &mut prop, uid, "", now)?;
                prop.tell(hub);
                Ok(None)
            }
            ToServer::PropGet { ids } => {
                let props = ids
                    .into_iter()
                    .take(50)
                    .filter_map(|id| Prop::load(hub, id).ok())
                    .filter(|p| p.seen_by(who))
                    .map(|mut prop| {
                        // (One whose time came is closed as it's looked at.)
                        if prop.state == PropState::Open && !prop.taking_bets(now) {
                            let _ = prop.save(hub);
                        }
                        prop.view_for(hub, who, true)
                    })
                    .collect();
                Ok(Some(ToApp::Props { props }))
            }
            ToServer::PropList { channel_id: 0 } if who.house => {
                // An app that knows the casino's own props: every open prop I can see (theirs
                // and members'), the ones I've something on, and the latest that are over.
                let mut seen: HashSet<u64> = HashSet::new();
                let mut props: Vec<PropView> = Vec::new();
                let read = |j: String| serde_json::from_str::<Prop>(&j).ok().filter(|prop| prop.seen_by(who));
                for prop in hub.db.props_open().into_iter().rev().filter_map(read).take(MAX_OPEN_LISTED) {
                    if seen.insert(prop.id) {
                        props.push(prop.view_for(hub, who, false));
                    }
                }
                for prop in hub.db.props_of(uid, MAX_MINE_LISTED).into_iter().filter_map(read) {
                    if seen.insert(prop.id) {
                        props.push(prop.view_for(hub, who, false));
                    }
                }
                // (One of the casino's own called off with nothing of mine on it is of no interest.)
                let over = hub.db.props_recent(&["settled", "void"], MAX_OVER_LISTED * 6).into_iter().filter_map(read).filter(|prop| !(prop.house && prop.state == PropState::Void));
                for prop in over.take(MAX_OVER_LISTED) {
                    if seen.insert(prop.id) {
                        props.push(prop.view_for(hub, who, false));
                    }
                }
                // A hundred a message: all but the last straight to the app that asked.
                let mut parts: Vec<Vec<PropView>> = props.chunks(A_MESSAGE).map(|c| c.to_vec()).collect();
                let last = parts.pop().unwrap_or_default();
                match who.conn {
                    Some(conn) => {
                        for part in parts {
                            hub.send_poker_to(conn, ToApp::Props { props: part });
                        }
                        Ok(Some(ToApp::Props { props: last }))
                    }
                    // (No connection to send the rest to: all at once.)
                    None => Ok(Some(ToApp::Props { props: parts.into_iter().flatten().chain(last).collect() })),
                }
            }
            ToServer::PropList { channel_id: 0 } => {
                // Every channel I can read: open and closed ones, and settled or called off.
                let props = hub
                    .db
                    .props_recent_in_channels(&["open", "closed", "settled", "void"], p::MAX_LISTED * 4)
                    .into_iter()
                    .filter_map(|j| serde_json::from_str::<Prop>(&j).ok())
                    .filter(|prop| prop.seen_by(who))
                    .take(p::MAX_LISTED)
                    .map(|prop| prop.view_for(hub, who, false))
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
                    .map(|prop| prop.view_for(hub, who, false))
                    .collect();
                Ok(Some(ToApp::Props { props }))
            }
            ToServer::PropStats { days } => {
                let days = days.min(3650);
                let since = if days == 0 { 0 } else { now - days as i64 * 86_400_000 };
                let rows = hub.db.prop_leaders(since);
                let leader = |r: &(String, i64, u64, u32, u32, i64)| PropLeader { uid: r.0.clone(), name: Poker::name_of(hub, &r.0), net: r.1, staked: r.2, props: r.3, wins: r.4, biggest: r.5 };
                let rank = rows.iter().position(|r| r.0 == uid).map(|i| i + 1).unwrap_or(0);
                let me = match rank {
                    0 => PropLeader { uid: uid.to_string(), name: Poker::name_of(hub, uid), ..Default::default() },
                    n => leader(&rows[n - 1]),
                };
                let stats = PropStats { days, leaders: rows.iter().take(25).map(leader).collect(), me, rank: rank as u32, players: rows.len() as u32 };
                Ok(Some(ToApp::PropStats { stats }))
            }
            ToServer::PropAuto { set } => {
                if let Some(set) = set {
                    if !who.manages {
                        return Err("Only someone who manages the server turns the casino's own props on or off.".into());
                    }
                    if set.depth != 0 && !p::DEPTHS.iter().any(|d| d.0 == set.depth) {
                        return Err("pick how deep they are: Friendly, Big or High roller".into());
                    }
                    let mut off: Vec<String> = set.off.iter().take(64).map(|c| p::category_id(c)).collect();
                    off.sort();
                    off.dedup();
                    let before = AutoKept::load(hub);
                    let kept = AutoKept { on: set.on, off, depth: if set.depth == 0 { before.depth } else { set.depth } };
                    hub.db.set_meta("props_auto", &serde_json::to_string(&kept).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                    info!(on = kept.on, off = ?kept.off, depth = kept.depth(), "the casino's own props set");
                    // What's no longer wanted and nobody bet on goes at once; the rest plays out.
                    Self::house_tidy(hub, &kept, now);
                    // (The feed is read now rather than in a few minutes.)
                    if let Some(f) = hub.props_feed.get() {
                        f.nudge();
                    }
                }
                Ok(Some(ToApp::PropAuto { auto: self.auto_view(hub, who.manages) }))
            }
            _ => Err("not a prop bets request".into()),
        }
    }

    /// The casino's own props here, as someone is told of them.
    fn auto_view(&self, hub: &Hub, can_set: bool) -> PropAuto {
        let kept = AutoKept::load(hub);
        let seen = self.props_feed.lock().clone();
        PropAuto { on: kept.on, off: kept.off.clone(), depth: kept.depth(), can_set, feed_at: seen.at, feed_props: seen.props, feed_error: seen.error }
    }

    /// Settles a prop on `outcome`: a chip a winning share, to everyone who holds some (`by`:
    /// who settled it, nobody for one that settled by itself; `note`: what happened).
    fn settle(hub: &Hub, st: &super::State, prop: &mut Prop, outcome: usize, by: &str, note: &str, now: i64) -> Result<(), String> {
        let id = prop.id;
        prop.state = PropState::Settled;
        prop.winner = Some(outcome);
        prop.settled_at = now;
        prop.settled_by = by.to_string();
        prop.note = note.to_string();
        let won = outcome_name(prop, outcome);
        let what = if note.is_empty() { won.clone() } else { format!("{won} ({note})") };
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
                let text = format!("“{}” settled: {}. You won {} chips ({}{}).", prop.question, what, progress_chips(pay), if net >= 0 { "+" } else { "-" }, progress_chips(net.unsigned_abs()));
                hub.send_poker(&u, ToApp::PropNote { id, text, chips: pay as i64 });
            } else if staked > 0 {
                let text = format!("“{}” settled: {}. Your bet didn't win this time.", prop.question, what);
                hub.send_poker(&u, ToApp::PropNote { id, text, chips: 0 });
            }
            if staked > 0 {
                let _ = hub.db.prop_result_record(&u, now, id, pay as i64 - staked as i64, staked, pay > 0);
            }
        }
        Self::back_to_author(hub, st, prop, funds - out, now)?;
        paid.sort_by(|a, b| b.1.cmp(&a.1));
        paid.truncate(5);
        prop.winners = paid;
        prop.save(hub)?;
        info!(prop = id, outcome, house = prop.house, "prop settled");
        // (The parlays riding on it hear.)
        if prop.house {
            parlay::prop_over(hub, st, prop, now);
        }
        Ok(())
    }

    /// Calls a prop off: everyone gets back what they paid for the shares they still hold (from
    /// what the prop has: if cashing out took more than was put in, a share of it each), and its
    /// author the rest of the backing.
    fn void(hub: &Hub, st: &super::State, prop: &mut Prop, by: &str, note: &str, now: i64) -> Result<(), String> {
        let funds = prop.backing + prop.pool;
        let owed: Vec<(String, u64)> = prop.holdings.iter().map(|(u, h)| (u.clone(), h.iter().map(|x| x.1).sum::<u64>())).collect();
        let total: u64 = owed.iter().map(|o| o.1).sum();
        let scale = if total > funds { funds as f64 / total as f64 } else { 1.0 };
        let why = if note.is_empty() { String::new() } else { format!(" ({note})") };
        let mut out = 0u64;
        for (u, owe) in owed {
            let back = ((owe as f64 * scale).floor() as u64).min(funds - out);
            if back > 0 {
                move_chips(hub, st, &u, back as i64)?;
                out += back;
                let _ = hub.db.prop_bet_record(prop.id, &u, 0, back as i64, 0.0, "refund", now);
                hub.send_poker(&u, ToApp::PropNote { id: prop.id, text: format!("“{}” was called off{why}. Your {} chips are back.", prop.question, progress_chips(back)), chips: back as i64 });
            }
        }
        Self::back_to_author(hub, st, prop, funds - out, now)?;
        prop.state = PropState::Void;
        prop.settled_at = now;
        prop.settled_by = by.to_string();
        prop.note = note.to_string();
        prop.save(hub)?;
        info!(prop = prop.id, house = prop.house, "prop called off");
        // (A parlay's leg on it counts as 1.00 from here.)
        if prop.house {
            parlay::prop_over(hub, st, prop, now);
        }
        Ok(())
    }

    /// What's left of a prop's chips when it's settled or called off goes back to its author (who
    /// put up its backing). One of the casino's own has no author: nobody put its backing up,
    /// and what its bets brought in beyond the winnings is the casino's.
    fn back_to_author(hub: &Hub, st: &super::State, prop: &Prop, left: u64, now: i64) -> Result<(), String> {
        if left == 0 || prop.house {
            return Ok(());
        }
        move_chips(hub, st, &prop.author, left as i64)?;
        let _ = hub.db.prop_bet_record(prop.id, &prop.author, 0, left as i64, 0.0, "backing_back", now);
        let text = format!("“{}”: {} of the {} chips you put up to back it came back to you.", prop.question, progress_chips(left), progress_chips(prop.backing));
        hub.send_poker(&prop.author, ToApp::PropNote { id: prop.id, text, chips: left as i64 });
        Ok(())
    }

    /// Now and then (from the casino's clock): props whose time came close (everyone who sees
    /// them hears), a member's left unsettled for `MAX_OPEN_DAYS` is called off, and so is one
    /// of the casino's own whose result never came (`HOUSE_WAIT_DAYS` after betting closed).
    /// The casino's own that are over and nobody bet on are forgotten after two days.
    pub(super) fn props_sweep(&self, hub: &Hub, st: &super::State) {
        let now = pyr_proto::now_ms();
        let mut told: Vec<Prop> = Vec::new();
        for j in hub.db.props_open() {
            let Ok(mut prop) = serde_json::from_str::<Prop>(&j) else { continue };
            let forgotten = if prop.house { prop.closes_at > 0 && now - prop.closes_at > p::HOUSE_WAIT_DAYS * 86_400_000 } else { now - prop.created_at > p::MAX_OPEN_DAYS * 86_400_000 };
            if forgotten {
                let note = if prop.house { "its result never came" } else { "" };
                let _ = Self::void(hub, st, &mut prop, "", note, now);
            } else if prop.state == PropState::Open && !prop.taking_bets(now) {
                let _ = prop.save(hub);
            } else {
                continue;
            }
            if prop.house {
                told.push(prop);
            } else {
                prop.tell(hub);
            }
        }
        tell_house(hub, &told);
        // Parlays left behind their props go on from where they were.
        if self.parlays_behind.swap(false, std::sync::atomic::Ordering::Relaxed) {
            parlay::catch_up(hub, st, now);
        }
        // The casino's props about itself: made here, from its own count of rounds.
        self.props_about_casino(hub, st, now);
        // (One called off here by hand is remembered until its betting would have closed:
        // the feed lists it open till then, and forgotten sooner it would open again.)
        for (id, j) in hub.db.props_house_unbet_over(4000) {
            let Ok(prop) = serde_json::from_str::<Prop>(&j) else { continue };
            if now - prop.settled_at.max(prop.closes_at) > FORGET_AFTER_MS {
                let _ = hub.db.prop_delete(id);
            }
        }
    }

    /// The casino's own props that are no longer wanted here (turned off, or their category)
    /// and that nobody has bet on: called off and forgotten at once. Ones with bets on them
    /// play out.
    fn house_tidy(hub: &Hub, kept: &AutoKept, now: i64) {
        let mut gone: Vec<Prop> = Vec::new();
        for j in hub.db.props_open() {
            let Ok(mut prop) = serde_json::from_str::<Prop>(&j) else { continue };
            // (A parlay's leg counts as a bet on it: it plays out.)
            if !prop.house || kept.takes(&p::category_id(&prop.category)) || !prop.holdings.is_empty() || hub.db.prop_in_parlays(prop.id) {
                continue;
            }
            prop.state = PropState::Void;
            prop.settled_at = now;
            let _ = hub.db.prop_delete(prop.id);
            gone.push(prop);
        }
        tell_house(hub, &gone);
    }

    /// One of the casino's own props, opened: its odds start at `prior`, it closes at
    /// `closes_at`, and the casino backs it (`fp`: what it is, as the feed words it).
    fn house_open(hub: &Hub, fp: &FeedProp, depth: u64, now: i64) -> Result<Prop, String> {
        let (question, outcomes) = p::check_new(&fp.q, &fp.outcomes)?;
        let b = depth as f64;
        let prior = p::tidy_prior(&fp.prior, outcomes.len());
        let mut prop = Prop {
            question,
            outcomes,
            b,
            q: p::q_from_prior(&prior, b),
            created_at: now,
            closes_at: fp.closes,
            backing: p::house_backing(&prior, b),
            category: p::category_id(&fp.cat),
            house: true,
            feed_id: fp.id.clone(),
            event: fp.event.clone(),
            event_name: fp.event_name.clone(),
            starts_at: fp.starts,
            detail: fp.detail.clone(),
            source: fp.source.clone(),
            rules: fp.rules.clone(),
            featured: fp.featured,
            ..Default::default()
        };
        prop.line = prop.q.clone();
        prop.mark(now);
        prop.insert(hub)?;
        hub.db.prop_feed_set(&fp.id, prop.id).map_err(|e| e.to_string())?;
        Ok(prop)
    }

    /// Follows the props feed: opens what's new there (when the casino's own props are on
    /// here, and its category is), moves what changed (a game put back, how it stands), and
    /// settles or calls off what came out, as the feed says. Whatever was opened here before
    /// plays out even once they're turned off.
    pub fn props_follow(&self, hub: &Hub, feed: &Feed) -> Followed {
        let guard = self.state.lock();
        let st = &*guard;
        let now = pyr_proto::now_ms();
        let kept = AutoKept::load(hub);
        let map = hub.db.prop_feed_map();
        let mut did = Followed::default();
        let mut told: Vec<Prop> = Vec::new();
        // (The feed is a couple of thousand props, most of them the same as two minutes ago:
        // those are passed over here, so following it holds the casino up for no time.)
        let mut followed = self.props_followed.lock();
        followed.retain(|id, _| map.contains_key(id));
        for fp in &feed.props {
            let category = p::category_id(&fp.cat);
            // (The casino's props about itself are each server's own: never the feed's.)
            if category == p::CASINO {
                continue;
            }
            match map.get(&fp.id) {
                Some(&id) => {
                    let counts = what_counts(fp);
                    if followed.get(&fp.id) == Some(&counts) {
                        continue;
                    }
                    let Ok(mut prop) = Prop::load(hub, id) else { continue };
                    if prop.over() || !prop.house {
                        followed.insert(fp.id.clone(), counts);
                        continue;
                    }
                    match &fp.result {
                        Some(r) => {
                            let done = match r.winner {
                                Some(w) if w < prop.outcomes.len() => Self::settle(hub, st, &mut prop, w, "", &r.note, now).map(|()| did.settled += 1),
                                _ => Self::void(hub, st, &mut prop, "", &r.note, now).map(|()| did.voided += 1),
                            };
                            if let Err(e) = done {
                                // (Tried again the next time.)
                                debug!(prop = id, "a prop of the feed's couldn't be settled: {e}");
                                continue;
                            }
                            followed.insert(fp.id.clone(), counts);
                        }
                        None => {
                            // (One opened before parlays gets its line here, the first time
                            // the feed is followed after: the feed's odds for a prop are the
                            // ones it opened with, for as long as it lists it.)
                            let lined = prop.line.is_empty() && {
                                prop.line = prop.line_from(fp);
                                true
                            };
                            if prop.closes_at == fp.closes && prop.starts_at == fp.starts && prop.detail == fp.detail && prop.featured == fp.featured && prop.event_name == fp.event_name {
                                if lined && prop.save(hub).is_err() {
                                    continue;
                                }
                                followed.insert(fp.id.clone(), counts);
                                // (Apps hear of it only if its odds have moved from it.)
                                if lined && prop.line != prop.q {
                                    told.push(prop);
                                }
                                continue;
                            }
                            // (Put back: bets are taken again until its new time.)
                            if prop.closes_at != fp.closes && prop.state == PropState::Closed && now < fp.closes {
                                prop.state = PropState::Open;
                            }
                            prop.closes_at = fp.closes;
                            prop.starts_at = fp.starts;
                            prop.detail = fp.detail.clone();
                            prop.featured = fp.featured;
                            prop.event_name = fp.event_name.clone();
                            prop.taking_bets(now);
                            if prop.save(hub).is_err() {
                                continue;
                            }
                            followed.insert(fp.id.clone(), counts);
                            did.changed += 1;
                        }
                    }
                    told.push(prop);
                }
                None => {
                    if fp.result.is_some() || fp.closes < now + WORTH_OPENING_MS || !kept.takes(&category) {
                        continue;
                    }
                    match Self::house_open(hub, fp, kept.depth(), now) {
                        Ok(prop) => {
                            did.opened += 1;
                            told.push(prop);
                        }
                        Err(e) => debug!(feed = %fp.id, "a prop of the feed's wasn't opened: {e}"),
                    }
                }
            }
        }
        tell_house(hub, &told);
        if did != Followed::default() {
            info!(opened = did.opened, settled = did.settled, called_off = did.voided, changed = did.changed, "followed the props feed");
        }
        did
    }

    /// What the feed's reader has to say (`crate::props_feed`): how the last read went.
    pub fn props_feed_seen(&self, seen: FeedSeen) {
        *self.props_feed.lock() = seen;
    }

    /// Whether the feed is worth reading now: the casino's own props are on here, or some
    /// from before are still waiting for their results.
    pub fn props_wants_feed(&self, hub: &Hub) -> bool {
        AutoKept::load(hub).on || hub.db.props_house_open() > 0
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
        let who = Asker { uid, sees: &|_| true, moderates: &|_| false, manages: false, house: false, conn: None };
        hub.poker.props(hub, &who, msg)
    }

    fn open(hub: &Hub, uid: &str, q: &str, outcomes: &[&str], closes_in: u64) -> PropView {
        let msg = ToServer::PropOpen { channel_id: 1, question: q.into(), outcomes: outcomes.iter().map(|o| o.to_string()).collect(), depth: 10_000, closes_in, category: String::new() };
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
    fn every_channel_s_props_for_prop_bets_window_only_the_ones_i_can_read() {
        let hub = hub();
        let mine = open(&hub, "alice", "Rain on Saturday?", &[], 0);
        let theirs = match ask(&hub, "alice", ToServer::PropOpen { channel_id: 2, question: "Staff only?".into(), outcomes: vec![], depth: 10_000, closes_in: 0, category: String::new() }).unwrap() {
            Some(ToApp::PropOpened { prop }) => prop,
            other => panic!("{other:?}"),
        };
        // Settled ones come too (what I won or lost).
        let done = open(&hub, "alice", "Snow?", &[], 0);
        ask(&hub, "alice", ToServer::PropSettle { id: done.id, outcome: 1 }).unwrap();
        let only_one = Asker { uid: "bob", sees: &|c| c == 1, moderates: &|_| false, manages: false, house: false, conn: None };
        let Some(ToApp::Props { props }) = hub.poker.props(&hub, &only_one, ToServer::PropList { channel_id: 0 }).unwrap() else { panic!() };
        let ids: Vec<u64> = props.iter().map(|p| p.id).collect();
        assert_eq!(ids, vec![done.id, mine.id], "newest first, channel 1's only");
        assert!(!ids.contains(&theirs.id));
        assert_eq!(props[0].state, PropState::Settled);
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
        let modr = Asker { uid: "mod", sees: &|_| true, moderates: &|_| true, manages: false, house: false, conn: None };
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
        let blind = Asker { uid: "eve", sees: &|_| false, moderates: &|_| false, manages: false, house: false, conn: None };
        assert!(hub.poker.props(&hub, &blind, ToServer::PropGet { ids: vec![prop.id] }).map(|r| matches!(r, Some(ToApp::Props { props }) if props.is_empty())).unwrap());
        // Five open at once each.
        give(&hub, "dan", 100_000);
        for i in 0..5 {
            open(&hub, "dan", &format!("Question number {i}?"), &[], 0);
        }
        let msg = ToServer::PropOpen { channel_id: 1, question: "One too many?".into(), outcomes: vec![], depth: 10_000, closes_in: 0, category: String::new() };
        assert!(ask(&hub, "dan", msg).unwrap_err().contains("5 props open"));
        // Only the depths and closing times offered.
        let msg = ToServer::PropOpen { channel_id: 1, question: "A strange depth?".into(), outcomes: vec![], depth: 12_345, closes_in: 0, category: String::new() };
        assert!(ask(&hub, "erin", msg).is_err());
        // Backing a prop takes chips its author has.
        give(&hub, "fay", 5_000);
        let msg = ToServer::PropOpen { channel_id: 1, question: "Can I afford it?".into(), outcomes: vec![], depth: 10_000, closes_in: 0, category: String::new() };
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
        let modr = Asker { uid: "mod", sees: &|_| true, moderates: &|_| true, manages: false, house: false, conn: None };
        hub.poker.props(&hub, &modr, ToServer::PropVoid { id: prop.id }).unwrap();
        assert_eq!(everyone(&hub, &who), start, "called off");
        let book = hub.db.prop_bets(prop.id);
        assert_eq!(book.first().map(|b| b.4.as_str()), Some("backing"));
    }

    // ---- the casino's own props (0.15)

    use pyr_poker::props::PropAutoSet;
    use pyr_proto::props_feed::FeedResult;

    /// Someone whose app knows the casino's own props (`manages`: Manage server).
    fn ask2(hub: &Hub, uid: &str, manages: bool, msg: ToServer) -> Result<Option<ToApp>, String> {
        let who = Asker { uid, sees: &|_| true, moderates: &|_| false, manages, house: true, conn: None };
        hub.poker.props(hub, &who, msg)
    }

    fn list2(hub: &Hub, uid: &str) -> Vec<PropView> {
        match ask2(hub, uid, false, ToServer::PropList { channel_id: 0 }).unwrap() {
            Some(ToApp::Props { props }) => props,
            other => panic!("{other:?}"),
        }
    }

    fn auto(hub: &Hub, on: bool, off: &[&str], depth: u64) -> PropAuto {
        let set = PropAutoSet { on, off: off.iter().map(|c| c.to_string()).collect(), depth };
        match ask2(hub, "owner", true, ToServer::PropAuto { set: Some(set) }).unwrap() {
            Some(ToApp::PropAuto { auto }) => auto,
            other => panic!("{other:?}"),
        }
    }

    /// (The same moment all through a test run: a feed made twice is the same feed.)
    fn in_an_hour() -> i64 {
        static AT: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
        *AT.get_or_init(|| pyr_proto::now_ms() + 3_600_000)
    }

    fn fp(id: &str, cat: &str, q: &str, outcomes: &[&str], prior: &[f64]) -> FeedProp {
        FeedProp {
            id: id.into(),
            cat: cat.into(),
            event: id.rsplit_once(':').map(|x| x.0.to_string()).unwrap_or_default(),
            event_name: "Chiefs at Bills".into(),
            q: q.into(),
            outcomes: outcomes.iter().map(|o| o.to_string()).collect(),
            prior: prior.to_vec(),
            closes: in_an_hour(),
            starts: in_an_hour(),
            detail: "Sunday 4:25 PM ET".into(),
            source: "ESPN".into(),
            rules: "Settles on the final score, overtime included.".into(),
            ..Default::default()
        }
    }

    fn feed(props: Vec<FeedProp>) -> Feed {
        Feed { v: 1, made: pyr_proto::now_ms(), props }
    }

    fn three() -> Vec<FeedProp> {
        vec![
            fp("nfl:1:win", "nfl", "Who wins: Chiefs at Bills?", &["Chiefs", "Bills"], &[0.42, 0.58]),
            fp("crypto:btc:20261118:up", "crypto", "Bitcoin: higher at midnight UTC than now?", &["Yes", "No"], &[0.5, 0.5]),
            fp("weather:nyc:20261118:high", "weather", "New York's high on Wednesday?", &["41 or lower", "42 to 44", "45 to 47", "48 or higher"], &[0.15, 0.35, 0.35, 0.15]),
        ]
    }

    #[test]
    fn the_casino_s_own_props_are_off_until_someone_who_manages_the_server_turns_them_on() {
        let hub = hub();
        let f = feed(three());
        // Off: the feed opens nothing here.
        assert_eq!(hub.poker.props_follow(&hub, &f), Followed::default());
        assert!(list2(&hub, "bob").is_empty());
        assert!(!hub.poker.props_wants_feed(&hub));
        // Anyone may ask how it stands; only a manager sets it.
        let Some(ToApp::PropAuto { auto: a }) = ask2(&hub, "bob", false, ToServer::PropAuto { set: None }).unwrap() else { panic!() };
        assert_eq!((a.on, a.can_set, a.depth), (false, false, p::HOUSE_DEPTH));
        let set = PropAutoSet { on: true, ..Default::default() };
        assert!(ask2(&hub, "bob", false, ToServer::PropAuto { set: Some(set) }).unwrap_err().contains("manages the server"));
        assert!(ask2(&hub, "owner", true, ToServer::PropAuto { set: Some(PropAutoSet { on: true, off: vec![], depth: 5 }) }).is_err(), "only the depths offered");
        // On, all but the weather, a million deep.
        let a = auto(&hub, true, &["Weather"], 1_000_000);
        assert_eq!((a.on, a.off.clone(), a.depth, a.can_set), (true, vec!["weather".to_string()], 1_000_000, true));
        assert!(hub.poker.props_wants_feed(&hub));
        assert_eq!(hub.poker.props_follow(&hub, &f), Followed { opened: 2, ..Default::default() });
        let props = list2(&hub, "bob");
        let mut cats: Vec<&str> = props.iter().map(|v| v.category.as_str()).collect();
        cats.sort();
        assert_eq!(cats, vec!["crypto", "nfl"]);
        assert!(props.iter().all(|v| v.b == 1_000_000.0));
        // The same feed again: nothing more to do.
        assert_eq!(hub.poker.props_follow(&hub, &f), Followed::default());
        // The weather turned on too (the depth as it was).
        let a = auto(&hub, true, &[], 0);
        assert_eq!((a.off.len(), a.depth), (0, 1_000_000));
        assert_eq!(hub.poker.props_follow(&hub, &f).opened, 1);
        assert_eq!(list2(&hub, "bob").len(), 3);
    }

    #[test]
    fn the_casino_s_own_props_open_from_the_feed_and_settle_by_themselves() {
        let hub = hub();
        for u in ["bob", "carol"] {
            give(&hub, u, 1_000_000);
        }
        auto(&hub, true, &[], 100_000);
        assert_eq!(hub.poker.props_follow(&hub, &feed(three())).opened, 3);
        let props = list2(&hub, "bob");
        let nfl = props.iter().find(|v| v.category == "nfl").unwrap().clone();
        // As the feed worded it, by the casino, at the feed's odds; nobody here settles it.
        assert_eq!((nfl.question.as_str(), nfl.author_name.as_str(), nfl.house, nfl.channel_id), ("Who wins: Chiefs at Bills?", HOUSE_NAME, true, 0));
        assert_eq!((nfl.event.as_str(), nfl.event_name.as_str(), nfl.source.as_str(), nfl.detail.as_str()), ("nfl:1", "Chiefs at Bills", "ESPN", "Sunday 4:25 PM ET"));
        assert!(nfl.starts_at > 0 && nfl.closes_at == nfl.starts_at && !nfl.rules.is_empty());
        let pr = nfl.prices();
        assert!((pr[0] - 0.42).abs() < 1e-9 && (pr[1] - 0.58).abs() < 1e-9, "{pr:?}");
        assert!(!nfl.can_settle);
        assert_eq!(nfl.backing, p::house_backing(&[0.42, 0.58], 100_000.0));
        let weather = props.iter().find(|v| v.category == "weather").unwrap();
        assert_eq!(weather.outcomes.len(), 4);
        // An app from before them isn't shown them, and can't bet on one.
        let Some(ToApp::Props { props: old }) = ask(&hub, "bob", ToServer::PropList { channel_id: 0 }).unwrap() else { panic!() };
        assert!(old.is_empty());
        let Some(ToApp::Props { props: old }) = ask(&hub, "bob", ToServer::PropGet { ids: vec![nfl.id] }).unwrap() else { panic!() };
        assert!(old.is_empty());
        assert!(ask(&hub, "bob", ToServer::PropBet { id: nfl.id, outcome: 1, chips: 100, min_shares: 0.0 }).unwrap_err().contains("gone"));
        // Bob takes the Bills, Carol the Chiefs: the odds move from where the feed put them.
        let shares = p::shares_for(&nfl.q, nfl.b, 1, 50_000.0);
        ask2(&hub, "bob", false, ToServer::PropBet { id: nfl.id, outcome: 1, chips: 50_000, min_shares: shares }).unwrap();
        ask2(&hub, "carol", false, ToServer::PropBet { id: nfl.id, outcome: 0, chips: 20_000, min_shares: 0.0 }).unwrap();
        let Some(ToApp::Props { props: one }) = ask2(&hub, "bob", false, ToServer::PropGet { ids: vec![nfl.id] }).unwrap() else { panic!() };
        assert_eq!((one[0].bets, one[0].bettors, one[0].pool, one[0].mine.len()), (2, 2, 70_000, 1));
        assert_eq!(one[0].history.len(), 3, "how the odds went: as it opened, and after each bet");
        assert!((one[0].history[0].p[0] - 0.42).abs() < 1e-6);
        assert!(list2(&hub, "bob").iter().all(|v| v.history.is_empty()), "a list comes without");
        // Nobody settles it by hand; only a manager calls it off.
        assert!(ask2(&hub, "owner", true, ToServer::PropSettle { id: nfl.id, outcome: 0 }).unwrap_err().contains("settles by itself, from ESPN"));
        assert!(ask2(&hub, "bob", false, ToServer::PropVoid { id: nfl.id }).unwrap_err().contains("manages the server"));
        // The game is over: the feed says who won, and the server pays, with nobody deciding.
        let mut f = three();
        f[0].result = Some(FeedResult { winner: Some(1), note: "Bills 27, Chiefs 24".into(), at: pyr_proto::now_ms() });
        assert_eq!(hub.poker.props_follow(&hub, &feed(f.clone())), Followed { settled: 1, ..Default::default() });
        assert_eq!(chips(&hub, "bob"), 950_000 + shares.floor() as u64);
        assert_eq!(chips(&hub, "carol"), 980_000);
        let done = list2(&hub, "bob").into_iter().find(|v| v.id == nfl.id).unwrap();
        assert_eq!((done.state, done.winner, done.note.as_str(), done.settled_by.as_str()), (PropState::Settled, Some(1), "Bills 27, Chiefs 24", ""));
        assert!(done.settled_at > 0);
        assert_eq!(hub.db.prop_bets(nfl.id).iter().map(|b| b.4.as_str()).collect::<Vec<_>>(), vec!["bet", "bet", "payout"], "no backing put up, none to give back");
        // Told again: settled is settled.
        assert_eq!(hub.poker.props_follow(&hub, &feed(f)), Followed::default());
        // The best at props: Bob ahead, Carol behind.
        let Some(ToApp::PropStats { stats }) = ask2(&hub, "carol", false, ToServer::PropStats { days: 7 }).unwrap() else { panic!() };
        assert_eq!((stats.players, stats.rank, stats.leaders.len()), (2, 2, 2));
        assert_eq!((stats.leaders[0].uid.as_str(), stats.leaders[0].net, stats.leaders[0].wins, stats.leaders[0].props), ("bob", shares.floor() as i64 - 50_000, 1, 1));
        assert_eq!((stats.me.net, stats.me.staked, stats.me.wins), (-20_000, 20_000, 0));
        let Some(ToApp::PropStats { stats }) = ask2(&hub, "dan", false, ToServer::PropStats { days: 0 }).unwrap() else { panic!() };
        assert_eq!((stats.rank, stats.me.net), (0, 0));
    }

    #[test]
    fn a_house_prop_called_off_gives_the_chips_back_and_one_put_back_takes_bets_again() {
        let hub = hub();
        give(&hub, "bob", 100_000);
        auto(&hub, true, &[], 10_000);
        let mut f = three();
        hub.poker.props_follow(&hub, &feed(f.clone()));
        let nfl = list2(&hub, "bob").into_iter().find(|v| v.category == "nfl").unwrap();
        ask2(&hub, "bob", false, ToServer::PropBet { id: nfl.id, outcome: 0, chips: 30_000, min_shares: 0.0 }).unwrap();
        // Its time came: closed.
        let mut kept: Prop = serde_json::from_str(&hub.db.prop(nfl.id).unwrap()).unwrap();
        kept.closes_at = pyr_proto::now_ms() - 1;
        kept.save(&hub).unwrap();
        assert!(ask2(&hub, "bob", false, ToServer::PropBet { id: nfl.id, outcome: 0, chips: 100, min_shares: 0.0 }).unwrap_err().contains("closed"));
        // The game is put back a day: bets are taken again, until its new time.
        f[0].closes += 86_400_000;
        f[0].starts += 86_400_000;
        f[0].detail = "Postponed to Monday".into();
        assert_eq!(hub.poker.props_follow(&hub, &feed(f.clone())), Followed { changed: 1, ..Default::default() });
        let again = list2(&hub, "bob").into_iter().find(|v| v.id == nfl.id).unwrap();
        assert_eq!((again.state, again.closes_at, again.detail.as_str()), (PropState::Open, f[0].closes, "Postponed to Monday"));
        ask2(&hub, "bob", false, ToServer::PropBet { id: nfl.id, outcome: 0, chips: 100, min_shares: 0.0 }).unwrap();
        // Then it's called off: everything back.
        f[0].result = Some(FeedResult { winner: None, note: "the game was canceled".into(), at: 1 });
        assert_eq!(hub.poker.props_follow(&hub, &feed(f)), Followed { voided: 1, ..Default::default() });
        assert_eq!(chips(&hub, "bob"), 100_000);
        let gone = list2(&hub, "bob").into_iter().find(|v| v.id == nfl.id).unwrap();
        assert_eq!((gone.state, gone.note.as_str()), (PropState::Void, "the game was canceled"));
        // A manager may call one off by hand too.
        let btc = list2(&hub, "bob").into_iter().find(|v| v.category == "crypto").unwrap();
        ask2(&hub, "bob", false, ToServer::PropBet { id: btc.id, outcome: 0, chips: 5_000, min_shares: 0.0 }).unwrap();
        ask2(&hub, "owner", true, ToServer::PropVoid { id: btc.id }).unwrap();
        assert_eq!(chips(&hub, "bob"), 100_000);
    }

    #[test]
    fn the_feed_opens_nothing_that_s_over_or_about_to_close_or_the_casino_s_own_kind() {
        let hub = hub();
        auto(&hub, true, &[], 0);
        let mut f = three();
        f[0].result = Some(FeedResult { winner: Some(0), ..Default::default() });
        f[1].closes = pyr_proto::now_ms() + 30_000;
        f[2].cat = "casino".into();
        // (And one whose question is too long for a prop here.)
        f.push(fp("nfl:9:win", "nfl", &"Why ".repeat(60), &["Yes", "No"], &[0.5, 0.5]));
        assert_eq!(hub.poker.props_follow(&hub, &feed(f)), Followed::default());
        assert!(list2(&hub, "bob").is_empty());
    }

    #[test]
    fn turned_off_the_ones_nobody_bet_on_go_at_once_and_the_rest_play_out() {
        let hub = hub();
        give(&hub, "bob", 100_000);
        auto(&hub, true, &[], 0);
        let mut f = three();
        hub.poker.props_follow(&hub, &feed(f.clone()));
        let nfl = list2(&hub, "bob").into_iter().find(|v| v.category == "nfl").unwrap();
        ask2(&hub, "bob", false, ToServer::PropBet { id: nfl.id, outcome: 1, chips: 10_000, min_shares: 0.0 }).unwrap();
        // Off: the two nobody bet on are gone; the one Bob bet on stays and still wants the feed.
        auto(&hub, false, &[], 0);
        let left = list2(&hub, "bob");
        assert_eq!(left.iter().map(|v| v.id).collect::<Vec<_>>(), vec![nfl.id]);
        assert!(hub.poker.props_wants_feed(&hub));
        // Nothing new opens while they're off, and Bob's still settles.
        f[0].result = Some(FeedResult { winner: Some(1), note: "Bills 27, Chiefs 24".into(), at: 1 });
        assert_eq!(hub.poker.props_follow(&hub, &feed(f.clone())), Followed { settled: 1, ..Default::default() });
        assert!(chips(&hub, "bob") > 100_000);
        assert!(!hub.poker.props_wants_feed(&hub));
        // On again: the others open afresh (the settled one stays settled).
        auto(&hub, true, &[], 0);
        assert_eq!(hub.poker.props_follow(&hub, &feed(f)), Followed { opened: 2, ..Default::default() });
    }

    #[test]
    fn the_casino_can_lose_no_more_than_a_house_prop_s_backing() {
        let hub = hub();
        let who = ["bob", "carol", "dan"];
        for u in who {
            give(&hub, u, 50_000_000);
        }
        auto(&hub, true, &[], 100_000);
        // Long odds on the first outcome, which wins after everyone piles onto it.
        let f = vec![fp("science:quake:w47", "science", "A magnitude 7 earthquake this week?", &["Yes", "No"], &[0.05, 0.95])];
        hub.poker.props_follow(&hub, &feed(f.clone()));
        let prop = list2(&hub, "bob").remove(0);
        assert_eq!(prop.backing, p::house_backing(&[0.05, 0.95], 100_000.0));
        let start = everyone(&hub, &who);
        for (u, o, c) in [("bob", 0, 40_000), ("carol", 0, 9_000_000), ("dan", 1, 700), ("dan", 0, 20_000_000), ("bob", 0, 150)] {
            ask2(&hub, u, false, ToServer::PropBet { id: prop.id, outcome: o, chips: c, min_shares: 0.0 }).unwrap();
        }
        let mut done = f;
        done[0].result = Some(FeedResult { winner: Some(0), note: "M7.1 near Tonga".into(), at: 1 });
        hub.poker.props_follow(&hub, &feed(done));
        let won = everyone(&hub, &who) as i64 - start as i64;
        assert!(won > 0 && won <= prop.backing as i64, "the casino lost {won}, its backing was {}", prop.backing);
    }

    #[test]
    fn a_house_prop_whose_result_never_comes_is_called_off_and_unbet_ones_are_forgotten() {
        let hub = hub();
        give(&hub, "bob", 100_000);
        auto(&hub, true, &[], 0);
        hub.poker.props_follow(&hub, &feed(three()));
        let all = list2(&hub, "bob");
        let nfl = all.iter().find(|v| v.category == "nfl").unwrap().clone();
        ask2(&hub, "bob", false, ToServer::PropBet { id: nfl.id, outcome: 0, chips: 10_000, min_shares: 0.0 }).unwrap();
        // Betting closed eleven days ago and the feed never said how any of them came out.
        for v in &all {
            let mut kept: Prop = serde_json::from_str(&hub.db.prop(v.id).unwrap()).unwrap();
            kept.closes_at = pyr_proto::now_ms() - 11 * 86_400_000;
            kept.save(&hub).unwrap();
        }
        {
            let guard = hub.poker.state.lock();
            hub.poker.props_sweep(&hub, &guard);
        }
        assert_eq!(chips(&hub, "bob"), 100_000, "Bob's bet came back");
        let over = get2(&hub, "bob", nfl.id);
        assert_eq!((over.state, over.note.as_str()), (PropState::Void, "its result never came"));
        // Two days on, the ones nobody bet on are forgotten; Bob's stays in the book.
        for v in &all {
            let mut kept: Prop = serde_json::from_str(&hub.db.prop(v.id).unwrap()).unwrap();
            kept.settled_at = pyr_proto::now_ms() - 3 * 86_400_000;
            kept.save(&hub).unwrap();
        }
        {
            let guard = hub.poker.state.lock();
            hub.poker.props_sweep(&hub, &guard);
        }
        let left: Vec<u64> = all.iter().filter(|v| hub.db.prop(v.id).is_some()).map(|v| v.id).collect();
        assert_eq!(left, vec![nfl.id]);
        assert_eq!(hub.db.prop_feed_map().len(), 1);
    }

    /// A manager calls one of the casino's own off, nobody having bet on it: it stays off
    /// while the feed still lists it open (0.15.0 forgot it after two days, and the feed
    /// opened it again).
    #[test]
    fn a_house_prop_called_off_by_hand_stays_off_while_the_feed_still_lists_it() {
        let hub = hub();
        auto(&hub, true, &[], 0);
        let mut f = three();
        for p in f.iter_mut() {
            p.closes += 20 * 86_400_000;
            p.starts = p.closes;
        }
        assert_eq!(hub.poker.props_follow(&hub, &feed(f.clone())).opened, 3);
        let btc = list2(&hub, "bob").into_iter().find(|v| v.category == "crypto").unwrap();
        ask2(&hub, "owner", true, ToServer::PropVoid { id: btc.id }).unwrap();
        // Three days on: still in the book, and the feed doesn't open it again.
        let mut kept: Prop = serde_json::from_str(&hub.db.prop(btc.id).unwrap()).unwrap();
        kept.settled_at = pyr_proto::now_ms() - 3 * 86_400_000;
        kept.save(&hub).unwrap();
        {
            let guard = hub.poker.state.lock();
            hub.poker.props_sweep(&hub, &guard);
        }
        assert!(hub.db.prop(btc.id).is_some());
        assert_eq!(hub.poker.props_follow(&hub, &feed(f.clone())), Followed::default());
        assert_eq!(list2(&hub, "bob").iter().filter(|v| v.category == "crypto" && v.state == PropState::Open).count(), 0);
        // Once its betting would have been closed for two days, it's forgotten like the rest.
        kept.closes_at = pyr_proto::now_ms() - 3 * 86_400_000;
        kept.save(&hub).unwrap();
        {
            let guard = hub.poker.state.lock();
            hub.poker.props_sweep(&hub, &guard);
        }
        assert!(hub.db.prop(btc.id).is_none());
    }

    /// As it happened on 0.15.0 (Nov 18): everything on a 76% favourite, a hundred times the
    /// market's depth. Its cash-out came out infinite ("worth 18,446,744,073,709,551,615"),
    /// which took a chip and the whole bet from anyone who had a chip left.
    #[test]
    fn a_bet_far_past_the_depth_cashes_out_for_what_it_cost() {
        let hub = hub();
        give(&hub, "bob", 100_000_001);
        auto(&hub, true, &[], 1_000_000);
        let f = feed(vec![fp("cbb:1:win", "cbb", "Who wins: Seattle U at California Baptist?", &["Seattle U", "California Baptist"], &[0.76, 0.24])]);
        assert_eq!(hub.poker.props_follow(&hub, &f).opened, 1);
        let prop = list2(&hub, "bob").remove(0);
        let Some(ToApp::PropNote { text, .. }) = ask2(&hub, "bob", false, ToServer::PropBet { id: prop.id, outcome: 0, chips: 100_000_000, min_shares: 0.0 }).unwrap() else { panic!() };
        assert!(text.contains("it pays 100,274,436 if it wins"), "{text}");
        assert_eq!(chips(&hub, "bob"), 1);
        // What it's worth, as the window works it out: what it cost (nobody else has bet).
        let mine = get2(&hub, "bob", prop.id);
        let worth = mine.worth(&mine.mine[0]);
        assert!((worth - 100_000_000.0).abs() < 1.0, "{worth}");
        let Some(ToApp::PropNote { chips: back, .. }) = ask2(&hub, "bob", false, ToServer::PropCashOut { id: prop.id, outcome: 0 }).unwrap() else { panic!() };
        assert!((99_999_999..=100_000_000).contains(&back), "{back}");
        assert_eq!(chips(&hub, "bob") as i64, 1 + back);
        let after = get2(&hub, "bob", prop.id);
        assert!(after.mine.is_empty() && (after.prices()[0] - 0.76).abs() < 1e-6, "{:?}", after.prices());
    }

    fn get2(hub: &Hub, uid: &str, id: u64) -> PropView {
        match ask2(hub, uid, false, ToServer::PropGet { ids: vec![id] }).unwrap() {
            Some(ToApp::Props { mut props }) => props.remove(0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_list_of_everything_has_the_open_ones_mine_and_the_latest_over_and_members_props_have_categories() {
        let hub = hub();
        give(&hub, "bob", 1_000_000);
        auto(&hub, true, &[], 0);
        // A hundred and thirty of the feed's, and two of Alice's own (one about the casino).
        let many: Vec<FeedProp> = (0..130).map(|i| fp(&format!("nba:{i}:win"), "nba", &format!("Who wins game {i}?"), &["Away", "Home"], &[0.5, 0.5])).collect();
        assert_eq!(hub.poker.props_follow(&hub, &feed(many.clone())).opened, 130);
        let hers = open(&hub, "alice", "Rain on Saturday?", &[], 0);
        assert_eq!(hers.category, "community");
        let msg = ToServer::PropOpen { channel_id: 1, question: "Who wins Friday's tourney?".into(), outcomes: vec!["Bluelight".into(), "Maanix".into()], depth: 10_000, closes_in: 0, category: "Casino".into() };
        let Some(ToApp::PropOpened { prop: casino }) = ask(&hub, "alice", msg).unwrap() else { panic!() };
        assert_eq!((casino.category.as_str(), casino.house), ("casino", false));
        let msg = ToServer::PropOpen { channel_id: 1, question: "A kind of my own?".into(), outcomes: vec![], depth: 10_000, closes_in: 0, category: "my-own-kind".into() };
        let Some(ToApp::PropOpened { prop: odd }) = ask(&hub, "alice", msg).unwrap() else { panic!() };
        assert_eq!(odd.category, "community");
        let all = list2(&hub, "bob");
        assert_eq!(all.len(), 133);
        assert_eq!(all.iter().filter(|v| v.house).count(), 130);
        // Newest first among the open ones.
        assert_eq!(all[0].id, odd.id);
        // Bob bets on one; it's settled; forty more come out with nothing of his on them:
        // his is still in the list, with the latest that are over.
        ask2(&hub, "bob", false, ToServer::PropBet { id: all[132].id, outcome: 0, chips: 1_000, min_shares: 0.0 }).unwrap();
        let mut f = many;
        for fp in f.iter_mut().take(41) {
            fp.result = Some(FeedResult { winner: Some(0), note: String::new(), at: 1 });
        }
        assert_eq!(hub.poker.props_follow(&hub, &feed(f)).settled, 41);
        let all = list2(&hub, "bob");
        assert_eq!(all.iter().filter(|v| v.state == PropState::Open).count(), 92);
        assert!(all.iter().any(|v| !v.mine.is_empty() && v.state == PropState::Settled));
        assert_eq!(all.len(), 133);
        // A prop of no channel isn't a member's to open.
        let msg = ToServer::PropOpen { channel_id: 0, question: "Everyone's prop?".into(), outcomes: vec![], depth: 10_000, closes_in: 0, category: String::new() };
        assert!(ask(&hub, "alice", msg).unwrap_err().contains("No such channel"));
    }

    #[test]
    fn how_the_odds_went_is_kept_and_thins_out() {
        let hub = hub();
        give(&hub, "bob", 10_000_000);
        let prop = open(&hub, "alice", "Will it ever end?", &[], 0);
        for i in 0..300 {
            ask(&hub, "bob", ToServer::PropBet { id: prop.id, outcome: i % 2, chips: 100, min_shares: 0.0 }).unwrap();
        }
        let v = get(&hub, "bob", prop.id);
        assert!(v.history.len() <= p::HISTORY && v.history.len() > p::HISTORY / 2, "{}", v.history.len());
        assert!(v.history.windows(2).all(|w| w[0].at <= w[1].at));
        assert_eq!(v.history[0].p, vec![0.5, 0.5], "where it started is kept");
        let last = v.history.last().unwrap();
        assert!((last.p[0] as f64 - v.prices()[0]).abs() < 1e-6, "and where it is now");
    }
}
