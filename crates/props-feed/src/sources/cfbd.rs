//! CollegeFootballData.com and CollegeBasketballData.com (one maker, one key, one monthly
//! count for both): college football's and men's college basketball's games, the Top 25 and
//! the bookmakers' lines. Read into games as `sports` knows them.
//!
//! Written from their documentation (apinext.collegefootballdata.com and
//! api.collegebasketballdata.com's OpenAPI, read Nov 18 2026); no answer of theirs could be
//! read from here, as they want a key. So everything is taken as optional but a game's id,
//! its start and its two teams.
//!
//! The free plan: a thousand calls a month for the two together, games with their results
//! and line scores, lines and rankings; no scoreboard of games under way (a dollar a month
//! has one). So a game is asked about once it could be over, and each sport keeps to its
//! part of the thousand ([`Budget`]): football 440, basketball 480. Their terms (Aug 12
//! 2026) allow commercial use and betting-related uses, keeping answers, and showing parts
//! of the data in a larger product; they forbid passing the raw answers on or standing in
//! for the API. The key stays on the server.

use serde_json::{Value, json};

use super::num;
use super::sports::{Budget, Game, League, Line, Scoreboard, Stage, Team};
use crate::http::Fetch;
use crate::math::{american, fair};
use crate::when::{self, DAY, HOUR, MIN};

const FOOTBALL: &str = "https://api.collegefootballdata.com";
const BASKETBALL: &str = "https://api.collegebasketballdata.com";
const NAME: &str = "CollegeFootballData";
const FOOTBALL_CALLS: Budget = Budget { month: 440, ahead: 90 };
const BASKETBALL_CALLS: Budget = Budget { month: 480, ahead: 60 };
/// How often the lines and the Top 25 are asked for again.
const LINES_EVERY: i64 = 11 * HOUR;
const RANKS_EVERY: i64 = 3 * DAY;

pub struct Cfbd {
    pub key: Option<String>,
}

/// The football season a moment is in: the year it kicked off (January's bowls are the
/// year before's).
fn season_of(now: i64) -> i64 {
    let (y, m, _) = when::ymd(now);
    if m <= 2 { y - 1 } else { y }
}

/// The day a game is on, as the leagues date them (US Eastern, near enough).
fn league_day(start: i64) -> String {
    when::iso_date(start - 5 * HOUR)
}

/// A moment for an address ("2026-11-18T05%3A00%3A00Z").
fn stamp(at: i64) -> String {
    when::iso(at).replace(':', "%3A")
}

impl Cfbd {
    fn budget(league: &League) -> &'static Budget {
        if league.cat == "cfb" { &FOOTBALL_CALLS } else { &BASKETBALL_CALLS }
    }

    /// One call, out of this sport's part of the month's.
    fn ask(&self, web: &dyn Fetch, league: &League, url: &str, now: i64, notes: &mut Value) -> Result<Value, String> {
        let Some(key) = self.key.as_deref() else {
            return Err(format!("{NAME}: no key yet (on the server: pyr-props-key collegefootballdata)"));
        };
        Self::budget(league).take(notes, now, NAME)?;
        super::json(&web.get_with(url, ("Authorization", &format!("Bearer {key}")))?, NAME)
    }

    /// The lines and the Top 25, when they're due (kept in the notes between).
    fn extras(&self, web: &dyn Fetch, league: &League, now: i64, notes: &mut Value, lines: &[String], ranks: Option<String>) {
        if now - notes["lines_at"].as_i64().unwrap_or(0) >= LINES_EVERY {
            let mut all = serde_json::Map::new();
            let mut ok = !lines.is_empty();
            for url in lines {
                match self.ask(web, league, url, now, notes) {
                    Ok(v) => all.extend(lines_of(&v)),
                    Err(_) => ok = false,
                }
            }
            if ok {
                notes["lines"] = Value::Object(all);
                notes["lines_at"] = json!(now);
            }
        }
        if let Some(url) = ranks.filter(|_| now - notes["ranks_at"].as_i64().unwrap_or(0) >= RANKS_EVERY) {
            if let Ok(v) = self.ask(web, league, &url, now, notes) {
                notes["ranks"] = Value::Object(if league.cat == "cfb" { football_ranks(&v) } else { basketball_ranks(&v) });
                notes["ranks_at"] = json!(now);
            }
        }
    }
}

