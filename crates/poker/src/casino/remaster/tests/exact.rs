//! The wild machines (Hot, Frozen, Spotlight, Espresso) worked out exactly, as
//! `rebuild/tools/reelrow/remaster_exact.py` does: every cell is drawn on its own, so a screen's
//! ways pay is a product over the reels, and respins are Markov chains over how many wilds stay
//! on each reel. What they pay back on a spin, with Ante, and on the feature spin (Bonus Chance's
//! screens are drawn till the bonus comes out as decided: played instead).

use std::collections::HashMap;

use super::super::*;
use super::{binom, mean};
use crate::casino::reelrow::tests::{top_worth, worth};

const N: usize = PAY_SYMBOLS as usize;
const MID: std::ops::Range<usize> = 1..REELS - 1;

/// A cell's chances: each pay symbol; a wild on reels 2 to 4 on the spin, and on respins; a
/// scatter (none on respins).
struct Cells {
    sym: [f64; N],
    wild: f64,
    later: f64,
    scatter: f64,
}

fn cells(m: &Machine, ante: bool) -> Cells {
    let mut sym = [0.0; N];
    for (k, x) in reels::WEIGHTS.iter().take(N).enumerate() {
        sym[k] = (x * 100) as f64 / OUT_OF as f64;
    }
    let p = |w: u32| w as f64 / OUT_OF as f64;
    Cells { sym, wild: p(m.wild), later: p(m.later_wild), scatter: p(if ante { m.ante_scatter } else { m.scatter }) }
}

/// The ways points of a screen, on average, given each reel's chance to count (`ec`, the
/// expected matching cells) and to stop a way (`pz`, no matching cell), symbol by symbol.
fn ways_points(per: impl Fn(usize, usize) -> (f64, f64)) -> f64 {
    let mut total = 0.0;
    for s in 0..N {
        let mut prod = 1.0;
        for n in 1..=REELS {
            prod *= per(s, n - 1).0;
            if n >= 3 {
                let stop = if n < REELS { per(s, n).1 } else { 1.0 };
                total += PAY[s][n - 3] as f64 * prod * stop;
            }
        }
    }
    total
}

/// A screen with `fixed[r]` wilds staying on reel r, the rest drawn (a wild with `wild` on reels
/// 2 to 4).
fn screen(c: &Cells, fixed: [usize; REELS], wild: f64) -> f64 {
    ways_points(|s, r| {
        let pw = if MID.contains(&r) { wild } else { 0.0 };
        let q = c.sym[s] + pw;
        let f = fixed[r];
        (f as f64 + (ROWS - f) as f64 * q, if f == 0 { (1.0 - q).powi(ROWS as i32) } else { 0.0 })
    })
}

/// A screen's scatters (each reel's chance of 0 to 3), added up: the chance of three or more.
fn bonus(per_reel: impl Fn(usize) -> Vec<f64>) -> f64 {
    let mut dist = vec![1.0];
    for r in 0..REELS {
        let d = per_reel(r);
        let mut new = vec![0.0; dist.len() + d.len() - 1];
        for (i, a) in dist.iter().enumerate() {
            for (j, b) in d.iter().enumerate() {
                new[i + j] += a * b;
            }
        }
        dist = new;
    }
    1.0 - dist.iter().take(3).sum::<f64>()
}

fn plain(cells: usize, p: f64) -> Vec<f64> {
    (0..=cells).map(|k| binom(cells, k, p)).collect()
}

/// A reel of three that a wild fills (and so covers its scatters) with chance `h`.
fn covered(c: &Cells, pw: f64) -> Vec<f64> {
    let h = 1.0 - (1.0 - pw).powi(ROWS as i32);
    let cc = c.scatter / (1.0 - pw);
    let mut d: Vec<f64> = (0..=ROWS).map(|k| (1.0 - h) * binom(ROWS, k, cc)).collect();
    d[0] += h;
    d
}

/// Hot Wilds: (ways pay times the bet, the bonus's chance), a spin or a Hot Spin.
fn hot(m: &Machine, c: &Cells, special: bool) -> (f64, f64) {
    let ex = mean(m.hot);
    let pw = c.wild;
    let h = 1.0 - (1.0 - pw).powi(ROWS as i32);
    let forced = |r: usize| special && r == REELS / 2;
    let pts = ways_points(|s, r| {
        let q = c.sym[s];
        if forced(r) {
            (3.0 * ex, 0.0)
        } else if MID.contains(&r) {
            (h * 3.0 * ex + (1.0 - h) * 3.0 * q / (1.0 - pw), (1.0 - pw - q).powi(3))
        } else {
            (3.0 * q, (1.0 - q).powi(3))
        }
    });
    let pb = bonus(|r| if forced(r) { vec![1.0] } else if MID.contains(&r) { covered(c, pw) } else { plain(ROWS, c.scatter) });
    (pts * m.point as f64 / 10_000.0, pb)
}

