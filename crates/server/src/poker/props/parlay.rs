//! Parlays (0.16; the rules and the sums are `pyr_poker::props`'s): two to ten of the casino's
//! own props in one bet. The server keeps each one (its legs as they were when it was placed,
//! their odds fixed then), moves the parlay book of each leg's prop as it's placed, and
//! settles it as its legs' props come out: lost at the first leg that loses, paid when the
//! last one left wins, a leg whose prop is called off counting as 1.00.
//!
//! The casino is the other side: the stake goes to it and what a parlay pays comes from it,
//! as with every other game here. What it can lose is kept in hand by the sums (a parlay
//! never pays more than its stake and its legs' rooms; every parlay moves its legs' parlay
//! odds; a leg pays the less of what its prop's odds say now and what they opened at, so a
//! single bet, which cashes out for what it cost, can't push a parlay's odds up) and by what
//! a leg can be (`parlay_kind`, `legs_clash`).

use pyr_poker::net::{ToApp, ToServer};
use pyr_poker::props::{self as p, LegState, ParlayLeg, ParlayState, ParlayView, PropState};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use super::{Asker, Prop, move_chips, outcome_name, tell_house};
use crate::hub::Hub;
use crate::poker::{Poker, State, progress_chips};

/// How many of someone's parlays that are over are listed with the ones riding.
const OVER_LISTED: usize = 60;

/// A parlay as the server keeps it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Parlay {
    pub id: u64,
    pub uid: String,
    pub chips: u64,
    pub legs: Vec<ParlayLeg>,
    /// Each leg's room, in the legs' order (`pyr_poker::props::leg_quote`): with the odds,
    /// what it pays is worked out again when a leg is called off.
    pub rooms: Vec<f64>,
    pub state: ParlayState,
    pub placed_at: i64,
    pub settled_at: i64,
    pub paid: u64,
}

fn state_name(s: ParlayState) -> &'static str {
    match s {
        ParlayState::Open => "open",
        ParlayState::Won => "won",
        ParlayState::Lost => "lost",
        ParlayState::Void => "void",
    }
}

impl Parlay {
    /// What it pays if every leg left in it wins.
    pub fn pays(&self) -> u64 {
        let left = self.legs.iter().zip(self.rooms.iter().chain(std::iter::repeat(&0.0))).filter(|(l, _)| l.state != LegState::Void).map(|(l, r)| (l.odds, *r));
        p::parlay_pays(self.chips, left).floor() as u64
    }

    pub fn view(&self) -> ParlayView {
        ParlayView { id: self.id, chips: self.chips, legs: self.legs.clone(), pays: self.pays(), state: self.state, placed_at: self.placed_at, settled_at: self.settled_at, paid: self.paid }
    }

    fn save(&self, hub: &Hub) -> Result<(), String> {
        let j = serde_json::to_string(self).map_err(|e| e.to_string())?;
        hub.db.parlay_save(self.id, state_name(self.state), &j).map_err(|e| e.to_string())
    }

    fn tell(&self, hub: &Hub) {
        hub.send_poker(&self.uid, ToApp::Parlays { parlays: vec![self.view()], all: false });
    }

    /// "3 legs" (the ones left in).
    fn legs_word(&self) -> String {
        let n = self.legs.iter().filter(|l| l.state != LegState::Void).count();
        format!("{n} leg{}", if n == 1 { "" } else { "s" })
    }

