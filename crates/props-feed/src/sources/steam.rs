//! Entertainment: how many people are playing the biggest games on Steam (its public count
//! of players in a game right now, `GetNumberOfCurrentPlayers`, which needs no key). The
//! counts are read once an hour, just after the hour, and kept a week; props are about the
//! count at 18:00 UTC on a day to come, and close a day before.
//!
//! Steam's API terms (read Nov 17 2026) allow 100,000 calls a day and ask for Valve's name
//! beside its data: the props name Steam as their source. Its answer's shape wasn't read from
//! the workspace this was written in (the fetch tool is refused by its robots rules): it's the
//! long-standing one, `{"response":{"player_count":N,"result":1}}`.

use serde_json::{Value, json};

use super::{num, short};
use crate::engine::{Kept, Out, Source};
use crate::http::Fetch;
use crate::math::{commas, phi};
use crate::when::{self, DAY, HOUR, MIN};
use pyr_proto::props_feed::FeedProp;

/// The games watched: ones that have been among Steam's biggest for years.
pub const GAMES: [(u32, &str); 14] = [
    (730, "Counter-Strike 2"),
    (570, "Dota 2"),
    (578080, "PUBG: Battlegrounds"),
    (1172470, "Apex Legends"),
    (252490, "Rust"),
    (1086940, "Baldur's Gate 3"),
    (1245620, "Elden Ring"),
    (413150, "Stardew Valley"),
    (440, "Team Fortress 2"),
    (230410, "Warframe"),
    (553850, "Helldivers 2"),
    (1623730, "Palworld"),
    (2767030, "Marvel Rivals"),
    (105600, "Terraria"),
];

/// The hour of the day (UTC) the props are about.
const AT_HOUR: i64 = 18;
/// A count is read within this long after the hour, or not at all for that hour.
const WITHIN: i64 = 20 * MIN;

fn players(web: &dyn Fetch, appid: u32) -> Result<u64, String> {
    let t = web.get(&format!("https://api.steampowered.com/ISteamUserStats/GetNumberOfCurrentPlayers/v1/?appid={appid}"))?;
    let v = super::json(&t, "Steam")?;
    num(&v["response"]["player_count"]).filter(|n| *n >= 0.0).map(|n| n as u64).ok_or_else(|| "Steam's answer has no player count".to_string())
}

fn name_of(appid: u64) -> &'static str {
    GAMES.iter().find(|g| g.0 as u64 == appid).map_or("", |g| g.1)
}

/// A count rounded to three figures: 1,243,817 as 1,240,000.
fn rounded(n: f64) -> u64 {
    if n < 1000.0 {
        return n.round() as u64;
    }
    let mag = 10f64.powi(n.log10().floor() as i32 - 2);
    ((n / mag).round() * mag) as u64
}

pub struct Steam;

impl Steam {
    /// The count kept for a game at an hour (the hour's number: hours since 1970).
    fn kept(memo: &Value, appid: u64, hour: i64) -> Option<f64> {
        memo["n"][appid.to_string()][hour.to_string()].as_f64()
    }

    /// The latest count kept for a game at the props' hour of day, on or before a day.
    fn latest_at_hour(memo: &Value, appid: u64, before: i64) -> Option<f64> {
        (0..8).find_map(|d| Self::kept(memo, appid, (when::day_start(before) - d * DAY) / HOUR + AT_HOUR).filter(|_| when::day_start(before) - d * DAY + AT_HOUR * HOUR <= before))
    }
}

