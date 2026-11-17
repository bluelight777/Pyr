//! Weather, in ten US cities: tomorrow's high (five ranges around the forecast) and whether
//! it rains or snows, as the National Weather Service has it. The forecast (api.weather.gov)
//! sets the ranges and the starting odds; the Service's daily climate report (its CLI
//! product, the same record the weather markets settle on) says what happened.
//!
//! Betting on a day closes at its midnight, local time: before any of it has happened.
//!
//! Not read from the workspace this was written in (the fetch tool is refused by
//! api.weather.gov's robots rules, and nothing was tried another way): the forecast's shape is
//! the documented one, and the climate report's layout the long-standing one. `pyr-props
//! check weather` on the server says whether both still read.

use serde_json::{Value, json};

use super::num;
use crate::engine::{Kept, Out, Source};
use crate::http::Fetch;
use crate::math::normal_between;
use crate::when::{self, DAY, HOUR, MIN};
use pyr_proto::props_feed::FeedProp;

pub struct City {
    pub id: &'static str,
    pub name: &'static str,
    /// The climate report's place (its `issuedby`), and what that station is called.
    pub cli: &'static str,
    pub station: &'static str,
    pub lat: f64,
    pub lon: f64,
}

pub const CITIES: [City; 10] = [
    City { id: "nyc", name: "New York", cli: "NYC", station: "Central Park", lat: 40.7794, lon: -73.9692 },
    City { id: "chi", name: "Chicago", cli: "ORD", station: "O'Hare Airport", lat: 41.9742, lon: -87.9073 },
    City { id: "lax", name: "Los Angeles", cli: "LAX", station: "Los Angeles Airport", lat: 33.9382, lon: -118.3886 },
    City { id: "mia", name: "Miami", cli: "MIA", station: "Miami Airport", lat: 25.7906, lon: -80.3164 },
    City { id: "dfw", name: "Dallas", cli: "DFW", station: "Dallas-Fort Worth Airport", lat: 32.8974, lon: -97.0220 },
    City { id: "den", name: "Denver", cli: "DEN", station: "Denver Airport", lat: 39.8466, lon: -104.6562 },
    City { id: "phx", name: "Phoenix", cli: "PHX", station: "Sky Harbor Airport", lat: 33.4278, lon: -112.0037 },
    City { id: "sea", name: "Seattle", cli: "SEA", station: "Seattle-Tacoma Airport", lat: 47.4447, lon: -122.3144 },
    City { id: "bos", name: "Boston", cli: "BOS", station: "Logan Airport", lat: 42.3606, lon: -71.0097 },
    City { id: "atl", name: "Atlanta", cli: "ATL", station: "Hartsfield-Jackson Airport", lat: 33.6297, lon: -84.4422 },
];

/// A day of a forecast: its date there, when it starts (its midnight, UTC), the high, and
/// the chance of rain or snow sometime in it.
#[derive(Clone, Debug, PartialEq)]
pub struct Day {
    /// "2026-11-19", and that day's midnight as a moment.
    pub date: String,
    pub starts: i64,
    pub high: i64,
    pub wet: f64,
}

/// The days of a forecast (api.weather.gov's twelve-hour periods).
pub fn days(text: &str) -> Result<Vec<Day>, String> {
    let v = super::json(text, "the Weather Service")?;
    let periods = v["properties"]["periods"].as_array().ok_or("the forecast has no periods")?;
    // Each period: its date as written (local), whether it's the day's or the night's, the
    // temperature and the chance of precipitation.
    let read: Vec<(String, i64, bool, Option<f64>, f64)> = periods
        .iter()
        .filter_map(|p| {
            let start = p["startTime"].as_str()?;
            let date = start.get(..10)?.to_string();
            let midnight = when::parse_date(&date)? - when::offset_minutes(start) * MIN;
            Some((date, midnight, p["isDaytime"].as_bool()?, num(&p["temperature"]), num(&p["probabilityOfPrecipitation"]["value"]).unwrap_or(0.0) / 100.0))
        })
        .collect();
    let mut out = Vec::new();
    for (i, (date, starts, daytime, temp, pop)) in read.iter().enumerate() {
        let (true, Some(high)) = (*daytime, temp) else { continue };
        // Rain or snow sometime that day: by day, or in the half of each night that's in it.
        let before = i.checked_sub(1).and_then(|j| read.get(j)).filter(|p| !p.2).map_or(0.0, |p| p.4);
        let after = read.get(i + 1).filter(|p| !p.2).map_or(0.0, |p| p.4);
        let dry = (1.0 - pop) * (1.0 - 0.5 * before) * (1.0 - 0.5 * after);
        out.push(Day { date: date.clone(), starts: *starts, high: high.round() as i64, wet: (1.0 - dry).clamp(0.04, 0.96) });
    }
    if out.is_empty() { Err("the forecast has no days".into()) } else { Ok(out) }
}