    /// After one of its legs came out: lost at once if that leg lost; paid if none is left
    /// to wait for; otherwise it rides on (`leg`: the one that just came out).
    fn step(&mut self, hub: &Hub, st: &State, leg: usize, now: i64) -> Result<(), String> {
        let l = self.legs[leg].clone();
        let what = if l.note.is_empty() { String::new() } else { format!(" ({})", l.note) };
        let lost = self.legs.iter().any(|l| l.state == LegState::Lost);
        let waiting = self.legs.iter().any(|l| l.state == LegState::Pending);
        // (What's said: its end, a leg called off, and the last leg left to wait for. The
        // legs that come in before that show on the ticket, without a word each.)
        let text;
        let mut chips = 0i64;
        if lost {
            self.state = ParlayState::Lost;
            self.settled_at = now;
            let _ = hub.db.parlay_result_record(&self.uid, now, self.id, -(self.chips as i64), self.chips, false);
            text = Some(format!("Your parlay lost: {} didn't come in{what}.", l.pick));
        } else if waiting {
            let to_go = self.legs.iter().filter(|l| l.state == LegState::Pending).count();
            let won = self.legs.iter().filter(|l| l.state == LegState::Won).count();
            text = match l.state {
                LegState::Void => Some(format!("A leg of your parlay was called off{what}: it counts as 1.00, and the parlay now pays {} chips.", progress_chips(self.pays()))),
                _ if to_go == 1 => Some(format!("{won} legs of your parlay are in, one to go: {} won{what}.", l.pick)),
                _ => None,
            };
        } else if self.legs.iter().all(|l| l.state == LegState::Void) {
            self.state = ParlayState::Void;
            self.settled_at = now;
            self.paid = self.chips;
            move_chips(hub, st, &self.uid, self.chips as i64)?;
            chips = self.chips as i64;
            text = Some(format!("Your parlay was called off: every leg was. Your {} chips are back.", progress_chips(self.chips)));
        } else {
            let pay = self.pays();
            self.state = ParlayState::Won;
            self.settled_at = now;
            self.paid = pay;
            move_chips(hub, st, &self.uid, pay as i64)?;
            chips = pay as i64;
            let net = pay as i64 - self.chips as i64;
            let _ = hub.db.parlay_result_record(&self.uid, now, self.id, net, self.chips, true);
            text = Some(format!("Your parlay came in: all {} won. You're paid {} chips (+{}).", self.legs_word(), progress_chips(pay), progress_chips(net.max(0) as u64)));
        }
        self.save(hub)?;
        self.tell(hub);
        if let Some(text) = text {
            hub.send_poker(&self.uid, ToApp::PropNote { id: 0, text, chips });
        }
        if self.state != ParlayState::Open {
            info!(parlay = self.id, state = state_name(self.state), paid = self.paid, "parlay over");
        }
        Ok(())
    }
}

/// One of the casino's own props is over (settled or called off): every parlay still riding
/// with a leg on it hears. (What goes wrong with one parlay is logged, and the rest go on.)
pub(super) fn prop_over(hub: &Hub, st: &State, prop: &Prop, now: i64) {
    for j in hub.db.parlays_on_prop(prop.id) {
        let Ok(mut t) = serde_json::from_str::<Parlay>(&j) else { continue };
        let Some(i) = t.legs.iter().position(|l| l.prop == prop.id && l.state == LegState::Pending) else { continue };
        t.legs[i].state = match (prop.state, prop.winner) {
            (PropState::Settled, Some(w)) if w == t.legs[i].outcome => LegState::Won,
            (PropState::Settled, Some(_)) => LegState::Lost,
            _ => LegState::Void,
        };
        t.legs[i].note = prop.note.clone();
        if let Err(e) = t.step(hub, st, i, now) {
            warn!(parlay = t.id, prop = prop.id, "a parlay's leg came out and it couldn't be settled: {e}");
        }
    }
}

