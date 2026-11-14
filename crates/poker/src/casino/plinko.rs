//! Plinko (0.13.5), one of Pyr's Originals: a ball dropped through 8 to 16 rows of pegs into the
//! buckets. Four difficulties, Low to Expert (up to 10,000 times the bet on Expert with 16 rows);
//! every table pays back 99%.
//!
//! As 0.13.28 played it, so a round from either can be checked by the other: at each row the
//! ball goes right on an odd number from the round's stream (`Stream::below(2)`), its bucket is
//! how many times it went right, and the bucket's multiple (hundredths) times the ball, rounded
//! down, is what it pays. The tables are 0.13.28's: worked out the way its server works them out
//! (`rebuild/tools/plinko_tables.py`, read from its `pyr-server`), checked against its screen and
//! its own rounds, and written down here so every Pyr has them the same.

use serde::{Deserialize, Serialize};

use crate::fair::Stream;

/// How wild the buckets are (0.13.28's names).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Low,
    #[default]
    Medium,
    High,
    Expert,
}

impl Risk {
    pub const ALL: [Risk; 4] = [Risk::Low, Risk::Medium, Risk::High, Risk::Expert];

    pub fn name(self) -> &'static str {
        match self {
            Risk::Low => "Low",
            Risk::Medium => "Medium",
            Risk::High => "High",
            Risk::Expert => "Expert",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

pub const MIN_ROWS: u8 = 8;
pub const MAX_ROWS: u8 = 16;

/// The least and most a ball may stake.
pub const MIN_BET: u64 = 1;
pub const MAX_BET: u64 = 100_000;

/// The most one ball pays (as 0.13.28).
pub const MAX_PAID: u64 = 50_000_000_000;

/// Every table, in hundredths of the bet: `TABLES[risk][rows - 8]`, bucket 0 at the left.
pub const TABLES: [[&[u64]; 9]; 4] = [
    [
        &[560, 301, 140, 72, 50, 72, 140, 301, 560],
        &[640, 370, 191, 90, 55, 55, 90, 191, 370, 640],
        &[730, 440, 240, 120, 67, 50, 67, 120, 240, 440, 730],
        &[830, 520, 299, 161, 80, 55, 55, 80, 161, 299, 520, 830],
        &[950, 610, 370, 201, 110, 61, 50, 61, 110, 201, 370, 610, 950],
        &[1100, 720, 450, 259, 140, 71, 55, 55, 71, 140, 259, 450, 720, 1100],
        &[1200, 840, 540, 320, 180, 90, 62, 50, 62, 90, 180, 320, 540, 840, 1200],
        &[1400, 970, 640, 390, 220, 121, 70, 54, 54, 70, 121, 220, 390, 640, 970, 1400],
        &[1600, 1100, 750, 480, 279, 160, 90, 54, 50, 54, 90, 160, 279, 480, 750, 1100, 1600],
    ],
    [
        &[1300, 489, 141, 44, 30, 44, 141, 489, 1300],
        &[1700, 700, 230, 60, 32, 32, 60, 230, 700, 1700],
        &[2200, 970, 339, 100, 37, 30, 37, 100, 339, 970, 2200],
        &[2900, 1300, 510, 159, 50, 29, 29, 50, 159, 510, 1300, 2900],
        &[3800, 2100, 730, 240, 71, 30, 30, 30, 71, 240, 730, 2100, 3800],
        &[4900, 2400, 1000, 369, 111, 40, 32, 32, 40, 111, 369, 1000, 2400, 4900],
        &[6400, 3200, 1600, 550, 170, 51, 30, 30, 30, 51, 170, 550, 1600, 3200, 6400],
        &[8400, 4400, 1900, 790, 260, 80, 40, 30, 30, 40, 80, 260, 790, 1900, 4400, 8400],
        &[11000, 5700, 2600, 1300, 400, 120, 40, 30, 29, 30, 40, 120, 400, 1300, 2600, 5700, 11000],
    ],
    [
        &[2900, 651, 90, 24, 20, 24, 90, 651, 2900],
        &[4500, 1100, 170, 29, 19, 19, 29, 170, 1100, 4500],
        &[7000, 1800, 310, 40, 21, 20, 21, 40, 310, 1800, 7000],
        &[10900, 3000, 541, 70, 21, 20, 20, 21, 70, 541, 3000, 10900],
        &[17000, 4400, 940, 141, 30, 20, 20, 20, 30, 141, 940, 4400, 17000],
        &[26500, 7300, 1600, 250, 39, 20, 20, 20, 20, 39, 250, 1600, 7300, 26500],
        &[41300, 11700, 2800, 440, 60, 20, 20, 18, 20, 20, 60, 440, 2800, 11700, 41300],
        &[64200, 18500, 4600, 790, 109, 20, 20, 20, 20, 20, 20, 109, 790, 4600, 18500, 64200],
        &[100000, 29500, 7300, 1400, 200, 30, 20, 20, 20, 20, 20, 30, 200, 1400, 7300, 29500, 100000],
    ],
    [
        &[6000, 601, 30, 12, 10, 12, 30, 601, 6000],
        &[11400, 1100, 54, 10, 10, 10, 10, 54, 1100, 11400],
        &[21600, 2000, 101, 10, 10, 10, 10, 10, 101, 2000, 21600],
        &[40900, 3700, 186, 10, 10, 10, 10, 10, 10, 186, 3700, 40900],
        &[77500, 6700, 349, 20, 10, 10, 10, 10, 10, 20, 349, 6700, 77500],
        &[150000, 12200, 660, 29, 10, 10, 10, 10, 10, 10, 29, 660, 12200, 150000],
        &[280000, 22900, 1300, 41, 10, 10, 10, 10, 10, 10, 10, 41, 1300, 22900, 280000],
        &[530000, 43100, 2400, 79, 10, 10, 10, 10, 10, 10, 10, 10, 79, 2400, 43100, 530000],
        &[1000000, 80500, 4600, 150, 10, 10, 10, 10, 10, 10, 10, 10, 10, 150, 4600, 80500, 1000000],
    ],
];

/// The buckets' multiples (hundredths) for a difficulty and a number of rows (8 to 16).
pub fn table(risk: Risk, rows: u8) -> &'static [u64] {
    TABLES[risk.index()][(rows.clamp(MIN_ROWS, MAX_ROWS) - MIN_ROWS) as usize]
}

/// The most a table pays (its edges), in hundredths.
pub fn top_x100(risk: Risk, rows: u8) -> u64 {
    table(risk, rows)[0]
}

/// The chance of each bucket (`C(rows, k) / 2^rows`).
pub fn chances(rows: u8) -> Vec<f64> {
    let n = rows.clamp(MIN_ROWS, MAX_ROWS) as u32;
    let total = 2f64.powi(n as i32);
    (0..=n)
        .map(|k| {
            let mut c = 1.0f64;
            for j in 1..=k {
                c = ((n - j + 1) as f64 * c) / j as f64;
            }
            c / total
        })
        .collect()
}

/// What a table pays back over time.
pub fn rtp(risk: Risk, rows: u8) -> f64 {
    chances(rows).iter().zip(table(risk, rows)).map(|(p, m)| p * *m as f64 / 100.0).sum()
}

/// How a ball came down (0.13.28's fields).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub chips: u64,
    pub risk: Risk,
    pub rows: u8,
    /// Each row, left (false) or right (true).
    pub path: Vec<bool>,
    /// The bucket it fell into (0 at the left): how many times it went right.
    pub bucket: u8,
    pub multiplier_x100: u64,
    pub paid: u64,
}

