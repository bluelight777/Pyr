//! Everything Pyr told the window, as it stands, and what's worked out from it: which props
//! are in a category, which belong to one game, what I hold and what it's worth.

use std::collections::{BTreeMap, HashMap};
use std::time::Instant;

use pyr_poker::props::{self as p, ParlayView, PropAuto, PropState, PropStats, PropView};
use serde_json::Value;

/// A change in an outcome's odds, lately: what it was, and when it changed (it flashes).
#[derive(Clone, Copy, Debug)]
pub struct Moved {
    pub from: f32,
    pub at: Instant,
}

#[derive(Default)]
pub struct Model {
    /// Heard from Pyr yet, and Pyr's gone (closed).
    pub heard: bool,
    pub gone: bool,
    /// The server showing in Pyr (its key and name), whether it has prop bets, and the
    /// casino's own (0.15 servers).
    pub server_key: Option<String>,
    pub server: Option<String>,
    pub props_here: bool,
    pub house_here: bool,
    /// This Pyr is from before 0.15: it knows nothing of categories, the casino's own props,
    /// how a prop's odds went, the best at props, or Share.
    pub old_pyr: bool,
    /// Me (my id), my chips (once the casino said), the server's text channels, and the one
    /// showing in Pyr (where Share posts).
    pub me: String,
    pub chips: Option<u64>,
    pub channels: Vec<(i64, String)>,
    pub channel: Option<i64>,
    /// Every prop, by its number.
    pub props: BTreeMap<u64, PropView>,
    /// The best at props, and how the casino's own props stand here, as last told.
    pub stats: Option<PropStats>,
    pub auto: Option<PropAuto>,
    /// What Pyr said lately: text, whether it went wrong, when.
    pub said: Option<(String, bool, Instant)>,
    /// Odds that just moved (prop, outcome).
    pub moved: HashMap<(u64, usize), Moved>,
    /// When I last won something (the window celebrates a moment).
    pub won_at: Option<Instant>,
    /// Parlays (0.16 servers): whether this server takes them, and mine (the ones riding
    /// first, then the latest that are over).
    pub parlays_here: bool,
    pub parlays: Vec<ParlayView>,
    /// The server just said a parlay of mine is on (the slip it was made on is done with).
    pub parlay_on: bool,
}

/// The props of one game or one day, together.
pub struct Event<'a> {
    pub id: String,
    pub name: String,
    pub starts: i64,
    pub props: Vec<&'a PropView>,
}

