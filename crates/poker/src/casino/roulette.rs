//! Roulette on a single-zero wheel (37 pockets): the house keeps 2.7% of every bet over time.
//!
//! The layout: twelve rows of three, row `r` holding `3r+1`, `3r+2` and `3r+3`, and the zero
//! at the head.

use serde::{Deserialize, Serialize};

use crate::fair::Stream;

/// The pockets in order round the wheel.
pub const WHEEL: [u8; 37] = [0, 32, 15, 19, 4, 21, 2, 25, 17, 34, 6, 27, 13, 36, 11, 30, 8, 23, 10, 5, 24, 16, 33, 1, 20, 14, 31, 9, 22, 18, 29, 7, 28, 12, 35, 3, 26];

pub const RED: [u8; 18] = [1, 3, 5, 7, 9, 12, 14, 16, 18, 19, 21, 23, 25, 27, 30, 32, 34, 36];

pub fn is_red(n: u8) -> bool {
    RED.contains(&n)
}

pub fn color_name(n: u8) -> &'static str {
    if n == 0 {
        "green"
    } else if is_red(n) {
        "red"
    } else {
        "black"
    }
}

/// Where chips can go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "on", rename_all = "snake_case")]
pub enum Spot {
    /// One number (0 to 36).
    Straight { n: u8 },
    /// Two side by side on the layout (`a` < `b`), or the zero and 1, 2 or 3.
    Split { a: u8, b: u8 },
    /// A row of three (0 to 11).
    Street { row: u8 },
    /// Four meeting at a corner: `a` is the top left (`a`, `a+1`, `a+3`, `a+4`).
    Corner { a: u8 },
    /// Two rows (six numbers): `row` and the one after it (0 to 10).
    Line { row: u8 },
    /// A column (0, 1, 2: the numbers ending that way down the layout).
    Column { c: u8 },
    /// 1-12, 13-24, 25-36 (0, 1, 2).
    Dozen { d: u8 },
    Red,
    Black,
    Odd,
    Even,
    Low,
    High,
}

impl Spot {
    pub fn check(&self) -> Result<(), String> {
        let ok = match *self {
            Spot::Straight { n } => n <= 36,
            Spot::Split { a, b } => (a == 0 && (1..=3).contains(&b)) || (a >= 1 && b <= 36 && (b == a + 3 || (b == a + 1 && a % 3 != 0))),
            Spot::Street { row } => row < 12,
            Spot::Corner { a } => a >= 1 && a <= 32 && a % 3 != 0,
            Spot::Line { row } => row < 11,
            Spot::Column { c } => c < 3,
            Spot::Dozen { d } => d < 3,
            _ => true,
        };
        if ok { Ok(()) } else { Err(format!("there's no such bet on the layout ({self:?})")) }
    }

    /// The numbers it covers.
    pub fn covers(&self, n: u8) -> bool {
        match *self {
            Spot::Straight { n: m } => n == m,
            Spot::Split { a, b } => n == a || n == b,
            Spot::Street { row } => n >= 1 && (n - 1) / 3 == row,
            Spot::Corner { a } => n == a || n == a + 1 || n == a + 3 || n == a + 4,
            Spot::Line { row } => n >= 1 && ((n - 1) / 3 == row || (n - 1) / 3 == row + 1),
            Spot::Column { c } => n >= 1 && (n - 1) % 3 == c,
            Spot::Dozen { d } => n >= 1 && (n - 1) / 12 == d,
            Spot::Red => is_red(n),
            Spot::Black => n >= 1 && !is_red(n),
            Spot::Odd => n >= 1 && n % 2 == 1,
            Spot::Even => n >= 1 && n % 2 == 0,
            Spot::Low => (1..=18).contains(&n),
            Spot::High => (19..=36).contains(&n),
        }
    }

    /// What it pays to one.
    pub fn pays(&self) -> u64 {
        match self {
            Spot::Straight { .. } => 35,
            Spot::Split { .. } => 17,
            Spot::Street { .. } => 11,
            Spot::Corner { .. } => 8,
            Spot::Line { .. } => 5,
            Spot::Column { .. } | Spot::Dozen { .. } => 2,
            _ => 1,
        }
    }

    pub fn name(&self) -> String {
        match *self {
            Spot::Straight { n } => format!("{n} straight up"),
            Spot::Split { a, b } => format!("the {a}/{b} split"),
            Spot::Street { row } => format!("the {}-{} street", row * 3 + 1, row * 3 + 3),
            Spot::Corner { a } => format!("the {a}/{}/{}/{} corner", a + 1, a + 3, a + 4),
            Spot::Line { row } => format!("the {}-{} line", row * 3 + 1, row * 3 + 6),
            Spot::Column { c } => format!("column {}", c + 1),
            Spot::Dozen { d } => format!("{}-{}", d * 12 + 1, d * 12 + 12),
            Spot::Red => "red".into(),
            Spot::Black => "black".into(),
            Spot::Odd => "odd".into(),
            Spot::Even => "even".into(),
            Spot::Low => "1-18".into(),
            Spot::High => "19-36".into(),
        }
    }
}

/// The most one spot may hold (so a straight-up win can pay at most 36 times this).
pub fn max_on(spot: &Spot) -> u64 {
    match spot {
        Spot::Straight { .. } => 2_000,
        Spot::Split { .. } => 4_000,
        Spot::Street { .. } => 6_000,
        Spot::Corner { .. } => 8_000,
        Spot::Line { .. } => 12_000,
        Spot::Column { .. } | Spot::Dozen { .. } => 25_000,
        _ => 50_000,
    }
}

/// Chips on a spot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chip {
    pub spot: Spot,
    pub chips: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    /// The pocket the ball landed in (its number), and its place round the wheel.
    pub number: u8,
    pub pocket: u8,
    pub chips: Vec<Chip>,
    /// What each bet paid back (stake included; 0 for a loser), in the order placed.
    pub returns: Vec<u64>,
    pub paid: u64,
}

pub fn spin(s: &mut Stream) -> (u8, u8) {
    let pocket = s.below(37) as u8;
    (WHEEL[pocket as usize], pocket)
}

pub fn settle(chips: &[Chip], number: u8) -> Vec<u64> {
    chips.iter().map(|c| if c.spot.covers(number) { c.chips * (c.spot.pays() + 1) } else { 0 }).collect()
}

pub fn play(chips: &[Chip], s: &mut Stream) -> Outcome {
    let (number, pocket) = spin(s);
    let returns = settle(chips, number);
    let paid = returns.iter().sum();
    Outcome { number, pocket, chips: chips.to_vec(), returns, paid }
}
