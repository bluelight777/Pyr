//! Dates and times, by hand (no time-zone database: everything is UTC unless a source gives
//! its own offset, and that offset is used as given).

pub const SEC: i64 = 1000;
pub const MIN: i64 = 60 * SEC;
pub const HOUR: i64 = 60 * MIN;
pub const DAY: i64 = 24 * HOUR;

/// Days since 1970-01-01 of a date (Howard Hinnant's `days_from_civil`).
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day since 1970-01-01.
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A moment (Unix ms) from a UTC date and time.
pub fn at(y: i64, m: u32, d: u32, h: u32, min: u32) -> i64 {
    days_from_civil(y, m, d) * DAY + h as i64 * HOUR + min as i64 * MIN
}

/// The UTC date of a moment.
pub fn ymd(ms: i64) -> (i64, u32, u32) {
    civil_from_days(ms.div_euclid(DAY))
}

/// 00:00 UTC of a moment's day.
pub fn day_start(ms: i64) -> i64 {
    ms.div_euclid(DAY) * DAY
}

/// The day of the week: 0 Monday to 6 Sunday.
pub fn weekday(ms: i64) -> u32 {
    ((ms.div_euclid(DAY) + 3).rem_euclid(7)) as u32
}

/// Monday 00:00 UTC of a moment's week.
pub fn week_start(ms: i64) -> i64 {
    day_start(ms) - weekday(ms) as i64 * DAY
}

const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// "Wednesday"
pub fn day_name(ms: i64) -> &'static str {
    DAYS[weekday(ms) as usize]
}

/// "November 18"
pub fn month_day(ms: i64) -> String {
    let (_, m, d) = ymd(ms);
    format!("{} {d}", MONTHS[m as usize - 1])
}

/// "Wednesday, November 18"
pub fn day_and_date(ms: i64) -> String {
    format!("{}, {}", day_name(ms), month_day(ms))
}

/// "20261118"
pub fn yyyymmdd(ms: i64) -> String {
    let (y, m, d) = ymd(ms);
    format!("{y:04}{m:02}{d:02}")
}

/// "2026-11-18"
pub fn iso_date(ms: i64) -> String {
    let (y, m, d) = ymd(ms);
    format!("{y:04}-{m:02}-{d:02}")
}

/// "2026-11-18T00:00:00Z"
pub fn iso(ms: i64) -> String {
    let (y, m, d) = ymd(ms);
    let t = ms.rem_euclid(DAY);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", t / HOUR, t % HOUR / MIN, t % MIN / SEC)
}

/// "18:00 UTC"
pub fn hhmm_utc(ms: i64) -> String {
    let t = ms.rem_euclid(DAY);
    format!("{:02}:{:02} UTC", t / HOUR, t % HOUR / MIN)
}

/// A month's number from its name or its first three letters ("November", "NOV", "Nov").
pub fn month_number(name: &str) -> Option<u32> {
    let n = name.trim().to_ascii_lowercase();
    if n.len() < 3 {
        return None;
    }
    MONTHS.iter().position(|m| m.to_ascii_lowercase().starts_with(&n[..3]) && m.to_ascii_lowercase().starts_with(&n)).map(|i| i as u32 + 1)
}

/// A date as "20261118" or "2026-11-18" or "11/18/2026": 00:00 UTC of it.
pub fn parse_date(s: &str) -> Option<i64> {
    let s = s.trim();
    let num = |t: &str| t.parse::<i64>().ok();
    let (y, m, d) = if s.len() == 8 && s.bytes().all(|b| b.is_ascii_digit()) {
        (num(&s[..4])?, num(&s[4..6])?, num(&s[6..])?)
    } else if let Some((a, rest)) = s.split_once('-') {
        let (b, c) = rest.split_once('-')?;
        (num(a)?, num(b)?, num(c.get(..2)?)?)
    } else {
        let mut p = s.split('/');
        let (m, d, y) = (num(p.next()?)?, num(p.next()?)?, num(p.next()?)?);
        (y, m, d)
    };
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || !(1970..=2200).contains(&y) {
        return None;
    }
    Some(days_from_civil(y, m as u32, d as u32) * DAY)
}

