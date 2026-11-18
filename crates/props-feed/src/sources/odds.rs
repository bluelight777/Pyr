//! The Odds API (`api.the-odds-api.com`), for the NHL: what's coming, and how each game
//! ended. Read into games as `sports` knows them.
//!
//! Written from its documentation (the v4 guide, read Nov 18 2026); no answer of its own
//! could be read from here, as it wants a key. So everything is taken as optional but a
//! game's id, its start and its two teams.
//!
//! Its free plan: 500 credits a month. The list of games to come costs none; the scores,
//! with the last three days' results, cost two a look: 230 looks a month ([`Budget`]), so a
//! night's games are asked about every fifty minutes once the first could be over. It says
//! who scored what and nothing of how (no periods, no overtime): props that need those
//! aren't opened from it. Its terms (Aug 31 2026) allow showing its data in an app,
//! commercial or not, and forbid passing it on as a data product of its own. The key goes
//! as a header, never in the address.

use serde_json::Value;

use super::num;
use super::sports::{Budget, Game, League, Scoreboard, Stage, Team};
use crate::http::Fetch;
use crate::when::{self, DAY, HOUR, MIN};

const HOST: &str = "https://api.the-odds-api.com/v4/sports";
const NAME: &str = "The Odds API";
const LOOKS: Budget = Budget { month: 230, ahead: 30 };

pub struct Odds {
    pub key: Option<String>,
}

/// Its name for a league.
fn sport(league: &League) -> &'static str {
    match league.cat {
        "nfl" => "americanfootball_nfl",
        "cfb" => "americanfootball_ncaaf",
        "nba" => "basketball_nba",
        "cbb" => "basketball_ncaab",
        "mlb" => "baseball_mlb",
        _ => "icehockey_nhl",
    }
}

impl Odds {
    fn key(&self) -> Result<&str, String> {
        self.key.as_deref().ok_or_else(|| format!("{NAME}: no key yet (on the server: pyr-props-key the-odds-api)"))
    }
}

impl Scoreboard for Odds {
    fn name(&self) -> &'static str {
        NAME
    }

    /// The games to come (and under way): a list that costs nothing.
    fn coming(&self, web: &dyn Fetch, league: &League, now: i64, _notes: &mut Value) -> Result<Vec<Game>, String> {
        let url = format!("{HOST}/{}/events?dateFormat=iso&commenceTimeFrom={}&commenceTimeTo={}", sport(league), when::iso(now - 6 * HOUR), when::iso(now + league.ahead + DAY));
        games(&web.get_with(&url, ("x-api-key", self.key()?))?, now)
    }

    /// The scores: every game of the last three days, whichever day was asked about.
    fn day(&self, web: &dyn Fetch, league: &League, _day: &str, now: i64, notes: &mut Value) -> Result<Vec<Game>, String> {
        let key = self.key()?;
        // (One look answers for every day: not asked twice in a round.)
        if notes["looked"].as_i64() == Some(now) {
            return Ok(Vec::new());
        }
        LOOKS.take(notes, now, NAME)?;
        notes["looked"] = serde_json::json!(now);
        games(&web.get_with(&format!("{HOST}/{}/scores/?daysFrom=3&dateFormat=iso", sport(league)), ("x-api-key", key))?, now)
    }

    fn coming_every(&self, _league: &League) -> i64 {
        6 * HOUR
    }

    fn day_every(&self, _league: &League) -> i64 {
        50 * MIN
    }

    /// It says how a game ended: asked once a game could be over.
    fn first_look(&self, _league: &League) -> i64 {
        145 * MIN
    }

    /// The final score is all it gives: who won, the total, the margin.
    fn settles(&self, _league: &League, kind: &str) -> bool {
        matches!(kind, "win" | "total" | "spread" | "margin")
    }
}