impl Scoreboard for Cfbd {
    fn name(&self) -> &'static str {
        NAME
    }

    fn coming(&self, web: &dyn Fetch, league: &League, now: i64, notes: &mut Value) -> Result<Vec<Game>, String> {
        let (from, to) = (now - DAY, now + league.ahead + DAY);
        if league.cat == "cfb" {
            // The season's games in one answer; which week each day's are in is kept, to
            // ask about a day by its week.
            let season = season_of(now);
            let v = self.ask(web, league, &format!("{FOOTBALL}/games?year={season}&seasonType=both&classification=fbs"), now, notes)?;
            let every = v.as_array().ok_or_else(|| format!("{NAME}'s answer isn't a list of games"))?;
            if !every.is_empty() && every.iter().all(|g| start_of(g).is_none()) {
                return Err(format!("{NAME}'s games aren't as this expects them ({} there, none with a start)", every.len()));
            }
            let list: Vec<&Value> = every.iter().filter(|g| start_of(g).is_some_and(|s| s >= from && s <= to)).collect();
            let mut weeks = serde_json::Map::new();
            let mut ahead: Vec<(String, i64)> = Vec::new();
            for g in &list {
                let (Some(start), Some(kind), Some(week)) = (start_of(g), g["seasonType"].as_str(), g["week"].as_i64()) else { continue };
                weeks.insert(league_day(start), json!([kind, week]));
                if start > now && !ahead.iter().any(|(k, w)| k == kind && *w == week) {
                    ahead.push((kind.to_string(), week));
                }
            }
            notes["weeks"] = Value::Object(weeks);
            let lines: Vec<String> = ahead.iter().take(2).map(|(kind, week)| format!("{FOOTBALL}/lines?year={season}&seasonType={kind}&week={week}")).collect();
            self.extras(web, league, now, notes, &lines, Some(format!("{FOOTBALL}/rankings?year={season}")));
            read(&list, now, notes)
        } else {
            let range = format!("startDateRange={}&endDateRange={}", stamp(from), stamp(to));
            let v = self.ask(web, league, &format!("{BASKETBALL}/games?{range}"), now, notes)?;
            let list = v.as_array().ok_or_else(|| format!("{NAME}'s answer isn't a list of games"))?;
            let season = list.iter().find_map(|g| g["season"].as_i64());
            let lines = vec![format!("{BASKETBALL}/lines?startDateRange={}&endDateRange={}", stamp(now), stamp(to))];
            self.extras(web, league, now, notes, &lines, season.map(|s| format!("{BASKETBALL}/rankings?season={s}&pollType=ap")));
            read(&list.iter().collect::<Vec<_>>(), now, notes)
        }
    }

    fn day(&self, web: &dyn Fetch, league: &League, day: &str, now: i64, notes: &mut Value) -> Result<Vec<Game>, String> {
        let at = when::parse_date(day).ok_or("a day that isn't one")?;
        let url = if league.cat == "cfb" {
            let season = season_of(at);
            match (notes["weeks"][day][0].as_str(), notes["weeks"][day][1].as_i64()) {
                (Some(kind), Some(week)) => format!("{FOOTBALL}/games?year={season}&seasonType={kind}&week={week}&classification=fbs"),
                _ => format!("{FOOTBALL}/games?year={season}&seasonType=both&classification=fbs"),
            }
        } else {
            // (The day in the East: from its midnight to two in the morning after.)
            format!("{BASKETBALL}/games?startDateRange={}&endDateRange={}", stamp(at + 4 * HOUR), stamp(at + 31 * HOUR))
        };
        let v = self.ask(web, league, &url, now, notes)?;
        read(&v.as_array().ok_or_else(|| format!("{NAME}'s answer isn't a list of games"))?.iter().collect::<Vec<_>>(), now, notes)
    }

    /// Twice a day: what's coming changes slowly, and every call counts.
    fn coming_every(&self, _league: &League) -> i64 {
        12 * HOUR
    }

    fn day_every(&self, league: &League) -> i64 {
        if league.cat == "cfb" { 15 * MIN } else { 30 * MIN }
    }

    /// It says how a game ended, not how it's going: asked once a game could be over.
    fn first_look(&self, league: &League) -> i64 {
        if league.cat == "cfb" { 3 * HOUR } else { 115 * MIN }
    }
}

