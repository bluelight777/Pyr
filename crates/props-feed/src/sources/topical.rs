//! Topical: numbers the world publishes about itself. The 10-year Treasury yield and the
//! national debt (the US Treasury), what the euro, the yen and the pound are worth (the
//! European Central Bank's reference rates, by way of frankfurter.dev), and executive orders
//! (the Federal Register). All public, none needing a key.
//!
//! With public data only (Blue's choice, Nov 17) nothing here reads the news: these are the
//! world's figures, not its headlines.

use serde_json::{Value, json};

use super::{next_week, num, week_of};
use crate::engine::{Kept, Out, Source};
use crate::http::Fetch;
use crate::math::{phi, poisson_between};
use crate::when::{self, DAY, HOUR};
use pyr_proto::props_feed::FeedProp;

fn base(event: &str, event_name: &str, closes: i64, starts: i64, source: &str) -> FeedProp {
    FeedProp { cat: "topical".into(), event: format!("topical:{event}"), event_name: event_name.to_string(), closes, starts, source: source.into(), featured: 1, ..Default::default() }
}

// ------------------------------------------------------------------ the Treasury

/// The 10-year Treasury yield at the end of next week, and the national debt at the end of
/// next month.
pub struct Treasury;

fn yield_url(month: i64) -> String {
    let (y, m, _) = when::ymd(month);
    let ym = format!("{y:04}{m:02}");
    format!("https://home.treasury.gov/resource-center/data-chart-center/interest-rates/daily-treasury-rates.csv/all/{ym}?type=daily_treasury_yield_curve&field_tdr_date_value_month={ym}&page&_format=csv")
}

/// The 10-year yields of a month's file: (day, percent), newest first.
pub fn ten_year(csv: &str) -> Result<Vec<(i64, f64)>, String> {
    let mut lines = csv.lines();
    let head: Vec<String> = lines.next().unwrap_or("").split(',').map(|h| h.trim().trim_matches('"').to_string()).collect();
    let col = head.iter().position(|h| h.eq_ignore_ascii_case("10 Yr")).ok_or("the Treasury's file has no 10 Yr column")?;
    let mut out: Vec<(i64, f64)> = lines
        .filter_map(|l| {
            let cells: Vec<&str> = l.split(',').collect();
            Some((when::parse_date(cells.first()?)?, cells.get(col)?.trim().trim_matches('"').parse::<f64>().ok().filter(|y| y.is_finite())?))
        })
        .collect();
    out.sort_by_key(|r| std::cmp::Reverse(r.0));
    Ok(out)
}

/// The yields of the month `at` is in, with last month's when this one has too few yet.
fn yields(web: &dyn Fetch, at: i64) -> Result<Vec<(i64, f64)>, String> {
    let mut rows = ten_year(&web.get(&yield_url(at))?)?;
    if rows.len() < 3 {
        let (y, m, _) = when::ymd(at);
        let before = when::at(y, m, 1, 0, 0) - DAY;
        if let Ok(more) = web.get(&yield_url(before)).and_then(|t| ten_year(&t)) {
            rows.extend(more);
        }
    }
    if rows.is_empty() { Err("the Treasury's file has no yields".into()) } else { Ok(rows) }
}

const DEBT: &str = "https://api.fiscaldata.treasury.gov/services/api/fiscal_service/v2/accounting/od/debt_to_penny";

/// The debt as last recorded on or before a day: (its day, dollars).
fn debt(web: &dyn Fetch, on_or_before: i64) -> Result<(i64, f64), String> {
    let t = web.get(&format!("{DEBT}?fields=record_date,tot_pub_debt_out_amt&filter=record_date:lte:{}&sort=-record_date&page[size]=1", when::iso_date(on_or_before)))?;
    let v = super::json(&t, "the Treasury")?;
    let r = &v["data"][0];
    match (r["record_date"].as_str().and_then(when::parse_date), num(&r["tot_pub_debt_out_amt"])) {
        (Some(day), Some(amount)) if amount > 0.0 => Ok((day, amount)),
        _ => Err("the Treasury's debt record has no amount".into()),
    }
}

