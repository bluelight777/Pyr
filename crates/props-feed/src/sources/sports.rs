//! Games: NFL, NBA, NHL, MLB, college football and college basketball. Each game on the
//! schedule gets a few props (who wins; against the spread and the total when the scoreboard
//! carries a bookmaker's lines; one game prop of the league's own), all closing as it starts
//! and settled from its final score.
//!
//! The scoreboard itself is behind [`Scoreboard`] (ESPN's: `espn`), so another source of
//! games can take its place.

use serde_json::{Value, json};

use super::short;
use crate::engine::{Kept, Moved, Out, Source};
use crate::http::Fetch;
use crate::math::{normal_between, phi};
use crate::when::{DAY, HOUR, MIN};
use pyr_proto::props_feed::FeedProp;

/// Where a game stands.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Stage {
    #[default]
    Before,
    During,
    Final,
    /// Postponed, canceled, suspended: its props are called off.
    Off,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Team {
    /// "Ravens", "Morehead St": what an outcome is called.
    pub name: String,
    /// "Baltimore Ravens"
    pub full: String,
    pub abbr: String,
    pub score: Option<i64>,
    pub won: Option<bool>,
    /// Wins and losses so far.
    pub record: Option<(u32, u32)>,
    /// Its place in the Top 25 (college).
    pub rank: Option<u32>,
    /// What it scored each period.
    pub periods: Vec<i64>,
}

/// A bookmaker's lines on a game, its margin taken out of the chances.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    /// The home team's spread (-4.5 when it's the favorite), and the total.
    pub spread_home: Option<f64>,
    pub total: Option<f64>,
    /// The chances the home team wins, covers its spread, and the total goes over.
    pub win_home: Option<f64>,
    pub cover_home: Option<f64>,
    pub over: Option<f64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Game {
    pub id: String,
    pub start: i64,
    /// Its starting time is known (not just its day).
    pub time_set: bool,
    pub stage: Stage,
    /// "11/16 - 8:15 PM EST", "5:21 - 3rd", "Final/OT".
    pub status: String,
    /// Why it's off: "postponed", "canceled".
    pub off_why: String,
    pub period: u32,
    pub home: Team,
    pub away: Team,
    pub venue: String,
    pub neutral: bool,
    pub tv: String,
    pub line: Option<Line>,
    /// The day it's on as its league dates it ("20261116"): what its scoreboard is asked by.
    pub day: String,
}

/// A source of games.
pub trait Scoreboard: Send + Sync {
    /// Its name, as the props say whose numbers settle them.
    fn name(&self) -> &'static str;
    /// The games coming up in a league (this week's, or today's and tomorrow's).
    fn coming(&self, web: &dyn Fetch, league: &League, now: i64) -> Result<Vec<Game>, String>;
    /// The games of one day of a league.
    fn day(&self, web: &dyn Fetch, league: &League, day: &str) -> Result<Vec<Game>, String>;
}

/// A league: where its games are, and how its props are made.
pub struct League {
    /// Its category ("nfl") and name.
    pub cat: &'static str,
    pub name: &'static str,
    /// ESPN's path and query for it.
    pub path: &'static str,
    pub query: &'static str,
    /// Its scoreboard with no date is a week of games (football).
    pub by_week: bool,
    /// What's scored: "points", "goals", "runs".
    pub scoring: &'static str,
    /// How far final margins stray from the spread (for the chances a spread stands for).
    pub sd: f64,
    /// How much being at home is worth, as a chance.
    pub home_edge: f64,
    /// Periods in regulation.
    pub periods: u32,
    /// Only games with a ranked team, or with a bookmaker's lines (college: hundreds a day).
    pub picky: bool,
    /// The most games opened of one day, and how far ahead a game is opened.
    pub most: usize,
    pub ahead: i64,
    /// The league's own game props.
    pub extras: &'static [&'static str],
    /// How much its props are worth showing first.
    pub featured: u8,
}

