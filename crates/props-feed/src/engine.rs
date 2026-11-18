//! The feed maker's core: what it has opened (kept in a state file, so a restart loses
//! nothing), the sources it asks, each on its own clock, and the feed made from it all.
//!
//! A source only ever says three things: props to open, how props of its own came out, and
//! props whose times or state changed. Everything else is here: a prop is opened once (its id
//! is its name for good), checked before it goes in the feed, called off if its result never
//! comes, kept in the feed a week after it's over (so a server that was off catches up) and
//! then forgotten.

use std::collections::BTreeMap;
use std::path::Path;

use pyr_proto::crypto::Identity;
use pyr_proto::props_feed::{self, Feed, FeedCert, FeedProp, FeedResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::http::Fetch;
use crate::when::{self, DAY, HOUR, MIN};

/// A prop the maker keeps: as the feed carries it, and what its source needs to settle it.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Kept {
    pub prop: FeedProp,
    /// The source that made it.
    pub by: String,
    /// Whatever that source wants kept with it (which game, which line, which hour).
    pub memo: Value,
    /// When its result was published (0: not yet).
    pub done_at: i64,
}

/// A prop whose times or state changed (only what's given changes).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Moved {
    pub id: String,
    pub closes: Option<i64>,
    pub starts: Option<i64>,
    pub detail: Option<String>,
}

/// What a source says after a look at the world.
#[derive(Clone, Debug, Default)]
pub struct Out {
    /// Props to open (ones whose id is already known are passed over).
    pub open: Vec<Kept>,
    /// How props of its own came out.
    pub results: Vec<(String, FeedResult)>,
    pub moved: Vec<Moved>,
    /// What it couldn't read this time (the rest still counts).
    pub trouble: Vec<String>,
}

impl Out {
    /// A prop to open: what it is, and what's kept to settle it.
    pub fn open(&mut self, prop: FeedProp, memo: Value) {
        self.open.push(Kept { prop, memo, ..Default::default() });
    }

    pub fn settle(&mut self, id: &str, winner: usize, note: impl Into<String>) {
        self.results.push((id.to_string(), FeedResult { winner: Some(winner), note: note.into(), at: 0 }));
    }

    pub fn call_off(&mut self, id: &str, note: impl Into<String>) {
        self.results.push((id.to_string(), FeedResult { winner: None, note: note.into(), at: 0 }));
    }
}

/// When this maker was built (see `build.rs`): "2026-11-18T08:30:00Z".
pub fn built() -> String {
    when::iso(env!("PYR_PROPS_BUILT").parse::<i64>().unwrap_or(0) * 1000)
}

/// One kind of props: where they come from and how they come out.
pub trait Source: Send + Sync {
    /// Its name (in the state file and the status file).
    fn name(&self) -> &'static str;
    /// How long until it's asked again (ms), given the props of its own that aren't over.
    fn every(&self, now: i64, mine: &[&Kept]) -> i64;
    /// A look at the world. `mine`: its props that aren't over. `memo`: its own notes, kept
    /// between runs. An error means nothing could be read at all.
    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], memo: &mut Value) -> Result<Out, String>;
}

/// What's kept about a source between runs.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct SourceState {
    pub memo: Value,
    pub last_run: i64,
    pub last_ok: i64,
    pub next_run: i64,
    /// What went wrong the last time (nothing: it went well), and what it couldn't read.
    pub error: String,
    pub trouble: Vec<String>,
    pub opened: u64,
    pub settled: u64,
}

/// Everything the maker knows (the state file).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct State {
    pub props: BTreeMap<String, Kept>,
    pub sources: BTreeMap<String, SourceState>,
}

/// How long a prop stays in the feed after it's over, and in the state after that.
pub const RESULT_DAYS: i64 = 7;
const FORGET_DAYS: i64 = 9;
/// A prop whose result hasn't come this long after betting closed is called off.
const GIVE_UP_DAYS: i64 = 8;
/// A source that failed is asked again after this long at least, and at most.
const RETRY: i64 = 2 * MIN;
const RETRY_MOST: i64 = 30 * MIN;
/// The feed is made again at least this often, changed or not (servers see it's alive).
pub const HEARTBEAT: i64 = 20 * MIN;