/// "$40.7 trillion"
fn trillions(dollars: f64) -> String {
    let t = dollars / 1e12;
    if (t * 10.0).round() / 10.0 == (t * 100.0).round() / 100.0 { format!("${t:.1} trillion") } else { format!("${t:.2} trillion") }
}

/// The first day of the month after a moment's.
fn next_month(ms: i64) -> i64 {
    let (y, m, _) = when::ymd(ms);
    if m == 12 { when::at(y + 1, 1, 1, 0, 0) } else { when::at(y, m + 1, 1, 0, 0) }
}

impl Source for Treasury {
    fn name(&self) -> &'static str {
        "treasury"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        3 * HOUR
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], _: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        // The 10-year on next week's Friday: over or under where it is now.
        let week = next_week(now);
        let friday = week + 4 * DAY;
        let id = format!("topical:ust10:{}:above", when::yyyymmdd(friday));
        if !mine.iter().any(|k| k.prop.id == id) && week - now > HOUR {
            match yields(web, now) {
                Ok(rows) => {
                    let (day, last) = rows[0];
                    // (To the nearest five hundredths: a line the yield is about as likely over as under.)
                    let line = (last * 20.0).round() / 20.0;
                    let days = ((friday - day) as f64 / DAY as f64).max(1.0);
                    let sd = 0.055 * (days * 5.0 / 7.0).sqrt();
                    let over = phi((last - line) / sd);
                    let mut p = base(&format!("ust10:{}", when::yyyymmdd(week)), "The 10-year Treasury yield", week, friday, "US Treasury");
                    p.id = id;
                    p.q = format!("The 10-year Treasury yield above {line:.2}% on {}?", when::day_and_date(friday));
                    p.outcomes = vec!["Yes".into(), "No".into()];
                    p.prior = vec![over, 1.0 - over];
                    p.detail = format!("{last:.2}% on {}", when::day_and_date(day));
                    p.rules = format!("Yes if the 10-year par yield the US Treasury publishes for {} (its Daily Treasury Par Yield Curve Rates, home.treasury.gov) is above {line:.2}%; No if it's {line:.2}% or below. If it publishes none for that day, the last one it published that week counts. Called off if it published none that week.", when::day_and_date(friday));
                    out.open(p, json!({"kind": "ust10", "line": line, "week": week}));
                }
                Err(e) => out.trouble.push(e),
            }
        }
        // The debt at the end of next month: over or under where it's heading.
        let month = next_month(now);
        let end = next_month(month) - DAY;
        let id = format!("topical:debt:{}", &when::yyyymmdd(month)[..6]);
        if !mine.iter().any(|k| k.prop.id == id) && month - now > DAY {
            match debt(web, now).and_then(|latest| Ok((latest, debt(web, latest.0 - 45 * DAY)?))) {
                Ok(((day, amount), (day0, amount0))) => {
                    // Where it's heading, at the pace of the last six weeks or so.
                    let pace = (amount - amount0) / ((day - day0) as f64 / DAY as f64).max(1.0);
                    let heading = amount + pace * ((end - day) as f64 / DAY as f64);
                    let line = (heading / 5e10).round() * 5e10;
                    let over = phi((heading - line) / 9e10);
                    let (_, m, _) = when::ymd(month);
                    let month_name = when::month_day(month).split(' ').next().unwrap_or("").to_string();
                    let mut p = base("debt", "The US national debt", month, end, "US Treasury");
                    p.id = id;
                    p.q = format!("The US national debt above {} at the end of {month_name}?", trillions(line));
                    p.outcomes = vec!["Yes".into(), "No".into()];
                    p.prior = vec![over, 1.0 - over];
                    p.detail = format!("{} on {}", trillions(amount), when::day_and_date(day));
                    p.rules = format!("Yes if the total public debt outstanding in the last Debt to the Penny record the US Treasury publishes for {month_name} (fiscaldata.treasury.gov) is above {}; No if it's that or less.", trillions(line));
                    out.open(p, json!({"kind": "debt", "line": line, "from": month, "to": end, "month": m}));
                }
                Err(e) => out.trouble.push(e),
            }
        }
        for k in mine {
            let id = &k.prop.id;
            let Some(line) = k.memo["line"].as_f64() else { continue };
            if k.memo["kind"] == "ust10" {
                let Some(week) = k.memo["week"].as_i64() else { continue };
                let friday = week + 4 * DAY;
                // (Published in the evening, US time: asked from Saturday's start.)
                if now < friday + DAY {
                    continue;
                }
                match yields(web, friday).map(|rows| rows.into_iter().filter(|r| r.0 >= week && r.0 <= friday).collect::<Vec<_>>()) {
                    Ok(rows) => match rows.first() {
                        Some((day, y)) if *day == friday || now > friday + 4 * DAY => out.settle(id, if *y > line { 0 } else { 1 }, format!("{y:.2}% on {}", when::day_and_date(*day))),
                        None if now > friday + 4 * DAY => out.call_off(id, "the Treasury published no yield that week"),
                        _ => {}
                    },
                    Err(e) => out.trouble.push(e),
                }
            } else if let (Some(from), Some(to)) = (k.memo["from"].as_i64(), k.memo["to"].as_i64()) {
                // (A record comes a business day or two after its day: asked from the 4th.)
                if now < to + 4 * DAY {
                    continue;
                }
                match debt(web, to) {
                    Ok((day, amount)) if day >= from => out.settle(id, if amount > line { 0 } else { 1 }, format!("{} on {}", trillions(amount), when::day_and_date(day))),
                    Ok(_) => out.call_off(id, "the Treasury published no record that month"),
                    Err(e) => out.trouble.push(e),
                }
            }
        }
        if out.open.is_empty() && out.results.is_empty() && out.trouble.len() >= 2 {
            return Err(out.trouble.join("; "));
        }
        Ok(out)
    }
}