fn start_of(g: &Value) -> Option<i64> {
    when::parse_iso(g["startDate"].as_str()?)
}

/// An answer's games; ones that are there and can't be read are a failure, not a quiet
/// nothing.
fn read(list: &[&Value], now: i64, notes: &Value) -> Result<Vec<Game>, String> {
    let got: Vec<Game> = list.iter().filter_map(|g| game(g, now, notes)).collect();
    if got.is_empty() && !list.is_empty() {
        return Err(format!("{NAME}'s games aren't as this expects them ({} there, none read)", list.len()));
    }
    Ok(got)
}

/// What a side scored each period ("homeLineScores" in football, "homePeriodPoints" in
/// basketball).
fn periods(g: &Value, side: &str) -> Vec<i64> {
    [format!("{side}LineScores"), format!("{side}PeriodPoints")].iter().find_map(|k| g[k.as_str()].as_array()).map(|a| a.iter().map(|x| num(x).unwrap_or(0.0) as i64).collect()).unwrap_or_default()
}

fn game(g: &Value, now: i64, notes: &Value) -> Option<Game> {
    let id = g["id"].as_u64()?;
    let start = start_of(g)?;
    let team = |side: &str| -> Option<Team> {
        let name = g[format!("{side}Team").as_str()].as_str()?.trim().to_string();
        if name.is_empty() {
            return None;
        }
        Some(Team {
            // (A school is called by its name: no nickname, no letters of its own here.)
            full: name.clone(),
            abbr: name.clone(),
            score: num(&g[format!("{side}Points").as_str()]).map(|x| x as i64),
            rank: notes["ranks"][name.as_str()].as_u64().filter(|r| (1..=25).contains(r)).map(|r| r as u32),
            periods: periods(g, side),
            name,
            ..Default::default()
        })
    };
    let (mut home, mut away) = (team("home")?, team("away")?);
    // Football says only whether it's over; basketball says how it stands.
    let status = g["status"].as_str().unwrap_or("");
    let stage = match status {
        "final" => Stage::Final,
        "postponed" | "cancelled" | "canceled" => Stage::Off,
        "in_progress" => Stage::During,
        "scheduled" => Stage::Before,
        _ if g["completed"].as_bool().unwrap_or(false) => Stage::Final,
        _ if now < start => Stage::Before,
        _ => Stage::During,
    };
    // (Over with no score isn't a result yet.)
    let stage = if stage == Stage::Final && (home.score.is_none() || away.score.is_none()) { Stage::During } else { stage };
    if stage == Stage::Before {
        (home.score, away.score) = (None, None);
    }
    let period = if stage == Stage::Final { home.periods.len().max(away.periods.len()) as u32 } else { 0 };
    let line = &notes["lines"][id.to_string().as_str()];
    let odds = |v: &Value| num(v).filter(|x| x.abs() >= 100.0).map(|x| american(x as i32));
    let line = line.is_array().then(|| Line {
        spread_home: num(&line[0]),
        total: num(&line[1]),
        win_home: odds(&line[2]).zip(odds(&line[3])).map(|(h, a)| fair(h, a).0),
        ..Default::default()
    });
    Some(Game {
        id: format!("c{id}"),
        day: league_day(start),
        start,
        time_set: !(g["startTimeTBD"].as_bool().or_else(|| g["startTimeTbd"].as_bool()).unwrap_or(false)),
        stage,
        status: String::new(),
        off_why: if stage == Stage::Off { status.replace("cancelled", "canceled") } else { String::new() },
        period,
        home,
        away,
        venue: g["venue"].as_str().unwrap_or("").trim().to_string(),
        neutral: g["neutralSite"].as_bool().unwrap_or(false),
        tv: String::new(),
        line: line.filter(|l| *l != Line::default()),
    })
}