/// Five ranges for a high around what's forecast, and their chances (a forecast for
/// tomorrow is off by about two and a half degrees; for the day after, more).
pub fn high_ranges(forecast: i64, sd: f64) -> (Vec<i64>, Vec<String>, Vec<f64>) {
    let f = forecast;
    let starts = vec![f - 3, f - 1, f + 2, f + 4];
    let names = vec![format!("{}° or lower", f - 4), format!("{}° to {}°", f - 3, f - 2), format!("{}° to {}°", f - 1, f + 1), format!("{}° to {}°", f + 2, f + 3), format!("{}° or higher", f + 4)];
    let (m, edge) = (f as f64, |s: i64| s as f64 - 0.5);
    let chances = vec![
        normal_between(m, sd, None, Some(edge(starts[0]))),
        normal_between(m, sd, Some(edge(starts[0])), Some(edge(starts[1]))),
        normal_between(m, sd, Some(edge(starts[1])), Some(edge(starts[2]))),
        normal_between(m, sd, Some(edge(starts[2])), Some(edge(starts[3]))),
        normal_between(m, sd, Some(edge(starts[3])), None),
    ];
    (starts, names, chances)
}

/// What a daily climate report says: the day it's for, whether it's the day's last word (not
/// the afternoon's so-far), the high, and the precipitation in inches (a trace is 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Climate {
    pub day: i64,
    pub last: bool,
    pub high: Option<i64>,
    pub wet: Option<f64>,
}

/// Reads a climate report: the product's page (its text is in a `pre`), or the bare text.
pub fn climate(page: &str) -> Result<Climate, String> {
    let text = match page.find("<pre") {
        Some(i) => {
            let rest = &page[i..];
            let from = rest.find('>').map_or(0, |j| j + 1);
            let to = rest.find("</pre>").unwrap_or(rest.len());
            rest[from..to.max(from)].to_string()
        }
        None => page.to_string(),
    };
    let lines: Vec<&str> = text.lines().collect();
    // "...THE CENTRAL PARK NY CLIMATE SUMMARY FOR NOVEMBER 18 2026..."
    let day = lines
        .iter()
        .find_map(|l| {
            let after = l.split("CLIMATE SUMMARY FOR").nth(1)?;
            let mut w = after.split(|c: char| c.is_whitespace() || c == '.').filter(|w| !w.is_empty());
            let (m, d, y) = (when::month_number(w.next()?)?, w.next()?.parse::<u32>().ok()?, w.next()?.parse::<i64>().ok()?);
            (1..=31).contains(&d).then(|| when::at(y, m, d, 0, 0))
        })
        .ok_or("the climate report doesn't say which day it's for")?;
    // (The afternoon's report is headed "VALID TODAY AS OF 0400 PM LOCAL TIME".)
    let last = !lines.iter().any(|l| l.contains("VALID TODAY AS OF") || l.contains("VALID AS OF"));
    // The first row with this label under a section's heading: its first value.
    let value = |section: &str, labels: &[&str]| -> Option<String> {
        let at = lines.iter().position(|l| l.trim_start().starts_with(section))?;
        lines[at + 1..].iter().take(8).find_map(|l| {
            let mut w = l.split_whitespace();
            let label = w.next()?;
            let v = w.next()?;
            (labels.contains(&label) && v.chars().next().is_some_and(|c| c.is_ascii_digit() || c == '-' || c == 'T' || c == 'M' || c == '.')).then(|| v.to_string())
        })
    };
    // ("72R": a record. "MM": missing. "T": a trace.)
    let number = |v: String| v.trim_end_matches(|c: char| c.is_ascii_alphabetic()).parse::<f64>().ok();
    let high = value("TEMPERATURE", &["MAXIMUM"]).and_then(number).map(|h| h.round() as i64);
    let wet = value("PRECIPITATION", &["YESTERDAY", "TODAY"]).and_then(|v| if v == "T" { Some(0.0) } else { number(v) });
    Ok(Climate { day, last, high, wet })
}

