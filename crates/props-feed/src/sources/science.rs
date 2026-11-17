//! Science: earthquakes (USGS), the Sun (NOAA's Space Weather Prediction Center), rocket
//! launches (Launch Library 2) and asteroids passing inside the Moon's orbit (NASA JPL).
//!
//! Nearly all of it is about next week: the props open during this week and close as theirs
//! starts, so nobody bets on a week once any of it has happened. Their starting odds come from
//! how often such things have happened lately, counted from the same sources.

use serde_json::{Value, json};

use super::{count_range_of, count_ranges, next_week, num, short, week_of};
use crate::engine::{Kept, Out, Source};
use crate::http::Fetch;
use crate::math::poisson;
use crate::when::{self, DAY, HOUR, MIN};
use pyr_proto::props_feed::FeedProp;

fn base(event: &str, event_name: String, closes: i64, source: &str) -> FeedProp {
    FeedProp { cat: "science".into(), event: format!("science:{event}"), event_name, closes, starts: closes, source: source.into(), featured: 1, ..Default::default() }
}

// ------------------------------------------------------------------ earthquakes

/// Earthquakes next week, counted from the USGS catalog a day after it ends.
pub struct Quakes;

const USGS: &str = "https://earthquake.usgs.gov/fdsnws/event/1";

fn quake_count(web: &dyn Fetch, from: i64, to: i64, mag: f64) -> Result<u32, String> {
    let t = web.get(&format!("{USGS}/count?format=geojson&starttime={}&endtime={}&minmagnitude={mag}", when::iso(from).trim_end_matches('Z'), when::iso(to).trim_end_matches('Z')))?;
    num(&super::json(&t, "USGS")?["count"]).map(|n| n as u32).ok_or_else(|| "USGS's count has no count".to_string())
}

/// The strongest earthquake of a span: "M6.6, 148 km WSW of Tual, Indonesia".
fn strongest(web: &dyn Fetch, from: i64, to: i64) -> Option<String> {
    let url = format!("{USGS}/query?format=geojson&starttime={}&endtime={}&minmagnitude=5.5&orderby=magnitude&limit=1", when::iso(from).trim_end_matches('Z'), when::iso(to).trim_end_matches('Z'));
    let v = super::json(&web.get(&url).ok()?, "USGS").ok()?;
    let p = &v["features"][0]["properties"];
    Some(format!("M{:.1}, {}", num(&p["mag"])?, p["place"].as_str()?))
}

impl Source for Quakes {
    fn name(&self) -> &'static str {
        "quakes"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        HOUR
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], _: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        let week = next_week(now);
        let tag = when::yyyymmdd(week);
        let ids = [format!("science:quake:{tag}:big"), format!("science:quake:{tag}:count")];
        if !mine.iter().any(|k| k.prop.id == ids[0]) && week - now > HOUR {
            // How often lately: the last fifty-two weeks.
            let year = 52.0;
            let big = quake_count(web, now - 364 * DAY, now, 6.5)? as f64 / year;
            let six = quake_count(web, now - 364 * DAY, now, 6.0)? as f64 / year;
            let name = format!("Earthquakes, {}", week_of(week));
            let rules = |what: &str| format!("{what}, anywhere in the world, from 00:00 UTC on {} to 00:00 UTC the Monday after, as the USGS earthquake catalog (earthquake.usgs.gov) lists them a day after that week ends.", when::day_and_date(week));
            let mut p = base(&format!("quakes:{tag}"), name.clone(), week, "USGS");
            p.id = ids[0].clone();
            p.q = format!("A magnitude 6.5 or stronger earthquake in {}?", week_of(week));
            p.outcomes = vec!["Yes".into(), "No".into()];
            let yes = 1.0 - poisson(0, big.max(0.05));
            p.prior = vec![yes, 1.0 - yes];
            p.detail = format!("{:.0} in the last year", big * year);
            p.rules = rules("Yes if there's at least one earthquake of magnitude 6.5 or more");
            out.open(p, json!({"kind": "big", "week": week}));
            let (starts, names, chances) = count_ranges(six.max(0.3));
            let mut p = base(&format!("quakes:{tag}"), name, week, "USGS");
            p.id = ids[1].clone();
            p.q = format!("How many magnitude 6+ earthquakes in {}?", week_of(week));
            p.outcomes = names;
            p.prior = chances;
            p.detail = format!("{:.0} in the last year: about {:.1} a week", six * year, six);
            p.rules = rules("Earthquakes of magnitude 6.0 or more");
            out.open(p, json!({"kind": "count", "week": week, "starts": starts}));
        }
        for k in mine {
            let w = k.memo["week"].as_i64().unwrap_or(0);
            if w == 0 || now < w + 8 * DAY {
                continue;
            }
            let mag = if k.memo["kind"] == "big" { 6.5 } else { 6.0 };
            match quake_count(web, w, w + 7 * DAY, mag) {
                Ok(n) => {
                    let top = strongest(web, w, w + 7 * DAY).map(|s| format!(": the strongest {s}")).unwrap_or_default();
                    if k.memo["kind"] == "big" {
                        out.settle(&k.prop.id, if n > 0 { 0 } else { 1 }, format!("{n} of magnitude 6.5 or more{top}"));
                    } else {
                        let starts: Vec<u32> = k.memo["starts"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as u32).collect()).unwrap_or_default();
                        if starts.len() + 1 == k.prop.outcomes.len() { out.settle(&k.prop.id, count_range_of(&starts, n), format!("{n} of magnitude 6 or more{top}")) } else { out.call_off(&k.prop.id, "its ranges were lost") }
                    }
                }
                Err(e) => out.trouble.push(e),
            }
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ the Sun