/// The lines of an answer's games, as they're kept: the game's id to its home spread, its
/// total, and the two moneylines (the bookmaker called "consensus" first, else the first
/// with a spread or a total).
fn lines_of(v: &Value) -> serde_json::Map<String, Value> {
    let mut out = serde_json::Map::new();
    for g in v.as_array().into_iter().flatten() {
        let Some(id) = g["id"].as_u64().or_else(|| g["gameId"].as_u64()) else { continue };
        let all: Vec<&Value> = g["lines"].as_array().into_iter().flatten().filter(|l| num(&l["spread"]).is_some() || num(&l["overUnder"]).is_some()).collect();
        let Some(l) = all.iter().find(|l| l["provider"].as_str().is_some_and(|p| p.eq_ignore_ascii_case("consensus"))).or_else(|| all.first()) else { continue };
        out.insert(id.to_string(), json!([num(&l["spread"]), num(&l["overUnder"]), num(&l["homeMoneyline"]), num(&l["awayMoneyline"])]));
    }
    out
}

/// Football's Top 25: the latest week's, the playoff committee's when it has ranked, else
/// the AP's, else whichever poll is there. School to rank.
fn football_ranks(v: &Value) -> serde_json::Map<String, Value> {
    let weeks = v.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let Some(latest) = weeks.iter().max_by_key(|w| (w["seasonType"] == "postseason", w["week"].as_i64().unwrap_or(0))) else { return Default::default() };
    let polls = latest["polls"].as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let named = |word: &str| polls.iter().find(|p| p["poll"].as_str().is_some_and(|n| n.to_lowercase().contains(word)));
    let Some(poll) = named("playoff").or_else(|| named("ap top")).or_else(|| named("coaches")).or_else(|| polls.first()) else { return Default::default() };
    poll["ranks"].as_array().into_iter().flatten().filter_map(|r| Some((r["school"].as_str()?.to_string(), json!(r["rank"].as_u64()?)))).collect()
}

