//! Crypto: Bitcoin, Ether, Solana, XRP and Dogecoin, priced by Coinbase's public candles
//! (Kraken's when Coinbase can't be read). Each day: up or down over the day (betting closes
//! as the day starts); for the bigger ones, which of five ranges the price is in at a
//! midnight two days off, and at next Monday's.
//!
//! A price "at 00:00 UTC" is the first trade price of that minute (its one-minute candle's
//! open): the same number for anyone who looks it up afterwards.

use serde_json::{Value, json};

use super::{dollars, num};
use crate::engine::{Kept, Moved, Out, Source};
use crate::http::Fetch;
use crate::math::normal_between;
use crate::when::{self, DAY, HOUR, MIN};
use pyr_proto::props_feed::FeedProp;

pub struct Coin {
    pub id: &'static str,
    pub name: &'static str,
    /// Its market on Coinbase, and on Kraken.
    pub coinbase: &'static str,
    pub kraken: &'static str,
    /// How far it moves in a day, about (as a share of its price).
    pub daily: f64,
    /// Whether it gets the range props too.
    pub ranges: bool,
    pub weekly: bool,
}

pub const COINS: [Coin; 5] = [
    Coin { id: "btc", name: "Bitcoin", coinbase: "BTC-USD", kraken: "XBTUSD", daily: 0.026, ranges: true, weekly: true },
    Coin { id: "eth", name: "Ether", coinbase: "ETH-USD", kraken: "ETHUSD", daily: 0.035, ranges: true, weekly: true },
    Coin { id: "sol", name: "Solana", coinbase: "SOL-USD", kraken: "SOLUSD", daily: 0.045, ranges: false, weekly: true },
    Coin { id: "xrp", name: "XRP", coinbase: "XRP-USD", kraken: "XRPUSD", daily: 0.045, ranges: false, weekly: false },
    Coin { id: "doge", name: "Dogecoin", coinbase: "DOGE-USD", kraken: "XDGUSD", daily: 0.05, ranges: false, weekly: false },
];

fn coin(id: &str) -> Option<&'static Coin> {
    COINS.iter().find(|c| c.id == id)
}

/// A coin's price now.
pub fn spot(web: &dyn Fetch, c: &Coin) -> Result<f64, String> {
    let coinbase = web.get(&format!("https://api.exchange.coinbase.com/products/{}/ticker", c.coinbase)).and_then(|t| {
        let v = super::json(&t, "Coinbase")?;
        num(&v["price"]).filter(|p| *p > 0.0).ok_or_else(|| "Coinbase's ticker has no price".to_string())
    });
    coinbase.or_else(|e| {
        let t = web.get(&format!("https://api.kraken.com/0/public/Ticker?pair={}", c.kraken)).map_err(|k| format!("{e}; {k}"))?;
        let v = super::json(&t, "Kraken")?;
        // (Its answer is under its own name for the pair, not the one asked.)
        v["result"].as_object().and_then(|r| r.values().next()).and_then(|p| num(&p["c"][0])).filter(|p| *p > 0.0).ok_or_else(|| format!("{e}; Kraken's ticker has no price"))
    })
}

/// A coin's price at a minute (Unix ms, on the minute): that minute's first trade price.
pub fn minute_open(web: &dyn Fetch, c: &Coin, at: i64) -> Result<f64, String> {
    let secs = at / 1000;
    let url = format!("https://api.exchange.coinbase.com/products/{}/candles?granularity=60&start={}&end={}", c.coinbase, when::iso(at), when::iso(at + MIN));
    let coinbase = web.get(&url).and_then(|t| {
        let v = super::json(&t, "Coinbase")?;
        // Rows of [time, low, high, open, close, volume], newest first.
        v.as_array().and_then(|rows| rows.iter().find(|r| r[0].as_i64() == Some(secs))).and_then(|r| num(&r[3])).filter(|p| *p > 0.0).ok_or_else(|| format!("Coinbase has no {} candle for {}", c.coinbase, when::iso(at)))
    });
    coinbase.or_else(|e| {
        let t = web.get(&format!("https://api.kraken.com/0/public/OHLC?pair={}&interval=1&since={}", c.kraken, secs - 60)).map_err(|k| format!("{e}; {k}"))?;
        let v = super::json(&t, "Kraken")?;
        // Rows of [time, open, high, low, close, vwap, volume, count], under its name for the pair.
        let rows = v["result"].as_object().and_then(|r| r.iter().find(|(k, _)| k.as_str() != "last")).and_then(|(_, rows)| rows.as_array().cloned()).unwrap_or_default();
        rows.iter().find(|r| r[0].as_i64() == Some(secs)).and_then(|r| num(&r[1])).filter(|p| *p > 0.0).ok_or_else(|| format!("{e}; Kraken has no candle for that minute either"))
    })
}