const LEAGUES: [League; 6] = [
    League { cat: "nfl", name: "NFL", path: "football/nfl", query: "", by_week: true, scoring: "points", sd: 13.5, home_edge: 0.04, periods: 4, picky: false, most: 20, ahead: 8 * DAY, extras: &["margin", "half"], featured: 5 },
    League { cat: "nba", name: "NBA", path: "basketball/nba", query: "", by_week: false, scoring: "points", sd: 12.0, home_edge: 0.06, periods: 4, picky: false, most: 16, ahead: 40 * HOUR, extras: &["q1"], featured: 3 },
    League { cat: "nhl", name: "NHL", path: "hockey/nhl", query: "", by_week: false, scoring: "goals", sd: 2.4, home_edge: 0.04, periods: 3, picky: false, most: 16, ahead: 40 * HOUR, extras: &["ot"], featured: 3 },
    League { cat: "mlb", name: "MLB", path: "baseball/mlb", query: "", by_week: false, scoring: "runs", sd: 4.3, home_edge: 0.035, periods: 9, picky: false, most: 16, ahead: 40 * HOUR, extras: &["first"], featured: 3 },
    League { cat: "cfb", name: "College Football", path: "football/college-football", query: "groups=80&limit=300", by_week: true, scoring: "points", sd: 16.0, home_edge: 0.06, periods: 4, picky: true, most: 16, ahead: 8 * DAY, extras: &["half"], featured: 3 },
    League { cat: "cbb", name: "College Basketball", path: "basketball/mens-college-basketball", query: "groups=50&limit=300", by_week: false, scoring: "points", sd: 11.0, home_edge: 0.08, periods: 2, picky: true, most: 10, ahead: 40 * HOUR, extras: &["half"], featured: 2 },
];

pub fn league(cat: &str) -> Option<&'static League> {
    LEAGUES.iter().find(|l| l.cat == cat)
}

/// A game is opened only this long before it starts at least.
const NOT_AFTER: i64 = 20 * MIN;
/// A game has to have been final this long before it's settled (a score put right).
const FINAL_FOR: i64 = 5 * MIN;

/// The two teams' names as outcomes (their full names when the short ones are the same).
fn names(g: &Game) -> (String, String) {
    if g.away.name.eq_ignore_ascii_case(&g.home.name) { (short(&g.away.full, 30), short(&g.home.full, 30)) } else { (short(&g.away.name, 30), short(&g.home.name, 30)) }
}

/// "Chargers at Ravens" ("vs" on neutral ground), with the Top 25's numbers.
fn matchup(g: &Game, full: bool) -> String {
    let ranked = |t: &Team| {
        let n = if full { t.full.clone() } else { t.name.clone() };
        match t.rank {
            Some(r) => format!("#{r} {n}"),
            None => n,
        }
    };
    format!("{} {} {}", ranked(&g.away), if g.neutral { "vs" } else { "at" }, ranked(&g.home))
}

/// The chance the home team wins, from the bookmaker's moneyline, else its spread, else the
/// two teams' records and who's at home.
fn win_home(l: &League, g: &Game) -> f64 {
    if let Some(p) = g.line.as_ref().and_then(|x| x.win_home) {
        return p;
    }
    if let Some(s) = g.line.as_ref().and_then(|x| x.spread_home).filter(|_| l.scoring == "points") {
        return phi(-s / l.sd);
    }
    // (Each record pulled toward even by ten games' worth: records say less than they seem to.)
    let rate = |t: &Team| t.record.map_or(0.5, |(w, lost)| (w as f64 + 5.0) / (w as f64 + lost as f64 + 10.0));
    let (h, a) = (rate(&g.home), rate(&g.away));
    let p = h * (1.0 - a) / (h * (1.0 - a) + a * (1.0 - h));
    let ranked = |t: &Team| t.rank.map_or(0.0, |r| 0.10 + (25 - r.min(25)) as f64 * 0.004);
    (p + if g.neutral { 0.0 } else { l.home_edge } + ranked(&g.home) - ranked(&g.away)).clamp(0.08, 0.92)
}

fn signed(x: f64) -> String {
    if x > 0.0 { format!("+{x}") } else { format!("{x}") }
}