/// Frozen Wilds: a spin or a Frozen Spin (two wilds at random on reels 2 to 4).
fn frozen(m: &Machine, c: &Cells, special: bool) -> (f64, f64) {
    fn chain(c: &Cells, f: [usize; 3], k: usize, memo: &mut HashMap<([usize; 3], usize), f64>) -> f64 {
        if k >= MAX_FROZEN {
            return 0.0;
        }
        if let Some(v) = memo.get(&(f, k)) {
            return *v;
        }
        let pl = c.later;
        let mut v = screen(c, [0, f[0], f[1], f[2], 0], pl);
        for a in 0..=ROWS - f[0] {
            for b in 0..=ROWS - f[1] {
                for d in 0..=ROWS - f[2] {
                    if a + b + d == 0 {
                        continue;
                    }
                    let pr = binom(ROWS - f[0], a, pl) * binom(ROWS - f[1], b, pl) * binom(ROWS - f[2], d, pl);
                    v += pr * chain(c, [f[0] + a, f[1] + b, f[2] + d], k + 1, memo);
                }
            }
        }
        memo.insert((f, k), v);
        v
    }
    let mut memo = HashMap::new();
    let mut starts: Vec<[usize; 3]> = Vec::new();
    if special {
        let places: Vec<usize> = (0..3 * ROWS).collect();
        for (i, a) in places.iter().enumerate() {
            for b in &places[i + 1..] {
                let mut f = [0; 3];
                f[a / ROWS] += 1;
                f[b / ROWS] += 1;
                starts.push(f);
            }
        }
    } else {
        starts.push([0; 3]);
    }
    let wt = 1.0 / starts.len() as f64;
    let (mut total, mut pb) = (0.0, 0.0);
    for forced in starts {
        let pw = c.wild;
        let mut v = screen(c, [0, forced[0], forced[1], forced[2], 0], pw);
        for a in 0..=ROWS - forced[0] {
            for b in 0..=ROWS - forced[1] {
                for d in 0..=ROWS - forced[2] {
                    let f0 = [forced[0] + a, forced[1] + b, forced[2] + d];
                    if f0.iter().sum::<usize>() == 0 {
                        continue;
                    }
                    let pr = binom(ROWS - forced[0], a, pw) * binom(ROWS - forced[1], b, pw) * binom(ROWS - forced[2], d, pw);
                    v += pr * chain(c, f0, 0, &mut memo);
                }
            }
        }
        total += wt * v;
        let n = PLACES - forced.iter().sum::<usize>();
        pb += wt * (1.0 - plain(n, c.scatter).iter().take(3).sum::<f64>());
    }
    (total * m.point as f64 / 10_000.0, pb)
}

/// Spotlight Wilds: a spin or an Encore Spin (a wild on reel 5).
fn spotlight(m: &Machine, c: &Cells, special: bool) -> (f64, f64) {
    fn chain(c: &Cells, w: [usize; 4], k: usize, memo: &mut HashMap<([usize; 4], usize), f64>) -> f64 {
        if k >= MAX_STEPS {
            return 0.0;
        }
        if let Some(v) = memo.get(&(w, k)) {
            return *v;
        }
        let pl = c.later;
        let mut v = screen(c, [w[0], w[1], w[2], w[3], 0], pl);
        for a in 0..=ROWS - w[1] {
            for b in 0..=ROWS - w[2] {
                for d in 0..=ROWS - w[3] {
                    let moved = [w[1] + a, w[2] + b, w[3] + d, 0];
                    if moved.iter().sum::<usize>() == 0 {
                        continue;
                    }
                    let pr = binom(ROWS - w[1], a, pl) * binom(ROWS - w[2], b, pl) * binom(ROWS - w[3], d, pl);
                    v += pr * chain(c, moved, k + 1, memo);
                }
            }
        }
        memo.insert((w, k), v);
        v
    }
    let mut memo = HashMap::new();
    let encore = special as usize;
    let pw = c.wild;
    let mut v = screen(c, [0, 0, 0, 0, encore], pw);
    for a in 0..=ROWS {
        for b in 0..=ROWS {
            for d in 0..=ROWS {
                let moved = [a, b, d, encore];
                if moved.iter().sum::<usize>() == 0 {
                    continue;
                }
                v += binom(ROWS, a, pw) * binom(ROWS, b, pw) * binom(ROWS, d, pw) * chain(c, moved, 1, &mut memo);
            }
        }
    }
    let pb = 1.0 - plain(PLACES - encore, c.scatter).iter().take(3).sum::<f64>();
    (v * m.point as f64 / 10_000.0, pb)
}