/// Solar flares and geomagnetic storms next week, from what NOAA's space weather center
/// publishes. Its files hold a week each, so they're read every hour and what they say is
/// kept: a week is settled only if it was watched from before it started to after it ended.
pub struct SpaceWeather;

const SWPC: &str = "https://services.swpc.noaa.gov";

/// A flare's class, to compare: its letter's place (A 0, B 1, C 2, M 3, X 4) and its number
/// ("M1.1": 3 and 1.1).
fn flare_rank(class: &str) -> Option<(u8, f64)> {
    let class = class.trim();
    let letter = "ABCMX".find(class.chars().next()?.to_ascii_uppercase())? as u8;
    let n: f64 = class[1..].trim().parse().ok().filter(|n: &f64| n.is_finite() && *n >= 0.0)?;
    Some((letter, n))
}

fn stronger(a: &(u8, f64), b: &(u8, f64)) -> std::cmp::Ordering {
    a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
}

/// Which of "C or weaker", "M1 to M4.9", "M5 to M9.9", "X" a week's strongest flare is.
fn flare_band(rank: (u8, f64)) -> usize {
    match rank {
        (4.., _) => 3,
        (3, n) if n >= 5.0 => 2,
        (3, _) => 1,
        _ => 0,
    }
}

impl SpaceWeather {
    /// Reads the two files into the notes: each M or X flare by its peak time, each
    /// three-hour Kp by its time.
    fn watch(now: i64, web: &dyn Fetch, memo: &mut Value) -> Result<(), String> {
        let flares = super::json(&web.get(&format!("{SWPC}/json/goes/primary/xray-flares-7-day.json"))?, "NOAA")?;
        let rows = flares.as_array().ok_or("NOAA's flare list isn't a list")?;
        for f in rows {
            let (Some(at), Some(class)) = (f["max_time"].as_str().and_then(when::parse_iso), f["max_class"].as_str()) else { continue };
            if flare_rank(class).is_some_and(|r| r.0 >= 3) {
                memo["flares"][at.to_string()] = json!(class);
            }
        }
        let kp = super::json(&web.get(&format!("{SWPC}/products/noaa-planetary-k-index.json"))?, "NOAA")?;
        for row in kp.as_array().ok_or("NOAA's Kp list isn't a list")? {
            // (Rows as objects, or as lists under a row of headings, as it used to be.)
            let (at, value) = if row.is_object() { (row["time_tag"].as_str(), num(&row["Kp"])) } else { (row[0].as_str(), num(&row[1])) };
            if let (Some(at), Some(v)) = (at.and_then(when::parse_iso), value) {
                memo["kp"][at.to_string()] = json!(v);
            }
        }
        // Watched since: the first read covered the week before it.
        if !memo["since"].is_i64() {
            memo["since"] = json!(now - 7 * DAY + 3 * HOUR);
        }
        // (A gap of more than six days between reads would have missed something.)
        let last = memo["read"].as_i64().unwrap_or(now);
        if now - last > 6 * DAY {
            memo["since"] = json!(now - 7 * DAY + 3 * HOUR);
        }
        memo["read"] = json!(now);
        for key in ["flares", "kp"] {
            if let Some(m) = memo[key].as_object_mut() {
                m.retain(|k, _| k.parse::<i64>().is_ok_and(|t| t > now - 30 * DAY));
            }
        }
        Ok(())
    }

    /// The strongest flare (its rank and class) and the highest Kp in a span, from the notes.
    fn in_span(memo: &Value, from: i64, to: i64) -> (Option<((u8, f64), String)>, f64) {
        let within = |k: &String| k.parse::<i64>().is_ok_and(|t| t >= from && t < to);
        let flare = memo["flares"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(k, _)| within(k))
            .filter_map(|(_, c)| c.as_str().and_then(|c| Some((flare_rank(c)?, c.to_string()))))
            .max_by(|a, b| stronger(&a.0, &b.0));
        let kp = memo["kp"].as_object().into_iter().flatten().filter(|(k, _)| within(k)).filter_map(|(_, v)| v.as_f64()).fold(0.0, f64::max);
        (flare, kp)
    }
}