impl Poker {
    /// A parlay request (`ToServer::Parlay*`), from `who`.
    pub fn parlays(&self, hub: &Hub, who: &Asker, msg: ToServer) -> Result<Option<ToApp>, String> {
        let guard = self.state.lock();
        let st = &*guard;
        let now = pyr_proto::now_ms();
        let uid = who.uid;
        match msg {
            ToServer::ParlayList => {
                let parlays = hub.db.parlays_of(uid, p::PARLAY_MAX_OPEN, OVER_LISTED).iter().filter_map(|j| serde_json::from_str::<Parlay>(j).ok()).map(|t| t.view()).collect();
                Ok(Some(ToApp::Parlays { parlays, all: true }))
            }
            ToServer::ParlayPlace { legs, chips, min_pays } => {
                if !who.house {
                    return Err("Update Pyr to place a parlay.".into());
                }
                if !(p::PARLAY_MIN_LEGS..=p::PARLAY_MAX_LEGS).contains(&legs.len()) {
                    return Err(format!("A parlay has {} to {} legs.", p::PARLAY_MIN_LEGS, p::PARLAY_MAX_LEGS));
                }
                if !(p::MIN_BET..=p::MAX_BET).contains(&chips) {
                    return Err(format!("A parlay's stake is {} to {} chips.", progress_chips(p::MIN_BET), progress_chips(p::MAX_BET)));
                }
                if hub.db.parlays_riding(uid) >= p::PARLAY_MAX_OPEN {
                    return Err(format!("You have {} parlays riding, the most at once: wait for one to settle.", p::PARLAY_MAX_OPEN));
                }
                // Each leg's prop, as it stands: the casino's own, taking bets, and no two
                // that hang together.
                let mut props: Vec<Prop> = Vec::with_capacity(legs.len());
                for (id, outcome) in &legs {
                    let mut prop = Prop::load(hub, *id)?;
                    if !prop.seen_by(who) {
                        return Err("That prop is gone.".into());
                    }
                    if !p::parlay_kind(prop.house, &prop.category) {
                        return Err(p::PARLAY_KIND.into());
                    }
                    if !prop.taking_bets(now) {
                        prop.save(hub)?;
                        prop.tell(hub);
                        return Err(format!("Betting on “{}” has closed: take it out of the parlay.", prop.question));
                    }
                    // (One opened before parlays, until the feed is next followed: a leg is
                    // priced against the odds its prop opened at, and those aren't known yet.)
                    if prop.line.len() != prop.q.len() {
                        return Err(format!("“{}” isn't ready for parlays yet: try again in a minute or two.", prop.question));
                    }
                    if *outcome >= prop.outcomes.len() {
                        return Err("there's no such outcome".into());
                    }
                    if let Some(why) = props.iter().find_map(|other| p::legs_clash((other.id, &other.event, &other.category), (prop.id, &prop.event, &prop.category))) {
                        return Err(why.into());
                    }
                    props.push(prop);
                }
                // Its odds, leg by leg, in each prop's parlay book (the less of what the
                // prop's odds say now and what they opened at); what it pays.
                let quotes: Vec<p::LegQuote> = props.iter().zip(&legs).map(|(prop, (_, outcome))| p::leg_price(&prop.q, &prop.line, &prop.pq, prop.b, *outcome, chips)).collect();
                if quotes.iter().any(|q| !q.odds.is_finite() || !q.shares.is_finite() || q.shares <= 0.0 || !q.room.is_finite()) {
                    return Err("That parlay is too big for these props.".into());
                }
                let pays = p::parlay_pays(chips, quotes.iter().map(|q| (q.odds, q.room)));
                if !pays.is_finite() || pays < chips as f64 + 1.0 {
                    return Err("That parlay would pay no more than its stake: these picks are all but sure, or the stake is far past what their markets take.".into());
                }
                if pays < min_pays * (1.0 - 1e-9) {
                    return Err("The odds moved: look again before you place it.".into());
                }
                move_chips(hub, st, uid, -(chips as i64))?;
                let mut t = Parlay {
                    uid: uid.to_string(),
                    chips,
                    legs: props
                        .iter()
                        .zip(&legs)
                        .zip(&quotes)
                        .map(|((prop, (_, outcome)), q)| ParlayLeg {
                            prop: prop.id,
                            outcome: *outcome,
                            question: prop.question.clone(),
                            pick: outcome_name(prop, *outcome),
                            category: p::category_id(&prop.category),
                            event_name: prop.event_name.clone(),
                            closes_at: prop.closes_at,
                            odds: q.odds,
                            state: LegState::Pending,
                            note: String::new(),
                        })
                        .collect(),
                    rooms: quotes.iter().map(|q| q.room).collect(),
                    state: ParlayState::Open,
                    placed_at: now,
                    ..Default::default()
                };
                let j = serde_json::to_string(&t).map_err(|e| e.to_string())?;
                let ids: Vec<u64> = props.iter().map(|prop| prop.id).collect();
                t.id = match hub.db.parlay_insert(uid, now, &j, &ids) {
                    Ok(id) => id,
                    Err(e) => {
                        // (Nothing kept of it: the stake goes back.)
                        let _ = move_chips(hub, st, uid, chips as i64);
                        return Err(e.to_string());
                    }
                };
                t.save(hub)?;
                // The parlay book of each leg's prop moves as a bet of the stake would.
                for ((prop, (_, outcome)), q) in props.iter_mut().zip(&legs).zip(&quotes) {
                    prop.pq.resize(prop.outcomes.len(), 0.0);
                    prop.pq[*outcome] += q.shares;
                    if let Err(e) = prop.save(hub) {
                        warn!(prop = prop.id, "a parlay's leg wasn't kept in its prop's parlay book: {e}");
                    }
                }
                tell_house(hub, &props);
                info!(parlay = t.id, legs = t.legs.len(), chips, pays = t.pays(), "parlay placed");
                t.tell(hub);
                let text = format!("Your parlay is on: {}, {} chips to win {}.", t.legs_word(), progress_chips(chips), progress_chips(t.pays()));
                Ok(Some(ToApp::PropNote { id: 0, text, chips: -(chips as i64) }))
            }
            _ => Err("not a parlay request".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::progress::Record;
    use pyr_proto::props_feed::{Feed, FeedProp, FeedResult};

    use super::*;
    use crate::db::Db;
    use crate::poker::props::Followed;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-parlay-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ask(hub: &Hub, uid: &str, msg: ToServer) -> Result<Option<ToApp>, String> {
        let who = Asker { uid, sees: &|_| true, moderates: &|_| false, manages: uid == "owner", house: true, conn: None };
        match msg {
            m @ (ToServer::ParlayPlace { .. } | ToServer::ParlayList) => hub.poker.parlays(hub, &who, m),
            m => hub.poker.props(hub, &who, m),
        }
    }

    fn chips(hub: &Hub, uid: &str) -> u64 {
        hub.db.poker_record(uid).0.chips
    }

    fn give(hub: &Hub, uid: &str, n: u64) {
        let mut rec = Record::new();
        rec.chips = n;
        hub.db.set_poker_record(uid, &rec, 0).unwrap();
    }

    fn in_an_hour() -> i64 {
        static AT: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
        *AT.get_or_init(|| pyr_proto::now_ms() + 3_600_000)
    }

    fn fp(id: &str, q: &str, outcomes: &[&str], prior: &[f64]) -> FeedProp {
        FeedProp {
            id: id.into(),
            cat: id.split(':').next().unwrap().into(),
            event: id.rsplit_once(':').map(|x| x.0.to_string()).unwrap_or_default(),
            event_name: q.into(),
            q: q.into(),
            outcomes: outcomes.iter().map(|o| o.to_string()).collect(),
            prior: prior.to_vec(),
            closes: in_an_hour(),
            starts: in_an_hour(),
            source: "ESPN".into(),
            rules: "Settles on the final score.".into(),
            ..Default::default()
        }
    }

    /// Three games' winners, a game's total, two coins and a city's high.
    fn board() -> Vec<FeedProp> {
        vec![
            fp("nfl:1:win", "Who wins: Chiefs at Bills?", &["Chiefs", "Bills"], &[0.4, 0.6]),
            fp("nfl:1:total", "Total points: Chiefs at Bills?", &["Over 44.5", "Under 44.5"], &[0.5, 0.5]),
            fp("nfl:2:win", "Who wins: Jets at Dolphins?", &["Jets", "Dolphins"], &[0.5, 0.5]),
            fp("nba:7:win", "Who wins: Jazz at Clippers?", &["Jazz", "Clippers"], &[0.25, 0.75]),
            fp("crypto:btc:d:up", "Bitcoin: higher at midnight?", &["Yes", "No"], &[0.5, 0.5]),
            fp("crypto:eth:d:up", "Ether: higher at midnight?", &["Yes", "No"], &[0.5, 0.5]),
            fp("weather:nyc:d:high", "New York's high?", &["41 or lower", "42 or higher"], &[0.5, 0.5]),
        ]
    }

    fn open(hub: &Hub) -> std::collections::HashMap<String, u64> {
        ask(hub, "owner", ToServer::PropAuto { set: Some(p::PropAutoSet { on: true, off: vec![], depth: 100_000 }) }).unwrap();
        assert_eq!(hub.poker.props_follow(hub, &Feed { v: 1, made: pyr_proto::now_ms(), props: board() }).opened, 7);
        hub.db.prop_feed_map()
    }

    fn settle(hub: &Hub, results: &[(&str, Option<usize>, &str)]) -> Followed {
        let mut props = board();
        for (id, winner, note) in results {
            props.iter_mut().find(|p| p.id == *id).unwrap().result = Some(FeedResult { winner: *winner, note: note.to_string(), at: 1 });
        }
        hub.poker.props_follow(hub, &Feed { v: 1, made: pyr_proto::now_ms(), props })
    }

    fn mine(hub: &Hub, uid: &str) -> Vec<ParlayView> {
        match ask(hub, uid, ToServer::ParlayList).unwrap() {
            Some(ToApp::Parlays { parlays, all: true }) => parlays,
            other => panic!("{other:?}"),
        }
    }

    fn place(hub: &Hub, uid: &str, legs: &[(u64, usize)], chips: u64) -> Result<Option<ToApp>, String> {
        ask(hub, uid, ToServer::ParlayPlace { legs: legs.to_vec(), chips, min_pays: 0.0 })
    }

    #[test]
    fn a_parlay_pays_when_every_leg_wins() {
        let hub = hub();
        give(&hub, "bob", 10_000);
        let ids = open(&hub);
        let (bills, dolphins, jazz) = (ids["nfl:1:win"], ids["nfl:2:win"], ids["nba:7:win"]);
        let Some(ToApp::PropNote { text, chips: c, .. }) = place(&hub, "bob", &[(bills, 1), (dolphins, 1), (jazz, 0)], 1_000).unwrap() else { panic!() };
        assert_eq!((chips(&hub, "bob"), c), (9_000, -1_000));
        let t = mine(&hub, "bob").remove(0);
        // 0.6, 0.5 and 0.25: about 1.67 x 2 x 4, a little under for the stake's own weight.
        let odds: Vec<f64> = t.legs.iter().map(|l| l.odds).collect();
        assert!(odds[0] > 1.65 && odds[0] < 1.667 && odds[1] > 1.98 && odds[1] < 2.0 && odds[2] > 3.9 && odds[2] < 4.0, "{odds:?}");
        assert_eq!((t.state, t.chips, t.legs.len(), t.legs[0].pick.as_str(), t.legs[2].category.as_str()), (ParlayState::Open, 1_000, 3, "Bills", "nba"));
        assert!((t.pays as f64 - 1_000.0 * t.times()).abs() < 1.0 && t.pays > 13_000 && t.pays < 13_340, "{}", t.pays);
        assert!(text.contains("3 legs") && text.contains(&progress_chips(t.pays)), "{text}");
        // Two legs come in: it rides on.
        settle(&hub, &[("nfl:1:win", Some(1), "Bills 27, Chiefs 24"), ("nfl:2:win", Some(1), "Dolphins 20, Jets 3")]);
        let t = mine(&hub, "bob").remove(0);
        assert_eq!((t.state, t.won_of(), t.legs[0].state, t.legs[0].note.as_str(), chips(&hub, "bob")), (ParlayState::Open, (2, 3), LegState::Won, "Bills 27, Chiefs 24", 9_000));
        // The last one too: paid, and counted among the server's best at props.
        settle(&hub, &[("nfl:1:win", Some(1), ""), ("nfl:2:win", Some(1), ""), ("nba:7:win", Some(0), "Jazz 132, Clippers 107")]);
        let t = mine(&hub, "bob").remove(0);
        assert_eq!((t.state, t.paid, chips(&hub, "bob")), (ParlayState::Won, t.pays, 9_000 + t.pays));
        let leaders = hub.db.prop_leaders(0);
        assert_eq!((leaders.len(), leaders[0].0.as_str(), leaders[0].1, leaders[0].2, leaders[0].4), (1, "bob", t.pays as i64 - 1_000, 1_000, 1));
    }

    #[test]
    fn a_parlay_is_lost_at_the_first_leg_that_loses() {
        let hub = hub();
        give(&hub, "bob", 10_000);
        let ids = open(&hub);
        place(&hub, "bob", &[(ids["nfl:1:win"], 1), (ids["nfl:2:win"], 1), (ids["nba:7:win"], 0)], 2_000).unwrap();
        settle(&hub, &[("nfl:2:win", Some(0), "Jets 9, Dolphins 6")]);
        let t = mine(&hub, "bob").remove(0);
        assert_eq!((t.state, t.paid, t.legs[1].state, t.legs[0].state, chips(&hub, "bob")), (ParlayState::Lost, 0, LegState::Lost, LegState::Pending, 8_000));
        // The legs that come out after change nothing.
        settle(&hub, &[("nfl:2:win", Some(0), ""), ("nfl:1:win", Some(1), ""), ("nba:7:win", Some(0), "")]);
        assert_eq!((mine(&hub, "bob")[0].state, chips(&hub, "bob")), (ParlayState::Lost, 8_000));
        assert_eq!(hub.db.prop_leaders(0)[0].1, -2_000);
    }

    /// A leg whose prop is called off counts as 1.00: the parlay goes on with the legs left,
    /// down to one; with none left the stake comes back.
    #[test]
    fn a_leg_called_off_counts_as_one() {
        let hub = hub();
        give(&hub, "bob", 10_000);
        let ids = open(&hub);
        place(&hub, "bob", &[(ids["nfl:1:win"], 1), (ids["nfl:2:win"], 1), (ids["nba:7:win"], 0)], 1_000).unwrap();
        place(&hub, "bob", &[(ids["nfl:1:win"], 0), (ids["nfl:2:win"], 0)], 500).unwrap();
        let before = mine(&hub, "bob").into_iter().find(|t| t.chips == 1_000).unwrap();
        // The Jets' game is called off, by the feed; the Chiefs', by whoever manages the server.
        settle(&hub, &[("nfl:2:win", None, "postponed")]);
        let t = mine(&hub, "bob").into_iter().find(|t| t.chips == 1_000).unwrap();
        assert_eq!((t.state, t.legs[1].state, t.won_of()), (ParlayState::Open, LegState::Void, (0, 2)));
        assert!((t.pays as f64 - 1_000.0 * t.legs[0].odds * t.legs[2].odds).abs() < 1.0 && t.pays < before.pays, "{} then {}", before.pays, t.pays);
        ask(&hub, "owner", ToServer::PropVoid { id: ids["nfl:1:win"] }).unwrap();
        // The two-leg one had both: its stake is back. The other is down to the Jazz alone.
        let all = mine(&hub, "bob");
        let small = all.iter().find(|t| t.chips == 500).unwrap();
        assert_eq!((small.state, small.paid, chips(&hub, "bob")), (ParlayState::Void, 500, 9_000));
        settle(&hub, &[("nfl:2:win", None, "postponed"), ("nba:7:win", Some(0), "Jazz 132, Clippers 107")]);
        let t = mine(&hub, "bob").into_iter().find(|t| t.chips == 1_000).unwrap();
        assert_eq!((t.state, t.won_of()), (ParlayState::Won, (1, 1)));
        assert!((t.paid as f64 - 1_000.0 * t.legs[2].odds).abs() < 1.0);
        assert_eq!(chips(&hub, "bob"), 9_000 + t.paid);
    }

    #[test]
    fn a_parlay_takes_the_casino_s_own_props_that_don_t_hang_together() {
        let hub = hub();
        give(&hub, "bob", 100_000);
        let ids = open(&hub);
        let (bills, total, jets, btc, eth, nyc) = (ids["nfl:1:win"], ids["nfl:1:total"], ids["nfl:2:win"], ids["crypto:btc:d:up"], ids["crypto:eth:d:up"], ids["weather:nyc:d:high"]);
        assert!(place(&hub, "bob", &[(bills, 1)], 100).unwrap_err().contains("2 to 10 legs"));
        assert!(place(&hub, "bob", &[(bills, 1), (bills, 0)], 100).unwrap_err().contains("already"));
        assert!(place(&hub, "bob", &[(bills, 1), (total, 0)], 100).unwrap_err().contains("same game"));
        assert!(place(&hub, "bob", &[(btc, 0), (eth, 0)], 100).unwrap_err().contains("One pick to a category"));
        assert!(place(&hub, "bob", &[(bills, 1), (jets, 5)], 100).unwrap_err().contains("no such outcome"));
        assert!(place(&hub, "bob", &[(bills, 1), (jets, 0)], 5).unwrap_err().contains("stake"));
        assert!(place(&hub, "bob", &[(bills, 1), (jets, 0)], 200_000).unwrap_err().contains("you have 100,000"));
        assert!(place(&hub, "bob", &[(bills, 1), (9_999, 0)], 100).unwrap_err().contains("gone"));
        // A member's own prop is no leg (its author settles it), and an app from before the
        // casino's own props places none.
        give(&hub, "alice", 100_000);
        let Some(ToApp::PropOpened { prop }) = ask(&hub, "alice", ToServer::PropOpen { channel_id: 1, question: "Will it rain?".into(), outcomes: vec![], depth: 10_000, closes_in: 0, category: String::new() }).unwrap() else { panic!() };
        assert!(place(&hub, "bob", &[(bills, 1), (prop.id, 0)], 100).unwrap_err().contains("settle by themselves"));
        let old = Asker { uid: "bob", sees: &|_| true, moderates: &|_| false, manages: false, house: false, conn: None };
        assert!(hub.poker.parlays(&hub, &old, ToServer::ParlayPlace { legs: vec![(bills, 1), (jets, 0)], chips: 100, min_pays: 0.0 }).unwrap_err().contains("Update Pyr"));
        // What it was shown to pay holds it: placed for less than that, it isn't.
        assert!(ask(&hub, "bob", ToServer::ParlayPlace { legs: vec![(bills, 1), (jets, 0)], chips: 100, min_pays: 1e9 }).unwrap_err().contains("odds moved"));
        assert_eq!((chips(&hub, "bob"), mine(&hub, "bob").len()), (100_000, 0));
        // Two games, a coin and a city's weather: four picks that don't hang together.
        place(&hub, "bob", &[(bills, 1), (jets, 0), (btc, 0), (nyc, 1)], 100).unwrap();
        assert_eq!(mine(&hub, "bob")[0].legs.len(), 4);
        // Betting closed on one of them: it's said which, and nothing is taken.
        let mut kept: Prop = serde_json::from_str(&hub.db.prop(jets).unwrap()).unwrap();
        kept.closes_at = pyr_proto::now_ms() - 1;
        kept.save(&hub).unwrap();
        let before = chips(&hub, "bob");
        assert!(place(&hub, "bob", &[(bills, 1), (jets, 0)], 100).unwrap_err().contains("Jets at Dolphins"));
        assert_eq!(chips(&hub, "bob"), before);
    }

    /// Every parlay moves the parlay book of each of its legs, as a bet of its stake would: the
    /// same picks again pay less, and the props' own odds (what single bets go by) don't move.
    #[test]
    fn parlays_move_their_legs_parlay_odds_and_not_the_props_own() {
        let hub = hub();
        give(&hub, "bob", 1_000_000);
        give(&hub, "carol", 1_000_000);
        let ids = open(&hub);
        let legs = [(ids["nfl:2:win"], 0), (ids["crypto:btc:d:up"], 0)];
        place(&hub, "bob", &legs, 50_000).unwrap();
        place(&hub, "carol", &legs, 50_000).unwrap();
        let (first, second) = (mine(&hub, "bob").remove(0), mine(&hub, "carol").remove(0));
        assert!(second.pays < first.pays * 8 / 10, "{} then {}", first.pays, second.pays);
        // The other side of both, after: longer odds than even.
        place(&hub, "carol", &[(ids["nfl:2:win"], 1), (ids["crypto:btc:d:up"], 1)], 1_000).unwrap();
        let other = mine(&hub, "carol").into_iter().find(|t| t.chips == 1_000).unwrap();
        assert!(other.legs.iter().all(|l| l.odds > 2.5), "{:?}", other.legs.iter().map(|l| l.odds).collect::<Vec<_>>());
        let Some(ToApp::Props { props }) = ask(&hub, "bob", ToServer::PropGet { ids: vec![ids["nfl:2:win"]] }).unwrap() else { panic!() };
        assert_eq!(props[0].prices(), vec![0.5, 0.5]);
        assert!(props[0].pq[0] > 140_000.0 && props[0].pq[1] > 0.0 && props[0].pq[1] < 10_000.0, "{:?}", props[0].pq);
        assert_eq!((props[0].bets, props[0].pool), (0, 0));
    }

    fn get(hub: &Hub, uid: &str, id: u64) -> p::PropView {
        match ask(hub, uid, ToServer::PropGet { ids: vec![id] }).unwrap() {
            Some(ToApp::Props { mut props }) => props.remove(0),
            other => panic!("{other:?}"),
        }
    }

    /// A single bet cashes out for what it cost, so it can't be let to lengthen a parlay's
    /// odds: someone would put a fortune on one side, take the other side in a parlay at the
    /// odds that leaves, and cash the fortune out. A leg pays the less of what its prop's
    /// odds say now and what they opened at.
    #[test]
    fn nobody_pushes_a_prop_s_odds_to_parlay_the_other_side() {
        let hub = hub();
        give(&hub, "bob", 1_000_000);
        let ids = open(&hub);
        let (jets, bills) = (ids["nfl:2:win"], ids["nfl:1:win"]);
        // As it opened: an app is told nothing but its odds.
        assert!(get(&hub, "bob", jets).line.is_empty());
        // 400,000 on the Jets: the Dolphins read 50 to 1 on the board.
        ask(&hub, "bob", ToServer::PropBet { id: jets, outcome: 0, chips: 400_000, min_shares: 0.0 }).unwrap();
        let v = get(&hub, "bob", jets);
        assert!(v.prices()[1] < 0.02 && v.line.len() == 2 && v.line != v.q, "{:?}", v.prices());
        // In a parlay they pay what they opened at (and the app works out the same odds).
        place(&hub, "bob", &[(jets, 1), (bills, 1)], 1_000).unwrap();
        let t = mine(&hub, "bob").remove(0);
        assert!(t.legs[0].odds > 1.95 && t.legs[0].odds < 2.0, "{}", t.legs[0].odds);
        assert_eq!(t.legs[0].odds, v.leg(1, 1_000).odds);
        assert!(t.pays < 3_400, "about 2 x 1.67, not a hundred times the stake: {}", t.pays);
        // The side that was pushed pays what it does now: next to nothing.
        place(&hub, "bob", &[(jets, 0), (bills, 1)], 1_000).unwrap();
        let t = mine(&hub, "bob").into_iter().find(|t| t.legs[0].outcome == 0).unwrap();
        assert!(t.legs[0].odds < 1.02, "{}", t.legs[0].odds);
        // The fortune cashed out, for what it cost: the board is where it opened, and nothing
        // came of it.
        ask(&hub, "bob", ToServer::PropCashOut { id: jets, outcome: 0 }).unwrap();
        assert_eq!(chips(&hub, "bob"), 1_000_000 - 2_000);
        let v = get(&hub, "bob", jets);
        assert!((v.prices()[0] - 0.5).abs() < 1e-9 && v.pq.len() == 2);
    }

    /// A prop opened before parlays has no line kept: it's no leg until the feed is next
    /// followed (every prop is looked at once after a server starts), which gives it the
    /// odds the feed opened it with.
    #[test]
    fn a_prop_from_before_parlays_gets_its_line_from_the_feed() {
        let hub = hub();
        give(&hub, "bob", 1_000_000);
        let ids = open(&hub);
        let (jets, jazz) = (ids["nfl:2:win"], ids["nba:7:win"]);
        // As 0.15 kept them: no line; and one of them with a bet on it.
        for id in [jets, jazz] {
            let mut kept: Prop = serde_json::from_str(&hub.db.prop(id).unwrap()).unwrap();
            kept.line.clear();
            kept.save(&hub).unwrap();
        }
        ask(&hub, "bob", ToServer::PropBet { id: jazz, outcome: 1, chips: 50_000, min_shares: 0.0 }).unwrap();
        assert!(place(&hub, "bob", &[(jets, 0), (jazz, 0)], 1_000).unwrap_err().contains("isn't ready for parlays yet"));
        assert_eq!(chips(&hub, "bob"), 950_000);
        // The server starts again (what it had followed is forgotten), and reads the feed.
        hub.poker.props_followed.lock().clear();
        assert_eq!(settle(&hub, &[]), Followed::default());
        let kept: Prop = serde_json::from_str(&hub.db.prop(jazz).unwrap()).unwrap();
        assert_eq!(kept.line, p::q_from_prior(&[0.25, 0.75], 100_000.0));
        assert!(kept.q[1] > kept.line[1] + 1.0);
        // The Jazz, whom the bet on the Clippers left long on the board: as they opened, 4.
        place(&hub, "bob", &[(jets, 0), (jazz, 0)], 1_000).unwrap();
        let t = mine(&hub, "bob").remove(0);
        assert!(t.legs[1].odds > 3.9 && t.legs[1].odds < 4.0, "{}", t.legs[1].odds);
        // (Followed again, nothing more to do: they're passed over.)
        assert_eq!(settle(&hub, &[]), Followed::default());
    }

    /// However long the shots and whatever the stake, no parlay wins more than its legs'
    /// rooms: what the casino could lose to those picks on their own props.
    #[test]
    fn the_casino_can_lose_no_more_to_a_parlay_than_to_its_legs() {
        let hub = hub();
        give(&hub, "bob", 100_000_000);
        let ids = open(&hub);
        let legs = [(ids["nfl:1:win"], 0), (ids["nfl:2:win"], 0), (ids["nba:7:win"], 0), (ids["crypto:btc:d:up"], 0), (ids["weather:nyc:d:high"], 0)];
        let room: f64 = [0.4f64, 0.5, 0.25, 0.5, 0.5].iter().map(|p| 100_000.0 * (1.0 / p).ln()).sum();
        for stake in [10u64, 5_000, 400_000, 50_000_000] {
            let before = chips(&hub, "bob");
            match place(&hub, "bob", &legs, stake) {
                Ok(_) => {
                    let t = mine(&hub, "bob").into_iter().find(|t| t.chips == stake).unwrap();
                    assert!(t.pays > stake && (t.pays - stake) as f64 <= room + 1.0, "{stake}: pays {}", t.pays);
                    assert_eq!(chips(&hub, "bob"), before - stake);
                }
                // (Far past what the markets take: it would win nothing, and isn't taken.)
                Err(e) => assert!(e.contains("no more than its stake") && chips(&hub, "bob") == before, "{stake}: {e}"),
            }
        }
        // Forty riding is the most.
        let hub = self::hub();
        give(&hub, "bob", 1_000_000);
        let ids = open(&hub);
        for _ in 0..p::PARLAY_MAX_OPEN {
            place(&hub, "bob", &[(ids["nfl:1:win"], 0), (ids["nfl:2:win"], 0)], 10).unwrap();
        }
        assert!(place(&hub, "bob", &[(ids["nfl:1:win"], 0), (ids["nfl:2:win"], 0)], 10).unwrap_err().contains("riding"));
    }

    /// A prop nobody bet on by itself, with a parlay's leg on it, plays out when its category
    /// is turned off (it used to go at once), and the parlay with it.
    #[test]
    fn a_prop_with_a_parlay_s_leg_on_it_plays_out() {
        let hub = hub();
        give(&hub, "bob", 10_000);
        let ids = open(&hub);
        place(&hub, "bob", &[(ids["nfl:1:win"], 1), (ids["nba:7:win"], 0)], 1_000).unwrap();
        ask(&hub, "owner", ToServer::PropAuto { set: Some(p::PropAutoSet { on: false, off: vec![], depth: 0 }) }).unwrap();
        assert!(hub.db.prop(ids["nfl:1:win"]).is_some() && hub.db.prop(ids["nba:7:win"]).is_some() && hub.db.prop(ids["nfl:2:win"]).is_none());
        settle(&hub, &[("nfl:1:win", Some(1), ""), ("nba:7:win", Some(0), "")]);
        let t = mine(&hub, "bob").remove(0);
        assert_eq!((t.state, chips(&hub, "bob")), (ParlayState::Won, 9_000 + t.paid));
    }
}