/// A moment as the sources write them: "2026-11-17T01:15Z", "2026-11-16T21:47:34.557583397Z",
/// "2026-11-18T06:00:00-05:00", "2026-11-09T00:00:00" (no zone: UTC), "2026-11-12" (00:00 UTC).
pub fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim();
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((d, t)) => (d, t),
        None => (s, ""),
    };
    let day = parse_date(date)?;
    if time.is_empty() {
        return Some(day);
    }
    // The zone: Z, +hh:mm, -hh:mm, +hhmm, or none.
    let (clock, offset_min) = if let Some(t) = time.strip_suffix(['Z', 'z']) {
        (t, 0)
    } else if let Some(i) = time.rfind(['+', '-']) {
        let (t, z) = time.split_at(i);
        let sign = if z.starts_with('-') { -1 } else { 1 };
        let digits: String = z[1..].chars().filter(|c| c.is_ascii_digit()).collect();
        if digits.len() < 2 {
            return None;
        }
        let h: i64 = digits[..2].parse().ok()?;
        let m: i64 = digits.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(0);
        (t, sign * (h * 60 + m))
    } else {
        (time, 0)
    };
    let mut parts = clock.split(':');
    let h: i64 = parts.next()?.parse().ok()?;
    let m: i64 = parts.next().map(|x| x.parse().ok()).unwrap_or(Some(0))?;
    let sec: f64 = parts.next().map(|x| x.parse().ok()).unwrap_or(Some(0.0))?;
    if !(0..24).contains(&h) || !(0..60).contains(&m) || !(0.0..61.0).contains(&sec) {
        return None;
    }
    Some(day + h * HOUR + m * MIN + (sec * 1000.0) as i64 - offset_min * MIN)
}

/// The offset from UTC (minutes) a moment was written with ("…-05:00": -300; none or Z: 0).
pub fn offset_minutes(s: &str) -> i64 {
    let Some((_, time)) = s.trim().split_once('T') else { return 0 };
    let Some(i) = time.rfind(['+', '-']) else { return 0 };
    let z = &time[i..];
    let sign = if z.starts_with('-') { -1 } else { 1 };
    let digits: String = z[1..].chars().filter(|c| c.is_ascii_digit()).collect();
    let h: i64 = digits.get(..2).and_then(|x| x.parse().ok()).unwrap_or(0);
    let m: i64 = digits.get(2..4).and_then(|x| x.parse().ok()).unwrap_or(0);
    sign * (h * 60 + m)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_go_there_and_back() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        for (y, m, d) in [(2026, 11, 17), (2024, 2, 29), (2000, 12, 31), (2027, 1, 1), (1999, 3, 1)] {
            assert_eq!(civil_from_days(days_from_civil(y, m, d)), (y, m, d));
        }
        // 2026-11-17 is a Tuesday; its week starts Monday the 16th.
        let t = at(2026, 11, 17, 12, 30);
        assert_eq!((weekday(t), day_name(t)), (1, "Tuesday"));
        assert_eq!(iso_date(week_start(t)), "2026-11-16");
        assert_eq!(iso(t), "2026-11-17T12:30:00Z");
        assert_eq!((yyyymmdd(t), month_day(t), day_and_date(t)), ("20261117".to_string(), "November 17".to_string(), "Tuesday, November 17".to_string()));
        assert_eq!(hhmm_utc(t), "12:30 UTC");
        assert_eq!(day_name(at(2026, 11, 22, 0, 0)), "Sunday");
    }

    #[test]
    fn moments_are_read_as_the_sources_write_them() {
        let t = at(2026, 11, 17, 1, 15);
        assert_eq!(parse_iso("2026-11-17T01:15Z"), Some(t));
        assert_eq!(parse_iso("2026-11-17T01:15:00Z"), Some(t));
        assert_eq!(parse_iso("2026-11-16T20:15:00-05:00"), Some(t));
        assert_eq!(parse_iso("2026-11-17T06:45:00+05:30"), Some(t));
        assert_eq!(parse_iso("2026-11-17T01:15:00"), Some(t));
        assert_eq!(parse_iso("2026-11-16T21:47:34.557583397Z"), Some(at(2026, 11, 16, 21, 47) + 34_557));
        assert_eq!(parse_iso("2026-11-12"), Some(at(2026, 11, 12, 0, 0)));
        assert_eq!(parse_date("11/16/2026"), Some(at(2026, 11, 16, 0, 0)));
        assert_eq!(parse_date("20261116"), Some(at(2026, 11, 16, 0, 0)));
        assert_eq!(parse_iso("yesterday"), None);
        assert_eq!(parse_iso("2026-13-01T00:00Z"), None);
        assert_eq!(offset_minutes("2026-11-18T06:00:00-05:00"), -300);
        assert_eq!(offset_minutes("2026-11-18T06:00:00Z"), 0);
        assert_eq!((month_number("NOV"), month_number("November"), month_number("Nope")), (Some(11), Some(11), None));
    }
}