/// Whether a prop can go in the feed as a Pyr server will take it: well formed
/// (`FeedProp::check`), a question of 5 to 150 characters, outcomes of 30 at most and all
/// different (`pyr_poker::props::check_new`'s rules), and still open for a while.
pub fn fit(prop: &FeedProp, now: i64) -> Result<(), String> {
    prop.check()?;
    let q = prop.q.chars().count();
    if !(5..=150).contains(&q) {
        return Err(format!("{}: a question of {q} characters", prop.id));
    }
    for (i, o) in prop.outcomes.iter().enumerate() {
        if o.trim().is_empty() || o.chars().count() > 30 {
            return Err(format!("{}: outcome {:?} is empty or over 30 characters", prop.id, o));
        }
        if prop.outcomes[..i].iter().any(|p| p.eq_ignore_ascii_case(o)) {
            return Err(format!("{}: two outcomes the same ({o})", prop.id));
        }
    }
    if prop.closes < now + 10 * MIN {
        return Err(format!("{}: betting would close at once", prop.id));
    }
    if prop.source.is_empty() || prop.rules.is_empty() {
        return Err(format!("{}: no source or rules", prop.id));
    }
    Ok(())
}

/// The maker: its state and its sources.
pub struct Maker {
    pub state: State,
    pub sources: Vec<Box<dyn Source>>,
}

impl Maker {
    pub fn new(state: State, sources: Vec<Box<dyn Source>>) -> Maker {
        Maker { state, sources }
    }

    /// Asks every source whose time has come (`all`: every one, now). Whether anything in the
    /// feed changed.
    pub fn step(&mut self, now: i64, web: &dyn Fetch, all: bool) -> bool {
        let mut changed = false;
        for i in 0..self.sources.len() {
            let name = self.sources[i].name();
            let due = self.state.sources.get(name).is_none_or(|s| s.next_run <= now);
            if !(due || all) {
                continue;
            }
            let mine: Vec<Kept> = self.state.props.values().filter(|k| k.by == name && k.done_at == 0).cloned().collect();
            let refs: Vec<&Kept> = mine.iter().collect();
            let mut memo = self.state.sources.get(name).map(|s| s.memo.clone()).unwrap_or(Value::Null);
            let got = self.sources[i].run(now, web, &refs, &mut memo);
            let st = self.state.sources.entry(name.to_string()).or_default();
            st.memo = memo;
            st.last_run = now;
            match got {
                Ok(out) => {
                    st.last_ok = now;
                    st.error.clear();
                    st.trouble = out.trouble.clone();
                    changed |= self.take(name, now, out);
                    let mine: Vec<&Kept> = self.state.props.values().filter(|k| k.by == name && k.done_at == 0).collect();
                    let every = self.sources[i].every(now, &mine).clamp(30 * when::SEC, 12 * HOUR);
                    self.state.sources.entry(name.to_string()).or_default().next_run = now + every;
                }
                Err(e) => {
                    // (Again in a couple of minutes, then less and less often while it's down.)
                    let since = (now - st.last_ok).clamp(RETRY, RETRY_MOST * 4);
                    st.next_run = now + (since / 4).clamp(RETRY, RETRY_MOST);
                    if st.error != e {
                        eprintln!("{} {name}: {e}", when::iso(now));
                    }
                    st.error = e;
                }
            }
        }
        changed |= self.tidy(now);
        changed
    }