// ------------------------------------------------------------------ money

/// What the euro, the yen and the pound are worth on next week's Friday, by the European
/// Central Bank's reference rates.
pub struct Money;

struct Pair {
    id: &'static str,
    /// "The euro above $1.1000", "The dollar above ¥159.50".
    what: &'static str,
    /// How a line reads, and its step.
    unit: &'static str,
    step: f64,
    places: usize,
    /// How far it moves in a week, as a share.
    weekly: f64,
}

const PAIRS: [Pair; 3] = [
    Pair { id: "eurusd", what: "The euro", unit: "$", step: 0.005, places: 3, weekly: 0.010 },
    Pair { id: "usdjpy", what: "The dollar", unit: "¥", step: 0.5, places: 1, weekly: 0.013 },
    Pair { id: "gbpusd", what: "The pound", unit: "$", step: 0.005, places: 3, weekly: 0.011 },
];

/// The three rates of a day's reference rates (euro in dollars, dollar in yen, pound in
/// dollars), and the day they're for. `day`: None for the latest.
fn rates(web: &dyn Fetch, day: Option<i64>) -> Result<(i64, [f64; 3]), String> {
    let which = day.map_or("latest".to_string(), when::iso_date);
    let v = super::json(&web.get(&format!("https://api.frankfurter.dev/v1/{which}?base=EUR&symbols=USD,GBP,JPY"))?, "Frankfurter")?;
    let r = &v["rates"];
    match (v["date"].as_str().and_then(when::parse_date), num(&r["USD"]), num(&r["GBP"]), num(&r["JPY"])) {
        (Some(d), Some(usd), Some(gbp), Some(jpy)) if usd > 0.0 && gbp > 0.0 && jpy > 0.0 => Ok((d, [usd, jpy / usd, usd / gbp])),
        _ => Err("Frankfurter's rates are missing one".into()),
    }
}