/// What a team is called for short: "Rangers" of "New York Rangers", "Maple Leafs" of
/// "Toronto Maple Leafs", "Utah" of "Utah Hockey Club".
pub fn nickname(full: &str) -> String {
    let full = full.trim();
    const TWO: [&str; 8] = ["Maple Leafs", "Red Wings", "Blue Jackets", "Golden Knights", "Trail Blazers", "Red Sox", "White Sox", "Blue Jays"];
    if let Some(two) = TWO.iter().find(|t| full.ends_with(*t)) {
        return two.to_string();
    }
    for club in [" Hockey Club", " Football Club", " FC"] {
        if let Some(place) = full.strip_suffix(club) {
            return place.to_string();
        }
    }
    full.rsplit(' ').next().unwrap_or(full).to_string()
}

/// A day the leagues' way (US Eastern, near enough).
fn league_day(start: i64) -> String {
    when::iso_date(start - 5 * HOUR)
}

/// An answer's games: the list of what's coming, or the scores (the same with scores).
pub fn games(text: &str, now: i64) -> Result<Vec<Game>, String> {
    let v = super::json(text, NAME)?;
    let list = v.as_array().ok_or_else(|| format!("{NAME}'s answer isn't a list of games"))?;
    let got: Vec<Game> = list.iter().filter_map(|g| game(g, now)).collect();
    if got.is_empty() && !list.is_empty() {
        return Err(format!("{NAME}'s games aren't as this expects them ({} there, none read)", list.len()));
    }
    Ok(got)
}