    /// Takes in what a source said. Whether anything in the feed changed.
    fn take(&mut self, name: &str, now: i64, out: Out) -> bool {
        let mut changed = false;
        let (mut opened, mut settled) = (0, 0);
        let mut trouble = Vec::new();
        for mut k in out.open {
            if self.state.props.contains_key(&k.prop.id) {
                continue;
            }
            k.prop.prior = crate::math::prior(&k.prop.prior);
            match fit(&k.prop, now) {
                Ok(()) => {
                    k.by = name.to_string();
                    k.done_at = 0;
                    k.prop.result = None;
                    self.state.props.insert(k.prop.id.clone(), k);
                    opened += 1;
                    changed = true;
                }
                Err(e) => trouble.push(format!("not opened: {e}")),
            }
        }
        for (id, mut result) in out.results {
            let Some(k) = self.state.props.get_mut(&id).filter(|k| k.by == name && k.done_at == 0) else { continue };
            if result.winner.is_some_and(|w| w >= k.prop.outcomes.len()) {
                trouble.push(format!("{id}: a winner that isn't an outcome"));
                continue;
            }
            result.note = result.note.chars().filter(|c| !c.is_control()).take(200).collect();
            result.at = now;
            k.prop.result = Some(result);
            k.done_at = now;
            settled += 1;
            changed = true;
        }
        for m in out.moved {
            let Some(k) = self.state.props.get_mut(&m.id).filter(|k| k.by == name && k.done_at == 0) else { continue };
            let before = (k.prop.closes, k.prop.starts, k.prop.detail.clone());
            if let Some(c) = m.closes.filter(|c| *c > 0) {
                k.prop.closes = c;
            }
            if let Some(s) = m.starts {
                k.prop.starts = s;
            }
            if let Some(d) = m.detail {
                k.prop.detail = d.chars().filter(|c| !c.is_control()).take(200).collect();
            }
            changed |= before != (k.prop.closes, k.prop.starts, k.prop.detail.clone());
        }
        let st = self.state.sources.entry(name.to_string()).or_default();
        st.opened += opened;
        st.settled += settled;
        st.trouble.extend(trouble);
        st.trouble.truncate(20);
        if opened + settled > 0 {
            eprintln!("{} {name}: opened {opened}, settled {settled}", when::iso(now));
        }
        changed
    }

    /// Calls off what never got its result, and forgets what's long over.
    fn tidy(&mut self, now: i64) -> bool {
        let mut changed = false;
        for k in self.state.props.values_mut() {
            if k.done_at == 0 && now - k.prop.closes > GIVE_UP_DAYS * DAY {
                k.prop.result = Some(FeedResult { winner: None, note: "its result never came".into(), at: now });
                k.done_at = now;
                changed = true;
            }
        }
        let before = self.state.props.len();
        self.state.props.retain(|_, k| k.done_at == 0 || now - k.done_at < FORGET_DAYS * DAY);
        changed | (self.state.props.len() != before)
    }

    /// The feed as it stands: what's open or waiting for its result, and what came out in the
    /// last week, by category and closing time.
    pub fn feed(&self, now: i64) -> Feed {
        let mut props: Vec<FeedProp> = self.state.props.values().filter(|k| k.done_at == 0 || now - k.done_at < RESULT_DAYS * DAY).map(|k| k.prop.clone()).collect();
        props.sort_by(|a, b| (a.cat.as_str(), a.closes, a.id.as_str()).cmp(&(b.cat.as_str(), b.closes, b.id.as_str())));
        Feed { v: props_feed::VERSION, made: now, props }
    }