/// The props of a game that hasn't started.
pub fn plan(l: &League, board: &str, g: &Game) -> Vec<(FeedProp, Value)> {
    let (away, home) = names(g);
    let event = format!("{}:{}", l.cat, g.id);
    let ph = win_home(l, g);
    let mut detail = g.venue.clone();
    if !g.tv.is_empty() {
        detail = if detail.is_empty() { g.tv.clone() } else { format!("{detail} · {}", g.tv) };
    }
    let ranked = g.home.rank.is_some() as u8 + g.away.rank.is_some() as u8;
    let base = FeedProp {
        cat: l.cat.to_string(),
        event: event.clone(),
        event_name: matchup(g, true),
        closes: g.start,
        starts: g.start,
        detail,
        source: board.to_string(),
        ..Default::default()
    };
    let over = format!("Settled on the final score as {board} reports it, overtime included. Called off if the game is postponed or canceled");
    let mut out: Vec<(FeedProp, Value)> = Vec::new();
    let mut add = |kind: &str, q: String, outcomes: Vec<String>, prior: Vec<f64>, rules: String, featured: u8, memo: Value| {
        let mut m = json!({"kind": kind, "league": l.cat, "event": g.id, "day": g.day});
        if let (Some(m), Some(extra)) = (m.as_object_mut(), memo.as_object()) {
            m.extend(extra.clone());
        }
        out.push((FeedProp { id: format!("{event}:{kind}"), q, outcomes, prior, rules, featured: (l.featured + ranked + featured).min(9), ..base.clone() }, m));
    };
    let vs = matchup(g, false);
    add("win", format!("Who wins: {}?", matchup(g, true)), vec![away.clone(), home.clone()], vec![1.0 - ph, ph], format!("{over}, or ends in a tie."), 2, json!({}));
    if let Some(line) = &g.line {
        if let Some(s) = line.spread_home.filter(|s| s.abs() >= 0.5 && s.abs() < 60.0) {
            let cover = line.cover_home.unwrap_or(0.5);
            let (a, h) = (format!("{} {}", short(&away, 23), signed(-s)), format!("{} {}", short(&home, 23), signed(s)));
            let rules = format!("{} {} means its score has {} points {} before the two are compared. {over}, or if that makes them level (a push).", short(&home, 23), signed(s), s.abs(), if s < 0.0 { "taken off" } else { "added" }).replace("points", l.scoring);
            add("spread", format!("Against the spread: {vs}?"), vec![a, h], vec![1.0 - cover, cover], rules, 1, json!({"line": s}));
        }
        if let Some(t) = line.total.filter(|t| *t > 0.0 && *t < 400.0) {
            let p = line.over.unwrap_or(0.5);
            let rules = format!("Both teams' {} added up. {over}, or if the total is exactly {t} (a push).", l.scoring);
            add("total", format!("Total {}: {vs}?", l.scoring), vec![format!("Over {t}"), format!("Under {t}")], vec![p, 1.0 - p], rules, 1, json!({"line": t}));
        }
    }
    for kind in l.extras {
        match *kind {
            "margin" => {
                // The winner's margin, as a normal around the spread (even when there's none).
                let mean = g.line.as_ref().and_then(|x| x.spread_home).map_or((ph - 0.5) * 20.0, |s| -s);
                let band = |lo: f64, hi: Option<f64>| normal_between(mean, l.sd, Some(lo - 0.5), hi.map(|h| h + 0.5)) + normal_between(mean, l.sd, hi.map(|h| -h - 0.5), Some(-lo + 0.5));
                let prior = vec![band(1.0, Some(3.0)), band(4.0, Some(7.0)), band(8.0, Some(14.0)), band(15.0, None)];
                let outcomes = ["1 to 3 points", "4 to 7 points", "8 to 14 points", "15 or more"].iter().map(|s| s.to_string()).collect();
                add("margin", format!("Margin of victory: {vs}?"), outcomes, prior, format!("How many points the winner wins by. {over}, or ends in a tie."), 0, json!({}));
            }
            "half" | "q1" => {
                let (what, tie) = if *kind == "q1" { ("after the first quarter", 0.05) } else { ("at halftime", if l.cat == "nfl" { 0.07 } else { 0.045 }) };
                // (The favorite leads early less surely than it wins.)
                let lead = (1.0 - tie) * (0.5 + 0.7 * (ph - 0.5));
                let rules = format!("The score {what} as {board}'s line score has it. Called off if the game is postponed or canceled, or has no line score.");
                add(kind, format!("Who leads {what}: {vs}?"), vec![away.clone(), home.clone(), "Tied".to_string()], vec![1.0 - tie - lead, lead, tie], rules, 0, json!({}));
            }
            "ot" => {
                // (About one hockey game in five; the closer the two, the likelier.)
                let p = 0.23 - 0.2 * (ph - 0.5).abs();
                let rules = format!("Yes if the game is decided in overtime or a shootout, as {board} reports it. Called off if the game is postponed or canceled.");
                add("ot", format!("Overtime or a shootout: {vs}?"), vec!["Yes".into(), "No".into()], vec![p, 1.0 - p], rules, 0, json!({}));
            }
            "first" => {
                let rules = format!("Yes if either team scores in the first inning, as {board}'s line score has it. Called off if the game is postponed or canceled, or has no line score.");
                add("first", format!("A run in the first inning: {vs}?"), vec!["Yes".into(), "No".into()], vec![0.48, 0.52], rules, 0, json!({}));
            }
            _ => {}
        }
    }
    out
}

/// "Ravens 24, Chargers 21 (OT)": the winner first.
fn final_score(l: &League, g: &Game) -> String {
    let (a, h) = (g.away.score.unwrap_or(0), g.home.score.unwrap_or(0));
    let (away, home) = names(g);
    let extra = match (l.cat, g.period.saturating_sub(l.periods)) {
        (_, 0) => "",
        ("nhl", 1) => " (OT)",
        ("nhl", _) => " (SO)",
        ("mlb", _) => " (extra innings)",
        _ => " (OT)",
    };
    if h >= a { format!("{home} {h}, {away} {a}{extra}") } else { format!("{away} {a}, {home} {h}{extra}") }
}