fn game(g: &Value, now: i64) -> Option<Game> {
    let id = g["id"].as_str()?;
    if id.is_empty() || id.len() > 40 || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let start = when::parse_iso(g["commence_time"].as_str()?)?;
    let scores = g["scores"].as_array();
    let team = |key: &str| -> Option<Team> {
        let full = g[key].as_str()?.trim().to_string();
        if full.is_empty() {
            return None;
        }
        let name = nickname(&full);
        let score = scores.and_then(|s| s.iter().find(|x| x["name"].as_str().is_some_and(|n| n.trim() == full))).and_then(|x| num(&x["score"])).map(|x| x as i64);
        Some(Team { abbr: name.clone(), name, full, score, ..Default::default() })
    };
    let (mut home, mut away) = (team("home_team")?, team("away_team")?);
    let scored = home.score.is_some() && away.score.is_some();
    let stage = match g["completed"].as_bool().unwrap_or(false) {
        true if scored => Stage::Final,
        _ if now < start && !scored => Stage::Before,
        _ => Stage::During,
    };
    if stage == Stage::Before {
        (home.score, away.score) = (None, None);
    }
    Some(Game { id: format!("o{id}"), day: league_day(start), start, time_set: true, stage, home, away, ..Default::default() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Canned;
    use crate::sources::sports::league;

    // As its documentation shows them, not answers read from it: it gives none without a key.
    const EVENTS: &str = r#"[
      {"id": "a512a48a58c4329048174217b2cc7ce0", "sport_key": "icehockey_nhl", "sport_title": "NHL", "commence_time": "2026-11-19T00:00:00Z", "home_team": "New York Rangers", "away_team": "Montréal Canadiens"},
      {"id": "b612a48a58c4329048174217b2cc7ce1", "sport_key": "icehockey_nhl", "sport_title": "NHL", "commence_time": "2026-11-19T02:30:00Z", "home_team": "Utah Mammoth", "away_team": "Toronto Maple Leafs"},
      {"id": "../etc", "commence_time": "2026-11-19T02:30:00Z", "home_team": "A B", "away_team": "C D"}
    ]"#;
    const SCORES: &str = r#"[
      {"id": "572d984e132eddaac3da93e5db332e7e", "sport_key": "icehockey_nhl", "sport_title": "NHL", "commence_time": "2026-11-18T00:10:00Z", "completed": true, "home_team": "Dallas Stars", "away_team": "Florida Panthers",
       "scores": [{"name": "Dallas Stars", "score": "2"}, {"name": "Florida Panthers", "score": "1"}], "last_update": "2026-11-18T02:48:19Z"},
      {"id": "e2296d6d1206f8d185466876e2b444ea", "sport_key": "icehockey_nhl", "commence_time": "2026-11-18T23:00:00Z", "completed": false, "home_team": "Detroit Red Wings", "away_team": "Vegas Golden Knights",
       "scores": [{"name": "Detroit Red Wings", "score": "1"}, {"name": "Vegas Golden Knights", "score": "3"}], "last_update": "2026-11-19T00:40:23Z"},
      {"id": "a512a48a58c4329048174217b2cc7ce0", "sport_key": "icehockey_nhl", "commence_time": "2026-11-19T00:00:00Z", "completed": false, "home_team": "New York Rangers", "away_team": "Montréal Canadiens", "scores": null, "last_update": null}
    ]"#;

    #[test]
    fn what_s_coming_and_the_scores_read() {
        let now = when::at(2026, 11, 18, 20, 0);
        let g = games(EVENTS, now).unwrap();
        assert_eq!(g.len(), 2, "an id that isn't one is left out");
        assert_eq!((g[0].id.as_str(), g[0].stage, g[0].start, g[0].day.as_str()), ("oa512a48a58c4329048174217b2cc7ce0", Stage::Before, when::at(2026, 11, 19, 0, 0), "2026-11-18"));
        assert_eq!((g[0].home.name.as_str(), g[0].home.full.as_str(), g[0].away.name.as_str(), g[0].home.score), ("Rangers", "New York Rangers", "Canadiens", None));
        assert_eq!((g[1].home.name.as_str(), g[1].away.name.as_str()), ("Mammoth", "Maple Leafs"));
        let s = games(SCORES, when::at(2026, 11, 19, 0, 45)).unwrap();
        // Over: who scored what, and nothing of how.
        assert_eq!((s[0].stage, s[0].home.score, s[0].away.score, s[0].period, s[0].home.periods.len()), (Stage::Final, Some(2), Some(1), 0, 0));
        assert_eq!((s[1].stage, s[1].away.name.as_str(), s[1].away.score), (Stage::During, "Golden Knights", Some(3)));
        // Its hour has come and it has no score yet: under way, as far as anyone knows.
        assert_eq!((s[2].stage, s[2].home.score), (Stage::During, None));
        assert_eq!((nickname("Utah Hockey Club"), nickname("St Louis Blues"), nickname("Columbus Blue Jackets")), ("Utah".to_string(), "Blues".to_string(), "Blue Jackets".to_string()));
        assert!(games("{}", now).is_err() && games("nope", now).is_err());
    }

    #[test]
    fn the_list_is_free_the_scores_are_counted_and_the_key_is_a_header() {
        let web = Canned::new(&[("/events", EVENTS), ("/scores", SCORES)]);
        let l = league("nhl").unwrap();
        let b = Odds { key: Some("K".into()) };
        let now = when::at(2026, 11, 18, 20, 0);
        let mut notes = Value::Null;
        assert_eq!(b.coming(&web, l, now, &mut notes).unwrap().len(), 2);
        assert_eq!(Budget::used(&notes, now), 0);
        assert_eq!(b.day(&web, l, "2026-11-17", now, &mut notes).unwrap().len(), 3);
        // (Another day in the same round: the one look answered for it.)
        assert_eq!(b.day(&web, l, "2026-11-18", now, &mut notes).unwrap().len(), 0);
        assert_eq!(Budget::used(&notes, now), 1);
        let asked = web.asked();
        assert_eq!(asked[0], "https://api.the-odds-api.com/v4/sports/icehockey_nhl/events?dateFormat=iso&commenceTimeFrom=2026-11-18T14:00:00Z&commenceTimeTo=2026-11-21T12:00:00Z [x-api-key: K]");
        assert_eq!(asked[1], "https://api.the-odds-api.com/v4/sports/icehockey_nhl/scores/?daysFrom=3&dateFormat=iso [x-api-key: K]");
        assert_eq!(asked.len(), 2);
        assert!(asked.iter().all(|a| !a.split(" [").next().unwrap().contains('K')), "the key is never in an address");
        // Who won, the total and the margin can be told from a final score; overtime can't.
        assert!(b.settles(l, "win") && b.settles(l, "total") && !b.settles(l, "ot") && !b.settles(l, "half"));
        let none = Odds { key: None };
        let e = none.day(&web, l, "2026-11-17", now, &mut notes).unwrap_err();
        assert!(e.contains("no key yet") && e.contains("pyr-props-key the-odds-api"), "{e}");
    }
}
