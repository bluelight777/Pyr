//! balldontlie's games (`api.balldontlie.io`), for the NFL, the NBA and MLB: the leagues
//! whose games its free plan gives. Read into games as `sports` knows them.
//!
//! Written from its documentation (nfl., nba. and mlb.balldontlie.io, read Nov 18 2026); no
//! answer of its own could be read from here, as it wants a key. So everything is taken as
//! optional but a game's id, its start and its two teams, and a game's stage is worked out
//! from whichever of its ways of saying it is there.
//!
//! Its free plan: a key (an account), five requests a minute (`http` waits thirteen seconds
//! between two), games with their scores by quarter or inning; no bookmakers' lines. Its
//! terms (Oct 20 2026) allow showing its data in an app, commercial or not, and keeping it;
//! they forbid making a data feed that stands in for its own, calling its data the leagues'
//! official data, and settling wagers on it alone without checking elsewhere: Blue was told
//! (Nov 18) and chose it.

use serde_json::Value;

use super::num;
use super::sports::{Game, League, Scoreboard, Stage, Team};
use crate::http::Fetch;
use crate::when::{self, DAY, HOUR, MIN};

const HOST: &str = "https://api.balldontlie.io";

pub struct Bdl {
    pub key: Option<String>,
}

impl Bdl {
    /// The address of a league's games on some days (`days`: YYYY-MM-DD).
    fn url(league: &League, days: &[String]) -> String {
        // (The NBA's is the one without its league in the path: it was there first.)
        let path = match league.cat {
            "nba" => "v1/games".to_string(),
            cat => format!("{cat}/v1/games"),
        };
        let dates: String = days.iter().map(|d| format!("&dates[]={d}")).collect();
        format!("{HOST}/{path}?per_page=100{dates}")
    }

    fn ask(&self, web: &dyn Fetch, league: &League, from: i64, days: i64, now: i64) -> Result<Vec<Game>, String> {
        let Some(key) = self.key.as_deref() else {
            return Err("balldontlie: no key yet (on the server: pyr-props-key balldontlie)".into());
        };
        let list: Vec<String> = (0..days).map(|d| when::iso_date(from + d * DAY)).collect();
        let text = web.get_with(&Self::url(league, &list), ("Authorization", key))?;
        games(league, &text, now)
    }
}

impl Scoreboard for Bdl {
    fn name(&self) -> &'static str {
        "balldontlie"
    }

    /// From yesterday to as far ahead as the league's games are opened (a day more each way:
    /// whichever clock its dates go by).
    fn coming(&self, web: &dyn Fetch, league: &League, now: i64, _notes: &mut Value) -> Result<Vec<Game>, String> {
        let days = (league.ahead / DAY + 3).clamp(3, 11);
        self.ask(web, league, when::day_start(now) - DAY, days, now)
    }

    /// The day itself and the ones beside it (a late game's date differs by the clock).
    fn day(&self, web: &dyn Fetch, league: &League, day: &str, now: i64, _notes: &mut Value) -> Result<Vec<Game>, String> {
        let at = when::parse_date(day).ok_or("a day that isn't one")?;
        self.ask(web, league, at - DAY, 3, now)
    }

    fn coming_every(&self, _league: &League) -> i64 {
        30 * MIN
    }

    /// No overtime to tell in a hockey game it doesn't give (and it gives none of hockey).
    fn settles(&self, league: &League, kind: &str) -> bool {
        !(league.cat == "nhl" && kind == "ot")
    }
}

/// The day a game is on, as the leagues date them (US Eastern, near enough).
fn league_day(start: i64) -> String {
    when::iso_date(start - 5 * HOUR)
}

