//! ESPN's public scoreboard (`site.api.espn.com/apis/site/v2/sports/<sport>/<league>/scoreboard`),
//! read into games as `sports` knows them. Its shape is as it was read on Nov 17 2026
//! (`samples/espn-*.json`); everything in it is taken as optional but a game's id, its start
//! and its two teams.
//!
//! Not used unless `--espn` is said (Blue, Nov 18 2026: other sites that allow it, since
//! Disney's terms of use forbid automated access). It's one `sports::Scoreboard` among the
//! others, kept for its tests and should Blue ever say otherwise.

use serde_json::Value;

use super::num;
use super::sports::{Game, League, Line, Scoreboard, Stage, Team};
use crate::http::Fetch;
use crate::math::{american, fair};
use crate::when;

const BASE: &str = "https://site.api.espn.com/apis/site/v2/sports";

pub struct Espn;

impl Espn {
    fn url(league: &League, day: Option<&str>) -> String {
        let mut q: Vec<String> = Vec::new();
        if let Some(d) = day {
            q.push(format!("dates={d}"));
        }
        if !league.query.is_empty() {
            q.push(league.query.to_string());
        }
        format!("{BASE}/{}/scoreboard{}{}", league.path, if q.is_empty() { "" } else { "?" }, q.join("&"))
    }
}

impl Scoreboard for Espn {
    fn name(&self) -> &'static str {
        "ESPN"
    }

    fn coming(&self, web: &dyn Fetch, league: &League, now: i64, _notes: &mut Value) -> Result<Vec<Game>, String> {
        if league.by_week {
            // (A football scoreboard with no date is the week's games.)
            return games(&web.get(&Self::url(league, None))?);
        }
        // Today and tomorrow, as the league dates them (US Eastern).
        let mut all = Vec::new();
        let mut trouble = None;
        for d in 0..2 {
            let day = league_day(now + d * when::DAY);
            match web.get(&Self::url(league, Some(&day))).and_then(|t| games(&t)) {
                Ok(g) => all.extend(g),
                Err(e) => trouble = Some(e),
            }
        }
        match (all.is_empty(), trouble) {
            (true, Some(e)) => Err(e),
            _ => Ok(all),
        }
    }

    fn day(&self, web: &dyn Fetch, league: &League, day: &str, _now: i64, _notes: &mut Value) -> Result<Vec<Game>, String> {
        games(&web.get(&Self::url(league, Some(day)))?)
    }
}

/// The day a game is on, as the leagues date them: the date in US Eastern time, near enough
/// (five hours behind UTC: nothing starts between midnight and one in the morning there).
pub fn league_day(start: i64) -> String {
    when::yyyymmdd(start - 5 * when::HOUR)
}

/// A scoreboard's games.
pub fn games(text: &str) -> Result<Vec<Game>, String> {
    let v = super::json(text, "ESPN")?;
    let events = v["events"].as_array().ok_or("ESPN's scoreboard has no events")?;
    Ok(events.iter().filter_map(game).collect())
}

fn team(c: &Value) -> Option<Team> {
    let t = &c["team"];
    let name = t["shortDisplayName"].as_str().or_else(|| t["name"].as_str())?.trim().to_string();
    let full = t["displayName"].as_str().unwrap_or(&name).trim().to_string();
    // "5-3-1": wins and losses (ties and overtime losses aside).
    let record = c["records"].as_array().and_then(|r| r.iter().find(|x| x["type"] == "total").or_else(|| r.first())).and_then(|r| r["summary"].as_str()).and_then(|s| {
        let mut p = s.split('-').map(|x| x.trim().parse::<u32>().ok());
        Some((p.next()??, p.next()??))
    });
    Some(Team {
        name,
        full,
        abbr: t["abbreviation"].as_str().unwrap_or("").to_string(),
        score: num(&c["score"]).map(|s| s as i64),
        won: c["winner"].as_bool(),
        record,
        rank: c["curatedRank"]["current"].as_u64().filter(|r| (1..=25).contains(r)).map(|r| r as u32),
        periods: c["linescores"].as_array().map(|l| l.iter().filter_map(|x| num(&x["value"])).map(|x| x as i64).collect()).unwrap_or_default(),
    })
}