/// A step that makes round edges for a price: 250 near 65,000, 10 near 3,000, 0.0005 near 0.15.
fn step_for(price: f64) -> f64 {
    let raw = price * 0.004;
    let mag = 10f64.powi(raw.log10().floor() as i32);
    let lead = raw / mag;
    mag * if lead < 1.6 { 1.0 } else if lead < 3.6 { 2.5 } else if lead < 7.5 { 5.0 } else { 10.0 }
}

/// A price as an edge reads: "$64,250", "$3,010", "$152.50", "$0.1525".
fn edge(x: f64, step: f64) -> String {
    if step >= 1.0 {
        format!("${}", crate::math::commas(x.round() as u64))
    } else if step >= 0.01 {
        format!("${x:.2}")
    } else {
        format!("${x:.4}")
    }
}

/// Five ranges around a price, for something that strays `sd` (a share of the price) by the
/// time it counts: their edges (four, rounded), their names and their chances.
pub fn ranges(price: f64, sd: f64) -> (Vec<f64>, Vec<String>, Vec<f64>) {
    let step = step_for(price);
    let round = |x: f64| (x / step).round() * step;
    let mut edges: Vec<f64> = [-1.3, -0.45, 0.45, 1.3].iter().map(|k| round(price * (1.0 + k * sd))).collect();
    // (Rounding mustn't make two the same.)
    for i in 1..edges.len() {
        if edges[i] <= edges[i - 1] {
            edges[i] = edges[i - 1] + step;
        }
    }
    let names = vec![
        format!("Under {}", edge(edges[0], step)),
        format!("{} to {}", edge(edges[0], step), edge(edges[1], step)),
        format!("{} to {}", edge(edges[1], step), edge(edges[2], step)),
        format!("{} to {}", edge(edges[2], step), edge(edges[3], step)),
        format!("{} or more", edge(edges[3], step)),
    ];
    let sigma = price * sd;
    let chances = vec![
        normal_between(price, sigma, None, Some(edges[0])),
        normal_between(price, sigma, Some(edges[0]), Some(edges[1])),
        normal_between(price, sigma, Some(edges[1]), Some(edges[2])),
        normal_between(price, sigma, Some(edges[2]), Some(edges[3])),
        normal_between(price, sigma, Some(edges[3]), None),
    ];
    (edges, names, chances)
}

/// Which of the ranges a price is in (at an edge: the range above).
pub fn range_of(edges: &[f64], price: f64) -> usize {
    edges.iter().filter(|e| price >= **e - 1e-9).count()
}

/// How long a price is waited for after it's due before its prop is called off.
const WAIT: i64 = 36 * HOUR;
/// A price is asked for this long after its minute (its candle has to be there).
const SETTLE_LAG: i64 = 75 * 1000;

pub struct Crypto;

impl Crypto {
    fn base(c: &Coin, closes: i64, starts: i64) -> FeedProp {
        FeedProp { cat: "crypto".into(), event: format!("crypto:{}", c.id), event_name: c.name.to_string(), closes, starts, source: "Coinbase".into(), featured: if c.id == "btc" { 3 } else if c.ranges { 2 } else { 1 }, ..Default::default() }
    }