/// An answer's games.
pub fn games(league: &League, text: &str, now: i64) -> Result<Vec<Game>, String> {
    let v = super::json(text, "balldontlie")?;
    let data = v["data"].as_array().ok_or("balldontlie's answer has no list of games in it")?;
    let got: Vec<Game> = data.iter().filter_map(|g| game(league, g, now)).collect();
    if got.is_empty() && !data.is_empty() {
        return Err(format!("balldontlie's games aren't as this expects them ({} there, none read)", data.len()));
    }
    Ok(got)
}

fn text(v: &Value) -> String {
    v.as_str().unwrap_or("").trim().to_string()
}

/// A team: the NFL's and the NBA's have `name` and `full_name`, MLB's `name` (or
/// `short_display_name`) and `display_name`.
fn team(t: &Value, fallback: &Value) -> Option<Team> {
    let name = [&t["short_display_name"], &t["name"]].into_iter().map(text).find(|s| !s.is_empty())?;
    let full = [&t["full_name"], &t["display_name"], fallback].into_iter().map(text).find(|s| !s.is_empty()).unwrap_or_else(|| name.clone());
    Some(Team { name, full, abbr: text(&t["abbreviation"]), ..Default::default() })
}

/// What a side scored each period: the keys there are, in order (a quarter with nothing in
/// it is a quarter with no points, once any has something).
fn periods(g: &Value, keys: &[String]) -> Vec<i64> {
    let got: Vec<Option<i64>> = keys.iter().map(|k| num(&g[k.as_str()]).map(|x| x as i64)).collect();
    let Some(last) = got.iter().rposition(|x| x.is_some()) else { return Vec::new() };
    got[..=last].iter().map(|x| x.unwrap_or(0)).collect()
}