fn climate_url(c: &City, version: u32) -> String {
    format!("https://forecast.weather.gov/product.php?site=NWS&issuedby={}&product=CLI&format=txt&version={version}&glossary=0", c.cli)
}

/// A day's last climate report for a city, if it's out (looked for among the latest three).
fn report(web: &dyn Fetch, c: &City, day: i64) -> Result<Option<Climate>, String> {
    let mut trouble = None;
    for version in 1..=3 {
        match web.get(&climate_url(c, version)).and_then(|t| climate(&t)) {
            Ok(r) if r.day == day && r.last => return Ok(Some(r)),
            // (An older one than the day asked for: the day's isn't out.)
            Ok(r) if r.day < day => return Ok(None),
            Ok(_) => {}
            Err(e) => trouble = Some(e),
        }
    }
    trouble.map_or(Ok(None), Err)
}

pub struct Weather;

impl Weather {
    /// A city's forecast, by the grid the Service puts it on (looked up once and kept).
    fn forecast(web: &dyn Fetch, memo: &mut Value, c: &City) -> Result<Vec<Day>, String> {
        let url = match memo["grid"][c.id].as_str() {
            Some(u) => u.to_string(),
            None => {
                let v = super::json(&web.get(&format!("https://api.weather.gov/points/{:.4},{:.4}", c.lat, c.lon))?, "the Weather Service")?;
                let u = v["properties"]["forecast"].as_str().filter(|u| u.starts_with("https://api.weather.gov/")).ok_or("the Weather Service gave no forecast for the place")?.to_string();
                memo["grid"][c.id] = json!(u);
                u
            }
        };
        days(&web.get(&url)?)
    }
}

