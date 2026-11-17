//! How numbers and times read.

use chrono::{Datelike, Local, TimeZone};

/// "12,345"
pub fn chips_text(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// "+1,234", "-560", "0".
pub fn signed(n: i64) -> String {
    match n {
        0 => "0".into(),
        n if n > 0 => format!("+{}", chips_text(n as u64)),
        n => format!("-{}", chips_text(n.unsigned_abs())),
    }
}

/// A big number in few characters: "950", "12.3K", "4.6M", "1.2B".
pub fn short_chips(n: u64) -> String {
    let (x, unit) = match n {
        0..=999 => return n.to_string(),
        1_000..=999_999 => (n as f64 / 1e3, "K"),
        1_000_000..=999_999_999 => (n as f64 / 1e6, "M"),
        _ => (n as f64 / 1e9, "B"),
    };
    if x >= 100.0 { format!("{x:.0}{unit}") } else { format!("{x:.1}{unit}").replace(".0", "") }
}

/// "1 bet", "1,204 bets".
pub fn count(n: u64, one: &str, many: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{} {many}", chips_text(n)) }
}

/// "62%" (never 0% or 100% for something that can still go either way).
pub fn pct(x: f64) -> String {
    format!("{}%", ((x * 100.0).round() as i64).clamp(1, 99))
}

/// The time left, closely: "2 d 4 h", "3 h 12 min", "14:05" under an hour.
pub fn countdown(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    if s >= 2 * 86_400 {
        format!("{} d {} h", s / 86_400, s % 86_400 / 3600)
    } else if s >= 3600 {
        format!("{} h {} min", s / 3600, s % 3600 / 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// How long ago: "just now", "5 min ago", "3 h ago", "2 days ago".
pub fn ago(ms: i64) -> String {
    let m = ms / 60_000;
    if m < 1 {
        "just now".into()
    } else if m < 60 {
        format!("{m} min ago")
    } else if m < 48 * 60 {
        format!("{} h ago", m / 60)
    } else {
        format!("{} days ago", m / 1440)
    }
}

/// A moment in the viewer's own time, said the short way: "Today 7:30 PM", "Tomorrow 1:00 PM",
/// "Sun 4:25 PM", "Nov 27, 8:00 PM".
pub fn local_when(ms: i64, now: i64) -> String {
    let (Some(t), Some(n)) = (Local.timestamp_millis_opt(ms).single(), Local.timestamp_millis_opt(now).single()) else { return String::new() };
    let days = t.date_naive().signed_duration_since(n.date_naive()).num_days();
    let time = t.format("%-I:%M %p").to_string();
    match days {
        0 => format!("Today {time}"),
        1 => format!("Tomorrow {time}"),
        -1 => format!("Yesterday {time}"),
        2..=6 => format!("{} {time}", t.format("%a")),
        _ if t.year() == n.year() => format!("{}, {time}", t.format("%b %-d")),
        _ => format!("{}, {time}", t.format("%b %-d %Y")),
    }
}

/// A moment as `local_when` says it, to go in the middle of a sentence: "today 7:30 PM",
/// "Sun 4:25 PM".
pub fn mid_sentence(when: &str) -> String {
    for day in ["Today", "Tomorrow", "Yesterday"] {
        if let Some(rest) = when.strip_prefix(day) {
            return format!("{}{rest}", day.to_lowercase());
        }
    }
    when.to_string()
}

/// What a chip on something this likely pays, about: "×2.38", "×12.5".
pub fn times(price: f64) -> String {
    let x = 1.0 / price.max(0.001);
    if x >= 10.0 { format!("×{x:.1}") } else { format!("×{x:.2}") }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_read_as_in_pyr() {
        assert_eq!(chips_text(1_234_567), "1,234,567");
        assert_eq!((signed(1234), signed(-560), signed(0)), ("+1,234".to_string(), "-560".to_string(), "0".to_string()));
        assert_eq!((short_chips(950), short_chips(12_300), short_chips(4_600_000), short_chips(1_200_000_000), short_chips(2_000), short_chips(250_000)), ("950".into(), "12.3K".into(), "4.6M".into(), "1.2B".into(), "2K".into(), "250K".into()));
        assert_eq!(count(1, "bet", "bets"), "1 bet");
        assert_eq!(count(1204, "bet", "bets"), "1,204 bets");
        assert_eq!((pct(0.625), pct(0.001), pct(0.999)), ("63%".to_string(), "1%".to_string(), "99%".to_string()));
        assert_eq!((countdown(3 * 86_400_000 + 5 * 3_600_000), countdown(3 * 3_600_000 + 12 * 60_000), countdown(14 * 60_000 + 5_000), countdown(-5)), ("3 d 5 h".to_string(), "3 h 12 min".to_string(), "14:05".to_string(), "0:00".to_string()));
        assert_eq!((ago(20_000), ago(5 * 60_000), ago(3 * 3_600_000), ago(3 * 86_400_000)), ("just now".to_string(), "5 min ago".to_string(), "3 h ago".to_string(), "3 days ago".to_string()));
        // (Whatever the time zone here: the same day says Today.)
        let now = now_ms();
        assert!(local_when(now, now).starts_with("Today "));
        assert_eq!((mid_sentence("Today 7:30 PM"), mid_sentence("Sun 4:25 PM")), ("today 7:30 PM".to_string(), "Sun 4:25 PM".to_string()));
        assert_eq!((times(0.42), times(0.08), times(0.5)), ("×2.38".to_string(), "×12.5".to_string(), "×2.00".to_string()));
    }
}