/// How one prop of a final game came out: the outcome that won and a note, or why it's
/// called off.
pub fn settle(l: &League, memo: &Value, g: &Game) -> Result<(usize, String), String> {
    let (Some(a), Some(h)) = (g.away.score, g.home.score) else { return Err("no final score".into()) };
    let score = final_score(l, g);
    let line = memo["line"].as_f64();
    // What each side scored in the first `n` periods.
    let first = |n: usize| -> Option<(i64, i64)> { (g.away.periods.len() >= n && g.home.periods.len() >= n && n > 0).then(|| (g.away.periods[..n].iter().sum(), g.home.periods[..n].iter().sum())) };
    let lead = |x: Option<(i64, i64)>, what: &str| match x {
        None => Err("no line score".to_string()),
        Some((a, h)) => Ok((if a > h { 0 } else if h > a { 1 } else { 2 }, format!("{} {a}, {} {h} {what}", g.away.abbr, g.home.abbr))),
    };
    match memo["kind"].as_str().unwrap_or("") {
        "win" => match a.cmp(&h) {
            std::cmp::Ordering::Equal => Err(format!("it ended in a tie, {a}-{h}")),
            std::cmp::Ordering::Greater => Ok((0, score)),
            std::cmp::Ordering::Less => Ok((1, score)),
        },
        "spread" => {
            let s = line.ok_or("no line")?;
            let adj = h as f64 + s - a as f64;
            if adj.abs() < 1e-9 { Err(format!("a push: {score}")) } else { Ok((if adj > 0.0 { 1 } else { 0 }, score)) }
        }
        "total" => {
            let t = line.ok_or("no line")?;
            let sum = (a + h) as f64;
            if (sum - t).abs() < 1e-9 { Err(format!("a push: {score}")) } else { Ok((if sum > t { 0 } else { 1 }, format!("{score}: {} in all", a + h))) }
        }
        "margin" => match (a - h).abs() {
            0 => Err(format!("it ended in a tie, {a}-{h}")),
            1..=3 => Ok((0, score)),
            4..=7 => Ok((1, score)),
            8..=14 => Ok((2, score)),
            _ => Ok((3, score)),
        },
        "half" => lead(first((l.periods / 2) as usize), "at the half"),
        "q1" => lead(first(1), "after one"),
        "ot" => Ok((if g.period > l.periods { 0 } else { 1 }, score)),
        "first" => first(1).map(|(a, h)| (if a + h > 0 { 0 } else { 1 }, format!("{} {a}, {} {h} after one inning", g.away.abbr, g.home.abbr))).ok_or_else(|| "no line score".to_string()),
        other => Err(format!("a kind of prop this doesn't know ({other})")),
    }
}

/// A league's props.
pub struct Sports {
    league: &'static League,
    board: Box<dyn Scoreboard>,
}

impl Sports {
    pub fn new(cat: &str) -> Sports {
        Sports { league: league(cat).expect("a league Pyr knows"), board: Box::new(super::espn::Espn) }
    }

    pub fn with(cat: &str, board: Box<dyn Scoreboard>) -> Sports {
        Sports { league: league(cat).expect("a league Pyr knows"), board }
    }
}