impl Source for Weather {
    fn name(&self) -> &'static str {
        "weather"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        HOUR
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], memo: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        let (mut tried, mut failed) = (0, 0);
        for c in &CITIES {
            // The next two days there: looked at once a day (when the last day planned for
            // the city is less than a day and a half off).
            let planned = memo["planned"][c.id].as_i64().unwrap_or(0);
            if planned - now < 36 * HOUR {
                tried += 1;
                match Self::forecast(web, memo, c) {
                    Ok(list) => {
                        for (lead, d) in list.iter().filter(|d| d.starts > now + 2 * HOUR && d.starts < now + 60 * HOUR).enumerate() {
                            let tag = d.date.replace('-', "");
                            let day = when::day_name(when::parse_date(&d.date).unwrap_or(d.starts));
                            let on = when::day_and_date(when::parse_date(&d.date).unwrap_or(d.starts));
                            let base = FeedProp { cat: "weather".into(), event: format!("weather:{}:{tag}", c.id), event_name: format!("{}, {on}", c.name), closes: d.starts, starts: d.starts, source: "National Weather Service".into(), featured: if c.id == "nyc" { 2 } else { 1 }, ..Default::default() };
                            let report = format!("the National Weather Service's daily climate report for {} ({}), which comes the morning after", c.station, c.name);
                            let (starts, names, chances) = high_ranges(d.high, if lead == 0 { 2.6 } else { 3.3 });
                            let mut p = base.clone();
                            p.id = format!("weather:{}:{tag}:high", c.id);
                            p.q = format!("{}: {day}'s high?", c.name);
                            p.outcomes = names;
                            p.prior = chances;
                            p.detail = format!("The forecast: {}°F", d.high);
                            p.rules = format!("The highest temperature on {on}, in degrees Fahrenheit, as {report}. Called off if no report comes within two days.");
                            out.open(p, json!({"kind": "high", "city": c.id, "day": when::parse_date(&d.date), "ends": d.starts + DAY, "starts": starts}));
                            let mut p = base;
                            p.id = format!("weather:{}:{tag}:wet", c.id);
                            p.q = format!("{}: rain or snow on {day}?", c.name);
                            p.outcomes = vec!["Yes".into(), "No".into()];
                            p.prior = vec![d.wet, 1.0 - d.wet];
                            p.detail = format!("The forecast: {:.0}% chance", d.wet * 100.0);
                            p.rules = format!("Yes if at least 0.01 inches of precipitation is measured on {on} (a trace doesn't count), as {report}. Called off if no report comes within two days.");
                            out.open(p, json!({"kind": "wet", "city": c.id, "day": when::parse_date(&d.date), "ends": d.starts + DAY}));
                            memo["planned"][c.id] = json!(d.starts.max(memo["planned"][c.id].as_i64().unwrap_or(0)));
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        out.trouble.push(format!("{}: {e}", c.name));
                    }
                }
            }
            // What's over there: the report comes an hour or two after the day ends.
            let due: Vec<&&Kept> = mine.iter().filter(|k| k.memo["city"] == c.id && k.memo["ends"].as_i64().is_some_and(|e| now > e + 90 * MIN)).collect();
            let mut days_due: Vec<i64> = due.iter().filter_map(|k| k.memo["day"].as_i64()).collect();
            days_due.sort();
            days_due.dedup();
            for day in days_due {
                tried += 1;
                let got = report(web, c, day);
                if let Err(e) = &got {
                    failed += 1;
                    out.trouble.push(format!("{}: {e}", c.name));
                }
                for k in due.iter().filter(|k| k.memo["day"] == day) {
                    let late = k.memo["ends"].as_i64().is_some_and(|e| now > e + 2 * DAY);
                    let id = &k.prop.id;
                    match (&got, k.memo["kind"].as_str()) {
                        (Ok(Some(r)), Some("high")) => match r.high {
                            Some(h) => {
                                let starts: Vec<i64> = k.memo["starts"].as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
                                if starts.len() + 1 == k.prop.outcomes.len() { out.settle(id, starts.iter().filter(|s| h >= **s).count(), format!("{h}°F")) } else { out.call_off(id, "its ranges were lost") }
                            }
                            None => out.call_off(id, "the report has no high for that day"),
                        },
                        (Ok(Some(r)), Some("wet")) => match r.wet {
                            Some(w) => out.settle(id, if w >= 0.005 { 0 } else { 1 }, if w >= 0.005 { format!("{w:.2} inches") } else { "none measured".to_string() }),
                            None => out.call_off(id, "the report has no precipitation for that day"),
                        },
                        _ if late => out.call_off(id, "no climate report came"),
                        _ => {}
                    }
                }
            }
        }
        if tried > 0 && failed == tried && out.open.is_empty() && out.results.is_empty() {
            return Err(out.trouble.first().cloned().unwrap_or_default());
        }
        out.trouble.truncate(6);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Maker, State};
    use crate::http::Canned;

    const POINT: &str = r#"{"id":"https://api.weather.gov/points/40.7794,-73.9692","type":"Feature","properties":{"gridId":"OKX","gridX":34,"gridY":38,"forecast":"https://api.weather.gov/gridpoints/OKX/34,38/forecast","forecastHourly":"https://api.weather.gov/gridpoints/OKX/34,38/forecast/hourly","timeZone":"America/New_York"}}"#;
    const FORECAST: &str = r#"{"type":"Feature","properties":{"units":"us","generatedAt":"2026-11-17T11:02:35+00:00","periods":[
        {"number":1,"name":"Today","startTime":"2026-11-17T06:00:00-05:00","endTime":"2026-11-17T18:00:00-05:00","isDaytime":true,"temperature":52,"temperatureUnit":"F","probabilityOfPrecipitation":{"unitCode":"wmoUnit:percent","value":10},"shortForecast":"Sunny"},
        {"number":2,"name":"Tonight","startTime":"2026-11-17T18:00:00-05:00","endTime":"2026-11-18T06:00:00-05:00","isDaytime":false,"temperature":41,"temperatureUnit":"F","probabilityOfPrecipitation":{"unitCode":"wmoUnit:percent","value":null},"shortForecast":"Clear"},
        {"number":3,"name":"Wednesday","startTime":"2026-11-18T06:00:00-05:00","endTime":"2026-11-18T18:00:00-05:00","isDaytime":true,"temperature":55,"temperatureUnit":"F","probabilityOfPrecipitation":{"unitCode":"wmoUnit:percent","value":60},"shortForecast":"Rain Likely"},
        {"number":4,"name":"Wednesday Night","startTime":"2026-11-18T18:00:00-05:00","endTime":"2026-11-19T06:00:00-05:00","isDaytime":false,"temperature":44,"temperatureUnit":"F","probabilityOfPrecipitation":{"unitCode":"wmoUnit:percent","value":80},"shortForecast":"Rain"},
        {"number":5,"name":"Thursday","startTime":"2026-11-19T06:00:00-05:00","endTime":"2026-11-19T18:00:00-05:00","isDaytime":true,"temperature":49,"temperatureUnit":"F","probabilityOfPrecipitation":{"unitCode":"wmoUnit:percent","value":20},"shortForecast":"Mostly Sunny"},
        {"number":6,"name":"Thursday Night","startTime":"2026-11-19T18:00:00-05:00","endTime":"2026-11-20T06:00:00-05:00","isDaytime":false,"temperature":38,"temperatureUnit":"F","probabilityOfPrecipitation":{"unitCode":"wmoUnit:percent","value":null},"shortForecast":"Clear"},
        {"number":7,"name":"Friday","startTime":"2026-11-20T06:00:00-05:00","endTime":"2026-11-20T18:00:00-05:00","isDaytime":true,"temperature":47,"temperatureUnit":"F","probabilityOfPrecipitation":{"unitCode":"wmoUnit:percent","value":null},"shortForecast":"Sunny"}
    ]}}"#;
    const REPORT: &str = "<html><body><pre class=\"glossaryProduct\">\n000\nCDUS41 KOKX 190627\nCLINYC\n\nCLIMATE REPORT\nNATIONAL WEATHER SERVICE NEW YORK, NY\n127 AM EST THU NOV 19 2026\n\n...................................\n\n...THE CENTRAL PARK NY CLIMATE SUMMARY FOR NOVEMBER 18 2026...\n\nCLIMATE NORMAL PERIOD: 1991 TO 2020\nCLIMATE RECORD PERIOD: 1869 TO 2026\n\n\nWEATHER ITEM   OBSERVED TIME   RECORD YEAR NORMAL DEPARTURE LAST\n                VALUE   (LST)  VALUE       VALUE  FROM      YEAR\n                                                  NORMAL\n...................................................................\nTEMPERATURE (F)\n YESTERDAY\n  MAXIMUM         56   2:14 PM  72    1928  53      3       49\n  MINIMUM         44  11:59 PM  20    1936  42      2       38\n  AVERAGE         50                        48      2       44\n\nPRECIPITATION (IN)\n  YESTERDAY        0.42          1.74 1954   0.11   0.31     0.00\n  MONTH TO DATE    1.61                      2.04  -0.43     0.89\n  SINCE SEP 1      9.87                     10.87  -1.00     6.42\n  SINCE JAN 1     41.25                     41.52  -0.27    38.27\n\nSNOWFALL (IN)\n  YESTERDAY        0.0           2.3  1892   0.0    0.0      0.0\n  MONTH TO DATE    0.0                       0.1   -0.1      0.0\n  SNOW DEPTH       0\n</pre></body></html>";

    #[test]
    fn a_forecast_reads_as_days_with_a_high_and_a_chance_of_rain() {
        let d = days(FORECAST).unwrap();
        assert_eq!(d.len(), 4);
        // Wednesday in New York starts at 05:00 UTC.
        assert_eq!((d[1].date.as_str(), d[1].starts, d[1].high), ("2026-11-18", when::at(2026, 11, 18, 5, 0), 55));
        // Rain likely by day and that night, none said the night before.
        assert!((d[1].wet - 0.76).abs() < 1e-9, "{}", d[1].wet);
        // Thursday: 20% by day, and half of Wednesday night's 80% falls in it.
        assert!((d[2].wet - 0.52).abs() < 1e-9, "{}", d[2].wet);
        assert_eq!(d[3].wet, 0.04, "never surer than the forecasters");
        assert!(days("{}").is_err());
        let (starts, names, chances) = high_ranges(55, 2.6);
        assert_eq!(starts, vec![52, 54, 57, 59]);
        assert_eq!(names, vec!["51° or lower", "52° to 53°", "54° to 56°", "57° to 58°", "59° or higher"]);
        assert!((chances.iter().sum::<f64>() - 1.0).abs() < 1e-9 && chances[2] > 0.4 && chances[0] > 0.07, "{chances:?}");
    }

    #[test]
    fn a_climate_report_reads_whatever_its_spacing() {
        let r = climate(REPORT).unwrap();
        assert_eq!(r, Climate { day: when::at(2026, 11, 18, 0, 0), last: true, high: Some(56), wet: Some(0.42) });
        // A record high, a trace of rain, and the bare text.
        let odd = REPORT.split("<pre class=\"glossaryProduct\">").nth(1).unwrap().replace("</pre></body></html>", "").replace("MAXIMUM         56 ", "MAXIMUM         73R").replace("YESTERDAY        0.42", "YESTERDAY        T   ");
        let r = climate(&odd).unwrap();
        assert_eq!((r.high, r.wet), (Some(73), Some(0.0)));
        // Missing values are missing, not zero; the afternoon's report isn't the last word.
        let gone = REPORT.replace("MAXIMUM         56 ", "MAXIMUM         MM ").replace("YESTERDAY        0.42", "YESTERDAY        MM  ");
        let r = climate(&gone).unwrap();
        assert_eq!((r.high, r.wet), (None, None));
        let so_far = REPORT.replace("CLIMATE RECORD PERIOD: 1869 TO 2026", "CLIMATE RECORD PERIOD: 1869 TO 2026\n\nVALID TODAY AS OF 0400 PM LOCAL TIME.").replace(" YESTERDAY\n", " TODAY\n").replace("  YESTERDAY        0.42", "  TODAY            0.10");
        let r = climate(&so_far).unwrap();
        assert_eq!((r.last, r.wet), (false, Some(0.10)));
        assert!(climate("<html>No products found.</html>").is_err());
    }

    #[test]
    fn a_day_s_weather_opens_from_the_forecast_and_settles_from_the_report() {
        // Tuesday morning in New York: Wednesday's and Thursday's props (the other cities'
        // forecasts are down).
        let now = when::at(2026, 11, 17, 15, 0);
        let web = Canned::new(&[("points/40.7794,-73.9692", POINT), ("gridpoints/OKX/34,38/forecast", FORECAST)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Weather)]);
        assert!(m.step(now, &web, false));
        let props = m.feed(now).props;
        assert_eq!(props.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["weather:nyc:20261118:high", "weather:nyc:20261118:wet", "weather:nyc:20261119:high", "weather:nyc:20261119:wet"]);
        let high = &props[0];
        assert_eq!((high.q.as_str(), high.closes, high.event_name.as_str(), high.detail.as_str()), ("New York: Wednesday's high?", when::at(2026, 11, 18, 5, 0), "New York, Wednesday, November 18", "The forecast: 55°F"));
        assert_eq!(props[1].q, "New York: rain or snow on Wednesday?");
        assert_eq!(props[1].prior, vec![0.76, 0.24]);
        assert!(props[2].prior[2] < high.prior[2], "a day further off is less sure");
        assert_eq!(m.state.sources["weather"].trouble.len(), 6, "the other nine cities (the first six said)");
        // An hour on: New York's grid is known and its days planned; nothing asked of it.
        let quiet = Canned::default();
        m.step(now + HOUR, &quiet, false);
        assert!(!quiet.asked().iter().any(|u| u.contains("40.7794") || u.contains("OKX")), "{:?}", quiet.asked());
        // Thursday 2 AM there: Wednesday's report is out: 56 degrees, 0.42 inches.
        let then = when::at(2026, 11, 19, 7, 0);
        let web = Canned::new(&[("issuedby=NYC&product=CLI&format=txt&version=1", REPORT)]);
        assert!(m.step(then, &web, false));
        let props = m.feed(then).props;
        let r = |id: &str| props.iter().find(|p| p.id == id).unwrap().result.clone();
        assert_eq!(r("weather:nyc:20261118:high").map(|r| (r.winner, r.note)), Some((Some(2), "56°F".to_string())));
        assert_eq!(r("weather:nyc:20261118:wet").map(|r| (r.winner, r.note)), Some((Some(0), "0.42 inches".to_string())));
        assert!(r("weather:nyc:20261119:high").is_none());
        // Thursday's report never comes: called off two days after the day ends.
        let late = when::at(2026, 11, 22, 6, 0);
        assert!(m.step(late, &Canned::new(&[("issuedby=NYC", REPORT)]), false));
        let props = m.feed(late).props;
        assert_eq!(props.iter().find(|p| p.id == "weather:nyc:20261119:wet").unwrap().result.clone().map(|r| (r.winner, r.note)), Some((None, "no climate report came".to_string())));
    }
}