impl Source for Steam {
    fn name(&self) -> &'static str {
        "steam"
    }

    /// Just after each hour.
    fn every(&self, now: i64, _: &[&Kept]) -> i64 {
        let into = now.rem_euclid(HOUR);
        if into < WITHIN { 5 * MIN } else { HOUR - into + 30 * 1000 }
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], memo: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        let hour = now.div_euclid(HOUR);
        // This hour's counts, if it's just after the hour and they aren't read yet.
        if now.rem_euclid(HOUR) < WITHIN {
            let (mut tried, mut failed) = (0, 0);
            for (appid, name) in GAMES {
                if Self::kept(memo, appid as u64, hour).is_some() {
                    continue;
                }
                tried += 1;
                match players(web, appid) {
                    Ok(n) => memo["n"][appid.to_string()][hour.to_string()] = json!(n),
                    Err(e) => {
                        failed += 1;
                        out.trouble.push(format!("{name}: {e}"));
                    }
                }
            }
            if tried > 0 && failed == tried {
                return Err(out.trouble.first().cloned().unwrap_or_default());
            }
            out.trouble.truncate(4);
        }
        // (A week and a day of them.)
        if let Some(games) = memo["n"].as_object_mut() {
            for hours in games.values_mut().filter_map(|g| g.as_object_mut()) {
                hours.retain(|h, _| h.parse::<i64>().is_ok_and(|h| h > hour - 8 * 24));
            }
        }
        // The games by how many play them at that hour, as last seen.
        let mut ranked: Vec<(u64, f64)> = GAMES.iter().filter_map(|g| Some((g.0 as u64, Self::latest_at_hour(memo, g.0 as u64, now)?))).collect();
        ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
        let base = |closes: i64, at: i64| FeedProp { cat: "entertainment".into(), event: "entertainment:steam".into(), event_name: "Steam's biggest games".into(), closes, starts: at, source: "Steam".into(), featured: 1, ..Default::default() };
        let read = "the number of players Steam reports in the game just after that hour (read within twenty minutes of it)";
        // Over or under, for the three biggest, at 18:00 on the day after tomorrow.
        let at = when::day_start(now) + 2 * DAY + AT_HOUR * HOUR;
        if at - now > 30 * HOUR {
            for (appid, _) in ranked.iter().take(3) {
                let id = format!("ent:steam:{appid}:{}:over", when::yyyymmdd(at));
                if mine.iter().any(|k| k.prop.id == id) || memo["planned"][&id].is_i64() {
                    continue;
                }
                // What it was a week before that day, if that's kept; else the last seen.
                let Some(usual) = Self::kept(memo, *appid, (at - 7 * DAY) / HOUR).or_else(|| Self::latest_at_hour(memo, *appid, now)) else { continue };
                let line = rounded(usual);
                let over = phi((usual - line as f64) / (0.06 * usual.max(1.0)));
                let mut p = base(at - DAY, at);
                p.id = id.clone();
                p.q = format!("{}: more than {} players on Steam at {}:00 UTC on {}?", name_of(*appid), commas(line), AT_HOUR, when::day_and_date(at));
                p.outcomes = vec!["Yes".into(), "No".into()];
                p.prior = vec![over, 1.0 - over];
                p.detail = format!("{} at that hour when last seen", commas(usual as u64));
                p.rules = format!("Yes if {read} is more than {}; No if it's that or fewer. Called off if Steam couldn't be read then.", commas(line));
                p.featured = 2;
                out.open(p, json!({"kind": "over", "a": appid, "line": line, "at": at}));
                memo["planned"][&id] = json!(at);
            }
        }
        // Which of two has more, on Saturday: pairs next to each other and close.
        let saturday = when::week_start(now) + 5 * DAY + AT_HOUR * HOUR;
        if saturday - now > 2 * DAY {
            let mut made = 0;
            for pair in ranked.windows(2).skip(1) {
                let ((a, na), (b, nb)) = (pair[0], pair[1]);
                if made >= 3 || na / nb.max(1.0) > 1.5 {
                    continue;
                }
                made += 1;
                let id = format!("ent:steam:{a}v{b}:{}", when::yyyymmdd(saturday));
                if mine.iter().any(|k| k.prop.id == id) || memo["planned"][&id].is_i64() {
                    continue;
                }
                let first = phi((na / nb.max(1.0)).ln() / 0.12);
                let mut p = base(saturday - DAY, saturday);
                p.id = id.clone();
                p.q = format!("More players on Steam at {}:00 UTC on {}: {} or {}?", AT_HOUR, when::day_and_date(saturday), name_of(a), name_of(b));
                p.outcomes = vec![short(name_of(a), 30), short(name_of(b), 30)];
                p.prior = vec![first, 1.0 - first];
                p.detail = format!("{} and {} at that hour when last seen", commas(na as u64), commas(nb as u64));
                p.rules = format!("Whichever has the higher of {read}. Called off if they're the same, or Steam couldn't be read then.").replace("in the game", "in each game");
                out.open(p, json!({"kind": "duel", "a": a, "b": b, "at": saturday}));
                memo["planned"][&id] = json!(saturday);
            }
        }
        if let Some(p) = memo["planned"].as_object_mut() {
            p.retain(|_, t| t.as_i64().is_some_and(|t| t > now - DAY));
        }
        // What's due: settled from the count kept for its hour.
        for k in mine {
            let (Some(at), Some(a)) = (k.memo["at"].as_i64(), k.memo["a"].as_u64()) else { continue };
            if now < at {
                continue;
            }
            let id = &k.prop.id;
            let count = |app: u64| Self::kept(memo, app, at / HOUR);
            if k.memo["kind"] == "over" {
                match (count(a), k.memo["line"].as_f64()) {
                    (Some(n), Some(line)) => out.settle(id, if n > line { 0 } else { 1 }, format!("{} players", commas(n as u64))),
                    _ if now > at + WITHIN + 10 * MIN => out.call_off(id, "Steam couldn't be read at that hour"),
                    _ => {}
                }
            } else {
                match (count(a), k.memo["b"].as_u64().and_then(count)) {
                    (Some(na), Some(nb)) if (na - nb).abs() < 0.5 => out.call_off(id, format!("{} each", commas(na as u64))),
                    (Some(na), Some(nb)) => out.settle(id, if na > nb { 0 } else { 1 }, format!("{} to {}", commas(na.max(nb) as u64), commas(na.min(nb) as u64))),
                    _ if now > at + WITHIN + 10 * MIN => out.call_off(id, "Steam couldn't be read at that hour"),
                    _ => {}
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Maker, State};
    use crate::http::Canned;

    /// Steam with these games' counts (others: down).
    fn steam(counts: &[(u32, u64)]) -> Canned {
        let answers: Vec<(String, String)> = counts.iter().map(|(app, n)| (format!("appid={app}"), format!(r#"{{"response":{{"player_count":{n},"result":1}}}}"#))).collect();
        // (Longest first: "appid=730" isn't "appid=7".)
        let mut refs: Vec<(&str, &str)> = answers.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
        refs.sort_by_key(|(a, _)| std::cmp::Reverse(a.len()));
        Canned::new(&refs)
    }

    const COUNTS: [(u32, u64); 5] = [(730, 1_243_817), (570, 612_044), (578080, 540_210), (1172470, 181_300), (252490, 96_512)];

    #[test]
    fn counts_are_read_once_an_hour_and_props_come_from_them() {
        assert_eq!((rounded(1_243_817.0), rounded(96_512.0), rounded(812.0)), (1_240_000, 96_500, 812));
        // Tuesday, a few minutes past 18:00: the first counts, and from them the props.
        let now = when::at(2026, 11, 17, 18, 3);
        let web = steam(&COUNTS);
        let mut m = Maker::new(State::default(), vec![Box::new(Steam)]);
        assert!(m.step(now, &web, false));
        assert_eq!(web.asked().len(), 14, "each game once");
        let props = m.feed(now).props;
        let q: Vec<&str> = props.iter().map(|p| p.q.as_str()).collect();
        // The three biggest at 18:00 on Thursday; on Saturday, the pairs that are close
        // (not the biggest against the second: that's no contest).
        assert!(q.contains(&"Counter-Strike 2: more than 1,240,000 players on Steam at 18:00 UTC on Thursday, November 19?"), "{q:?}");
        assert!(q.contains(&"Dota 2: more than 612,000 players on Steam at 18:00 UTC on Thursday, November 19?"));
        assert!(q.contains(&"More players on Steam at 18:00 UTC on Saturday, November 21: Dota 2 or PUBG: Battlegrounds?"), "{q:?}");
        assert_eq!(props.len(), 4, "{q:?}");
        let over = props.iter().find(|p| p.id == "ent:steam:730:20261119:over").unwrap();
        assert_eq!((over.closes, over.starts, over.detail.as_str()), (when::at(2026, 11, 18, 18, 0), when::at(2026, 11, 19, 18, 0), "1,243,817 at that hour when last seen"));
        assert!(over.prior[0] > 0.5 && over.prior[0] < 0.55);
        let duel = props.iter().find(|p| p.id == "ent:steam:570v578080:20261121").unwrap();
        assert_eq!(duel.outcomes, vec!["Dota 2", "PUBG: Battlegrounds"]);
        assert!(duel.prior[0] > 0.8, "{:?}", duel.prior);
        // Five minutes on: this hour's are read already; only the ones that failed are asked again.
        let again = steam(&COUNTS);
        m.step(now + 5 * MIN, &again, false);
        assert_eq!(again.asked().len(), 9);
        // Half past: nothing is read until the next hour.
        let quiet = Canned::default();
        m.step(now + 30 * MIN, &quiet, true);
        assert!(quiet.asked().is_empty());
        assert_eq!(m.state.sources["steam"].next_run, when::at(2026, 11, 17, 19, 0) + 30 * 1000);
    }

    #[test]
    fn a_prop_settles_from_the_count_just_after_its_hour() {
        let now = when::at(2026, 11, 17, 18, 3);
        let mut m = Maker::new(State::default(), vec![Box::new(Steam)]);
        m.step(now, &steam(&COUNTS), false);
        // Thursday 18:02: Counter-Strike has 1,239,000 (not more than the line); Dota 2 is
        // down just then.
        let thu = when::at(2026, 11, 19, 18, 2);
        assert!(m.step(thu, &steam(&[(730, 1_239_000), (578080, 500_000)]), false));
        let props = m.feed(thu).props;
        let r = |id: &str| props.iter().find(|p| p.id == id).unwrap().result.clone();
        assert_eq!(r("ent:steam:730:20261119:over").map(|r| (r.winner, r.note)), Some((Some(1), "1,239,000 players".to_string())));
        assert!(r("ent:steam:570:20261119:over").is_none(), "asked again while it's still just after the hour");
        // Still down at 18:35: called off, not guessed from a later count.
        assert!(m.step(thu + 33 * MIN, &Canned::default(), true));
        let props = m.feed(thu + 33 * MIN).props;
        assert_eq!(props.iter().find(|p| p.id == "ent:steam:570:20261119:over").unwrap().result.clone().map(|r| r.winner), Some(None));
        // Saturday: PUBG ahead of Dota 2.
        let sat = when::at(2026, 11, 21, 18, 1);
        m.step(sat, &steam(&[(570, 601_000), (578080, 644_500)]), false);
        let props = m.feed(sat).props;
        let r = props.iter().find(|p| p.id == "ent:steam:570v578080:20261121").unwrap().result.clone().unwrap();
        assert_eq!((r.winner, r.note.as_str()), (Some(1), "644,500 to 601,000"));
    }
}