impl Source for Sports {
    fn name(&self) -> &'static str {
        self.league.cat
    }

    /// Every five minutes while a game of its is on or about to start; else every half hour.
    fn every(&self, now: i64, mine: &[&Kept]) -> i64 {
        if mine.iter().any(|k| k.prop.starts <= now + 10 * MIN) { 5 * MIN } else { 30 * MIN }
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], memo: &mut Value) -> Result<Out, String> {
        let l = self.league;
        let mut out = Out::default();
        let mut games: Vec<Game> = Vec::new();
        // What's coming (every half hour is plenty), to open.
        let listed = memo["listed"].as_i64().unwrap_or(0);
        let mut coming_failed = None;
        if now - listed >= 25 * MIN {
            match self.board.coming(web, l, now) {
                Ok(g) => {
                    memo["listed"] = json!(now);
                    games.extend(g);
                }
                Err(e) => coming_failed = Some(e),
            }
        }
        // The days of the games that have started and aren't settled, to settle from.
        let mut days: Vec<String> = mine.iter().filter(|k| k.prop.starts <= now + 10 * MIN).filter_map(|k| k.memo["day"].as_str().map(|d| d.to_string())).collect();
        days.sort();
        days.dedup();
        for day in days.iter().take(6) {
            match self.board.day(web, l, day) {
                // (A day's own scoreboard is the newer word on its games.)
                Ok(g) => {
                    games.retain(|old| !g.iter().any(|n| n.id == old.id));
                    games.extend(g);
                }
                Err(e) => out.trouble.push(format!("{day}: {e}")),
            }
        }
        if let Some(e) = coming_failed {
            if games.is_empty() {
                return Err(e);
            }
            out.trouble.push(e);
        }
        // Open what's new: games not started, with a time, far enough off and near enough.
        let mut fresh: Vec<&Game> = games
            .iter()
            .filter(|g| g.stage == Stage::Before && g.time_set && g.start > now + NOT_AFTER && g.start < now + l.ahead)
            .filter(|g| !l.picky || g.home.rank.is_some() || g.away.rank.is_some() || g.line.is_some())
            .filter(|g| !mine.iter().any(|k| k.memo["event"] == g.id.as_str()))
            .collect();
        // (The most of a day: ranked teams first, then games with lines, then the soonest.)
        fresh.sort_by_key(|g| (std::cmp::Reverse(g.home.rank.is_some() as u8 + g.away.rank.is_some() as u8), g.line.is_none(), g.start));
        let mut per_day: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
        for g in fresh {
            let n = per_day.entry(g.day.as_str()).or_default();
            let already = mine.iter().filter(|k| k.memo["day"] == g.day.as_str() && k.memo["kind"] == "win").count();
            if *n + already >= l.most {
                continue;
            }
            *n += 1;
            for (prop, m) in plan(l, self.board.name(), g) {
                out.open(prop, m);
            }
        }
        // And what's open: moved, under way, final, or off.
        let finals = memo["finals"].as_object().cloned().unwrap_or_default();
        let mut still: serde_json::Map<String, Value> = serde_json::Map::new();
        for k in mine {
            let Some(g) = k.memo["event"].as_str().and_then(|id| games.iter().find(|g| g.id == id)) else { continue };
            let id = &k.prop.id;
            match g.stage {
                Stage::Off => out.call_off(id, if g.off_why.is_empty() { "postponed".to_string() } else { g.off_why.clone() }),
                Stage::Before => {
                    if g.time_set && g.start != k.prop.starts {
                        out.moved.push(Moved { id: id.clone(), closes: Some(g.start), starts: Some(g.start), detail: None });
                    }
                }
                Stage::During => {
                    let line = format!("{} · {} {}, {} {}", g.status, g.away.abbr, g.away.score.unwrap_or(0), g.home.abbr, g.home.score.unwrap_or(0));
                    out.moved.push(Moved { id: id.clone(), detail: Some(line), ..Default::default() });
                }
                Stage::Final => {
                    let since = finals.get(&g.id).and_then(|v| v.as_i64()).unwrap_or(now);
                    still.insert(g.id.clone(), json!(since));
                    if now - since < FINAL_FOR {
                        out.moved.push(Moved { id: id.clone(), detail: Some(format!("Final: {}", final_score(l, g))), ..Default::default() });
                        continue;
                    }
                    match settle(l, &k.memo, g) {
                        Ok((winner, note)) => out.settle(id, winner, note),
                        Err(why) => out.call_off(id, why),
                    }
                }
            }
        }
        memo["finals"] = Value::Object(still);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Maker, State, fit};
    use crate::http::Canned;
    use crate::sources::espn;
    use crate::when;

    const NFL: &str = include_str!("../../samples/espn-nfl.json");
    const NHL: &str = include_str!("../../samples/espn-nhl.json");

    fn nfl_game(i: usize) -> Game {
        espn::games(NFL).unwrap().remove(i)
    }

    #[test]
    fn a_game_gets_its_props_at_the_bookmaker_s_odds() {
        let l = league("nfl").unwrap();
        let g = nfl_game(1);
        let props = plan(l, "ESPN", &g);
        let ids: Vec<&str> = props.iter().map(|p| p.0.id.as_str()).collect();
        assert_eq!(ids, vec!["nfl:401873065:win", "nfl:401873065:spread", "nfl:401873065:total", "nfl:401873065:margin", "nfl:401873065:half"]);
        let now = g.start - 3 * DAY;
        for (p, _) in &props {
            let mut p = p.clone();
            p.prior = crate::math::prior(&p.prior);
            fit(&p, now).unwrap();
            assert_eq!((p.cat.as_str(), p.event.as_str(), p.event_name.as_str(), p.closes, p.starts), ("nfl", "nfl:401873065", "Los Angeles Chargers at Baltimore Ravens", g.start, g.start));
            assert_eq!((p.source.as_str(), p.detail.as_str()), ("ESPN", "M&T Bank Stadium · ESPN"));
        }
        let win = &props[0].0;
        assert_eq!((win.q.as_str(), win.outcomes.clone()), ("Who wins: Los Angeles Chargers at Baltimore Ravens?", vec!["Chargers".to_string(), "Ravens".into()]));
        assert!((win.prior[1] - 0.669).abs() < 0.001);
        let spread = &props[1];
        assert_eq!((spread.0.q.as_str(), spread.0.outcomes.clone()), ("Against the spread: Chargers at Ravens?", vec!["Chargers +4.5".to_string(), "Ravens -4.5".into()]));
        assert_eq!(spread.1, json!({"kind": "spread", "league": "nfl", "event": "401873065", "day": "20261116", "line": -4.5}));
        assert!(spread.0.rules.contains("Ravens -4.5 means its score has 4.5 points taken off"), "{}", spread.0.rules);
        let total = &props[2].0;
        assert_eq!((total.q.as_str(), total.outcomes.clone()), ("Total points: Chargers at Ravens?", vec!["Over 39.5".to_string(), "Under 39.5".into()]));
        // The margin: a favorite by 4.5, so a close game is likelier than a rout but not by much.
        let margin = &props[3].0;
        assert!((margin.prior.iter().sum::<f64>() - 1.0).abs() < 0.03, "{:?}", margin.prior);
        assert!(margin.prior.iter().all(|p| *p > 0.15 && *p < 0.40), "{:?}", margin.prior);
        let half = &props[4].0;
        assert_eq!(half.outcomes, vec!["Chargers".to_string(), "Ravens".into(), "Tied".into()]);
        assert!(half.prior[1] > half.prior[0] && (half.prior[2] - 0.07).abs() < 1e-9);
    }

    #[test]
    fn with_no_lines_the_records_and_home_say_who_s_likelier() {
        let l = league("nfl").unwrap();
        let mut g = nfl_game(1);
        g.line = None;
        // 0-8 at 4-5: the home team well ahead, but not as a bookmaker would have it.
        let p = win_home(l, &g);
        assert!(p > 0.62 && p < 0.80, "{p}");
        assert_eq!(plan(l, "ESPN", &g).len(), 3, "no spread or total without lines");
        // Hockey: its own prop, and its lines in goals.
        let h = league("nhl").unwrap();
        let g = espn::games(NHL).unwrap().remove(0);
        let props = plan(h, "ESPN", &g);
        assert_eq!(props.iter().map(|p| p.1["kind"].as_str().unwrap()).collect::<Vec<_>>(), vec!["win", "spread", "total", "ot"]);
        assert_eq!(props[1].0.outcomes, vec!["Canadiens -1.5".to_string(), "Rangers +1.5".into()]);
        assert!(props[1].0.rules.contains("1.5 goals added"), "{}", props[1].0.rules);
        assert_eq!(props[2].0.q, "Total goals: Canadiens at Rangers?");
        assert!((props[3].0.prior[0] - 0.228).abs() < 0.01);
        // College: ranks in the name, a neutral court says "vs".
        let mut c = nfl_game(1);
        c.home.rank = Some(3);
        c.neutral = true;
        assert_eq!(matchup(&c, false), "Chargers vs #3 Ravens");
        // Two teams of one name go by their full names.
        c.away.name = "Ravens".into();
        assert_eq!(names(&c), ("Los Angeles Chargers".to_string(), "Baltimore Ravens".to_string()));
    }

    #[test]
    fn a_final_score_settles_each_kind() {
        let l = league("nfl").unwrap();
        // Commanders 24 at Giants 21 (0-10-7-7 and 0-7-7-7).
        let g = nfl_game(0);
        let s = |kind: &str, line: Option<f64>| settle(l, &json!({"kind": kind, "line": line}), &g);
        assert_eq!(s("win", None), Ok((0, "Commanders 24, Giants 21".to_string())));
        assert_eq!(s("spread", Some(-2.5)).unwrap().0, 0, "the Giants, giving 2.5, lost");
        assert_eq!(s("spread", Some(3.5)).unwrap().0, 1, "the Giants, getting 3.5, cover");
        assert_eq!(s("spread", Some(3.0)), Err("a push: Commanders 24, Giants 21".to_string()));
        assert_eq!(s("total", Some(44.5)), Ok((0, "Commanders 24, Giants 21: 45 in all".to_string())));
        assert_eq!(s("total", Some(45.5)).unwrap().0, 1);
        assert!(s("total", Some(45.0)).unwrap_err().contains("push"));
        assert_eq!(s("margin", None).unwrap().0, 0);
        assert_eq!(s("half", None), Ok((0, "WSH 10, NYG 7 at the half".to_string())));
        assert_eq!(s("q1", None), Ok((2, "WSH 0, NYG 0 after one".to_string())));
        assert!(s("coin", None).is_err());
        // Overtime: said in the note; a tie calls the winner's prop off.
        let ot = nfl_game(2);
        assert_eq!(settle(l, &json!({"kind": "win"}), &ot), Ok((1, "Lions 30, Patriots 24 (OT)".to_string())));
        assert_eq!(settle(l, &json!({"kind": "margin"}), &ot).unwrap().0, 1);
        let mut tie = nfl_game(0);
        tie.home.score = Some(24);
        assert!(settle(l, &json!({"kind": "win"}), &tie).unwrap_err().contains("tie"));
        // Hockey: overtime or a shootout; baseball: a run in the first.
        let h = league("nhl").unwrap();
        let mut g = espn::games(NHL).unwrap().remove(0);
        (g.home.score, g.away.score, g.period) = (Some(3), Some(2), 5);
        assert_eq!(settle(h, &json!({"kind": "ot"}), &g), Ok((0, "Rangers 3, Canadiens 2 (SO)".to_string())));
        g.period = 3;
        assert_eq!(settle(h, &json!({"kind": "ot"}), &g).unwrap().0, 1);
        let m = league("mlb").unwrap();
        (g.home.periods, g.away.periods) = (vec![0, 2], vec![1, 0]);
        assert_eq!(settle(m, &json!({"kind": "first"}), &g).unwrap().0, 0);
        g.away.periods = vec![0, 0];
        assert_eq!(settle(m, &json!({"kind": "first"}), &g).unwrap().0, 1);
        g.home.periods.clear();
        assert!(settle(m, &json!({"kind": "first"}), &g).is_err());
    }

    /// The scoreboard as it stands at each step of a game's life.
    fn web(nfl: &str) -> Canned {
        Canned::new(&[("football/nfl/scoreboard", nfl)])
    }

    #[test]
    fn a_game_s_props_open_follow_it_and_settle_by_themselves() {
        let kickoff = when::at(2026, 11, 17, 1, 15);
        let mut m = Maker::new(State::default(), vec![Box::new(Sports::new("nfl"))]);
        // Two days before: the game that hasn't started gets its five props; the two that are
        // over get none.
        let t0 = kickoff - 2 * DAY;
        assert!(m.step(t0, &web(NFL), false));
        let feed = m.feed(t0);
        assert_eq!(feed.props.len(), 5);
        assert!(feed.props.iter().all(|p| p.event == "nfl:401873065" && p.closes == kickoff && p.result.is_none()));
        // Asked again half an hour on: nothing new, nothing changed.
        assert!(!m.step(t0 + 30 * MIN, &web(NFL), false));
        // Flexed to an earlier kickoff: its props close then.
        let flexed = NFL.replace("\"date\": \"2026-11-17T01:15Z\"", "\"date\": \"2026-11-16T21:25Z\"");
        assert!(m.step(t0 + HOUR, &web(&flexed), false));
        let earlier = when::at(2026, 11, 16, 21, 25);
        assert!(m.feed(t0 + HOUR).props.iter().all(|p| p.closes == earlier && p.starts == earlier));
        // Under way: asked every five minutes, by its day; how it stands shows.
        let live = flexed.replace("\"state\": \"pre\"", "\"state\": \"in\"").replace("11/16 - 8:15 PM EST", "5:21 - 3rd").replace("\"id\": \"33\", \"homeAway\": \"home\", \"score\": \"0\"", "\"id\": \"33\", \"homeAway\": \"home\", \"score\": \"17\"");
        assert_ne!(live, flexed.replace("\"state\": \"pre\"", "\"state\": \"in\"").replace("11/16 - 8:15 PM EST", "5:21 - 3rd"));
        let t1 = earlier + HOUR;
        let w = web(&live);
        assert!(m.step(t1, &w, false));
        assert!(w.asked().iter().any(|u| u.ends_with("scoreboard?dates=20261116")), "{:?}", w.asked());
        assert_eq!(m.feed(t1).props[0].detail, "5:21 - 3rd · LAC 0, BAL 17");
        assert_eq!(m.state.sources["nfl"].next_run, t1 + 5 * MIN);
        // Final: Ravens 24, Chargers 21. Settled once it's been final five minutes.
        let done = live
            .replace("\"state\": \"in\", \"completed\": false", "\"state\": \"post\", \"completed\": true")
            .replace("5:21 - 3rd", "Final")
            .replace("\"score\": \"17\"", "\"winner\": true, \"linescores\": [{\"value\": 7.0}, {\"value\": 10.0}, {\"value\": 0.0}, {\"value\": 7.0}], \"score\": \"24\"")
            .replace("\"id\": \"24\", \"homeAway\": \"away\", \"score\": \"0\"", "\"id\": \"24\", \"homeAway\": \"away\", \"winner\": false, \"linescores\": [{\"value\": 7.0}, {\"value\": 0.0}, {\"value\": 7.0}, {\"value\": 7.0}], \"score\": \"21\"")
            .replace("\"period\": 0", "\"period\": 4");
        let t2 = t1 + 2 * HOUR;
        assert!(m.step(t2, &web(&done), false));
        assert!(m.feed(t2).props.iter().all(|p| p.result.is_none() && p.detail == "Final: Ravens 24, Chargers 21"));
        assert!(m.step(t2 + 5 * MIN, &web(&done), false));
        let feed = m.feed(t2 + 5 * MIN);
        let result = |kind: &str| feed.props.iter().find(|p| p.id.ends_with(kind)).unwrap().result.clone().unwrap();
        assert_eq!((result("win").winner, result("win").note.as_str()), (Some(1), "Ravens 24, Chargers 21"));
        assert_eq!(result("spread").winner, Some(0), "the Ravens, giving 4.5, won by 3");
        assert_eq!((result("total").winner, result("total").note.as_str()), (Some(0), "Ravens 24, Chargers 21: 45 in all"));
        assert_eq!(result("margin").winner, Some(0));
        assert_eq!((result("half").winner, result("half").note.as_str()), (Some(1), "LAC 7, BAL 17 at the half"));
        // Nothing left to ask about: back to every half hour.
        assert_eq!(m.state.sources["nfl"].next_run, t2 + 35 * MIN);
    }

    #[test]
    fn a_postponed_game_is_called_off_and_a_scoreboard_that_s_down_is_said_so() {
        let kickoff = when::at(2026, 11, 17, 1, 15);
        let mut m = Maker::new(State::default(), vec![Box::new(Sports::new("nfl"))]);
        let t0 = kickoff - DAY;
        m.step(t0, &web(NFL), false);
        let off = NFL.replace("STATUS_SCHEDULED", "STATUS_POSTPONED").replace("\"description\": \"Scheduled\"", "\"description\": \"Postponed\"");
        assert!(m.step(t0 + HOUR, &web(&off), false));
        let feed = m.feed(t0 + HOUR);
        assert!(feed.props.iter().all(|p| p.result.as_ref().is_some_and(|r| r.winner.is_none() && r.note == "postponed")));
        // Down: an error, nothing changed, asked again soon.
        let mut m = Maker::new(State::default(), vec![Box::new(Sports::new("nfl"))]);
        assert!(!m.step(t0, &Canned::default(), false));
        assert!(m.state.sources["nfl"].error.contains("no answer"), "{}", m.state.sources["nfl"].error);
        // Something that isn't a scoreboard at all.
        assert!(!m.step(t0 + HOUR, &web("<html>Access Denied</html>"), false));
        assert!(m.state.sources["nfl"].error.contains("isn't JSON"));
    }

    #[test]
    fn college_takes_the_ranked_and_the_lined_and_a_day_has_its_most() {
        let l = league("cbb").unwrap();
        assert!(l.picky);
        // Hockey's two games both open (by day: today's and tomorrow's scoreboards asked).
        let now = when::at(2026, 11, 16, 15, 0);
        let w = Canned::new(&[("dates=20261116", NHL), ("dates=20261117", r#"{"events":[]}"#)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Sports::new("nhl"))]);
        assert!(m.step(now, &w, false));
        assert_eq!(w.asked().len(), 2);
        assert_eq!(m.feed(now).props.len(), 8);
        // The same games as college basketball's with no ranks and no lines: none taken.
        let bare = NHL.replace("\"odds\"", "\"no_odds\"");
        let w = Canned::new(&[("dates=20261116", bare.as_str()), ("dates=20261117", r#"{"events":[]}"#)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Sports::new("cbb"))]);
        assert!(!m.step(now, &w, false));
        // One of them ranked: taken, with its rank in its name and the halftime leader.
        let ranked = bare.replace("\"id\": \"13\", \"homeAway\": \"home\", \"score\": \"0\", \"curatedRank\": {\"current\": 99}", "\"id\": \"13\", \"homeAway\": \"home\", \"score\": \"0\", \"curatedRank\": {\"current\": 7}");
        let w = Canned::new(&[("dates=20261116", ranked.as_str()), ("dates=20261117", r#"{"events":[]}"#)]);
        assert!(m.step(now + HOUR, &w, false));
        let feed = m.feed(now + HOUR);
        assert_eq!(feed.props.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["cbb:401892726:half", "cbb:401892726:win"]);
        assert_eq!(feed.props[1].event_name, "Montreal Canadiens at #7 New York Rangers");
        assert!(feed.props[1].featured > feed.props[0].featured);
    }
}