    /// The price at a minute, from the source's notes or the exchange (and noted).
    fn price_at(web: &dyn Fetch, memo: &mut Value, c: &Coin, at: i64) -> Result<f64, String> {
        let key = format!("{}:{}", c.id, at / 1000);
        if let Some(p) = memo["at"][&key].as_f64() {
            return Ok(p);
        }
        let p = minute_open(web, c, at)?;
        memo["at"][&key] = json!(p);
        Ok(p)
    }
}

impl Source for Crypto {
    fn name(&self) -> &'static str {
        "crypto"
    }

    /// Soon after the next minute it needs a price for; every quarter of an hour at least.
    fn every(&self, now: i64, mine: &[&Kept]) -> i64 {
        let next = mine.iter().flat_map(|k| [k.prop.closes, k.memo["settles"].as_i64().unwrap_or(0)]).filter(|t| *t + SETTLE_LAG > now).min();
        next.map_or(15 * MIN, |t| (t + SETTLE_LAG - now).clamp(MIN, 15 * MIN))
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], memo: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        let today = when::day_start(now);
        let known = |id: &str, memo: &Value| mine.iter().any(|k| k.prop.id == id) || memo["planned"][id].is_i64();
        let mut failed = 0;
        let mut tried = 0;
        for c in &COINS {
            // Up or down over a day: the next two days' (betting closes as each starts).
            for d in 1..=2 {
                let start = today + d * DAY;
                let id = format!("crypto:{}:{}:up", c.id, when::yyyymmdd(start));
                if known(&id, memo) {
                    continue;
                }
                let day = when::day_and_date(start);
                let mut p = Self::base(c, start, start);
                p.id = id.clone();
                p.q = format!("{}: up or down on {day}?", c.name);
                p.outcomes = vec!["Up".into(), "Down".into()];
                p.prior = vec![0.5, 0.5];
                p.detail = format!("From 00:00 UTC on {} to 00:00 UTC on {}", when::day_name(start), when::day_name(start + DAY));
                p.rules = format!("Up if {}'s price on Coinbase ({}) at 00:00 UTC on {} is above its price at 00:00 UTC on {day}; Down if it's below. Each price is the first trade of that minute. Called off if the two are the same or can't be had.", c.name, c.coinbase, when::day_and_date(start + DAY));
                out.open(p, json!({"kind": "up", "coin": c.id, "from": start, "settles": start + DAY}));
                memo["planned"][&id] = json!(start + DAY);
            }
            // Where it is at a midnight two days off, and at next Monday's.
            let mut targets: Vec<(&str, i64, i64, i64)> = Vec::new();
            if c.ranges {
                for d in 1..=3 {
                    // (Opened sixty hours ahead, closing twelve hours before.)
                    let t = today + d * DAY;
                    if t - now <= 60 * HOUR && t - now > 36 * HOUR {
                        targets.push(("range", t, t - 12 * HOUR, 60));
                    }
                }
            }
            if c.weekly {
                // (Next Monday's, opened from the Monday before, closing two days ahead.)
                let monday = when::week_start(now) + 7 * DAY;
                if monday - now > 3 * DAY {
                    targets.push(("week", monday, monday - 2 * DAY, 168));
                }
            }
            for (kind, t, closes, hours) in targets {
                let id = format!("crypto:{}:{}:{kind}", c.id, when::yyyymmdd(t));
                if known(&id, memo) {
                    continue;
                }
                tried += 1;
                let price = match spot(web, c) {
                    Ok(p) => p,
                    Err(e) => {
                        failed += 1;
                        out.trouble.push(format!("{}: {e}", c.name));
                        continue;
                    }
                };
                let sd = c.daily * ((t - now) as f64 / DAY as f64).sqrt().min((hours as f64 / 24.0).sqrt());
                let (edges, names, chances) = ranges(price, sd);
                let mut p = Self::base(c, closes, t);
                p.id = id.clone();
                p.q = format!("{}'s price at 00:00 UTC on {}?", c.name, when::day_and_date(t));
                p.outcomes = names;
                p.prior = chances;
                p.detail = format!("{} when this opened", dollars(price));
                p.rules = format!("{}'s price on Coinbase ({}) at 00:00 UTC on {}: the first trade of that minute. A price exactly on an edge counts for the range above it. Called off if the price can't be had.", c.name, c.coinbase, when::day_and_date(t));
                out.open(p, json!({"kind": kind, "coin": c.id, "edges": edges, "settles": t}));
                memo["planned"][&id] = json!(t);
            }
        }
        // What's open: the day's opening price as its day starts; the result when it's due.
        for k in mine {
            let Some(c) = k.memo["coin"].as_str().and_then(coin) else { continue };
            let settles = k.memo["settles"].as_i64().unwrap_or(0);
            let id = &k.prop.id;
            let up = k.memo["kind"] == "up";
            let from = k.memo["from"].as_i64().unwrap_or(0);
            if up && now >= from + SETTLE_LAG && now < settles {
                tried += 1;
                match Self::price_at(web, memo, c, from) {
                    Ok(p) => out.moved.push(Moved { id: id.clone(), detail: Some(format!("Started the day at {}", dollars(p))), ..Default::default() }),
                    Err(e) => {
                        failed += 1;
                        out.trouble.push(format!("{}: {e}", c.name));
                    }
                }
            }
            if now < settles + SETTLE_LAG {
                continue;
            }
            tried += 1;
            let end = Self::price_at(web, memo, c, settles);
            let begin = if up { Self::price_at(web, memo, c, from).map(Some) } else { Ok(None) };
            match (end, begin) {
                (Ok(end), Ok(begin)) => match begin {
                    Some(b) if (end - b).abs() < 1e-9 => out.call_off(id, format!("it was {} at both", dollars(end))),
                    Some(b) => out.settle(id, if end > b { 0 } else { 1 }, format!("{} to {} ({:+.2}%)", dollars(b), dollars(end), (end / b - 1.0) * 100.0)),
                    None => {
                        let edges: Vec<f64> = k.memo["edges"].as_array().map(|a| a.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
                        if edges.len() + 1 == k.prop.outcomes.len() { out.settle(id, range_of(&edges, end), dollars(end)) } else { out.call_off(id, "its ranges were lost") }
                    }
                },
                (Err(e), _) | (_, Err(e)) => {
                    failed += 1;
                    if now - settles > WAIT {
                        out.call_off(id, "the price couldn't be had");
                    } else {
                        out.trouble.push(format!("{}: {e}", c.name));
                    }
                }
            }
        }
        // Notes that are no use any more.
        for key in ["planned", "at"] {
            if let Some(m) = memo[key].as_object_mut() {
                m.retain(|k, v| match key {
                    "planned" => v.as_i64().is_some_and(|t| t > now - DAY),
                    _ => k.rsplit(':').next().and_then(|s| s.parse::<i64>().ok()).is_some_and(|s| s * 1000 > now - 4 * DAY),
                });
            }
        }
        if tried > 0 && failed == tried && out.open.is_empty() && out.results.is_empty() {
            return Err(out.trouble.first().cloned().unwrap_or_else(|| "no prices".into()));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Maker, State};
    use crate::http::Canned;

    const TICKER: &str = r#"{"ask":"65231.34","bid":"65231.2","volume":"13354.06273083","trade_id":1132443366,"price":"65231.19","size":"0.00000002","time":"2026-11-16T21:47:34.557583397Z","rfq_volume":"66.432181"}"#;
    const CANDLES: &str = "[[1794873780,64727,64780.61,64770.3,64727,9.65391219],[1794873720,64714.07,64793.63,64793.63,64770.3,9.38109364],[1794873660,64780.59,64819.73,64819.73,64793.63,9.19701327],[1794873600,64812.82,64887.83,64816.79,64819.74,11.95955558]]";
    const KRAKEN_TICKER: &str = r#"{"error":[],"result":{"XXBTZUSD":{"a":["64525.40000","3","3.000"],"b":["64525.30000","1","1.000"],"c":["64535.30000","0.00116215"],"v":["295.54476796","4251.08477856"],"p":["64624.08849","64759.02693"],"t":[4225,198626],"l":["64382.20000","63840.00000"],"h":["64859.50000","65633.50000"],"o":"64813.70000"}}}"#;
    const KRAKEN_OHLC: &str = r#"{"error":[],"result":{"XXBTZUSD":[[1794873600,"64815.7","64817.1","64773.0","64773.0","64781.8","20.93479377",191],[1794873660,"64774.2","64774.2","64734.2","64769.9","64768.2","54.31502334",298]],"last":1794916800}}"#;

    #[test]
    fn prices_come_from_coinbase_and_from_kraken_when_it_s_down() {
        let btc = coin("btc").unwrap();
        let midnight = when::at(2026, 11, 17, 0, 0);
        assert_eq!(midnight, 1_794_873_600_000);
        let both = Canned::new(&[("coinbase.com/products/BTC-USD/ticker", TICKER), ("coinbase.com/products/BTC-USD/candles", CANDLES), ("kraken.com/0/public/Ticker", KRAKEN_TICKER), ("kraken.com/0/public/OHLC", KRAKEN_OHLC)]);
        assert_eq!(spot(&both, btc), Ok(65_231.19));
        assert_eq!(minute_open(&both, btc, midnight), Ok(64_816.79));
        assert!(both.asked()[1].ends_with("candles?granularity=60&start=2026-11-17T00:00:00Z&end=2026-11-17T00:01:00Z"), "{:?}", both.asked());
        assert!(minute_open(&both, btc, midnight - MIN).unwrap_err().contains("no candle"), "a minute neither has");
        let kraken = Canned::new(&[("kraken.com/0/public/Ticker", KRAKEN_TICKER), ("kraken.com/0/public/OHLC", KRAKEN_OHLC)]);
        assert_eq!(spot(&kraken, btc), Ok(64_535.3));
        assert_eq!(minute_open(&kraken, btc, midnight), Ok(64_815.7));
        assert!(kraken.asked().last().unwrap().ends_with("OHLC?pair=XBTUSD&interval=1&since=1794873540"));
        assert!(spot(&Canned::default(), btc).is_err());
    }

    #[test]
    fn ranges_have_round_edges_and_chances_that_add_up() {
        let (edges, names, chances) = ranges(65_231.19, 0.026 * 2.5f64.sqrt());
        assert_eq!(edges, vec![61_750.0, 64_000.0, 66_500.0, 68_750.0]);
        assert_eq!(names, vec!["Under $61,750", "$61,750 to $64,000", "$64,000 to $66,500", "$66,500 to $68,750", "$68,750 or more"]);
        assert!((chances.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(chances[2] > 0.3 && chances[0] > 0.07 && chances[0] < 0.13, "{chances:?}");
        assert!(names.iter().all(|n| n.chars().count() <= 30));
        assert_eq!((range_of(&edges, 61_000.0), range_of(&edges, 61_750.0), range_of(&edges, 65_000.0), range_of(&edges, 70_000.0)), (0, 1, 2, 4));
        // Small prices get small steps, and still five different ranges.
        let (edges, names, _) = ranges(0.1523, 0.05);
        assert!(edges.windows(2).all(|w| w[1] > w[0]), "{edges:?}");
        assert_eq!(names[0], "Under $0.1425");
        let (_, names, _) = ranges(3_012.4, 0.035 * 2.5f64.sqrt());
        assert_eq!(names[2], "$2,940 to $3,090");
        for (price, step) in [(65_231.0, 250.0), (3_012.0, 10.0), (152.0, 0.5), (2.31, 0.01)] {
            assert!((step_for(price) / step - 1.0).abs() < 1e-9, "{price}: {}", step_for(price));
        }
    }

    #[test]
    fn a_day_s_up_or_down_and_a_range_open_and_settle_from_the_candles() {
        // Sunday noon: Monday's and Tuesday's up-or-down open for each coin; Bitcoin's and
        // Ether's ranges for Tuesday midnight (sixty hours off); nothing weekly this late in
        // the week.
        let now = when::at(2026, 11, 15, 12, 0);
        let midnight = when::at(2026, 11, 17, 0, 0);
        let web = Canned::new(&[("BTC-USD/ticker", TICKER), ("BTC-USD/candles", CANDLES)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Crypto)]);
        assert!(m.step(now, &web, false));
        let feed = m.feed(now);
        let ids: Vec<&str> = feed.props.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids.iter().filter(|i| i.ends_with(":up")).count(), 10);
        assert!(ids.contains(&"crypto:btc:20261118:range"), "{ids:?}");
        assert!(!ids.iter().any(|i| i.contains("eth") && i.ends_with("range")), "Ether's price couldn't be had: no range for it");
        assert_eq!(m.state.sources["crypto"].trouble.len(), 1);
        let up = feed.props.iter().find(|p| p.id == "crypto:btc:20261116:up").unwrap();
        assert_eq!((up.q.as_str(), up.closes, up.event_name.as_str()), ("Bitcoin: up or down on Monday, November 16?", when::at(2026, 11, 16, 0, 0), "Bitcoin"));
        let range = feed.props.iter().find(|p| p.id == "crypto:btc:20261118:range").unwrap();
        assert_eq!((range.q.as_str(), range.closes, range.starts), ("Bitcoin's price at 00:00 UTC on Wednesday, November 18?", when::at(2026, 11, 17, 12, 0), when::at(2026, 11, 18, 0, 0)));
        assert_eq!(range.detail, "$65,231 when this opened");
        // Tuesday's up-or-down, as Tuesday starts: the day's opening price shows.
        let web = Canned::new(&[("BTC-USD/candles", CANDLES)]);
        assert!(m.step(midnight + 2 * MIN, &web, true));
        let tue = m.feed(midnight + 2 * MIN).props.into_iter().find(|p| p.id == "crypto:btc:20261117:up").unwrap();
        assert_eq!(tue.detail, "Started the day at $64,817");
        // A day on: Wednesday's first minute is $66,000.50: up, and it's in the middle range.
        let wed = CANDLES.replace("1794873600,64812.82,64887.83,64816.79", &format!("{},65990,66010,66000.5", (midnight + DAY) / 1000));
        let web = Canned::new(&[("start=2026-11-18T00:00:00Z", wed.as_str()), ("BTC-USD/candles", CANDLES)]);
        assert!(m.step(midnight + DAY + 2 * MIN, &web, true));
        let feed = m.feed(midnight + DAY + 2 * MIN);
        let result = |id: &str| feed.props.iter().find(|p| p.id == id).unwrap().result.clone();
        let r = result("crypto:btc:20261117:up").unwrap();
        assert_eq!((r.winner, r.note.as_str()), (Some(0), "$64,817 to $66,001 (+1.83%)"));
        let r = result("crypto:btc:20261118:range").unwrap();
        assert_eq!((r.winner, r.note.as_str()), (Some(2), "$66,001"));
        // The other coins' props wait for their prices (down, here), and are called off after
        // a day and a half without.
        assert!(result("crypto:eth:20261117:up").is_none());
        assert!(m.step(midnight + DAY + WAIT + HOUR, &Canned::default(), true));
        let feed = m.feed(midnight + DAY + WAIT + HOUR);
        let r = feed.props.iter().find(|p| p.id == "crypto:eth:20261117:up").unwrap().result.clone().unwrap();
        assert_eq!((r.winner, r.note.as_str()), (None, "the price couldn't be had"));
    }

    #[test]
    fn next_monday_s_price_opens_early_in_the_week() {
        // Monday afternoon: next Monday's ranges for Bitcoin (the others' prices are down).
        let now = when::at(2026, 11, 16, 15, 0);
        let web = Canned::new(&[("BTC-USD/ticker", TICKER)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Crypto)]);
        m.step(now, &web, false);
        let feed = m.feed(now);
        let week = feed.props.iter().find(|p| p.id == "crypto:btc:20261123:week").unwrap();
        assert_eq!((week.q.as_str(), week.closes), ("Bitcoin's price at 00:00 UTC on Monday, November 23?", when::at(2026, 11, 21, 0, 0)));
        assert_eq!(week.outcomes.len(), 5);
        // Asked again: the same props aren't planned twice (no price asked for).
        let quiet = Canned::default();
        m.step(now + 20 * MIN, &quiet, true);
        assert!(!quiet.asked().iter().any(|u| u.contains("BTC-USD/ticker")), "{:?}", quiet.asked());
    }
}