/// A multiple as 0.13.28 writes it, on the buckets and in a big win's words: "10K×", "1.5K×",
/// "1K×", "114×", "7.5×", "4.0×", "2.79×", "0.29×".
pub fn label(x100: u64) -> String {
    if x100 >= 100_000 {
        format!("{:.1}K×", x100 as f64 / 100_000.0).replace(".0K", "K")
    } else if x100 >= 1000 {
        format!("{}×", x100 / 100)
    } else if x100 % 100 == 0 {
        format!("{}.0×", x100 / 100)
    } else if x100 % 10 == 0 {
        format!("{}.{}×", x100 / 100, (x100 % 100) / 10)
    } else {
        format!("{}.{:02}×", x100 / 100, x100 % 100)
    }
}

/// A ball dropped.
pub fn play(chips: u64, risk: Risk, rows: u8, s: &mut Stream) -> Outcome {
    let rows = rows.clamp(MIN_ROWS, MAX_ROWS);
    let path: Vec<bool> = (0..rows).map(|_| s.below(2) == 1).collect();
    let bucket = path.iter().filter(|r| **r).count();
    let m = table(risk, rows)[bucket];
    let paid = ((chips as u128 * m as u128 / 100) as u64).min(MAX_PAID);
    Outcome { chips, risk, rows, path, bucket: bucket as u8, multiplier_x100: m, paid }
}