fn game(league: &League, g: &Value, now: i64) -> Option<Game> {
    let id = g["id"].as_u64()?;
    let mlb = league.cat == "mlb";
    // When it starts: the NBA's `datetime` (its `date` is the day alone), the others' `date`.
    let stamp = g["datetime"].as_str().or_else(|| g["date"].as_str())?;
    let (start, mut time_set) = match when::parse_iso(stamp) {
        Some(t) => (t, stamp.contains('T')),
        None => (when::parse_date(stamp)? + 17 * HOUR, false),
    };
    // (Midnight in the East is a day with no time yet, not a time.)
    if (start - 4 * HOUR) % DAY == 0 || (start - 5 * HOUR) % DAY == 0 {
        time_set = false;
    }
    let (home_key, away_key) = ("home_team", if g["away_team"].is_object() { "away_team" } else { "visitor_team" });
    let mut home = team(&g[home_key], &g["home_team_name"])?;
    let mut away = team(&g[away_key], &g["away_team_name"])?;
    // Its scores and line scores.
    if mlb {
        let side = |d: &Value| (num(&d["runs"]).map(|x| x as i64), d["inning_scores"].as_array().map(|a| a.iter().map(|x| num(x).unwrap_or(0.0) as i64).collect::<Vec<_>>()).unwrap_or_default());
        (home.score, home.periods) = side(&g["home_team_data"]);
        (away.score, away.periods) = side(&g["away_team_data"]);
    } else {
        home.score = num(&g["home_team_score"]).map(|x| x as i64);
        away.score = num(&g["visitor_team_score"]).map(|x| x as i64);
        let keys = |side: &str| -> Vec<String> {
            // The NFL: home_team_q1..q4 and home_team_ot. The NBA: home_q1..q4, home_ot1..3.
            if league.cat == "nfl" {
                (1..=4).map(|q| format!("{side}_team_q{q}")).chain([format!("{side}_team_ot")]).collect()
            } else {
                (1..=4).map(|q| format!("{side}_q{q}")).chain((1..=3).map(|o| format!("{side}_ot{o}"))).collect()
            }
        };
        home.periods = periods(g, &keys("home"));
        away.periods = periods(g, &keys("visitor"));
    }
    // Where it stands: `status_state` where it's given, else what `status` reads like.
    let status = text(&g["status"]);
    let upper = status.to_uppercase();
    // (An overtime with noughts for both that nothing calls overtime wasn't played.)
    let regulation = league.periods as usize;
    let noughts = |t: &Team| t.periods.iter().skip(regulation).all(|p| *p == 0);
    if !mlb && !upper.contains("OT") && g["period"].as_u64().is_none_or(|p| p as usize <= regulation) && noughts(&home) && noughts(&away) {
        home.periods.truncate(regulation);
        away.periods.truncate(regulation);
    }
    let off = g["postponed"].as_bool().unwrap_or(false) || ["POSTPONED", "CANCELED", "CANCELLED", "SUSPENDED", "ABANDONED", "FORFEIT"].iter().any(|w| upper.contains(w));
    let stage = match g["status_state"].as_str().unwrap_or("") {
        "final" => Stage::Final,
        "postponed" | "canceled" | "cancelled" | "suspended" | "abandoned" => Stage::Off,
        _ if off => Stage::Off,
        "in_progress" => Stage::During,
        "scheduled" => Stage::Before,
        // ("delayed", "unknown", or nothing: by what it says and the clock.)
        _ if upper.starts_with("FINAL") || upper.contains("STATUS_FINAL") => Stage::Final,
        _ if now < start => Stage::Before,
        _ => Stage::During,
    };
    if stage == Stage::Before {
        (home.score, away.score) = (None, None);
        home.periods.clear();
        away.periods.clear();
    }
    // How many periods were played: said (the NBA, MLB), or as many as have a score.
    let played = home.periods.len().max(away.periods.len()) as u32;
    let period = g["period"].as_u64().map(|p| p as u32).filter(|p| *p > 0).unwrap_or(played).max(if stage == Stage::Final { played.min(league.periods + 3) } else { 0 });
    let why = match g["status_state"].as_str() {
        Some(s @ ("postponed" | "canceled" | "cancelled" | "suspended" | "abandoned")) => s.to_string(),
        _ if off => "postponed".to_string(),
        _ => String::new(),
    };
    Some(Game {
        id: format!("b{id}"),
        day: league_day(start),
        start,
        time_set,
        stage,
        // (MLB's is a code, "STATUS_FINAL": not for showing.)
        status: if mlb || stage == Stage::Before { String::new() } else { status },
        off_why: why,
        period,
        home,
        away,
        venue: text(&g["venue"]),
        neutral: false,
        tv: String::new(),
        line: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Canned;
    use crate::sources::sports::league;

    // As its documentation shows them (the field names and what's in them), not answers read
    // from it: it gives none without a key.
    const NFL: &str = r#"{"data": [
      {"id": 7001, "visitor_team": {"id": 6, "conference": "AFC", "division": "NORTH", "location": "Baltimore", "name": "Ravens", "full_name": "Baltimore Ravens", "abbreviation": "BAL"},
       "home_team": {"id": 14, "conference": "AFC", "division": "WEST", "location": "Kansas City", "name": "Chiefs", "full_name": "Kansas City Chiefs", "abbreviation": "KC"},
       "summary": "Chiefs hold on", "venue": "GEHA Field at Arrowhead Stadium", "updated_at": "2026-11-20T04:20:00.000Z", "week": 11, "date": "2026-11-20T01:20:00.000Z", "season": 2026, "postseason": false,
       "status": "Final", "status_state": "final", "home_team_score": 27, "home_team_q1": 7, "home_team_q2": 6, "home_team_q3": 7, "home_team_q4": 7, "home_team_ot": null,
       "visitor_team_score": 20, "visitor_team_q1": 7, "visitor_team_q2": 3, "visitor_team_q3": null, "visitor_team_q4": 10, "visitor_team_ot": null},
      {"id": 7002, "visitor_team": {"id": 8, "location": "Dallas", "name": "Cowboys", "full_name": "Dallas Cowboys", "abbreviation": "DAL"},
       "home_team": {"id": 21, "location": "Philadelphia", "name": "Eagles", "full_name": "Philadelphia Eagles", "abbreviation": "PHI"},
       "venue": "Lincoln Financial Field", "week": 11, "date": "2026-11-22T18:00:00.000Z", "season": 2026, "postseason": false,
       "status": "11/22 - 1:00 PM EST", "status_state": "scheduled", "home_team_score": null, "visitor_team_score": null},
      {"id": 7003, "visitor_team": {"id": 3, "location": "Buffalo", "name": "Bills", "full_name": "Buffalo Bills", "abbreviation": "BUF"},
       "home_team": {"id": 20, "location": "New York", "name": "Jets", "full_name": "New York Jets", "abbreviation": "NYJ"},
       "venue": "MetLife Stadium", "week": 11, "date": "2026-11-22T21:25:00.000Z", "season": 2026,
       "status": "Final/OT", "home_team_score": 23, "home_team_q1": 3, "home_team_q2": 7, "home_team_q3": 3, "home_team_q4": 7, "home_team_ot": 3,
       "visitor_team_score": 20, "visitor_team_q1": 0, "visitor_team_q2": 10, "visitor_team_q3": 7, "visitor_team_q4": 3, "visitor_team_ot": 0},
      {"id": 7004, "visitor_team": {"name": "Giants", "full_name": "New York Giants", "abbreviation": "NYG"}, "home_team": {"name": "Bears", "full_name": "Chicago Bears", "abbreviation": "CHI"},
       "date": "2026-11-22T18:00:00.000Z", "status": "Postponed", "status_state": "postponed"},
      {"id": "x", "date": "2026-11-22T18:00:00.000Z"}
    ], "meta": {"per_page": 100, "next_cursor": null}}"#;

    const NBA: &str = r#"{"data": [
      {"id": 15907925, "date": "2026-11-18", "season": 2026, "status": "Final", "status_state": "final", "period": 5, "time": "Final", "postseason": false, "postponed": false,
       "home_team_score": 115, "visitor_team_score": 110, "datetime": "2026-11-19T00:00:00.000Z",
       "home_q1": 29, "home_q2": 24, "home_q3": 28, "home_q4": 24, "home_ot1": 10, "home_ot2": null, "home_ot3": null,
       "visitor_q1": 23, "visitor_q2": 25, "visitor_q3": 30, "visitor_q4": 27, "visitor_ot1": 5, "visitor_ot2": null, "visitor_ot3": null,
       "home_team": {"id": 6, "conference": "East", "division": "Central", "city": "Cleveland", "name": "Cavaliers", "full_name": "Cleveland Cavaliers", "abbreviation": "CLE"},
       "visitor_team": {"id": 4, "city": "Charlotte", "name": "Hornets", "full_name": "Charlotte Hornets", "abbreviation": "CHA"}},
      {"id": 15907926, "date": "2026-11-18", "season": 2026, "status": "7:30 pm ET", "period": 0, "time": " ", "postponed": false,
       "home_team_score": 0, "visitor_team_score": 0, "datetime": "2026-11-19T00:30:00.000Z",
       "home_team": {"city": "Boston", "name": "Celtics", "full_name": "Boston Celtics", "abbreviation": "BOS"},
       "visitor_team": {"city": "Orlando", "name": "Magic", "full_name": "Orlando Magic", "abbreviation": "ORL"}},
      {"id": 15907927, "date": "2026-11-18", "status": "3rd Qtr", "status_state": "in_progress", "period": 3, "time": "5:21", "postponed": false,
       "home_team_score": 61, "visitor_team_score": 58, "datetime": "2026-11-19T01:00:00.000Z", "home_q1": 30, "home_q2": 25, "home_q3": 6, "visitor_q1": 28, "visitor_q2": 22, "visitor_q3": 8,
       "home_team": {"city": "Denver", "name": "Nuggets", "full_name": "Denver Nuggets", "abbreviation": "DEN"},
       "visitor_team": {"city": "Utah", "name": "Jazz", "full_name": "Utah Jazz", "abbreviation": "UTA"}}
    ], "meta": {"per_page": 100}}"#;

    const MLB: &str = r#"{"data": [
      {"id": 58590, "home_team_name": "New York Yankees", "away_team_name": "Los Angeles Dodgers",
       "home_team": {"id": 19, "slug": "new-york-yankees", "abbreviation": "NYY", "display_name": "New York Yankees", "short_display_name": "Yankees", "name": "Yankees", "location": "New York"},
       "away_team": {"id": 14, "abbreviation": "LAD", "display_name": "Los Angeles Dodgers", "short_display_name": "Dodgers", "name": "Dodgers", "location": "Los Angeles"},
       "season": 2026, "postseason": true, "season_type": "postseason", "date": "2026-10-30T00:08:00.000Z",
       "home_team_data": {"hits": 9, "runs": 11, "errors": 0, "inning_scores": [0, 1, 4, 0, 0, 1, 0, 5]},
       "away_team_data": {"hits": 6, "runs": 4, "errors": 1, "inning_scores": [2, 0, 0, 0, 2, 0, 0, 0, 0]},
       "venue": "Yankee Stadium", "attendance": 49354, "status": "STATUS_FINAL", "status_state": "final", "period": 9, "clock": 0, "display_clock": "0:00"},
      {"id": 58591, "home_team": {"abbreviation": "BOS", "display_name": "Boston Red Sox", "short_display_name": "Red Sox", "name": "Red Sox"},
       "away_team": {"abbreviation": "TOR", "display_name": "Toronto Blue Jays", "short_display_name": "Blue Jays", "name": "Blue Jays"},
       "date": "2027-04-02T23:10:00.000Z", "home_team_data": {"runs": 0, "inning_scores": []}, "away_team_data": {"runs": 0, "inning_scores": []}, "venue": "Fenway Park", "status": "STATUS_SCHEDULED", "period": 0}
    ], "meta": {"per_page": 100}}"#;

    #[test]
    fn the_nfl_s_games_read_with_their_quarters() {
        let l = league("nfl").unwrap();
        let g = games(l, NFL, when::at(2026, 11, 21, 12, 0)).unwrap();
        assert_eq!(g.len(), 4, "the one with no number for an id is left out");
        let done = &g[0];
        assert_eq!((done.id.as_str(), done.stage, done.period, done.status.as_str()), ("b7001", Stage::Final, 4, "Final"));
        assert_eq!((done.away.name.as_str(), done.away.full.as_str(), done.away.abbr.as_str(), done.away.score), ("Ravens", "Baltimore Ravens", "BAL", Some(20)));
        assert_eq!((done.home.name.as_str(), done.home.score), ("Chiefs", Some(27)));
        // (A quarter with nothing in it is a quarter with no points.)
        assert_eq!((done.home.periods.clone(), done.away.periods.clone()), (vec![7, 6, 7, 7], vec![7, 3, 0, 10]));
        // Thursday night in the East is Friday by the world's clock: its day is Thursday's.
        assert_eq!((done.start, done.day.as_str(), done.time_set, done.venue.as_str()), (when::at(2026, 11, 20, 1, 20), "2026-11-19", true, "GEHA Field at Arrowhead Stadium"));
        let soon = &g[1];
        assert_eq!((soon.stage, soon.home.score, soon.period, soon.status.as_str(), soon.line.is_none()), (Stage::Before, None, 0, "", true));
        // Overtime, said only by its fifth score and "Final/OT".
        assert_eq!((g[2].stage, g[2].period, g[2].home.periods.len(), g[2].away.periods.clone()), (Stage::Final, 5, 5, vec![0, 10, 7, 3, 0]));
        assert_eq!((g[3].stage, g[3].off_why.as_str()), (Stage::Off, "postponed"));
        assert!(games(l, "<html>", 0).is_err() && games(l, "{}", 0).is_err());
    }

    #[test]
    fn the_nba_s_and_mlb_s_read_too() {
        let now = when::at(2026, 11, 19, 1, 30);
        let g = games(league("nba").unwrap(), NBA, now).unwrap();
        assert_eq!(g.len(), 3);
        // Final in overtime: five periods, each side's scores.
        assert_eq!((g[0].stage, g[0].period, g[0].home.periods.clone(), g[0].away.score), (Stage::Final, 5, vec![29, 24, 28, 24, 10], Some(110)));
        assert_eq!((g[0].start, g[0].day.as_str(), g[0].home.name.as_str(), g[0].away.full.as_str()), (when::at(2026, 11, 19, 0, 0), "2026-11-18", "Cavaliers", "Charlotte Hornets"));
        // Not started: its status is its time, its scores noughts that aren't scores. (No
        // `status_state` here: an older answer's shape. The clock says it has begun; a game
        // with no period yet and a time for a status hasn't.)
        let before = games(league("nba").unwrap(), NBA, when::at(2026, 11, 18, 20, 0)).unwrap();
        assert_eq!((before[1].stage, before[1].home.score, before[1].status.as_str()), (Stage::Before, None, ""));
        assert_eq!((g[2].stage, g[2].period, g[2].status.as_str(), g[2].home.score), (Stage::During, 3, "3rd Qtr", Some(61)));
        // Baseball: runs and innings (the home side doesn't bat in the ninth when it's ahead).
        let m = games(league("mlb").unwrap(), MLB, when::at(2026, 10, 30, 12, 0)).unwrap();
        assert_eq!((m[0].id.as_str(), m[0].stage, m[0].period, m[0].home.score, m[0].away.score), ("b58590", Stage::Final, 9, Some(11), Some(4)));
        assert_eq!((m[0].home.name.as_str(), m[0].home.full.as_str(), m[0].away.periods[0], m[0].home.periods.len(), m[0].status.as_str()), ("Yankees", "New York Yankees", 2, 8, ""));
        assert_eq!((m[1].stage, m[1].home.score, m[1].away.name.as_str(), m[1].venue.as_str()), (Stage::Before, None, "Blue Jays", "Fenway Park"));
    }

    #[test]
    fn it_asks_with_its_key_as_a_header_and_says_when_it_has_none() {
        let web = Canned::new(&[("/nfl/v1/games", NFL), ("/v1/games", NBA)]);
        let now = when::at(2026, 11, 21, 12, 0);
        let b = Bdl { key: Some("K".into()) };
        let mut notes = Value::Null;
        assert_eq!(b.coming(&web, league("nfl").unwrap(), now, &mut notes).unwrap().len(), 4);
        assert_eq!(b.day(&web, league("nba").unwrap(), "2026-11-18", now, &mut notes).unwrap().len(), 3);
        let asked = web.asked();
        // The week ahead for football (and a day each side); a day and its neighbors after.
        assert!(asked[0].starts_with("https://api.balldontlie.io/nfl/v1/games?per_page=100&dates[]=2026-11-20&dates[]=2026-11-21&"), "{}", asked[0]);
        assert!(asked[0].contains("&dates[]=2026-11-30") && !asked[0].contains("2026-12-01") && asked[0].ends_with(" [Authorization: K]"), "{}", asked[0]);
        assert_eq!(asked[1], "https://api.balldontlie.io/v1/games?per_page=100&dates[]=2026-11-17&dates[]=2026-11-18&dates[]=2026-11-19 [Authorization: K]");
        // No key: nothing asked, and it says how to give it one.
        let none = Bdl { key: None };
        let e = none.coming(&web, league("mlb").unwrap(), now, &mut notes).unwrap_err();
        assert!(e.contains("no key yet") && e.contains("pyr-props-key balldontlie"), "{e}");
        assert_eq!(web.asked().len(), 2);
    }
}