    /// How it's all going, for people (the status file beside the feed).
    pub fn status(&self, now: i64) -> Value {
        let mut by_cat: BTreeMap<&str, (u32, u32, u32)> = BTreeMap::new();
        for k in self.state.props.values() {
            let c = by_cat.entry(k.prop.cat.as_str()).or_default();
            if k.done_at > 0 {
                c.2 += 1;
            } else if k.prop.closes > now {
                c.0 += 1;
            } else {
                c.1 += 1;
            }
        }
        let cats: BTreeMap<&str, Value> = by_cat.iter().map(|(c, n)| (*c, json!({"open": n.0, "waiting": n.1, "over": n.2}))).collect();
        let time = |t: i64| if t == 0 { Value::Null } else { json!(when::iso(t)) };
        let sources: BTreeMap<&str, Value> = self
            .sources
            .iter()
            .map(|s| {
                let st = self.state.sources.get(s.name()).cloned().unwrap_or_default();
                let open = self.state.props.values().filter(|k| k.by == s.name() && k.done_at == 0).count();
                (s.name(), json!({"ok": st.error.is_empty() && st.last_ok > 0, "last_run": time(st.last_run), "last_ok": time(st.last_ok), "next_run": time(st.next_run), "error": st.error, "trouble": st.trouble, "not_over": open, "opened": st.opened, "settled": st.settled}))
            })
            .collect();
        json!({"made": when::iso(now), "version": env!("CARGO_PKG_VERSION"), "built": built(), "categories": cats, "sources": sources})
    }
}

/// Writes a file whole or not at all (beside it first, then moved into place).
pub fn write_whole(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)
}