impl Source for Money {
    fn name(&self) -> &'static str {
        "money"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        3 * HOUR
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], _: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        let week = next_week(now);
        let friday = week + 4 * DAY;
        let tag = when::yyyymmdd(friday);
        if !mine.iter().any(|k| k.memo["week"] == week) && week - now > HOUR {
            let (day, now_at) = rates(web, None)?;
            for (i, pair) in PAIRS.iter().enumerate() {
                let rate = now_at[i];
                let line = (rate / pair.step).round() * pair.step;
                let weeks = ((friday - day) as f64 / (7 * DAY) as f64).max(0.2);
                let over = phi((rate - line) / (rate * pair.weekly * weeks.sqrt()));
                let shown = format!("{}{line:.*}", pair.unit, pair.places);
                let mut p = base(&format!("money:{}", when::yyyymmdd(week)), "Currencies", week, friday, "European Central Bank");
                p.id = format!("topical:{}:{tag}:above", pair.id);
                p.q = format!("{} above {shown} on {}?", pair.what, when::day_and_date(friday));
                p.outcomes = vec!["Yes".into(), "No".into()];
                p.prior = vec![over, 1.0 - over];
                p.detail = format!("{}{rate:.*} on {}", pair.unit, pair.places + 1, when::day_and_date(day));
                p.rules = format!("By the European Central Bank's euro reference rates for {} (as frankfurter.dev serves them){}: Yes if it's above {shown}, No if it's that or below. If the bank sets none that day, the last it set that week counts. Called off if it set none that week.", when::day_and_date(friday), match pair.id {
                    "usdjpy" => ", the yen's rate divided by the dollar's",
                    "gbpusd" => ", the dollar's rate divided by the pound's",
                    _ => ", the dollar's rate",
                });
                out.open(p, json!({"kind": pair.id, "pair": i, "line": line, "week": week}));
            }
        }
        // (Set at about 14:15 UTC and served a little after: asked from 17:00.)
        let due: Vec<&&Kept> = mine.iter().filter(|k| k.memo["week"].as_i64().is_some_and(|w| now >= w + 4 * DAY + 17 * HOUR)).collect();
        let mut by_week: std::collections::BTreeMap<i64, Result<(i64, [f64; 3]), String>> = std::collections::BTreeMap::new();
        for k in due {
            let (Some(week), Some(i), Some(line)) = (k.memo["week"].as_i64(), k.memo["pair"].as_u64().map(|i| i as usize).filter(|i| *i < PAIRS.len()), k.memo["line"].as_f64()) else { continue };
            let friday = week + 4 * DAY;
            match by_week.entry(week).or_insert_with(|| rates(web, Some(friday))) {
                // (A day with no rates is answered with the last day that had them.)
                Ok((day, r)) if *day >= week && (*day == friday || now > friday + 2 * DAY) => {
                    let pair = &PAIRS[i];
                    out.settle(&k.prop.id, if r[i] > line + 1e-12 { 0 } else { 1 }, format!("{}{:.*} on {}", pair.unit, pair.places + 1, r[i], when::day_and_date(*day)));
                }
                Ok((day, _)) if *day < week && now > friday + 2 * DAY => out.call_off(&k.prop.id, "the bank set no rates that week"),
                Ok(_) => {}
                Err(e) => out.trouble.push(e.clone()),
            }
        }
        out.trouble.dedup();
        Ok(out)
    }
}

// ------------------------------------------------------------------ executive orders

/// How many executive orders the Federal Register publishes next week.
pub struct Orders;

fn orders(web: &dyn Fetch, from: i64, to: i64) -> Result<u32, String> {
    let url = format!(
        "https://www.federalregister.gov/api/v1/documents.json?per_page=1&conditions[presidential_document_type][]=executive_order&conditions[publication_date][gte]={}&conditions[publication_date][lte]={}",
        when::iso_date(from),
        when::iso_date(to)
    );
    let v = super::json(&web.get(&url)?, "the Federal Register")?;
    match (num(&v["count"]), v["results"].as_array()) {
        (Some(n), _) => Ok(n as u32),
        // (None found: it may leave the count out.)
        (None, None) if v.is_object() => Ok(0),
        (None, Some(r)) => Ok(r.len() as u32),
        _ => Err("the Federal Register's answer has no count".into()),
    }
}