/// The bookmaker's lines, as chances with its margin taken out.
fn line(o: &Value) -> Option<Line> {
    let odds = |v: &Value| num(v).filter(|x| x.abs() >= 100.0).map(|x| american(x as i32));
    let pair = |a: &Value, b: &Value| Some(fair(odds(a)?, odds(b)?).0);
    let l = Line {
        // (The home team's line: -4.5 when it's the favorite.)
        spread_home: num(&o["spread"]).or_else(|| num(&o["pointSpread"]["home"]["close"]["line"])),
        total: num(&o["overUnder"]).or_else(|| num(&o["total"]["over"]["close"]["line"])),
        win_home: pair(&o["moneyline"]["home"]["close"]["odds"], &o["moneyline"]["away"]["close"]["odds"]).or_else(|| pair(&o["homeTeamOdds"]["moneyLine"], &o["awayTeamOdds"]["moneyLine"])),
        cover_home: pair(&o["pointSpread"]["home"]["close"]["odds"], &o["pointSpread"]["away"]["close"]["odds"]),
        over: pair(&o["total"]["over"]["close"]["odds"], &o["total"]["under"]["close"]["odds"]),
    };
    (l != Line::default()).then_some(l)
}

fn game(e: &Value) -> Option<Game> {
    let c = &e["competitions"][0];
    let id = e["id"].as_str()?.to_string();
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    let start = when::parse_iso(c["date"].as_str().or_else(|| e["date"].as_str())?)?;
    let sides = c["competitors"].as_array()?;
    let side = |which: &str| sides.iter().find(|s| s["homeAway"] == which).and_then(team);
    let (home, away) = (side("home")?, side("away")?);
    let status = if c["status"]["type"].is_object() { &c["status"] } else { &e["status"] };
    let kind = &status["type"];
    let name = kind["name"].as_str().unwrap_or("");
    let stage = if ["POSTPONED", "CANCELED", "CANCELLED", "SUSPENDED", "FORFEIT", "ABANDONED"].iter().any(|w| name.contains(w)) {
        Stage::Off
    } else {
        match kind["state"].as_str().unwrap_or("") {
            "post" if kind["completed"].as_bool().unwrap_or(false) => Stage::Final,
            "in" | "post" => Stage::During,
            _ => Stage::Before,
        }
    };
    let tv = c["broadcasts"].as_array().and_then(|b| b.iter().find(|x| x["market"] == "national").or_else(|| b.first())).and_then(|b| b["names"][0].as_str()).unwrap_or("").to_string();
    Some(Game {
        day: league_day(start),
        id,
        start,
        time_set: c["timeValid"].as_bool().unwrap_or(true),
        stage,
        status: kind["shortDetail"].as_str().or_else(|| kind["description"].as_str()).unwrap_or("").to_string(),
        off_why: kind["description"].as_str().unwrap_or("").to_lowercase(),
        period: status["period"].as_u64().unwrap_or(0) as u32,
        home,
        away,
        venue: c["venue"]["fullName"].as_str().unwrap_or("").to_string(),
        neutral: c["neutralSite"].as_bool().unwrap_or(false),
        tv,
        line: c["odds"].as_array().and_then(|o| o.first()).and_then(line),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::sports::league;

    const NFL: &str = include_str!("../../samples/espn-nfl.json");
    const NHL: &str = include_str!("../../samples/espn-nhl.json");

    #[test]
    fn a_scoreboard_reads_as_games() {
        let g = games(NFL).unwrap();
        assert_eq!(g.len(), 3);
        // Final: Commanders 24 at Giants 21.
        let done = &g[0];
        assert_eq!((done.id.as_str(), done.stage, done.period), ("401873052", Stage::Final, 4));
        assert_eq!((done.away.name.as_str(), done.away.score, done.away.won), ("Commanders", Some(24), Some(true)));
        assert_eq!((done.home.full.as_str(), done.home.score, done.home.won, done.home.record), ("New York Giants", Some(21), Some(false), Some((5, 3))));
        assert_eq!(done.home.periods, vec![0, 7, 7, 7]);
        assert_eq!((done.start, done.day.as_str()), (when::at(2026, 11, 13, 1, 15), "20261112"));
        assert!(done.line.is_none());
        // Not started, with the bookmaker's lines: Ravens -4.5, 39.5, -230 / +190.
        let soon = &g[1];
        assert_eq!((soon.stage, soon.venue.as_str(), soon.tv.as_str(), soon.status.as_str()), (Stage::Before, "M&T Bank Stadium", "ESPN", "11/16 - 8:15 PM EST"));
        let l = soon.line.as_ref().unwrap();
        assert_eq!((l.spread_home, l.total), (Some(-4.5), Some(39.5)));
        assert!((l.win_home.unwrap() - 0.669).abs() < 0.001, "{l:?}");
        assert!((l.cover_home.unwrap() - 0.504).abs() < 0.002 && (l.over.unwrap() - 0.496).abs() < 0.002, "{l:?}");
        assert_eq!(soon.away.record, Some((0, 8)));
        // Overtime, on neutral ground.
        assert_eq!((g[2].period, g[2].neutral, g[2].home.periods.len()), (5, true, 5));
    }

    #[test]
    fn hockey_s_lines_read_too_and_the_addresses_are_right() {
        let g = games(NHL).unwrap();
        assert_eq!(g.len(), 2);
        let l = g[0].line.as_ref().unwrap();
        // Montreal -115 at the Rangers -105: near even, the visitors a shade ahead.
        assert!((l.win_home.unwrap() - 0.489).abs() < 0.002, "{l:?}");
        assert_eq!((l.spread_home, l.total), (Some(1.5), Some(6.5)));
        assert!(g[1].line.as_ref().unwrap().win_home.unwrap() > 0.74);
        assert_eq!(g[1].tv, "ESPN+");
        assert_eq!(Espn::url(league("nfl").unwrap(), None), "https://site.api.espn.com/apis/site/v2/sports/football/nfl/scoreboard");
        assert_eq!(Espn::url(league("nhl").unwrap(), Some("20261116")), "https://site.api.espn.com/apis/site/v2/sports/hockey/nhl/scoreboard?dates=20261116");
        assert_eq!(Espn::url(league("cbb").unwrap(), Some("20261116")), "https://site.api.espn.com/apis/site/v2/sports/basketball/mens-college-basketball/scoreboard?dates=20261116&groups=50&limit=300");
        // Something that isn't a scoreboard: said so, not guessed at.
        assert!(games("<html>").is_err());
        assert!(games("{}").is_err());
        assert_eq!(games(r#"{"events":[{"id":"1"},{"id":"../2","competitions":[{}]}]}"#).unwrap().len(), 0);
    }

    #[test]
    fn a_game_put_off_or_under_way_is_told_apart() {
        let off = NFL.replace("STATUS_SCHEDULED", "STATUS_POSTPONED").replace("\"description\": \"Scheduled\"", "\"description\": \"Postponed\"");
        let g = games(&off).unwrap();
        assert_eq!((g[1].stage, g[1].off_why.as_str()), (Stage::Off, "postponed"));
        let live = NFL.replace("\"state\": \"pre\"", "\"state\": \"in\"").replace("STATUS_SCHEDULED", "STATUS_IN_PROGRESS");
        assert_eq!(games(&live).unwrap()[1].stage, Stage::During);
        // Over but not yet marked complete: still under way.
        let nearly = NFL.replace("\"state\": \"pre\"", "\"state\": \"post\"");
        assert_eq!(games(&nearly).unwrap()[1].stage, Stage::During);
    }
}