/// How 0.13.28 works its tables out (see the module's notes): kept to show where `TABLES` came
/// from, and to check them.
#[cfg(test)]
fn worked_out(risk: Risk, rows: u8) -> Vec<u64> {
    const E8: [f64; 4] = [5.6, 13.0, 29.0, 60.0];
    const E16: [f64; 4] = [16.0, 110.0, 1000.0, 10000.0];
    const FLOOR: [f64; 4] = [0.5, 0.3, 0.2, 0.1];
    let r = risk.index();
    let n = rows.clamp(MIN_ROWS, MAX_ROWS) as usize;
    let probs = chances(n as u8);
    let edge = (E16[r] / E8[r]).powf((n as i32 - 8) as f64 * 0.125) * E8[r];
    let center = n as f64 * 0.5;
    let dist: Vec<f64> = (0..=n).map(|k| ((k as f64 - center).abs() / center).clamp(0.0, 1.0)).collect();
    let mut p = 1.0f64;
    let base = loop {
        let mut s = 0.0;
        for k in 0..=n {
            s += dist[k].powf(p) * probs[k];
        }
        let base = (0.99 - edge * s) / (1.0 - s);
        if base >= FLOOR[r] || p > 40.0 {
            break base;
        }
        p += 0.05;
    };
    let round = |x: f64| -> f64 { x.round() };
    let mut h: Vec<u64> = dist
        .iter()
        .map(|d| {
            let m = d.powf(p) * (edge - base) + base;
            let u = if m < 10.0 { 10.0 } else if m < 1000.0 { 100.0 } else { 10000.0 };
            (round(m * 100.0 / u) * u).max(10.0) as u64
        })
        .collect();
    let c = n >> 1;
    for _ in 0..500 {
        let mut rtp = -0.0f64;
        for k in 0..=n {
            rtp += h[k] as f64 * probs[k] / 100.0;
        }
        let cur = (rtp + -0.99).abs();
        if 0.0004 > cur {
            break;
        }
        let mut best: Option<(f64, usize, u64)> = None;
        for i in 1..=c {
            let v = h[i];
            let step = if v < 1000 { 1 } else if v < 100_000 { 100 } else { 10_000 };
            let mirror = n - i;
            let pr = (if mirror != i { probs[mirror] } else { 0.0 }) + probs[i];
            for cand in [v + step, v.saturating_sub(step)] {
                if cand < 10 || cand > h[i - 1] || (i < c && cand < h[i + 1]) {
                    continue;
                }
                let err = ((cand as f64 - v as f64) * pr / 100.0 + rtp + -0.99).abs();
                if !(cur > err) || best.is_some_and(|b| b.0 <= err) {
                    continue;
                }
                best = Some((err, i, cand));
            }
        }
        let Some((_, i, cand)) = best else { break };
        h[i] = cand;
        h[n - i] = cand;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A big win's words, as 0.13.28's server sent them.
    #[test]
    fn a_big_wins_words_are_0_13_28s() {
        let words = |chips: u64, risk: Risk, rows: u8, bucket: u8| {
            let m = table(risk, rows)[bucket as usize];
            super::super::Outcome::Plinko(Outcome { chips, risk, rows, path: Vec::new(), bucket, multiplier_x100: m, paid: chips * m / 100 }).headline()
        };
        assert_eq!(words(100, Risk::Expert, 9, 0), "114× on Expert with 9 rows");
        assert_eq!(words(100, Risk::Expert, 8, 8), "60× on Expert with 8 rows");
        assert_eq!(words(47, Risk::Expert, 10, 0), "216× on Expert with 10 rows");
        assert_eq!(words(100_000, Risk::Low, 9, 1), "3.7× on Low with 9 rows");
        assert_eq!(words(1, Risk::Expert, 13, 13), "1.5K× on Expert with 13 rows");
        assert_eq!((label(150_000), label(100_000), label(1_000_000)), ("1.5K×".to_string(), "1K×".to_string(), "10K×".to_string()));
        assert_eq!((label(80_500), label(29_500)), ("805×".to_string(), "295×".to_string()));
        assert_eq!((label(400), label(279), label(29)), ("4.0×".to_string(), "2.79×".to_string(), "0.29×".to_string()));
    }

    /// The tables written down are the ones 0.13.28 works out.
    #[test]
    fn the_tables_are_0_13_28s() {
        for risk in Risk::ALL {
            for rows in MIN_ROWS..=MAX_ROWS {
                assert_eq!(table(risk, rows), worked_out(risk, rows).as_slice(), "{risk:?} {rows}");
            }
        }
        // As its screen shows Medium with 16 rows, and the most Expert pays.
        assert_eq!(table(Risk::Medium, 16), &[11000, 5700, 2600, 1300, 400, 120, 40, 30, 29, 30, 40, 120, 400, 1300, 2600, 5700, 11000]);
        assert_eq!(top_x100(Risk::Expert, 16), 1_000_000);
    }

    /// Every table pays back 99% (within 0.04%), and is the same either side.
    #[test]
    fn every_table_pays_back_99_percent() {
        for risk in Risk::ALL {
            for rows in MIN_ROWS..=MAX_ROWS {
                let r = rtp(risk, rows);
                assert!((r - 0.99).abs() < 0.0004, "{risk:?} {rows}: {r}");
                let t = table(risk, rows);
                assert_eq!(t.len(), rows as usize + 1);
                assert!(t.iter().eq(t.iter().rev()));
            }
        }
    }

    /// The bets 0.13.28 took, and its words for the ones it didn't.
    #[test]
    fn bets_are_taken_as_0_13_28_took_them() {
        use crate::casino::Bet;
        let bet = |chips: u64, rows: u8| Bet::Plinko { chips, risk: Risk::Medium, rows }.check();
        assert!(bet(1, 8).is_ok() && bet(100_000, 16).is_ok());
        assert_eq!(bet(100_001, 16), Err("the most bet is 100000".into()));
        assert_eq!(bet(0, 16), Err("a bet of nothing".into()));
        assert_eq!((bet(10, 20), bet(10, 4)), (Err("Plinko has 8 to 16 rows".into()), Err("Plinko has 8 to 16 rows".into())));
    }

    /// 0.13.28's own rounds, played again from their seeds, come out the same.
    #[test]
    fn rounds_come_out_as_0_13_28s() {
        let hex = |s: &str| -> [u8; 32] { std::array::from_fn(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap()) };
        let server = hex("a631569173535f985c4d231a99f3856fb57f8d853edea0e2bd4169d5b3f39e9e");
        let client = hex("431be4218139ea5951ddeb2cc0eb31ccaf3d4cdf7b2e48ea3217851082c4b310");
        let mut s = crate::casino::stream(&server, &client, crate::casino::Game::Plinko, 1);
        let o = play(100, Risk::Medium, 16, &mut s);
        let path: String = o.path.iter().map(|r| if *r { 'R' } else { 'L' }).collect();
        assert_eq!((path.as_str(), o.bucket, o.multiplier_x100, o.paid), ("RLLRLLRRLRLLRLLL", 6, 40, 40));
    }
}