impl Source for Orders {
    fn name(&self) -> &'static str {
        "orders"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        6 * HOUR
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], _: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        let week = next_week(now);
        let id = format!("topical:orders:{}:count", when::yyyymmdd(week));
        if !mine.iter().any(|k| k.prop.id == id) && week - now > HOUR {
            // The last eight weeks' pace.
            let n = orders(web, now - 56 * DAY, now)?;
            let mean = (n as f64 / 8.0).max(0.15);
            let mut p = base(&format!("orders:{}", when::yyyymmdd(week)), "Executive orders", week, week, "Federal Register");
            p.id = id;
            p.q = format!("How many executive orders are published in {}?", week_of(week));
            p.outcomes = vec!["None".into(), "One".into(), "Two or three".into(), "Four or more".into()];
            p.prior = vec![poisson_between(mean, 0, Some(0)), poisson_between(mean, 1, Some(1)), poisson_between(mean, 2, Some(3)), poisson_between(mean, 4, None)];
            p.detail = format!("{n} in the last eight weeks");
            p.rules = format!("Executive orders with a publication date from {} to the Sunday after, as the Federal Register (federalregister.gov) lists them the day after that week ends. An order counts in the week it's published, not the week it's signed.", when::day_and_date(week));
            out.open(p, json!({"week": week}));
        }
        for k in mine {
            let Some(w) = k.memo["week"].as_i64().filter(|w| now >= w + 8 * DAY) else { continue };
            match orders(web, w, w + 6 * DAY) {
                Ok(n) => out.settle(&k.prop.id, match n {
                    0 => 0,
                    1 => 1,
                    2 | 3 => 2,
                    _ => 3,
                }, format!("{n} published")),
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

    const YIELDS: &str = "Date,\"1 Mo\",\"1.5 Month\",\"2 Mo\",\"3 Mo\",\"4 Mo\",\"6 Mo\",\"1 Yr\",\"2 Yr\",\"3 Yr\",\"5 Yr\",\"7 Yr\",\"10 Yr\",\"20 Yr\",\"30 Yr\"\n11/16/2026,4.10,4.19,4.27,4.34,4.33,4.42,4.62,4.83,4.87,4.93,5.01,5.12,5.51,5.46\n11/13/2026,4.10,4.18,4.26,4.32,4.34,4.42,4.65,4.88,4.92,4.98,5.05,5.15,5.52,5.47\n11/12/2026,4.12,4.18,4.26,4.32,4.35,4.42,4.65,4.86,4.90,4.95,5.02,5.10,5.48,5.43\n";
    const DEBT_NOW: &str = r#"{"data":[{"record_date":"2026-11-12","tot_pub_debt_out_amt":"40608506856903.25"}],"meta":{"count":1,"total-count":8433},"links":{"self":"&page%5Bnumber%5D=1&page%5Bsize%5D=1"}}"#;
    const DEBT_BEFORE: &str = r#"{"data":[{"record_date":"2026-09-28","tot_pub_debt_out_amt":"40308506856903.25"}],"meta":{"count":1}}"#;
    const RATES: &str = r#"{"amount":1.0,"base":"EUR","date":"2026-11-16","rates":{"GBP":0.8345,"JPY":175.78,"USD":1.1014}}"#;

    fn result(m: &Maker, now: i64, id: &str) -> pyr_proto::props_feed::FeedResult {
        m.feed(now).props.into_iter().find(|p| p.id == id).unwrap_or_else(|| panic!("no {id}")).result.unwrap_or_else(|| panic!("{id} isn't over"))
    }

    #[test]
    fn the_treasury_s_files_read_as_they_come() {
        let rows = ten_year(YIELDS).unwrap();
        assert_eq!(rows, vec![(when::at(2026, 11, 16, 0, 0), 5.12), (when::at(2026, 11, 13, 0, 0), 5.15), (when::at(2026, 11, 12, 0, 0), 5.10)]);
        assert!(ten_year("Date,\"1 Mo\"\n11/16/2026,4.10\n").is_err());
        assert_eq!(ten_year("Date,\"10 Yr\"\n").unwrap(), vec![]);
        assert!(yield_url(when::at(2026, 11, 17, 0, 0)).contains("/all/202611?type=daily_treasury_yield_curve&field_tdr_date_value_month=202611"));
        assert_eq!((trillions(4.07e13), trillions(4.075e13)), ("$40.7 trillion".to_string(), "$40.75 trillion".to_string()));
        assert_eq!(next_month(when::at(2026, 12, 9, 3, 0)), when::at(2027, 1, 1, 0, 0));
    }

    #[test]
    fn the_ten_year_and_the_debt_open_at_fair_lines_and_settle_from_the_treasury() {
        let now = when::at(2026, 11, 17, 12, 0);
        let web = Canned::new(&[("daily-treasury-rates.csv", YIELDS), ("record_date:lte:2026-11-17", DEBT_NOW), ("record_date:lte:2026-09-28", DEBT_BEFORE)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Treasury)]);
        assert!(m.step(now, &web, false));
        let props = m.feed(now).props;
        assert_eq!(props.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["topical:ust10:20261127:above", "topical:debt:202612"]);
        let ten = &props[0];
        assert_eq!((ten.q.as_str(), ten.closes, ten.starts, ten.detail.as_str()), ("The 10-year Treasury yield above 5.10% on Friday, November 27?", when::at(2026, 11, 23, 0, 0), when::at(2026, 11, 27, 0, 0), "5.12% on Monday, November 16"));
        assert!(ten.prior[0] > 0.5 && ten.prior[0] < 0.65, "5.12 against a line of 5.10: {:?}", ten.prior);
        // The debt: up $300 billion in 45 days, so by the end of December about $40.94 trillion.
        let debt = &props[1];
        assert_eq!((debt.q.as_str(), debt.closes, debt.starts), ("The US national debt above $40.95 trillion at the end of December?", when::at(2026, 12, 1, 0, 0), when::at(2026, 12, 31, 0, 0)));
        assert_eq!(debt.detail, "$40.61 trillion on Thursday, November 12");
        assert!(debt.prior[0] > 0.4 && debt.prior[0] < 0.55, "{:?}", debt.prior);
        // Saturday after that Friday: 5.08% published for it: No.
        let then = when::at(2026, 11, 28, 6, 0);
        let later = YIELDS.replace("11/16/2026,", "11/27/2026,").replace(",5.12,", ",5.08,");
        let web = Canned::new(&[("daily-treasury-rates.csv", later.as_str()), ("record_date:lte", DEBT_NOW)]);
        assert!(m.step(then, &web, false));
        let r = result(&m, then, "topical:ust10:20261127:above");
        assert_eq!((r.winner, r.note.as_str()), (Some(1), "5.08% on Friday, November 27"));
        // January 4th: December's last record, $41.02 trillion: Yes.
        let jan = when::at(2027, 1, 4, 6, 0);
        let dec = r#"{"data":[{"record_date":"2026-12-30","tot_pub_debt_out_amt":"41020000000000.00"}]}"#;
        let web = Canned::new(&[("record_date:lte:2026-12-31", dec), ("daily-treasury-rates.csv", YIELDS), ("record_date:lte", DEBT_NOW)]);
        assert!(m.step(jan, &web, false));
        let r = result(&m, jan, "topical:debt:202612");
        assert_eq!((r.winner, r.note.as_str()), (Some(0), "$41.02 trillion on Wednesday, December 30"));
    }

    #[test]
    fn a_week_with_no_friday_yield_takes_the_last_of_the_week_or_is_called_off() {
        let now = when::at(2026, 11, 17, 12, 0);
        let web = Canned::new(&[("daily-treasury-rates.csv", YIELDS), ("record_date:lte", DEBT_NOW)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Treasury)]);
        m.step(now, &web, false);
        // Thursday the 26th is the last published that week (a holiday Friday, say): it
        // counts once four days have gone by without Friday's.
        let thu = YIELDS.replace("11/16/2026,", "11/26/2026,").replace(",5.12,", ",5.20,");
        let web = Canned::new(&[("daily-treasury-rates.csv", thu.as_str()), ("record_date:lte", DEBT_NOW)]);
        let sat = when::at(2026, 11, 28, 6, 0);
        m.step(sat, &web, false);
        assert!(m.feed(sat).props.iter().find(|p| p.id == "topical:ust10:20261127:above").unwrap().result.is_none(), "not yet: Friday's may still come");
        let wed = when::at(2026, 12, 2, 6, 0);
        m.step(wed, &web, false);
        let r = result(&m, wed, "topical:ust10:20261127:above");
        assert_eq!((r.winner, r.note.as_str()), (Some(0), "5.20% on Thursday, November 26"));
        // None at all that week: called off.
        let mut m = Maker::new(State::default(), vec![Box::new(Treasury)]);
        m.step(now, &Canned::new(&[("daily-treasury-rates.csv", YIELDS), ("record_date:lte", DEBT_NOW)]), false);
        m.step(wed, &Canned::new(&[("daily-treasury-rates.csv", YIELDS), ("record_date:lte", DEBT_NOW)]), false);
        assert_eq!(result(&m, wed, "topical:ust10:20261127:above").winner, None);
    }

    #[test]
    fn the_euro_the_yen_and_the_pound_go_by_the_bank_s_rates() {
        let now = when::at(2026, 11, 17, 12, 0);
        let web = Canned::new(&[("frankfurter.dev/v1/latest", RATES)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Money)]);
        assert!(m.step(now, &web, false));
        let props = m.feed(now).props;
        let q: Vec<&str> = props.iter().map(|p| p.q.as_str()).collect();
        assert_eq!(q, vec!["The euro above $1.100 on Friday, November 27?", "The pound above $1.320 on Friday, November 27?", "The dollar above ¥159.5 on Friday, November 27?"]);
        assert_eq!(props[0].detail, "$1.1014 on Monday, November 16");
        assert_eq!(props[2].detail, "¥159.60 on Monday, November 16");
        assert!(props.iter().all(|p| p.closes == when::at(2026, 11, 23, 0, 0) && p.prior[0] > 0.4 && p.prior[0] < 0.6), "{props:?}");
        // That Friday at six in the evening: the euro at 1.0950, the yen weaker, the pound up.
        let then = when::at(2026, 11, 27, 18, 0);
        let fri = r#"{"amount":1.0,"base":"EUR","date":"2026-11-27","rates":{"GBP":0.8200,"JPY":176.30,"USD":1.0950}}"#;
        let web = Canned::new(&[("frankfurter.dev/v1/2026-11-27", fri), ("frankfurter.dev/v1/latest", RATES)]);
        assert!(m.step(then, &web, false));
        assert_eq!(web.asked().iter().filter(|u| u.contains("2026-11-27")).count(), 1, "one request settles all three");
        let r = result(&m, then, "topical:eurusd:20261127:above");
        assert_eq!((r.winner, r.note.as_str()), (Some(1), "$1.0950 on Friday, November 27"));
        assert_eq!(result(&m, then, "topical:usdjpy:20261127:above").note, "¥161.00 on Friday, November 27");
        assert_eq!(result(&m, then, "topical:usdjpy:20261127:above").winner, Some(0));
        assert_eq!(result(&m, then, "topical:gbpusd:20261127:above").winner, Some(0));
    }

    #[test]
    fn executive_orders_are_counted_by_the_week_they_re_published() {
        let now = when::at(2026, 11, 17, 12, 0);
        let web = Canned::new(&[("federalregister.gov", r#"{"description":"Documents published on or after 09/22/2026","count":7,"total_pages":7,"results":[{"title":"Decreasing Housing Costs for the American People","type":"Presidential Document","document_number":"2026-22051","publication_date":"2026-10-28"}]}"#)]);
        let mut m = Maker::new(State::default(), vec![Box::new(Orders)]);
        assert!(m.step(now, &web, false));
        assert!(web.asked()[0].ends_with("conditions[publication_date][gte]=2026-09-22&conditions[publication_date][lte]=2026-11-17"), "{:?}", web.asked());
        let p = m.feed(now).props.remove(0);
        assert_eq!((p.id.as_str(), p.q.as_str(), p.detail.as_str()), ("topical:orders:20261123:count", "How many executive orders are published in the week of November 23?", "7 in the last eight weeks"));
        // Seven in eight weeks: most weeks none or one.
        assert!(p.prior[0] > 0.38 && p.prior[0] < 0.45 && p.prior[3] >= 0.025, "{:?}", p.prior);
        // The Tuesday after that week: two were published in it.
        let then = when::at(2026, 12, 1, 12, 0);
        let web = Canned::new(&[("[gte]=2026-11-23&conditions[publication_date][lte]=2026-11-29", r#"{"count":2,"results":[{}]}"#), ("federalregister.gov", r#"{"count":0}"#)]);
        assert!(m.step(then, &web, false));
        let r = result(&m, then, "topical:orders:20261123:count");
        assert_eq!((r.winner, r.note.as_str()), (Some(2), "2 published"));
        // A week with none: its count may be left out.
        assert_eq!(orders(&Canned::new(&[("federalregister", r#"{"description":"none"}"#)]), now, now), Ok(0));
    }
}