/// Espresso Shots: a spin or a Double Shot (shots on reels 2 and 4).
fn espresso(m: &Machine, c: &Cells, special: bool) -> (f64, f64) {
    /// A screen with `shots` full of wilds, the other middle reels a shot with any wild landing.
    fn screen_shots(c: &Cells, shots: [bool; REELS], pw: f64) -> f64 {
        let h = 1.0 - (1.0 - pw).powi(ROWS as i32);
        ways_points(|s, r| {
            let q = c.sym[s];
            if shots[r] {
                (3.0, 0.0)
            } else if MID.contains(&r) {
                (3.0 * h + 3.0 * q * (1.0 - pw).powi(2), (1.0 - pw - q).powi(3))
            } else {
                (3.0 * q, (1.0 - q).powi(3))
            }
        })
    }
    fn chain(c: &Cells, staying: [bool; REELS], k: usize, memo: &mut HashMap<([bool; REELS], usize), f64>) -> f64 {
        if k >= MAX_STEPS {
            return 0.0;
        }
        if let Some(v) = memo.get(&(staying, k)) {
            return *v;
        }
        let pl = c.later;
        let hl = 1.0 - (1.0 - pl).powi(ROWS as i32);
        let mut v = screen_shots(c, staying, pl);
        let free: Vec<usize> = MID.filter(|&r| !staying[r]).collect();
        for bits in 1..(1u32 << free.len()) {
            let mut new = [false; REELS];
            let mut pr = 1.0;
            for (i, &r) in free.iter().enumerate() {
                if bits >> i & 1 == 1 {
                    new[r] = true;
                    pr *= hl;
                } else {
                    pr *= 1.0 - hl;
                }
            }
            v += pr * chain(c, new, k + 1, memo);
        }
        memo.insert((staying, k), v);
        v
    }
    let mut memo = HashMap::new();
    let pw = c.wild;
    let h = 1.0 - (1.0 - pw).powi(ROWS as i32);
    let mut forced = [false; REELS];
    if special {
        forced[1] = true;
        forced[REELS - 2] = true;
    }
    let mut v = screen_shots(c, forced, pw);
    let free: Vec<usize> = MID.filter(|&r| !forced[r]).collect();
    for bits in 0..(1u32 << free.len()) {
        let mut shots = forced;
        let mut pr = 1.0;
        for (i, &r) in free.iter().enumerate() {
            if bits >> i & 1 == 1 {
                shots[r] = true;
                pr *= h;
            } else {
                pr *= 1.0 - h;
            }
        }
        if shots.iter().any(|&s| s) {
            v += pr * chain(c, shots, 1, &mut memo);
        }
    }
    let pb = bonus(|r| if forced[r] { vec![1.0] } else if MID.contains(&r) { covered(c, pw) } else { plain(ROWS, c.scatter) });
    (v * m.point as f64 / 10_000.0, pb)
}

/// What a wild machine pays back on a spin (with Ante or without) or its feature spin, exactly;
/// None for the others (and Bonus Chance).
pub(crate) fn rtp(machine: u8, feature: Feature, ante: bool) -> Option<f64> {
    let m = &MACHINES[machine as usize];
    if !matches!(feature, Feature::Spin | Feature::Wild) {
        return None;
    }
    let c = cells(m, ante);
    let special = feature == Feature::Wild;
    let (ways, pb) = match m.family {
        Hot => hot(m, &c, special),
        Frozen => frozen(m, &c, special),
        Spotlight => spotlight(m, &c, special),
        Espresso => espresso(m, &c, special),
        _ => return None,
    };
    let w = worth(machine) / 100.0 + top_worth(machine);
    Some((ways + pb * w) / (price(machine, feature, ante) as f64 / 100.0))
}

/// Bonus Chance on a wild machine: its screen drawn till the bonus comes out as decided (1 in
/// 44.5), so the reels pay a spin's, moved toward what they pay on screens with three scatters
/// or more. That part's played (`n` such screens; it counts a sixtieth as much as the rest).
/// What it pays back, and the bonus's chance (per million) for 99%.
pub(crate) fn chance_rtp(machine: u8, n: u64, seed: u64) -> Option<(f64, f64)> {
    let m = &MACHINES[machine as usize];
    let c = cells(m, false);
    let (spin, ps) = match m.family {
        Hot => hot(m, &c, false),
        Frozen => frozen(m, &c, false),
        Spotlight => spotlight(m, &c, false),
        Espresso => espresso(m, &c, false),
        _ => return None,
    };
    let mut d = super::Quick::new(seed);
    let play = Play::Chance(true);
    let mut points = 0u64;
    for _ in 0..n {
        points += match m.family {
            Hot => super::super::hot(m, false, play, 1, &mut d),
            Frozen => super::super::frozen(m, false, play, 1, &mut d),
            Spotlight => super::super::spotlight(m, false, play, 1, &mut d),
            _ => super::super::espresso(m, false, play, 1, &mut d),
        }
        .points;
    }
    let with_bonus = points as f64 / n as f64 * m.point as f64 / 10_000.0;
    let w = worth(machine) / 100.0 + top_worth(machine);
    let price = price(machine, Feature::Chance, false) as f64 / 100.0;
    let at = |pc: f64| (spin + (pc - ps) / (1.0 - ps) * (with_bonus - spin) + pc * w) / price;
    // (What it pays is a straight line in the bonus's chance: the chance for 99%.)
    let (a, b) = (at(0.0), at(1.0) - at(0.0));
    Some((at(m.chance_ppm as f64 / 1e6), (0.99 - a) / b * 1e6))
}
