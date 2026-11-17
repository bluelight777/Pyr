//! The sources: each a kind of props, where they come from and how they come out.

use serde_json::Value;

use crate::engine::Source;

pub mod art;
pub mod crypto;
pub mod espn;
pub mod science;
pub mod sports;
pub mod steam;
pub mod topical;
pub mod weather;

/// Every source, as the service runs them (`only`: just the ones named, for `check`).
pub fn all(only: &[String]) -> Vec<Box<dyn Source>> {
    let every: Vec<Box<dyn Source>> = vec![
        Box::new(sports::Sports::new("nfl")),
        Box::new(sports::Sports::new("nba")),
        Box::new(sports::Sports::new("nhl")),
        Box::new(sports::Sports::new("mlb")),
        Box::new(sports::Sports::new("cfb")),
        Box::new(sports::Sports::new("cbb")),
        Box::new(crypto::Crypto),
        Box::new(weather::Weather),
        Box::new(science::Quakes),
        Box::new(science::SpaceWeather),
        Box::new(science::Launches),
        Box::new(science::Asteroids),
        Box::new(topical::Treasury),
        Box::new(topical::Money),
        Box::new(topical::Orders),
        Box::new(steam::Steam),
        Box::new(art::Gallery),
        Box::new(art::Picture),
    ];
    every.into_iter().filter(|s| only.is_empty() || only.iter().any(|o| o == s.name())).collect()
}

/// A number, however the source wrote it: 4.5, "4.5", "-230", "+190", "o39.5", "1,234".
pub fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            let t: String = s.trim().chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'e' | 'E')).collect();
            // (A sign in the middle isn't a number: "2026-11-17".)
            if t.is_empty() || t[1..].contains('-') && !t.contains(['e', 'E']) { None } else { t.parse().ok().filter(|x: &f64| x.is_finite()) }
        }
        _ => None,
    }
}

/// Reads a source's answer as JSON, saying whose it was when it isn't.
pub fn json(text: &str, whose: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| format!("{whose} answered something that isn't JSON: {e}"))
}

/// A name cut to fit an outcome (30 characters), on a word where it can be.
pub fn short(name: &str, max: usize) -> String {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.chars().count() <= max {
        return name;
    }
    let cut: String = name.chars().take(max - 1).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i >= max / 2 => cut[..i].to_string(),
        _ => cut,
    };
    format!("{}…", cut.trim_end_matches([' ', ',', '-']))
}

/// Dollars as people say them: 65231.19 as "$65,231", 0.1523 as "$0.1523", 2.31 as "$2.31".
pub fn dollars(x: f64) -> String {
    if x >= 1000.0 {
        format!("${}", crate::math::commas(x.round() as u64))
    } else if x >= 1.0 {
        format!("${x:.2}")
    } else {
        format!("${x:.4}")
    }
}

/// The week after this one: its Monday at 00:00 UTC. Props about "next week" open during this
/// one and close as theirs starts, so nothing's bet once any of it has happened.
pub fn next_week(now: i64) -> i64 {
    crate::when::week_start(now) + 7 * crate::when::DAY
}

/// "the week of November 23" (the week that starts on that Monday, UTC).
pub fn week_of(monday: i64) -> String {
    format!("the week of {}", crate::when::month_day(monday))
}

/// Four ranges for a count that's `mean` on average: where the second, third and fourth
/// start, their names ("4 or fewer", "5 or 6", "7 or 8", "9 or more") and their chances
/// (things that happen independently: a Poisson count).
pub fn count_ranges(mean: f64) -> (Vec<u32>, Vec<String>, Vec<f64>) {
    let mean = mean.clamp(0.3, 400.0);
    let m = (mean.round() as u32).max(3);
    let w = ((mean.sqrt() * 0.8).round() as u32).max(2).min(m - 1);
    let starts = vec![m - w + 1, m + 1, m + w + 1];
    let span = |a: u32, b: u32| if b == a + 1 { format!("{a} or {b}") } else { format!("{a} to {b}") };
    let names = vec![format!("{} or fewer", starts[0] - 1), span(starts[0], starts[1] - 1), span(starts[1], starts[2] - 1), format!("{} or more", starts[2])];
    let p = crate::math::poisson_between;
    let chances = vec![p(mean, 0, Some(starts[0] - 1)), p(mean, starts[0], Some(starts[1] - 1)), p(mean, starts[1], Some(starts[2] - 1)), p(mean, starts[2], None)];
    (starts, names, chances)
}

/// Which of the ranges a count is in.
pub fn count_range_of(starts: &[u32], n: u32) -> usize {
    starts.iter().filter(|s| n >= **s).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_get_four_ranges_around_what_s_usual() {
        let (starts, names, chances) = count_ranges(6.2);
        assert_eq!(starts, vec![5, 7, 9]);
        assert_eq!(names, vec!["4 or fewer", "5 or 6", "7 or 8", "9 or more"]);
        assert!((chances.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert!(chances.iter().all(|c| *c > 0.1 && *c < 0.4), "{chances:?}");
        assert_eq!((count_range_of(&starts, 0), count_range_of(&starts, 4), count_range_of(&starts, 5), count_range_of(&starts, 8), count_range_of(&starts, 30)), (0, 0, 1, 2, 3));
        // Few: never fewer than "1 or fewer"; many: wider ranges.
        assert_eq!(count_ranges(0.8).1, vec!["1 or fewer", "2 or 3", "4 or 5", "6 or more"]);
        assert_eq!(count_ranges(2.6).1[1], "2 or 3");
        assert_eq!(count_ranges(40.0).1, vec!["35 or fewer", "36 to 40", "41 to 45", "46 or more"]);
        // Tuesday the 17th: next week is the 23rd's.
        let now = crate::when::at(2026, 11, 17, 12, 0);
        assert_eq!(next_week(now), crate::when::at(2026, 11, 23, 0, 0));
        assert_eq!(week_of(next_week(now)), "the week of November 23");
    }

    #[test]
    fn numbers_are_read_however_they_re_written() {
        assert_eq!(num(&json!(4.5)), Some(4.5));
        assert_eq!(num(&json!("-230")), Some(-230.0));
        assert_eq!(num(&json!("+190")), Some(190.0));
        assert_eq!(num(&json!("o39.5")), Some(39.5));
        assert_eq!(num(&json!("40608506856903.25")), Some(40_608_506_856_903.25));
        assert_eq!(num(&json!("1,234")), Some(1234.0));
        assert_eq!(num(&json!("2026-11-17")), None);
        assert_eq!(num(&json!("EVEN")), None);
        assert_eq!(num(&json!(null)), None);
    }

    #[test]
    fn names_fit_and_money_reads_right() {
        assert_eq!(short("Ravens", 30), "Ravens");
        assert_eq!(short("Falcon 9 Block 5 (Starlink Group 17-34) from Vandenberg", 30), "Falcon 9 Block 5 (Starlink…");
        assert!(short("Supercalifragilisticexpialidocious-and-then-some", 30).chars().count() <= 30);
        assert_eq!(dollars(65_231.19), "$65,231");
        assert_eq!(dollars(2.31), "$2.31");
        assert_eq!(dollars(0.1523), "$0.1523");
        let names: Vec<&str> = all(&[]).iter().map(|s| s.name()).collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "every source has its own name");
        assert_eq!(all(&["crypto".to_string()]).len(), 1);
    }
}