/// The feed, signed, as the file servers read.
pub fn signed(key: &Identity, cert: &FeedCert, feed: &Feed) -> String {
    serde_json::to_string(&props_feed::sign(key, cert, feed)).expect("a signed feed serializes")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Canned;

    /// A source that says what it's told to.
    struct Told(std::sync::Mutex<Vec<Result<Out, String>>>);

    impl Source for Told {
        fn name(&self) -> &'static str {
            "told"
        }
        fn every(&self, _: i64, _: &[&Kept]) -> i64 {
            10 * MIN
        }
        fn run(&self, _: i64, _: &dyn Fetch, _: &[&Kept], memo: &mut Value) -> Result<Out, String> {
            *memo = json!(memo.as_i64().unwrap_or(0) + 1);
            self.0.lock().unwrap().remove(0)
        }
    }

    fn prop(id: &str, closes: i64) -> FeedProp {
        FeedProp { id: id.into(), cat: "nfl".into(), q: "Who wins: Chargers at Ravens?".into(), outcomes: vec!["Chargers".into(), "Ravens".into()], prior: vec![0.331, 0.669], closes, starts: closes, source: "ESPN".into(), rules: "Settles on the final score.".into(), ..Default::default() }
    }

    #[test]
    fn a_prop_is_opened_once_settled_once_kept_a_week_and_forgotten() {
        let t0 = when::at(2026, 11, 17, 12, 0);
        let mut first = Out::default();
        first.open(prop("nfl:1:win", t0 + 2 * HOUR), json!({"event": "1"}));
        // (Not fit for a server: a question too long, two outcomes the same, about to close.)
        let mut long = prop("nfl:2:win", t0 + 2 * HOUR);
        long.q = "Why ".repeat(60);
        first.open(long, Value::Null);
        let mut same = prop("nfl:3:win", t0 + 2 * HOUR);
        same.outcomes = vec!["Ravens".into(), "ravens".into()];
        first.open(same, Value::Null);
        first.open(prop("nfl:4:win", t0 + MIN), Value::Null);
        let mut again = Out::default();
        again.open(prop("nfl:1:win", t0 + 5 * HOUR), Value::Null);
        again.moved.push(Moved { id: "nfl:1:win".into(), detail: Some("Q3 5:21 · LAC 10, BAL 17".into()), ..Default::default() });
        let mut over = Out::default();
        over.settle("nfl:1:win", 1, "Ravens 24, Chargers 21");
        over.settle("nfl:9:win", 0, "never opened");
        let mut twice = Out::default();
        twice.settle("nfl:1:win", 0, "changed its mind");
        let said = vec![Ok(first), Ok(again), Err("down".into()), Ok(over), Ok(twice), Ok(Out::default()), Ok(Out::default())];
        let mut m = Maker::new(State::default(), vec![Box::new(Told(std::sync::Mutex::new(said)))]);
        let web = Canned::default();
        // Opened: the one that fits.
        assert!(m.step(t0, &web, false));
        let f = m.feed(t0);
        assert_eq!(f.props.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["nfl:1:win"]);
        assert_eq!(m.state.sources["told"].trouble.len(), 3, "{:?}", m.state.sources["told"].trouble);
        assert_eq!(m.state.props["nfl:1:win"].memo, json!({"event": "1"}));
        // Not asked again before its time.
        assert!(!m.step(t0 + MIN, &web, false));
        // Said again: opened once only (its closing time as it first was); what changed, changes.
        assert!(m.step(t0 + 10 * MIN, &web, false));
        let f = m.feed(t0 + 10 * MIN);
        assert_eq!((f.props.len(), f.props[0].closes, f.props[0].detail.as_str()), (1, t0 + 2 * HOUR, "Q3 5:21 · LAC 10, BAL 17"));
        // Down: nothing changes, and it's asked again in a couple of minutes.
        assert!(!m.step(t0 + 20 * MIN, &web, false));
        assert_eq!(m.state.sources["told"].error, "down");
        assert!(m.state.sources["told"].next_run <= t0 + 20 * MIN + RETRY_MOST);
        // Settled, once.
        let t1 = t0 + 4 * HOUR;
        assert!(m.step(t1, &web, false));
        assert!(m.state.sources["told"].error.is_empty());
        assert!(!m.step(t1 + 10 * MIN, &web, false));
        let r = m.feed(t1 + 10 * MIN).props[0].result.clone().unwrap();
        assert_eq!((r.winner, r.note.as_str(), r.at), (Some(1), "Ravens 24, Chargers 21", t1));
        // In the feed a week after, then out of it, then forgotten.
        assert_eq!(m.feed(t1 + 6 * DAY).props.len(), 1);
        assert_eq!(m.feed(t1 + 8 * DAY).props.len(), 0);
        assert!(m.step(t1 + 10 * DAY, &web, true));
        assert!(m.state.props.is_empty());
        assert_eq!(m.state.sources["told"].memo, json!(6), "its notes are kept between runs");
    }

    #[test]
    fn a_prop_whose_result_never_comes_is_called_off() {
        let t0 = when::at(2026, 11, 17, 12, 0);
        let mut first = Out::default();
        first.open(prop("nfl:1:win", t0 + 2 * HOUR), Value::Null);
        let mut m = Maker::new(State::default(), vec![Box::new(Told(std::sync::Mutex::new(vec![Ok(first), Ok(Out::default())])))]);
        let web = Canned::default();
        m.step(t0, &web, false);
        assert!(m.step(t0 + 9 * DAY, &web, false));
        let p = &m.feed(t0 + 9 * DAY).props[0];
        assert_eq!(p.result.as_ref().map(|r| (r.winner, r.note.as_str())), Some((None, "its result never came")));
        // The status says how it's going.
        let s = m.status(t0 + 9 * DAY);
        assert_eq!(s["categories"]["nfl"]["over"], 1);
        assert_eq!(s["sources"]["told"]["ok"], true);
    }

    #[test]
    fn the_feed_it_signs_is_one_a_server_takes() {
        let release = Identity::generate();
        let key = Identity::generate();
        let cert = props_feed::certify(&release, &key.user_id(), 4_000_000_000);
        let t0 = when::at(2026, 11, 17, 12, 0);
        let mut first = Out::default();
        first.open(prop("nfl:1:win", t0 + 2 * HOUR), Value::Null);
        let mut m = Maker::new(State::default(), vec![Box::new(Told(std::sync::Mutex::new(vec![Ok(first)])))]);
        m.step(t0, &Canned::default(), false);
        let text = signed(&key, &cert, &m.feed(t0));
        let got = props_feed::open(&text, &release.user_id()).unwrap();
        assert_eq!((got.made, got.props.len(), got.props[0].prior.clone()), (t0, 1, vec![0.331, 0.669]));
        // The state file reads back as it was written.
        let again: State = serde_json::from_str(&serde_json::to_string(&m.state).unwrap()).unwrap();
        assert_eq!(again.props, m.state.props);
    }
}