impl Source for SpaceWeather {
    fn name(&self) -> &'static str {
        "sun"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        HOUR
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], memo: &mut Value) -> Result<Out, String> {
        Self::watch(now, web, memo)?;
        let mut out = Out::default();
        let week = next_week(now);
        let tag = when::yyyymmdd(week);
        let ids = [format!("science:sun:{tag}:flare"), format!("science:sun:{tag}:storm")];
        if !mine.iter().any(|k| k.prop.id == ids[0]) && week - now > HOUR {
            // The last week as a guide to the next (the Sun's moods last weeks at a time),
            // pulled toward what's usual these years.
            let (top, kp) = Self::in_span(memo, now - 7 * DAY, now);
            let band = top.as_ref().map_or(0, |t| flare_band(t.0));
            let usual = [0.10, 0.48, 0.20, 0.22];
            let prior: Vec<f64> = (0..4).map(|i| 0.6 * usual[i] + 0.4 * if i == band { 1.0 } else { 0.0 }).collect();
            let name = format!("The Sun, {}", week_of(week));
            let span = format!("from 00:00 UTC on {} to 00:00 UTC the Monday after", when::day_and_date(week));
            let mut p = base(&format!("sun:{tag}"), name.clone(), week, "NOAA Space Weather Prediction Center");
            p.id = ids[0].clone();
            p.q = format!("The strongest solar flare in {}?", week_of(week));
            p.outcomes = vec!["C class or weaker".into(), "M1 to M4.9".into(), "M5 to M9.9".into(), "X class".into()];
            p.prior = prior;
            p.detail = top.map_or("No M or X flare in the last week".to_string(), |t| format!("The last week's strongest: {}", t.1));
            p.rules = format!("The highest class among the flares that peaked {span}, as NOAA's GOES X-ray flare list (services.swpc.noaa.gov) has them. Called off if the list couldn't be read all that week.");
            out.open(p, json!({"kind": "flare", "week": week}));
            let mut p = base(&format!("sun:{tag}"), name, week, "NOAA Space Weather Prediction Center");
            p.id = ids[1].clone();
            p.q = format!("A geomagnetic storm of Kp 6 or more in {}?", week_of(week));
            p.outcomes = vec!["Yes".into(), "No".into()];
            let yes = if kp >= 6.0 { 0.32 } else if kp >= 5.0 { 0.24 } else { 0.15 };
            p.prior = vec![yes, 1.0 - yes];
            p.detail = format!("The last week's highest Kp: {kp:.2}");
            p.rules = format!("Yes if NOAA's planetary K index (the three-hourly Kp at services.swpc.noaa.gov) reaches 6.00 or more {span}: a G2 storm or stronger, the kind that brings the northern lights south. Called off if the index couldn't be read all that week.");
            out.open(p, json!({"kind": "storm", "week": week}));
        }
        let since = memo["since"].as_i64().unwrap_or(now);
        for k in mine {
            let w = k.memo["week"].as_i64().unwrap_or(0);
            // (Three hours after it ends: the last Kp of the week is in.)
            if w == 0 || now < w + 7 * DAY + 3 * HOUR {
                continue;
            }
            if since > w {
                out.call_off(&k.prop.id, "the week wasn't watched from its start");
                continue;
            }
            let (top, kp) = Self::in_span(memo, w, w + 7 * DAY);
            if k.memo["kind"] == "flare" {
                let (band, note) = top.map_or((0, "no M or X flare".to_string()), |t| (flare_band(t.0), format!("the strongest was {}", t.1)));
                out.settle(&k.prop.id, band, note);
            } else {
                out.settle(&k.prop.id, if kp >= 6.0 { 0 } else { 1 }, format!("the highest Kp was {kp:.2}"));
            }
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ launches

/// Rocket launches (Launch Library 2, thespacedevs.com: fifteen requests an hour without a
/// key, so this asks twice an hour at most and a few things each time): whether each launch
/// that's nearly due lifts off within a day of its time, and how many there are next week.
pub struct Launches;

const LL2: &str = "https://ll.thespacedevs.com/2.3.0";
/// The most launches with a prop open at once.
const MOST_LAUNCHES: usize = 5;

/// A launch as the list gives it.
#[derive(Clone, Debug, PartialEq)]
struct Launch {
    id: String,
    name: String,
    net: i64,
    /// 1 Go, 2 TBD, 3 Success, 4 Failure, 5 Hold, 6 In flight, 7 Partial failure, 8 TBC.
    status: u64,
    /// How exactly its time is known: 0 the second, 1 the minute, 2 the hour, then vaguer.
    precision: u64,
}

fn launch(l: &Value) -> Option<Launch> {
    Some(Launch { id: l["id"].as_str()?.to_string(), name: l["name"].as_str()?.to_string(), net: when::parse_iso(l["net"].as_str()?)?, status: l["status"]["id"].as_u64()?, precision: l["net_precision"]["id"].as_u64().unwrap_or(99) })
}

/// A list of launches: how many there are in all, and the ones on this page.
fn launches(text: &str) -> Result<(u64, Vec<Launch>), String> {
    let v = super::json(text, "Launch Library")?;
    let list = v["results"].as_array().ok_or("Launch Library's answer has no results")?;
    Ok((v["count"].as_u64().unwrap_or(0), list.iter().filter_map(launch).collect()))
}

/// Launches that lifted off in a span (whether or not they reached orbit).
fn launch_count(web: &dyn Fetch, from: i64, to: i64) -> Result<u64, String> {
    let t = web.get(&format!("{LL2}/launches/previous/?format=json&mode=list&limit=1&net__gte={}&net__lt={}", when::iso(from), when::iso(to)))?;
    Ok(launches(&t)?.0)
}

impl Source for Launches {
    fn name(&self) -> &'static str {
        "launches"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        30 * MIN
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], _: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        let (_, list) = launches(&web.get(&format!("{LL2}/launches/upcoming/?format=json&mode=list&limit=30"))?)?;
        // Each launch that's a few days off, confirmed, with a time: off the pad within a day
        // of that time?
        let open_now = mine.iter().filter(|k| k.memo["kind"] == "launch").count();
        let mut soon: Vec<&Launch> = list.iter().filter(|l| matches!(l.status, 1 | 8) && l.precision <= 2 && l.net > now + 9 * HOUR && l.net < now + 5 * DAY).collect();
        soon.sort_by_key(|l| l.net);
        for l in soon.into_iter().take(MOST_LAUNCHES.saturating_sub(open_now)) {
            let tag: String = l.id.chars().filter(|c| c.is_ascii_hexdigit()).take(10).collect::<String>().to_lowercase();
            if tag.len() < 8 || mine.iter().any(|k| k.memo["launch"] == l.id.as_str()) {
                continue;
            }
            let by = l.net + DAY;
            // "Falcon 9 Block 5 | USSF-317": the rocket, and what it carries.
            let (rocket, load) = l.name.split_once('|').map_or((l.name.trim(), ""), |(r, p)| (r.trim(), p.trim()));
            let what = if load.is_empty() || load.eq_ignore_ascii_case("Unknown Payload") { rocket.to_string() } else { format!("{rocket} ({load})") };
            let mut p = base(&format!("launch:{tag}"), short(&what, 80), l.net - 6 * HOUR, "Launch Library 2");
            p.id = format!("science:launch:{tag}");
            p.starts = l.net;
            p.q = format!("{}: liftoff by {} on {}?", short(&what, 70), when::hhmm_utc(by), when::day_and_date(by));
            p.outcomes = vec!["Yes".into(), "No".into()];
            let yes = if l.status == 1 { 0.75 } else { 0.55 };
            p.prior = vec![yes, 1.0 - yes];
            p.detail = format!("Due at {} on {}", when::hhmm_utc(l.net), when::day_and_date(l.net));
            p.rules = format!("Yes if Launch Library 2 (thespacedevs.com) shows it lifted off by {} on {}, a day after the time it was due when this opened, whether or not it reached orbit. No if it hasn't by then.", when::hhmm_utc(by), when::day_and_date(by));
            p.featured = 2;
            out.open(p, json!({"kind": "launch", "launch": l.id, "by": by}));
        }
        // How many next week (from the last four weeks' count).
        let week = next_week(now);
        let tag = when::yyyymmdd(week);
        let id = format!("science:launches:{tag}:count");
        if !mine.iter().any(|k| k.prop.id == id) && week - now > HOUR {
            match launch_count(web, now - 28 * DAY, now) {
                Ok(n) => {
                    let mean = (n as f64 / 4.0).max(1.0);
                    let (starts, names, chances) = count_ranges(mean);
                    let mut p = base(&format!("launches:{tag}"), format!("Launches, {}", week_of(week)), week, "Launch Library 2");
                    p.id = id;
                    p.q = format!("How many orbital launch attempts in {}?", week_of(week));
                    p.outcomes = names;
                    p.prior = chances;
                    p.detail = format!("{n} in the last four weeks");
                    p.rules = format!("Launches that lifted off, anywhere in the world, from 00:00 UTC on {} to 00:00 UTC the Monday after, as Launch Library 2 (thespacedevs.com) lists them twelve hours after that week ends. A failed launch counts; a scrubbed one doesn't.", when::day_and_date(week));
                    out.open(p, json!({"kind": "count", "week": week, "starts": starts}));
                }
                Err(e) => out.trouble.push(e),
            }
        }
        for k in mine {
            let id = &k.prop.id;
            if k.memo["kind"] == "launch" {
                let by = k.memo["by"].as_i64().unwrap_or(0);
                // As the list has it (a launch stays in "upcoming" a day after it flies); one
                // that's gone from the list is asked for by itself.
                let found = match k.memo["launch"].as_str().and_then(|lid| list.iter().find(|l| l.id == lid)) {
                    Some(l) => Some(l.clone()),
                    None if now > k.prop.closes => {
                        let lid = k.memo["launch"].as_str().unwrap_or("");
                        match web.get(&format!("{LL2}/launches/{lid}/?format=json&mode=list")).and_then(|t| super::json(&t, "Launch Library")) {
                            Ok(one) => launch(&one),
                            Err(e) => {
                                out.trouble.push(e);
                                continue;
                            }
                        }
                    }
                    None => None,
                };
                match found {
                    Some(l) if matches!(l.status, 3 | 4 | 6 | 7) && l.net <= by => out.settle(id, 0, format!("lifted off at {} on {}", when::hhmm_utc(l.net), when::day_and_date(l.net))),
                    Some(l) if now > by + 30 * MIN => out.settle(id, 1, if l.net > by { format!("now due {}", when::day_and_date(l.net)) } else { "it hadn't lifted off by then".to_string() }),
                    None if now > by + 2 * DAY => out.call_off(id, "the launch is no longer listed"),
                    _ => {}
                }
            } else if let Some(w) = k.memo["week"].as_i64().filter(|w| now >= w + 7 * DAY + 12 * HOUR) {
                match launch_count(web, w, w + 7 * DAY) {
                    Ok(n) => {
                        let starts: Vec<u32> = k.memo["starts"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as u32).collect()).unwrap_or_default();
                        if starts.len() + 1 == k.prop.outcomes.len() { out.settle(id, count_range_of(&starts, n as u32), format!("{n} launches")) } else { out.call_off(id, "its ranges were lost") }
                    }
                    Err(e) => out.trouble.push(e),
                }
            }
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ asteroids

/// Asteroids passing closer than the Moon next week, as NASA JPL's close-approach table
/// lists them two days after the week ends (small ones are often found only as they pass).
pub struct Asteroids;

fn passes(web: &dyn Fetch, from: i64, to: i64) -> Result<u32, String> {
    let day = |t: i64| when::iso(t).trim_end_matches('Z').to_string();
    let t = web.get(&format!("https://ssd-api.jpl.nasa.gov/cad.api?dist-max=1LD&date-min={}&date-max={}", day(from), day(to)))?;
    let v = super::json(&t, "JPL")?;
    num(&v["count"]).map(|n| n as u32).ok_or_else(|| "JPL's table has no count".to_string())
}

impl Source for Asteroids {
    fn name(&self) -> &'static str {
        "asteroids"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        6 * HOUR
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], _: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        let week = next_week(now);
        let tag = when::yyyymmdd(week);
        let id = format!("science:asteroids:{tag}:count");
        if !mine.iter().any(|k| k.prop.id == id) && week - now > HOUR {
            // (The eight weeks that ended two days ago: the latest passes aren't all listed yet.)
            let n = passes(web, now - 58 * DAY, now - 2 * DAY)?;
            let (starts, names, chances) = count_ranges((n as f64 / 8.0).max(1.0));
            let mut p = base(&format!("asteroids:{tag}"), format!("Asteroids, {}", week_of(week)), week, "NASA JPL");
            p.id = id;
            p.q = format!("How many asteroids pass closer than the Moon in {}?", week_of(week));
            p.outcomes = names;
            p.prior = chances;
            p.detail = format!("{n} in the last eight weeks");
            p.rules = format!("Asteroids whose closest approach to Earth is within one lunar distance, from 00:00 UTC on {} to 00:00 UTC the Monday after, as NASA JPL's close-approach table (ssd-api.jpl.nasa.gov) lists them two days after that week ends.", when::day_and_date(week));
            out.open(p, json!({"kind": "count", "week": week, "starts": starts}));
        }
        for k in mine {
            let Some(w) = k.memo["week"].as_i64().filter(|w| now >= w + 9 * DAY) else { continue };
            match passes(web, w, w + 7 * DAY) {
                Ok(n) => {
                    let starts: Vec<u32> = k.memo["starts"].as_array().map(|a| a.iter().filter_map(|x| x.as_u64()).map(|x| x as u32).collect()).unwrap_or_default();
                    if starts.len() + 1 == k.prop.outcomes.len() { out.settle(&k.prop.id, count_range_of(&starts, n), format!("{n} passed inside the Moon's orbit")) } else { out.call_off(&k.prop.id, "its ranges were lost") }
                }
                Err(e) => out.trouble.push(e),
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

    const FLARES: &str = r#"[{"time_tag":"2026-11-08T18:03:00Z","begin_time":"2026-11-08T18:03:00Z","begin_class":"B9.2","max_time":"2026-11-08T18:13:00Z","max_class":"C1.8","max_xrlong":1.8783563291435712e-06,"max_ratio":0.19389108113062034,"max_ratio_time":"2026-11-08T18:10:49Z","current_int_xrlong":0.0011047649895772338,"end_time":"2026-11-08T18:15:00Z","end_class":"C1.2","satellite":18},{"time_tag":"2026-11-08T18:15:00Z","begin_time":"2026-11-08T18:15:00Z","begin_class":"C1.2","max_time":"2026-11-08T18:27:00Z","max_class":"M1.1","max_xrlong":1.1699958122335374e-05,"max_ratio":0.23338692788214793,"max_ratio_time":"2026-11-08T18:16:48Z","current_int_xrlong":0.007782366126775742,"end_time":"2026-11-08T18:35:00Z","end_class":"C6.3","satellite":18},{"time_tag":"2026-11-15T17:14:00Z","begin_time":"2026-11-15T17:14:00Z","begin_class":"B5.5","max_time":"2026-11-15T17:20:00Z","max_class":"B8.5","max_xrlong":8.56913629831979e-07,"max_ratio":null,"max_ratio_time":null,"current_int_xrlong":0.0004462093929760158,"end_time":"2026-11-15T17:23:00Z","end_class":"B6.8","satellite":18}]"#;
    const KP: &str = r#"[{"time_tag":"2026-11-09T00:00:00","Kp":3.00,"a_running":15,"station_count":8},{"time_tag":"2026-11-09T03:00:00","Kp":3.33,"a_running":18,"station_count":8},{"time_tag":"2026-11-16T09:00:00","Kp":0.67,"a_running":3,"station_count":8}]"#;
    const UPCOMING: &str = r#"{"count":472,"next":"https://ll.thespacedevs.com/2.3.0/launches/upcoming/?format=json&limit=3&mode=list&offset=3","previous":null,"results":[{"id":"e0a11eb1-0bda-4872-b36b-140247fd5c60","url":"https://ll.thespacedevs.com/2.3.0/launches/e0a11eb1-0bda-4872-b36b-140247fd5c60/?format=json","name":"Long March 2D | Unknown Payload","response_mode":"list","slug":"long-march-2d-unknown-payload","launch_designator":null,"status":{"id":1,"name":"Go for Launch","abbrev":"Go","description":"Current T-0 confirmed by official or reliable sources."},"last_updated":"2026-11-12T19:18:17Z","net":"2026-11-18T22:50:00Z","net_precision":{"id":2,"name":"Hour","abbrev":"HR","description":"The T-0 is accurate to the hour."},"window_end":"2026-11-18T23:05:00Z","window_start":"2026-11-18T22:43:00Z","image":null,"infographic":null},{"id":"76617033-4dc0-4448-b50c-e5b0d5a18539","name":"Falcon 9 Block 5 | USSF-317","status":{"id":1,"name":"Go for Launch","abbrev":"Go"},"last_updated":"2026-11-17T00:50:52Z","net":"2026-11-19T03:00:00Z","net_precision":{"id":1,"name":"Minute","abbrev":"MIN"},"window_end":"2026-11-19T07:00:00Z","window_start":"2026-11-19T03:00:00Z"},{"id":"a2975167-7c6f-4403-99e1-b40df17c8140","name":"Long March 8A | SatNet LEO Group TBD?","status":{"id":8,"name":"To Be Confirmed","abbrev":"TBC"},"last_updated":"2026-10-30T13:59:24Z","net":"2026-11-20T11:00:00Z","net_precision":{"id":2,"name":"Hour","abbrev":"HR"}},{"id":"b1b1b1b1-0000-4000-8000-000000000001","name":"Starship | Flight 14","status":{"id":2,"name":"To Be Determined","abbrev":"TBD"},"net":"2026-11-21T00:00:00Z","net_precision":{"id":5,"name":"Day","abbrev":"DAY"}}]}"#;
    const JPL: &str = r#"{"signature":{"version":"1.5","source":"NASA/JPL SBDB Close Approach Data API"},"count":17,"fields":["des","orbit_id","jd","cd","dist","dist_min","dist_max","v_rel","v_inf","t_sigma_f","h"],"data":[["2026 VE3","11","2461345.541777194","2026-Nov-01 01:00","0.00137048221800833","0.00136978290892877","0.00137118144796956","2.95745682796744","2.20412537729622","< 00:01","28.485"]]}"#;

    fn feed_of(m: &Maker, now: i64) -> Vec<FeedProp> {
        m.feed(now).props
    }

    #[test]
    fn next_week_s_earthquakes_open_at_the_year_s_odds_and_settle_from_the_catalog() {
        let now = when::at(2026, 11, 17, 12, 0);
        let week = when::at(2026, 11, 23, 0, 0);
        // 41 of 6.5+ and 135 of 6+ in the last year.
        let web = Canned::new(&[("minmagnitude=6.5", r#"{"count":41,"maxAllowed":20000}"#), ("minmagnitude=6", r#"{"count":135,"maxAllowed":20000}"#)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Quakes)]);
        assert!(m.step(now, &web, false));
        assert!(web.asked()[0].contains("count?format=geojson&starttime=2025-11-18T12:00:00&endtime=2026-11-17T12:00:00&minmagnitude=6.5"), "{:?}", web.asked());
        let props = feed_of(&m, now);
        assert_eq!(props.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["science:quake:20261123:big", "science:quake:20261123:count"]);
        let big = &props[0];
        assert_eq!((big.q.as_str(), big.closes, big.event_name.as_str()), ("A magnitude 6.5 or stronger earthquake in the week of November 23?", week, "Earthquakes, the week of November 23"));
        assert!((big.prior[0] - 0.546).abs() < 0.002, "{:?}", big.prior);
        let count = &props[1];
        assert_eq!(count.outcomes, vec!["1 or fewer", "2 or 3", "4 or 5", "6 or more"]);
        assert_eq!(count.detail, "135 in the last year: about 2.6 a week");
        // An hour on: nothing new to ask.
        let quiet = Canned::default();
        assert!(!m.step(now + HOUR, &quiet, false));
        assert!(quiet.asked().is_empty());
        // A day after that week ends: five of 6+, none of 6.5+.
        let then = week + 8 * DAY + HOUR;
        let web = Canned::new(&[
            ("starttime=2026-11-23T00:00:00&endtime=2026-11-30T00:00:00&minmagnitude=6.5", r#"{"count":0,"maxAllowed":20000}"#),
            ("count?format=geojson&starttime=2026-11-23T00:00:00&endtime=2026-11-30T00:00:00&minmagnitude=6", r#"{"count":5,"maxAllowed":20000}"#),
            ("orderby=magnitude", r#"{"type":"FeatureCollection","metadata":{"count":1},"features":[{"type":"Feature","properties":{"mag":6.4,"place":"30 km WSW of Lata, Solomon Islands","time":1794195299466,"status":"reviewed","type":"earthquake"},"id":"us7000tx16"}]}"#),
            ("count?", r#"{"count":40,"maxAllowed":20000}"#),
        ]);
        assert!(m.step(then, &web, false));
        let props = feed_of(&m, then);
        let r = |id: &str| props.iter().find(|p| p.id == id).unwrap().result.clone().unwrap();
        assert_eq!((r("science:quake:20261123:big").winner, r("science:quake:20261123:big").note.as_str()), (Some(1), "0 of magnitude 6.5 or more: the strongest M6.4, 30 km WSW of Lata, Solomon Islands"));
        assert_eq!(r("science:quake:20261123:count").winner, Some(2));
        // And the week after's are open by then.
        assert!(props.iter().any(|p| p.id == "science:quake:20261207:big" && p.result.is_none()));
    }

    #[test]
    fn the_sun_is_watched_all_week_and_settled_from_what_was_seen() {
        assert_eq!((flare_rank("C1.8"), flare_rank("M1.1"), flare_rank("x12"), flare_rank("?")), (Some((2, 1.8)), Some((3, 1.1)), Some((4, 12.0)), None));
        let band = |c: &str| flare_band(flare_rank(c).unwrap());
        assert_eq!((band("C9.9"), band("M1.0"), band("M4.9"), band("M5.0"), band("X1.0")), (0, 1, 1, 2, 3));
        assert_eq!(stronger(&(3, 9.9), &(4, 1.0)), std::cmp::Ordering::Less);
        let now = when::at(2026, 11, 16, 12, 0);
        let week = when::at(2026, 11, 23, 0, 0);
        // (The M1.1 of the sample, four days ago.)
        let flares = FLARES.replace("2026-11-08T18:27:00Z", "2026-11-12T18:27:00Z");
        let web = Canned::new(&[("xray-flares-7-day", flares.as_str()), ("planetary-k-index", KP)]);
        let mut m = Maker::new(State::default(), vec![Box::new(SpaceWeather)]);
        assert!(m.step(now, &web, false));
        let props = feed_of(&m, now);
        assert_eq!(props.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["science:sun:20261123:flare", "science:sun:20261123:storm"]);
        // The last week's strongest was an M1.1: that band is likelier than usual.
        assert_eq!(props[0].detail, "The last week's strongest: M1.1");
        assert!((props[0].prior[1] - 0.688).abs() < 0.002, "{:?}", props[0].prior);
        assert_eq!(props[1].detail, "The last week's highest Kp: 0.67");
        // Read every hour through the week: an X1.4 on Wednesday, Kp 6.33 on Friday (by the
        // old shape of the Kp file, rows under headings).
        let x = FLARES.replace("2026-11-15T17:20:00Z\",\"max_class\":\"B8.5\"", "2026-11-25T09:10:00Z\",\"max_class\":\"X1.4\"");
        let kp = r#"[["time_tag","Kp","a_running","station_count"],["2026-11-27 15:00:00.000","6.33","80","8"]]"#;
        for day in 1..=13 {
            let at = now + day * DAY;
            let (f, k) = if at > when::at(2026, 11, 25, 12, 0) { (x.as_str(), kp) } else { (FLARES, KP) };
            m.step(at, &Canned::new(&[("xray-flares-7-day", f), ("planetary-k-index", k)]), false);
        }
        // Three hours after the week ends it's settled.
        let then = week + 7 * DAY + 4 * HOUR;
        assert!(m.step(then, &web, false));
        let props = feed_of(&m, then);
        let r = |id: &str| props.iter().find(|p| p.id == id).unwrap().result.clone().unwrap();
        assert_eq!((r("science:sun:20261123:flare").winner, r("science:sun:20261123:flare").note.as_str()), (Some(3), "the strongest was X1.4"));
        assert_eq!((r("science:sun:20261123:storm").winner, r("science:sun:20261123:storm").note.as_str()), (Some(0), "the highest Kp was 6.33"));
        // A week that wasn't watched from its start is called off, not guessed.
        let mut late = Maker::new(State::default(), vec![Box::new(SpaceWeather)]);
        late.step(now, &web, false);
        late.state.sources.get_mut("sun").unwrap().memo["since"] = json!(week + DAY);
        late.step(then, &web, true);
        assert!(feed_of(&late, then).iter().filter(|p| p.id.contains("20261123")).all(|p| p.result.as_ref().is_some_and(|r| r.winner.is_none())));
    }

    #[test]
    fn launches_nearly_due_get_a_prop_each_and_next_week_s_are_counted() {
        let now = when::at(2026, 11, 17, 12, 0);
        let web = Canned::new(&[("launches/upcoming", UPCOMING), ("launches/previous", r#"{"count":25,"next":null,"previous":null,"results":[]}"#)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Launches)]);
        assert!(m.step(now, &web, false));
        assert_eq!(web.asked().len(), 2, "two requests a run: fifteen an hour are allowed");
        let props = feed_of(&m, now);
        let ids: Vec<&str> = props.iter().map(|p| p.id.as_str()).collect();
        // The three with a time and a go (or nearly); not the one that's only a day so far.
        assert_eq!(ids, vec!["science:launch:e0a11eb10b", "science:launch:766170334d", "science:launch:a29751677c", "science:launches:20261123:count"]);
        let f9 = &props[1];
        assert_eq!(f9.q, "Falcon 9 Block 5 (USSF-317): liftoff by 03:00 UTC on Friday, November 20?");
        assert_eq!((f9.closes, f9.starts, f9.prior.clone(), f9.detail.as_str()), (when::at(2026, 11, 18, 21, 0), when::at(2026, 11, 19, 3, 0), vec![0.75, 0.25], "Due at 03:00 UTC on Thursday, November 19"));
        assert_eq!(props[0].q, "Long March 2D: liftoff by 22:50 UTC on Thursday, November 19?");
        assert_eq!(props[2].prior, vec![0.55, 0.45]);
        let count = &props[3];
        assert_eq!((count.outcomes.clone(), count.detail.as_str()), (vec!["4 or fewer".to_string(), "5 or 6".into(), "7 or 8".into(), "9 or more".into()], "25 in the last four weeks"));
        // The Falcon flies on time; the Long March 2D slips two days; the third vanishes from
        // the list and, asked for by itself, has failed after lifting off in time.
        let later = UPCOMING
            .replace(r#""name":"Falcon 9 Block 5 | USSF-317","status":{"id":1,"name":"Go for Launch","abbrev":"Go"}"#, r#""name":"Falcon 9 Block 5 | USSF-317","status":{"id":3,"name":"Launch Successful","abbrev":"Success"}"#)
            .replace(r#""net":"2026-11-18T22:50:00Z""#, r#""net":"2026-11-20T22:50:00Z""#)
            .replace("a2975167-7c6f-4403-99e1-b40df17c8140", "ffffffff-7c6f-4403-99e1-b40df17c8140");
        let one = r#"{"id":"a2975167-7c6f-4403-99e1-b40df17c8140","name":"Long March 8A | SatNet LEO Group TBD?","status":{"id":4,"name":"Launch Failure","abbrev":"Failure"},"net":"2026-11-20T11:02:11Z","net_precision":{"id":0}}"#;
        let then = when::at(2026, 11, 21, 12, 0);
        let web = Canned::new(&[("launches/upcoming", later.as_str()), ("launches/a2975167-7c6f-4403-99e1-b40df17c8140/", one), ("launches/previous", r#"{"count":25,"results":[]}"#)]);
        assert!(m.step(then, &web, false));
        let props = feed_of(&m, then);
        let r = |id: &str| props.iter().find(|p| p.id == id).unwrap().result.clone().unwrap();
        assert_eq!((r("science:launch:766170334d").winner, r("science:launch:766170334d").note.as_str()), (Some(0), "lifted off at 03:00 UTC on Thursday, November 19"));
        assert_eq!((r("science:launch:e0a11eb10b").winner, r("science:launch:e0a11eb10b").note.as_str()), (Some(1), "now due Friday, November 20"));
        assert_eq!((r("science:launch:a29751677c").winner, r("science:launch:a29751677c").note.as_str()), (Some(0), "lifted off at 11:02 UTC on Friday, November 20"));
        // The week's count, twelve hours after it ends: eight.
        let end = when::at(2026, 11, 30, 13, 0);
        let web = Canned::new(&[("launches/upcoming", r#"{"count":0,"results":[]}"#), ("net__gte=2026-11-23T00:00:00Z&net__lt=2026-11-30T00:00:00Z", r#"{"count":8,"results":[]}"#), ("launches/previous", r#"{"count":25,"results":[]}"#)]);
        assert!(m.step(end, &web, false));
        let props = feed_of(&m, end);
        let done = props.iter().find(|p| p.id == "science:launches:20261123:count").unwrap().result.clone().unwrap();
        assert_eq!((done.winner, done.note.as_str()), (Some(2), "8 launches"));
    }

    #[test]
    fn asteroids_inside_the_moon_s_orbit_are_counted_two_days_after_the_week() {
        let now = when::at(2026, 11, 17, 12, 0);
        let web = Canned::new(&[("cad.api", JPL)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Asteroids)]);
        assert!(m.step(now, &web, false));
        assert!(web.asked()[0].ends_with("cad.api?dist-max=1LD&date-min=2026-09-20T12:00:00&date-max=2026-11-15T12:00:00"), "{:?}", web.asked());
        let p = &feed_of(&m, now)[0];
        assert_eq!((p.id.as_str(), p.q.as_str()), ("science:asteroids:20261123:count", "How many asteroids pass closer than the Moon in the week of November 23?"));
        // Seventeen in eight weeks: about two a week.
        assert_eq!(p.outcomes, vec!["1 or fewer", "2 or 3", "4 or 5", "6 or more"]);
        let then = when::at(2026, 12, 2, 6, 0);
        let web = Canned::new(&[("date-min=2026-11-23T00:00:00&date-max=2026-11-30T00:00:00", r#"{"signature":{"version":"1.5"},"count":"4"}"#), ("cad.api", JPL)]);
        assert!(m.step(then, &web, false));
        let done = feed_of(&m, then).into_iter().find(|p| p.id == "science:asteroids:20261123:count").unwrap().result.unwrap();
        assert_eq!((done.winner, done.note.as_str()), (Some(2), "4 passed inside the Moon's orbit"));
    }
}