impl Model {
    pub fn hear(&mut self, v: &Value) {
        match v["op"].as_str() {
            Some("props") => {
                self.heard = true;
                let key = v["server"]["key"].as_str().map(String::from);
                // (A Pyr from before 0.15 tells every prop each time and doesn't say so.)
                let key_changed = key != self.server_key;
                let full = v["full"].as_bool().unwrap_or(true) || key_changed;
                self.old_pyr = v["full"].is_null();
                self.server_key = key;
                self.server = v["server"]["name"].as_str().map(String::from);
                self.props_here = v["props_here"].as_bool().unwrap_or(false);
                self.house_here = v["house_here"].as_bool().unwrap_or(false);
                self.parlays_here = v["parlays_here"].as_bool().unwrap_or(false);
                // (My parlays come whole, when they changed.)
                if let Some(list) = v["parlays"].as_array() {
                    self.parlays = list.iter().filter_map(|x| serde_json::from_value(x.clone()).ok()).collect();
                } else if full && key_changed {
                    self.parlays.clear();
                }
                self.me = v["me"].as_str().unwrap_or_default().to_string();
                self.chips = v["chips"].as_u64();
                self.channel = v["channel"].as_i64();
                self.channels = v["channels"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|c| Some((c["id"].as_i64()?, c["name"].as_str()?.to_string()))).collect())
                    .unwrap_or_default();
                let list: Vec<PropView> = v["list"].as_array().map(|a| a.iter().filter_map(|x| serde_json::from_value(x.clone()).ok()).collect()).unwrap_or_default();
                if full {
                    self.props.clear();
                    self.moved.clear();
                }
                let now = Instant::now();
                for prop in list {
                    // Odds that moved since they were last told flash a moment.
                    if let Some(old) = self.props.get(&prop.id) {
                        let (was, is) = (old.prices(), prop.prices());
                        for (i, (a, b)) in was.iter().zip(&is).enumerate() {
                            if (a - b).abs() > 0.0005 {
                                self.moved.insert((prop.id, i), Moved { from: *a as f32, at: now });
                            }
                        }
                    }
                    self.props.insert(prop.id, prop);
                }
                self.stats = serde_json::from_value(v["stats"].clone()).ok().filter(|_| !v["stats"].is_null());
                self.auto = serde_json::from_value(v["auto"].clone()).ok().filter(|_| !v["auto"].is_null());
            }
            Some("said") => {
                if let Some(text) = v["text"].as_str().filter(|t| !t.trim().is_empty()) {
                    let error = v["error"].as_bool().unwrap_or(false);
                    if !error && (text.contains("You won") || text.starts_with("Your parlay came in")) {
                        self.won_at = Some(Instant::now());
                    }
                    if !error && text.starts_with("Your parlay is on") {
                        self.parlay_on = true;
                    }
                    self.said = Some((text.to_string(), error, Instant::now()));
                }
            }
            Some("pyr_gone") => self.gone = true,
            _ => {}
        }
    }

    pub fn channel_name(&self, id: i64) -> Option<&str> {
        self.channels.iter().find(|c| c.0 == id).map(|c| c.1.as_str())
    }

    /// Whether a prop is still to come out (taking bets, or waiting for its result).
    pub fn live(prop: &PropView) -> bool {
        matches!(prop.state, PropState::Open | PropState::Closed)
    }

    /// The props still to come out, the soonest to close first (ones that close when they're
    /// settled last, the newest first).
    pub fn open(&self) -> Vec<&PropView> {
        let mut v: Vec<&PropView> = self.props.values().filter(|x| Self::live(x)).collect();
        v.sort_by_key(|x| (if x.closes_at == 0 { i64::MAX } else { x.closes_at }, std::cmp::Reverse(x.id)));
        v
    }

    /// The categories that have props to come out, with how many, in Pyr's order (ones Pyr
    /// doesn't know yet after).
    pub fn categories(&self) -> Vec<(String, usize)> {
        let mut n: BTreeMap<String, usize> = BTreeMap::new();
        for x in self.props.values().filter(|x| Self::live(x)) {
            *n.entry(x.category_id()).or_default() += 1;
        }
        let mut out: Vec<(String, usize)> = p::CATEGORIES.iter().map(|c| (c.0.to_string(), n.remove(c.0).unwrap_or(0))).collect();
        out.extend(n);
        out
    }

    /// A category's props to come out, by what they're about (a game, a day), the soonest
    /// first; props about nothing in particular are each their own.
    pub fn events(&self, category: &str) -> Vec<Event<'_>> {
        let mut by: Vec<Event> = Vec::new();
        for x in self.open().into_iter().filter(|x| x.category_id() == category) {
            let id = if x.event.is_empty() { format!("#{}", x.id) } else { x.event.clone() };
            match by.iter_mut().find(|e| e.id == id) {
                Some(e) => e.props.push(x),
                None => by.push(Event { id, name: if x.event_name.is_empty() { String::new() } else { x.event_name.clone() }, starts: if x.starts_at > 0 { x.starts_at } else { x.closes_at }, props: vec![x] }),
            }
        }
        for e in by.iter_mut() {
            // (Who wins first, then the rest as they come.)
            e.props.sort_by_key(|x| (std::cmp::Reverse(x.featured), x.id));
        }
        by.sort_by_key(|e| (if e.starts == 0 { i64::MAX } else { e.starts }, e.id.clone()));
        by
    }

    /// The props worth showing first: taking bets, the most featured, then the most bet on.
    pub fn featured(&self, now: i64, most: usize) -> Vec<&PropView> {
        let mut v: Vec<&PropView> = self.props.values().filter(|x| x.open_at(now)).collect();
        v.sort_by_key(|x| (std::cmp::Reverse(x.featured), std::cmp::Reverse(x.pool), if x.closes_at == 0 { i64::MAX } else { x.closes_at }));
        // (One of a game at most: its others are a click away.)
        let mut seen: Vec<&str> = Vec::new();
        v.retain(|x| x.event.is_empty() || !seen.contains(&x.event.as_str()) && { seen.push(x.event.as_str()); true });
        // A category at a time, in turn, so there's a bit of everything (the one with the
        // most to show first).
        let mut by: Vec<(String, std::collections::VecDeque<&PropView>)> = Vec::new();
        for x in v {
            let c = x.category_id();
            match by.iter_mut().find(|b| b.0 == c) {
                Some(b) => b.1.push_back(x),
                None => by.push((c, std::collections::VecDeque::from([x]))),
            }
        }
        let mut out = Vec::new();
        while out.len() < most && by.iter().any(|b| !b.1.is_empty()) {
            for b in by.iter_mut() {
                if let Some(x) = b.1.pop_front() {
                    out.push(x);
                }
            }
        }
        out.truncate(most);
        out
    }

    /// The props with every word of `q` in what they ask, their outcomes, what they're about
    /// or their category: those to come out (the most featured and the most bet on first),
    /// and those that are over (the latest first).
    pub fn search(&self, q: &str, most: usize, most_over: usize) -> (Vec<&PropView>, Vec<&PropView>) {
        let words: Vec<String> = q.split_whitespace().map(|w| w.to_lowercase()).collect();
        if words.is_empty() {
            return (Vec::new(), Vec::new());
        }
        let has = |x: &PropView| {
            let text = format!("{} {} {} {} {}", x.question, x.outcomes.join(" "), x.event_name, p::category_name(&x.category_id()), x.detail).to_lowercase();
            words.iter().all(|w| text.contains(w.as_str()))
        };
        let mut live: Vec<&PropView> = self.props.values().filter(|x| Self::live(x) && has(x)).collect();
        live.sort_by_key(|x| (std::cmp::Reverse(x.featured), std::cmp::Reverse(x.pool), if x.closes_at == 0 { i64::MAX } else { x.closes_at }, x.id));
        live.truncate(most);
        let mut over: Vec<&PropView> = self.props.values().filter(|x| !Self::live(x) && has(x)).collect();
        over.sort_by_key(|x| (std::cmp::Reverse(x.settled_at), std::cmp::Reverse(x.id)));
        over.truncate(most_over);
        (live, over)
    }

    /// The props closing within `within` ms, the soonest first.
    pub fn closing_soon(&self, now: i64, within: i64, most: usize) -> Vec<&PropView> {
        let mut v: Vec<&PropView> = self.props.values().filter(|x| x.open_at(now) && x.closes_at > 0 && x.closes_at - now < within).collect();
        v.sort_by_key(|x| (x.closes_at, std::cmp::Reverse(x.featured)));
        let mut seen: Vec<&str> = Vec::new();
        v.retain(|x| x.event.is_empty() || !seen.contains(&x.event.as_str()) && { seen.push(x.event.as_str()); true });
        v.truncate(most);
        v
    }

    /// The props I've something on that are still to come out, the soonest to close first.
    pub fn mine_live(&self) -> Vec<&PropView> {
        self.open().into_iter().filter(|x| x.mine.iter().any(|h| h.shares > 1e-9)).collect()
    }

    /// The props I had something on that are over, the latest first.
    pub fn mine_over(&self) -> Vec<&PropView> {
        let mut v: Vec<&PropView> = self.props.values().filter(|x| !Self::live(x) && !x.mine.is_empty()).collect();
        v.sort_by_key(|x| std::cmp::Reverse((x.settled_at, x.id)));
        v
    }

    /// The props that are over, the latest first (the casino's own called off with nothing
    /// of mine on them left out).
    pub fn over(&self) -> Vec<&PropView> {
        let mut v: Vec<&PropView> = self.props.values().filter(|x| !Self::live(x) && !(x.house && x.state == PropState::Void && x.mine.is_empty())).collect();
        v.sort_by_key(|x| std::cmp::Reverse((x.settled_at, x.id)));
        v
    }

    /// The latest bets anywhere: (the prop, its line), newest first.
    pub fn latest_bets(&self, most: usize) -> Vec<(&PropView, &p::BetLine)> {
        let mut v: Vec<(&PropView, &p::BetLine)> = self.props.values().flat_map(|x| x.latest.iter().map(move |l| (x, l))).collect();
        v.sort_by_key(|x| std::cmp::Reverse(x.1.at));
        v.truncate(most);
        v
    }
}