/// Basketball's: the AP poll's latest week. Team to ranking.
fn basketball_ranks(v: &Value) -> serde_json::Map<String, Value> {
    let rows = v.as_array().map(|a| a.as_slice()).unwrap_or(&[]);
    let order = |r: &Value| (r["seasonType"] == "postseason", r["week"].as_i64().unwrap_or(0));
    let Some(latest) = rows.iter().map(order).max() else { return Default::default() };
    rows.iter().filter(|r| order(r) == latest).filter_map(|r| Some((r["team"].as_str()?.to_string(), json!(num(&r["ranking"])? as u64)))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Canned;
    use crate::sources::sports::league;

    // As their documentation describes them (the field names and what's in them), not
    // answers read from them: they give none without a key.
    const GAMES: &str = r#"[
      {"id": 401752800, "season": 2026, "week": 12, "seasonType": "regular", "startDate": "2026-11-14T17:00:00.000Z", "startTimeTBD": false, "completed": true, "neutralSite": false,
       "venue": "Ohio Stadium", "homeId": 194, "homeTeam": "Ohio State", "homeClassification": "fbs", "homePoints": 31, "homeLineScores": [7, 10, 7, 7],
       "awayId": 130, "awayTeam": "Michigan", "awayClassification": "fbs", "awayPoints": 24, "awayLineScores": [3, 7, 7, 7], "notes": null},
      {"id": 401752900, "season": 2026, "week": 13, "seasonType": "regular", "startDate": "2026-11-21T20:30:00.000Z", "startTimeTBD": false, "completed": false, "neutralSite": false,
       "venue": "Bryant-Denny Stadium", "homeTeam": "Alabama", "homePoints": null, "homeLineScores": null, "awayTeam": "Auburn", "awayPoints": null, "awayLineScores": null},
      {"id": 401752901, "season": 2026, "week": 13, "seasonType": "regular", "startDate": "2026-11-21T05:00:00.000Z", "startTimeTBD": true, "completed": false, "neutralSite": true,
       "venue": null, "homeTeam": "Rice", "homePoints": null, "awayTeam": "Tulsa", "awayPoints": null},
      {"id": 401752000, "season": 2026, "week": 2, "seasonType": "regular", "startDate": "2026-09-05T17:00:00.000Z", "completed": true, "homeTeam": "Texas", "homePoints": 40, "awayTeam": "Rice", "awayPoints": 3}
    ]"#;
    const LINES: &str = r#"[
      {"id": 401752900, "season": 2026, "week": 13, "homeTeam": "Alabama", "awayTeam": "Auburn", "lines": [
        {"provider": "Bovada", "spread": -9, "formattedSpread": "Alabama -9", "overUnder": 47, "homeMoneyline": -360, "awayMoneyline": 280},
        {"provider": "consensus", "spread": -9.5, "formattedSpread": "Alabama -9.5", "overUnder": 47.5, "homeMoneyline": -350, "awayMoneyline": 275}]},
      {"id": 401752901, "lines": [{"provider": "Bovada", "spread": null, "overUnder": null, "homeMoneyline": null, "awayMoneyline": null}]}
    ]"#;
    const RANKS: &str = r#"[
      {"season": 2026, "seasonType": "regular", "week": 11, "polls": [{"poll": "AP Top 25", "ranks": [{"rank": 1, "school": "Michigan"}]}]},
      {"season": 2026, "seasonType": "regular", "week": 12, "polls": [
        {"poll": "Coaches Poll", "ranks": [{"rank": 3, "school": "Alabama"}]},
        {"poll": "AP Top 25", "ranks": [{"rank": 2, "school": "Alabama"}, {"rank": 4, "school": "Ohio State"}]},
        {"poll": "Playoff Committee Rankings", "ranks": [{"rank": 5, "school": "Alabama", "conference": "SEC", "firstPlaceVotes": 0, "points": 0}, {"rank": 1, "school": "Ohio State"}, {"rank": 24, "school": "Auburn"}]}]}
    ]"#;
    const HOOPS: &str = r#"[
      {"id": 55001, "sourceId": "401800001", "season": 2027, "seasonType": "regular", "startDate": "2026-11-18T00:00:00.000Z", "startTimeTbd": false, "neutralSite": false, "status": "final",
       "homeTeam": "Kansas", "homePoints": 78, "homePeriodPoints": [40, 38], "awayTeam": "Duke", "awayPoints": 81, "awayPeriodPoints": [35, 46], "venue": "Allen Fieldhouse", "city": "Lawrence", "state": "KS"},
      {"id": 55002, "season": 2027, "startDate": "2026-11-19T00:30:00.000Z", "startTimeTbd": false, "neutralSite": true, "status": "scheduled",
       "homeTeam": "Gonzaga", "homePoints": null, "homePeriodPoints": null, "awayTeam": "Houston", "awayPoints": null, "venue": "T-Mobile Arena"},
      {"id": 55003, "season": 2027, "startDate": "2026-11-19T01:00:00.000Z", "status": "postponed", "homeTeam": "UTEP", "awayTeam": "Rice"},
      {"id": 55004, "season": 2027, "startDate": "2026-11-18T23:00:00.000Z", "status": "in_progress", "homeTeam": "Purdue", "homePoints": 30, "awayTeam": "Marquette", "awayPoints": 28}
    ]"#;
    const HOOPS_LINES: &str = r#"[{"gameId": 55002, "homeTeam": "Gonzaga", "awayTeam": "Houston", "lines": [{"provider": "ESPN BET", "spread": 2.5, "overUnder": 139.5, "homeMoneyline": 120, "awayMoneyline": -142}]}]"#;
    const HOOPS_RANKS: &str = r#"[
      {"season": 2027, "seasonType": "regular", "week": 2, "pollType": "ap", "team": "Duke", "ranking": 9},
      {"season": 2027, "seasonType": "regular", "week": 3, "pollType": "ap", "team": "Duke", "ranking": 5},
      {"season": 2027, "seasonType": "regular", "week": 3, "pollType": "ap", "team": "Houston", "ranking": 1},
      {"season": 2027, "seasonType": "regular", "week": 3, "pollType": "ap", "team": "Kansas", "ranking": null}]"#;

    #[test]
    fn football_s_week_reads_with_its_lines_and_its_top_25() {
        let web = Canned::new(&[("/rankings", RANKS), ("/lines", LINES), ("collegefootballdata.com/games", GAMES)]);
        let l = league("cfb").unwrap();
        let b = Cfbd { key: Some("K".into()) };
        let now = when::at(2026, 11, 14, 21, 0);
        let mut notes = Value::Null;
        let g = b.coming(&web, l, now, &mut notes).unwrap();
        // September's game is out of the window; the three near it are in.
        assert_eq!(g.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["c401752800", "c401752900", "c401752901"]);
        // Over: its score, and the committee's ranks (the latest week's: Michigan isn't in it).
        assert_eq!((g[0].stage, g[0].home.score, g[0].home.rank, g[0].away.rank), (Stage::Final, Some(31), Some(1), None));
        assert_eq!((g[0].home.periods.clone(), g[0].away.periods.clone(), g[0].period, g[0].day.as_str()), (vec![7, 10, 7, 7], vec![3, 7, 7, 7], 4, "2026-11-14"));
        let bama = &g[1];
        assert_eq!((bama.stage, bama.time_set, bama.home.name.as_str(), bama.home.rank, bama.away.rank, bama.venue.as_str()), (Stage::Before, true, "Alabama", Some(5), Some(24), "Bryant-Denny Stadium"));
        // The consensus line: Alabama by 9.5, 47.5, -350 against +275.
        let line = bama.line.as_ref().unwrap();
        assert_eq!((line.spread_home, line.total), (Some(-9.5), Some(47.5)));
        assert!((line.win_home.unwrap() - 0.745).abs() < 0.005, "{line:?}");
        // A time not set yet, on neutral ground, a line with nothing in it.
        assert_eq!((g[2].time_set, g[2].neutral, g[2].line.is_none()), (false, true, true));
        // Three calls: the season, the week's lines (the one week ahead), the rankings.
        let asked = web.asked();
        assert_eq!(asked.len(), 3);
        assert_eq!(asked[0], "https://api.collegefootballdata.com/games?year=2026&seasonType=both&classification=fbs [Authorization: Bearer K]");
        assert_eq!(asked[1], "https://api.collegefootballdata.com/lines?year=2026&seasonType=regular&week=13 [Authorization: Bearer K]");
        assert_eq!(asked[2], "https://api.collegefootballdata.com/rankings?year=2026 [Authorization: Bearer K]");
        assert_eq!(Budget::used(&notes, now), 3);
        // A day is asked about by its week; the lines and ranks aren't asked for again so soon.
        let d = b.day(&web, l, "2026-11-14", now + HOUR, &mut notes).unwrap();
        assert_eq!(d.len(), 4);
        assert_eq!(web.asked()[3], "https://api.collegefootballdata.com/games?year=2026&seasonType=regular&week=12&classification=fbs [Authorization: Bearer K]");
        b.coming(&web, l, now + 2 * HOUR, &mut notes).unwrap();
        assert_eq!((web.asked().len(), Budget::used(&notes, now)), (5, 5));
        // January's bowls are the season before's.
        assert_eq!((season_of(when::at(2027, 1, 2, 0, 0)), season_of(when::at(2026, 8, 29, 0, 0))), (2026, 2026));
    }

    #[test]
    fn basketball_s_days_read_with_how_each_game_stands() {
        let web = Canned::new(&[("/rankings", HOOPS_RANKS), ("/lines", HOOPS_LINES), ("collegebasketballdata.com/games", HOOPS)]);
        let l = league("cbb").unwrap();
        let b = Cfbd { key: Some("K".into()) };
        let now = when::at(2026, 11, 18, 12, 0);
        let mut notes = Value::Null;
        let g = b.coming(&web, l, now, &mut notes).unwrap();
        assert_eq!(g.len(), 4);
        // Final, with its halves and the AP's latest week (a team it doesn't rank has none).
        assert_eq!((g[0].id.as_str(), g[0].stage, g[0].period, g[0].away.score, g[0].away.rank, g[0].home.rank), ("c55001", Stage::Final, 2, Some(81), Some(5), None));
        assert_eq!((g[0].home.periods.clone(), g[0].day.as_str()), (vec![40, 38], "2026-11-17"));
        // To come: Houston the favorite by 2.5 away from home, on neutral ground.
        let soon = &g[1];
        assert_eq!((soon.stage, soon.neutral, soon.away.rank, soon.line.as_ref().unwrap().spread_home, soon.line.as_ref().unwrap().total), (Stage::Before, true, Some(1), Some(2.5), Some(139.5)));
        assert!(soon.line.as_ref().unwrap().win_home.unwrap() < 0.45);
        assert_eq!((g[2].stage, g[2].off_why.as_str(), g[3].stage), (Stage::Off, "postponed", Stage::During));
        let asked = web.asked();
        assert_eq!(asked[0], "https://api.collegebasketballdata.com/games?startDateRange=2026-11-17T12%3A00%3A00Z&endDateRange=2026-11-21T04%3A00%3A00Z [Authorization: Bearer K]");
        assert_eq!(asked[1], "https://api.collegebasketballdata.com/lines?startDateRange=2026-11-18T12%3A00%3A00Z&endDateRange=2026-11-21T04%3A00%3A00Z [Authorization: Bearer K]");
        assert_eq!(asked[2], "https://api.collegebasketballdata.com/rankings?season=2027&pollType=ap [Authorization: Bearer K]");
        b.day(&web, l, "2026-11-17", now, &mut notes).unwrap();
        assert_eq!(web.asked()[3], "https://api.collegebasketballdata.com/games?startDateRange=2026-11-17T04%3A00%3A00Z&endDateRange=2026-11-18T07%3A00%3A00Z [Authorization: Bearer K]");
    }

    #[test]
    fn it_keeps_to_its_part_of_the_month_and_says_when_it_has_no_key() {
        let web = Canned::new(&[("/rankings", RANKS), ("/lines", LINES), ("/games", GAMES)]);
        let l = league("cfb").unwrap();
        let none = Cfbd { key: None };
        let mut notes = Value::Null;
        let e = none.coming(&web, l, when::at(2026, 11, 14, 12, 0), &mut notes).unwrap_err();
        assert!(e.contains("no key yet") && e.contains("pyr-props-key collegefootballdata"), "{e}");
        assert!(web.asked().is_empty());
        // The first of the month: 90 calls to start on, and no more until the month earns them.
        let b = Cfbd { key: Some("K".into()) };
        let first = when::at(2026, 12, 1, 0, 0);
        let mut made = 0;
        while b.day(&web, l, "2026-11-28", first, &mut notes).is_ok() {
            made += 1;
            assert!(made < 200);
        }
        assert_eq!(made, 90);
        let e = b.day(&web, l, "2026-11-28", first, &mut notes).unwrap_err();
        assert!(e.contains("as many calls as its free plan allows for now (90 of 440 used this month)"), "{e}");
        // Ten days on it has earned more; the month never gives more than its 440.
        let later = first + 10 * DAY;
        assert!(b.day(&web, l, "2026-11-28", later, &mut notes).is_ok());
        notes["used"] = json!(440);
        assert!(b.day(&web, l, "2026-11-28", first + 30 * DAY, &mut notes).is_err());
        // A new month starts from nothing.
        assert_eq!(Budget::used(&notes, when::at(2027, 1, 1, 0, 0)), 0);
    }
}