/// My parlays in sums: the chips on the ones riding, and over the ones that are over, what
/// was won and what was lost (one called off: neither).
pub fn parlay_sums(parlays: &[pyr_poker::props::ParlayView]) -> (u64, i64, i64) {
    use pyr_poker::props::ParlayState;
    parlays.iter().fold((0, 0, 0), |a, t| match t.state {
        ParlayState::Open => (a.0 + t.chips, a.1, a.2),
        ParlayState::Won => (a.0, a.1 + (t.paid as i64 - t.chips as i64).max(0), a.2),
        ParlayState::Lost => (a.0, a.1, a.2 + t.chips as i64),
        ParlayState::Void => a,
    })
}

/// What I put into a prop, and what it's worth: sold back now for one still taking bets;
/// what it paid for one that's settled; what came back for one called off.
pub fn stake_and_worth(prop: &PropView) -> (u64, u64) {
    let cost: u64 = prop.mine.iter().map(|h| h.cost).sum();
    let worth = match prop.state {
        PropState::Open | PropState::Closed => prop.mine.iter().map(|h| prop.worth(h).max(0.0).floor() as u64).sum(),
        PropState::Settled => prop.mine.iter().filter(|h| Some(h.outcome) == prop.winner).map(|h| h.shares.max(0.0).floor() as u64).sum(),
        PropState::Void => cost,
    };
    (cost, worth)
}

/// A question as it goes in a line with other things: without its question mark.
pub fn question_short(q: &str) -> &str {
    q.trim().trim_end_matches('?')
}

/// What a row of an event's card calls a prop: the part of its question that isn't the
/// event's name ("Who wins: Chargers at Ravens?" is "Who wins"; "New York: Wednesday's
/// high?" is "Wednesday's high").
pub fn row_label(prop: &PropView) -> String {
    let q = prop.question.trim().trim_end_matches('?');
    // (The day its card's title already says: "… on Wednesday, November 18" under "The casino
    // on Wednesday, November 18".)
    let q = match prop.event_name.rsplit_once(" on ") {
        Some((_, day)) if day.len() >= 6 => q.strip_suffix(&format!(" on {day}")).unwrap_or(q),
        _ => q,
    };
    let Some((head, tail)) = q.split_once(": ") else { return q.to_string() };
    // The half that names the event goes; the other is the label.
    let names: Vec<String> = prop.event_name.split([',', ' ']).filter(|w| w.chars().count() > 2).map(|w| w.to_lowercase()).collect();
    // (A half is about the event when the event's name starts with it, "Chicago" of "Chicago,
    // Wednesday, November 18", or when it has most of the name's words.)
    let whole = prop.event_name.to_lowercase();
    let about = |s: &str| {
        let s = s.to_lowercase();
        whole.starts_with(&s) || !names.is_empty() && names.iter().filter(|w| s.contains(w.as_str())).count() * 2 >= names.len().min(4)
    };
    if about(tail) && !about(head) {
        head.to_string()
    } else if about(head) && !about(tail) {
        let mut t = tail.to_string();
        if let Some(first) = t.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        t
    } else {
        q.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_poker::props::Holding;
    use serde_json::json;

    /// My parlays come whole when they changed, stay as they were told when a line comes
    /// without them, and go with the server they were on.
    #[test]
    fn parlays_are_told_whole_when_they_change() {
        use pyr_poker::props::{ParlayLeg, ParlayState};
        let mut m = Model::default();
        let ticket = ParlayView { id: 4, chips: 1_000, pays: 3_960, legs: vec![ParlayLeg { prop: 1, outcome: 0, pick: "Bills".into(), odds: 1.99, ..Default::default() }, ParlayLeg { prop: 2, outcome: 1, pick: "Jazz".into(), odds: 1.99, ..Default::default() }], ..Default::default() };
        let mut line = told(vec![prop(1, PropState::Open, false)], Some(true));
        line["parlays_here"] = json!(true);
        line["parlays"] = json!([ticket]);
        m.hear(&line);
        assert_eq!((m.parlays_here, m.parlays.len(), m.parlays[0].legs[1].pick.as_str(), m.parlays[0].state), (true, 1, "Jazz", ParlayState::Open));
        // A prop changed: the line has no parlays, and mine are as they were.
        let mut line = told(vec![prop(1, PropState::Open, true)], Some(false));
        line["parlays_here"] = json!(true);
        m.hear(&line);
        assert_eq!(m.parlays.len(), 1);
        // What the server says of a parlay placed or paid: the slip is done with; gold falls.
        m.hear(&json!({"op": "said", "text": "Your parlay is on: 2 legs, 1,000 chips to win 3,960.", "error": false}));
        assert!(m.parlay_on && m.won_at.is_none());
        m.hear(&json!({"op": "said", "text": "Your parlay came in: both legs won. You're paid 3,960 chips (+2,960).", "error": false}));
        assert!(m.won_at.is_some());
        // Another server, from a Pyr or a server that knows nothing of parlays.
        let mut line = told(vec![], Some(true));
        line["server"]["key"] = json!("other");
        m.hear(&line);
        assert_eq!((m.parlays_here, m.parlays.len()), (false, 0));
        // In sums, for My bets: the stakes riding, what was won, what was lost.
        let t = |chips: u64, state: ParlayState, paid: u64| ParlayView { chips, state, paid, ..Default::default() };
        assert_eq!(parlay_sums(&[t(1_000, ParlayState::Open, 0), t(500, ParlayState::Open, 0), t(1_000, ParlayState::Won, 17_375), t(2_000, ParlayState::Lost, 0), t(300, ParlayState::Void, 300)]), (1_500, 16_375, 2_000));
        assert_eq!(question_short(" Who wins: Chiefs at Bills? "), "Who wins: Chiefs at Bills");
    }

    fn prop(id: u64, state: PropState, mine: bool) -> PropView {
        PropView {
            id,
            channel_id: 3,
            question: format!("Question {id}?"),
            outcomes: vec!["Yes".into(), "No".into()],
            b: 10_000.0,
            q: vec![0.0, 0.0],
            state,
            mine: if mine { vec![Holding { outcome: 0, shares: 120.0, cost: 100 }] } else { Vec::new() },
            ..Default::default()
        }
    }

    fn told(list: Vec<PropView>, full: Option<bool>) -> Value {
        let mut v = json!({
            "op": "props",
            "server": {"key": "k", "name": "Brethren of Blood"},
            "props_here": true,
            "house_here": true,
            "me": "u1",
            "chips": 25_000,
            "channels": [{"id": 3, "name": "general"}, {"id": 4, "name": "casino"}],
            "channel": 4,
            "list": list,
        });
        if let Some(f) = full {
            v["full"] = json!(f);
        }
        serde_json::from_str(&v.to_string()).unwrap()
    }

    #[test]
    fn what_pyr_tells_it_is_what_it_shows() {
        let mut m = Model::default();
        assert!(!m.heard);
        m.hear(&told(vec![prop(4, PropState::Settled, true), prop(9, PropState::Open, false), prop(7, PropState::Closed, true), prop(2, PropState::Void, false)], None));
        assert!(m.heard && m.props_here && m.house_here);
        assert_eq!(m.server.as_deref(), Some("Brethren of Blood"));
        assert_eq!((m.chips, m.channel, m.channel_name(4)), (Some(25_000), Some(4), Some("casino")));
        let ids = |v: Vec<&PropView>| v.iter().map(|p| p.id).collect::<Vec<_>>();
        assert_eq!(ids(m.open()), vec![9, 7], "to come out: the newest first when none has a closing time");
        assert_eq!(ids(m.mine_live()), vec![7]);
        assert_eq!(ids(m.mine_over()), vec![4]);
        assert_eq!(ids(m.over()), vec![4, 2]);
        // From before categories: every one is the community's.
        assert_eq!(m.categories().iter().find(|c| c.0 == "community").map(|c| c.1), Some(2));
        // No server showing in Pyr; then what Pyr said; then Pyr closing.
        m.hear(&json!({"op": "props", "server": null, "props_here": false, "list": []}));
        assert!(m.server.is_none() && m.server_key.is_none() && m.props.is_empty() && m.chips.is_none());
        m.hear(&json!({"op": "said", "text": "The odds moved: try again.", "error": true}));
        assert!(m.said.as_ref().is_some_and(|s| s.1 && s.0.starts_with("The odds")) && m.won_at.is_none());
        m.hear(&json!({"op": "said", "text": "“Rain?” settled: Yes. You won 1,200 chips (+200).", "error": false}));
        assert!(m.won_at.is_some());
        m.hear(&json!({"op": "something new"}));
        assert!(!m.gone);
        m.hear(&json!({"op": "pyr_gone"}));
        assert!(m.gone);
    }

    #[test]
    fn only_what_changed_is_told_after_the_first_time_and_odds_that_moved_flash() {
        let mut m = Model::default();
        m.hear(&told(vec![prop(1, PropState::Open, false), prop(2, PropState::Open, false)], Some(true)));
        // Prop 2 changed: a bet moved its odds. Prop 1 isn't told again and stays.
        let mut two = prop(2, PropState::Open, false);
        two.q = vec![3_000.0, 0.0];
        two.bets = 1;
        m.hear(&told(vec![two], Some(false)));
        assert_eq!(m.props.len(), 2);
        assert_eq!(m.props[&2].bets, 1);
        assert!(m.moved.contains_key(&(2, 0)) && m.moved.contains_key(&(2, 1)) && !m.moved.contains_key(&(1, 0)));
        assert!((m.moved[&(2, 0)].from - 0.5).abs() < 1e-6);
        // Another server: everything from the one before goes.
        let mut other = told(vec![prop(5, PropState::Open, false)], Some(false));
        other["server"]["key"] = json!("k2");
        m.hear(&other);
        assert_eq!(m.props.keys().copied().collect::<Vec<_>>(), vec![5]);
        assert!(m.moved.is_empty());
        // The best at props and the casino's own props come along.
        let mut with = told(vec![], Some(false));
        with["server"]["key"] = json!("k2");
        with["stats"] = json!({"days": 7, "leaders": [{"uid": "u2", "name": "Maanix", "net": 5000, "staked": 1000, "props": 2, "wins": 2, "biggest": 4000}], "me": {"uid": "u1", "name": "Blue", "net": -200}, "rank": 2, "players": 2});
        with["auto"] = json!({"on": true, "off": ["weather"], "depth": 100000, "can_set": true, "feed_at": 5, "feed_props": 312, "feed_error": ""});
        m.hear(&with);
        assert_eq!(m.props.len(), 1, "nothing changed: nothing goes");
        assert_eq!(m.stats.as_ref().map(|s| (s.rank, s.leaders[0].name.as_str(), s.me.net)), Some((2, "Maanix", -200)));
        assert_eq!(m.auto.as_ref().map(|a| (a.on, a.can_set, a.feed_props)), Some((true, true, 312)));
    }

    fn game(id: u64, kind: &str, featured: u8) -> PropView {
        PropView {
            id,
            category: "nfl".into(),
            house: true,
            event: "nfl:1".into(),
            event_name: "Los Angeles Chargers at Baltimore Ravens".into(),
            question: format!("{kind}: Chargers at Ravens?"),
            outcomes: vec!["Chargers".into(), "Ravens".into()],
            b: 100_000.0,
            q: vec![0.0, 0.0],
            closes_at: 2_000_000_000_000,
            starts_at: 2_000_000_000_000,
            featured,
            ..Default::default()
        }
    }

    #[test]
    fn a_game_s_props_are_together_and_named_for_what_they_ask() {
        let mut m = Model::default();
        let mut weather = prop(20, PropState::Open, false);
        weather.category = "weather".into();
        weather.event = "weather:nyc:20261118".into();
        weather.event_name = "New York, Wednesday, November 18".into();
        weather.question = "New York: Wednesday's high?".into();
        weather.closes_at = 1_900_000_000_000;
        let mut later = game(13, "Who wins", 7);
        later.event = "nfl:2".into();
        later.event_name = "Dallas Cowboys at New York Giants".into();
        later.question = "Who wins: Dallas Cowboys at New York Giants?".into();
        later.starts_at += 3_600_000;
        later.closes_at += 3_600_000;
        m.hear(&told(vec![game(10, "Total points", 6), game(11, "Who wins", 7), game(12, "Against the spread", 6), later, weather, prop(30, PropState::Open, false)], Some(true)));
        let cats = m.categories();
        assert_eq!(cats.iter().filter(|c| c.1 > 0).map(|c| (c.0.as_str(), c.1)).collect::<Vec<_>>(), vec![("nfl", 4), ("weather", 1), ("community", 1)]);
        assert_eq!(cats.len(), p::CATEGORIES.len(), "every category Pyr knows, in its order");
        let events = m.events("nfl");
        assert_eq!(events.iter().map(|e| (e.name.as_str(), e.props.len())).collect::<Vec<_>>(), vec![("Los Angeles Chargers at Baltimore Ravens", 3), ("Dallas Cowboys at New York Giants", 1)]);
        // Who wins first; each row named for what it asks.
        assert_eq!(events[0].props.iter().map(|x| row_label(x)).collect::<Vec<_>>(), vec!["Who wins", "Total points", "Against the spread"]);
        assert_eq!(row_label(m.events("weather")[0].props[0]), "Wednesday's high");
        assert_eq!(row_label(&m.props[&30]), "Question 30");
        // (A city of one word; a coin, which its question may name or not.)
        let mut other = m.props[&20].clone();
        other.event_name = "Chicago, Wednesday, November 18".into();
        other.question = "Chicago: rain or snow on Wednesday?".into();
        assert_eq!(row_label(&other), "Rain or snow on Wednesday");
        other.event_name = "Bitcoin".into();
        other.question = "Bitcoin: up or down on Wednesday, November 18?".into();
        assert_eq!(row_label(&other), "Up or down on Wednesday, November 18");
        other.question = "Bitcoin's price at 00:00 UTC on Thursday, November 19?".into();
        assert_eq!(row_label(&other), "Bitcoin's price at 00:00 UTC on Thursday, November 19");
        // (One about the casino's day: the day is in its card's title.)
        other.event_name = "The casino on Wednesday, November 18".into();
        other.question = "Does the house come out ahead on Wednesday, November 18?".into();
        assert_eq!(row_label(&other), "Does the house come out ahead");
        // The front page: one of a game, the most featured first, a category at a time.
        let now = 1_800_000_000_000;
        assert_eq!(m.featured(now, 5).iter().map(|x| x.id).collect::<Vec<_>>(), vec![11, 20, 30, 13]);
        assert_eq!(m.featured(now, 2).iter().map(|x| x.id).collect::<Vec<_>>(), vec![11, 20]);
        assert_eq!(m.closing_soon(now, 200_000_000_000, 5).iter().map(|x| x.id).collect::<Vec<_>>(), vec![20]);
        // Looked for by their words: every word, anywhere in what a prop says (its category too).
        let found = |q: &str| m.search(q, 9, 9).0.iter().map(|x| x.id).collect::<Vec<_>>();
        assert_eq!(found("chargers TOTAL"), vec![10]);
        assert_eq!(found("ravens"), vec![11, 13, 10, 12], "the most featured first, then the soonest");
        assert_eq!(found("weather"), vec![20]);
        assert_eq!(found("giants cowboys who"), vec![13]);
        assert!(found("zebra").is_empty() && found("  ").is_empty());
        // A category Pyr doesn't know yet comes after the ones it does.
        let mut odd = prop(40, PropState::Open, false);
        odd.category = "chess".into();
        m.hear(&told(vec![odd], Some(false)));
        assert_eq!(m.categories().last().map(|c| (c.0.as_str(), c.1)), Some(("chess", 1)));
        // (A Pyr that says "full" or not is one that knows the casino's own props.)
        assert!(!m.old_pyr);
        let mut old = told(vec![], Some(true));
        old.as_object_mut().unwrap().remove("full");
        m.hear(&old);
        assert!(m.old_pyr);
    }

    #[test]
    fn what_i_hold_is_worth_what_it_would_sell_for_or_what_it_paid() {
        // Still taking bets: sold back now, about what was paid (nobody else has bet).
        let mut open = prop(1, PropState::Open, false);
        let shares = p::shares_for(&open.q, open.b, 0, 4_000.0);
        open.q[0] += shares;
        open.mine = vec![Holding { outcome: 0, shares, cost: 4_000 }];
        let (cost, worth) = stake_and_worth(&open);
        assert!(cost == 4_000 && (3_998..=4_000).contains(&worth), "{worth}");
        // Settled my way: a chip a share. The other way: nothing. Called off: what I paid.
        let mut won = open.clone();
        (won.state, won.winner) = (PropState::Settled, Some(0));
        assert_eq!(stake_and_worth(&won), (4_000, shares.floor() as u64));
        let mut lost = won.clone();
        lost.winner = Some(1);
        assert_eq!(stake_and_worth(&lost), (4_000, 0));
        let mut off = open.clone();
        off.state = PropState::Void;
        assert_eq!(stake_and_worth(&off), (4_000, 4_000));
    }
}
