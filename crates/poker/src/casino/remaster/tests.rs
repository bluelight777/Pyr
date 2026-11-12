use super::*;
use crate::casino::reelrow::tests::{top_worth, worth};

/// A quick generator, to play millions of rounds (xoshiro256**, seeded by SplitMix64).
pub(crate) struct Quick([u64; 4]);

impl Quick {
    pub(crate) fn new(seed: u64) -> Quick {
        let mut z = seed;
        let mut next = || {
            z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut x = z;
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        Quick([next(), next(), next(), next()])
    }

    fn next(&mut self) -> u64 {
        let s = &mut self.0;
        let out = s[1].wrapping_mul(5).rotate_left(7).wrapping_mul(9);
        let t = s[1] << 17;
        s[2] ^= s[0];
        s[3] ^= s[1];
        s[1] ^= s[2];
        s[0] ^= s[3];
        s[2] ^= t;
        s[3] = s[3].rotate_left(45);
        out
    }
}

impl Draw for Quick {
    fn below(&mut self, n: u32) -> u32 {
        (((self.next() >> 32) * n as u64) >> 32) as u32
    }
}

/// A way to play over `n` rounds: what the reels paid (times the bet; the Grand apart, and
/// each round's at most the machine's cap), its square (for the error), how often the bonus
/// came and all fifteen places filled; and a few things to see the play by.
#[derive(Clone, Default, Debug)]
pub(crate) struct Figures {
    pub n: u64,
    pub reels: f64,
    pub reels_sq: f64,
    pub bonuses: u64,
    pub grands: u64,
    pub wins: u64,
    pub steps: u64,
    pub holds: u64,
    pub bombs: u64,
}

impl Figures {
    fn add(&mut self, o: &Figures) {
        self.n += o.n;
        self.reels += o.reels;
        self.reels_sq += o.reels_sq;
        self.bonuses += o.bonuses;
        self.grands += o.grands;
        self.wins += o.wins;
        self.steps += o.steps;
        self.holds += o.holds;
        self.bombs += o.bombs;
    }

    /// What it pays back, and its standard error.
    pub(crate) fn rtp(&self, machine: u8, feature: Feature, ante: bool) -> (f64, f64) {
        let n = self.n as f64;
        let w = worth(machine) / 100.0 + top_worth(machine);
        let grand = if reelrow::progressive(machine) { 0.0 } else { GRAND_X as f64 };
        let cost = price(machine, feature, ante) as f64 / 100.0;
        let mean = self.reels / n;
        let pay = mean + self.bonuses as f64 / n * w + self.grands as f64 / n * grand;
        let var = (self.reels_sq / n - mean * mean).max(0.0);
        (pay / cost, (var / n).sqrt() / cost)
    }
}

pub(crate) fn figures(machine: u8, feature: Feature, ante: bool, n: u64, seed: u64) -> Figures {
    let threads = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(2).min(8) as u64;
    let m = &MACHINES[machine as usize];
    let cap = if m.cap_x > 0 { m.cap_x as f64 } else { f64::MAX };
    let parts: Vec<Figures> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                sc.spawn(move || {
                    let mut d = Quick::new(seed.wrapping_mul(1_000_003).wrapping_add(t));
                    let mut f = Figures::default();
                    let rounds = n / threads + if t < n % threads { 1 } else { 0 };
                    for _ in 0..rounds {
                        let r = spin(machine, ante, feature, 100_000, &mut d);
                        let x = r.x(m.point).min(cap);
                        f.n += 1;
                        f.reels += x;
                        f.reels_sq += x * x;
                        f.bonuses += (r.scatters >= 3) as u64;
                        f.grands += r.grand as u64;
                        f.wins += (x > 0.0) as u64;
                        f.steps += r.steps.len() as u64;
                        f.holds += !r.hold.is_empty() as u64;
                        f.bombs += r.steps.last().map(|s| count(&s.grid, BOMB) as u64).unwrap_or(0);
                    }
                    f
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut all = Figures::default();
    for p in &parts {
        all.add(p);
    }
    all
}

fn rounds() -> u64 {
    std::env::var("PYR_RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(2_000_000)
}

/// Every machine, every way to play on the reels: what it pays back (`PYR_RR_ROUNDS` rounds
/// each; `cargo test -p pyr-poker remaster_table -- --ignored --nocapture`).
#[test]
#[ignore]
fn remaster_table() {
    let n = rounds();
    let only: Option<u8> = std::env::var("PYR_RR_MACHINE").ok().and_then(|v| v.parse().ok());
    for machine in 0..12u8 {
        if only.is_some_and(|o| o != machine) {
            continue;
        }
        let m = &MACHINES[machine as usize];
        for (what, feature, ante) in [("spin", Feature::Spin, false), ("ante", Feature::Spin, true), ("chance", Feature::Chance, false), (m.special_name, Feature::Wild, false)] {
            let f = figures(machine, feature, ante, n, 7 + machine as u64);
            let (rtp, se) = f.rtp(machine, feature, ante);
            let nn = f.n as f64;
            println!(
                "{:>2} {:<18} {:<15} rtp {:>7.3}% ±{:.3}  reels {:.4}  bonus 1 in {:>6.1}  hits {:.3}  steps {:.2}  holds 1 in {:>7.1}  bombs {:.3}  grand 1 in {:.0}",
                machine,
                reels::machine_name(machine),
                what,
                rtp * 100.0,
                se * 100.0,
                f.reels / nn,
                nn / f.bonuses.max(1) as f64,
                f.wins as f64 / nn,
                f.steps as f64 / nn,
                nn / f.holds.max(1) as f64,
                f.bombs as f64 / nn,
                nn / f.grands.max(1) as f64,
            );
        }
    }
}

/// Rounds played on a machine as given, adding up apart what depends on what bombs, coins and
/// Hot Wilds are worth, so what they're worth can be solved for (and the tests needn't wait on
/// a rare big one): play with every natural bomb worth 1 and every Bomb Spin bomb 1,000, every
/// coin a hundredth of the bet, and Hot Wilds counted by how many a win runs through.
#[derive(Clone, Default, Debug)]
pub(crate) struct Parts {
    pub n: u64,
    /// Times the bet: what paid as it is (ways with no bomb on the reels, or on a Hold & Win
    /// machine); Hot Wilds: ways through 0 to 3 of them (to multiply by E[x] to that power);
    /// a tumbling chain times its natural bombs, and its Bomb Spin bombs (to multiply by what
    /// one is worth on average); coins (to multiply by what a coin is worth, times the bet).
    pub plain: f64,
    pub hot: [f64; 4],
    pub natural: f64,
    pub start: f64,
    pub coins: f64,
    pub bonuses: u64,
    pub grands: u64,
    pub holds: u64,
}

impl Parts {
    fn add(&mut self, o: &Parts) {
        self.n += o.n;
        self.plain += o.plain;
        for k in 0..4 {
            self.hot[k] += o.hot[k];
        }
        self.natural += o.natural;
        self.start += o.start;
        self.coins += o.coins;
        self.bonuses += o.bonuses;
        self.grands += o.grands;
        self.holds += o.holds;
    }

    /// What the reels pay on average (times the bet), bombs, coins and Hot Wilds worth this
    /// much on average.
    pub(crate) fn reels(&self, hot_x: f64, natural: f64, start: f64, coin: f64) -> f64 {
        let n = self.n as f64;
        (self.plain + (0..4).map(|k| self.hot[k] * hot_x.powi(k as i32)).sum::<f64>() + self.natural * natural + self.start * start + self.coins * coin) / n
    }

    pub(crate) fn bonus_rate(&self) -> f64 {
        self.bonuses as f64 / self.n as f64
    }

    pub(crate) fn grand_rate(&self) -> f64 {
        self.grands as f64 / self.n as f64
    }
}

/// The machine as tuned, its values counted rather than drawn (see [`Parts`]).
pub(crate) fn counting(m: &Machine) -> Machine {
    Machine { bombs: &[(1, 1)], start_bombs: &[(1_000, 1)], coins: &[(1, 1)], ..*m }
}

pub(crate) fn parts(m: &Machine, feature: Feature, ante: bool, n: u64, seed: u64) -> Parts {
    let threads = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(2).min(8) as u64;
    let c = counting(m);
    let point = m.point as f64 / 10_000.0;
    let all: Vec<Parts> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                let c = &c;
                sc.spawn(move || {
                    let mut d = Quick::new(seed.wrapping_mul(1_000_003).wrapping_add(t));
                    let mut p = Parts::default();
                    let rounds = n / threads + if t < n % threads { 1 } else { 0 };
                    for _ in 0..rounds {
                        let r = spin_on(c, ante, feature, 100_000, &mut d);
                        p.n += 1;
                        p.bonuses += (r.scatters >= 3) as u64;
                        p.grands += r.grand as u64;
                        p.holds += !r.hold.is_empty() as u64;
                        p.coins += (r.collected + r.held) as f64 / 100.0;
                        match c.family {
                            Hot => {
                                let s = &r.steps[0];
                                for w in &s.wins {
                                    let k = (0..w.count as usize).filter(|&i| s.x[i] > 0).count();
                                    p.hot[k] += (w.points / w.x as u64) as f64 * point;
                                }
                            }
                            Tumble => {
                                let last = r.steps.last().unwrap();
                                let (mut kn, mut ks) = (0u64, 0u64);
                                for reel in 0..REELS {
                                    for row in 0..ROWS {
                                        if last.grid[reel][row] == BOMB {
                                            if last.values[reel][row] >= 1_000 { ks += 1 } else { kn += 1 }
                                        }
                                    }
                                }
                                let x = r.points as f64 * point;
                                if kn + ks == 0 {
                                    p.plain += x;
                                } else {
                                    p.natural += x * kn as f64;
                                    p.start += x * ks as f64;
                                }
                            }
                            _ => p.plain += r.points as f64 * point,
                        }
                    }
                    p
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut p = Parts::default();
    for x in &all {
        p.add(x);
    }
    p
}

pub(crate) fn mean(t: &[(u32, u32)]) -> f64 {
    if t.is_empty() {
        return 0.0;
    }
    let w: f64 = t.iter().map(|x| x.1 as f64).sum();
    t.iter().map(|&(v, n)| v as f64 * n as f64).sum::<f64>() / w
}

/// A machine's way to play, as tuned: what it pays back (the reels' part from `p`).
pub(crate) fn rtp_of(machine: u8, m: &Machine, p: &Parts, feature: Feature, ante: bool) -> f64 {
    let w = worth(machine) / 100.0 + top_worth(machine);
    let grand = if reelrow::progressive(machine) { 0.0 } else { GRAND_X as f64 };
    let reels = p.reels(mean(m.hot), mean(m.bombs).max(0.0), mean(m.start_bombs).max(0.0), if m.coins.is_empty() { 0.0 } else { mean(m.coins) });
    (reels + p.bonus_rate() * w + p.grand_rate() * grand) / (price(machine, feature, ante) as f64 / 100.0)
}

/// Every machine as it is: each way to play on the reels, worked out with values averaged
/// (`PYR_RR_ROUNDS` rounds each; `cargo test -p pyr-poker remaster_parts -- --ignored --nocapture`).
#[test]
#[ignore]
fn remaster_parts() {
    let n = rounds();
    let only: Option<u8> = std::env::var("PYR_RR_MACHINE").ok().and_then(|v| v.parse().ok());
    for machine in 0..12u8 {
        if only.is_some_and(|o| o != machine) {
            continue;
        }
        let m = &MACHINES[machine as usize];
        for (what, feature, ante) in [("spin", Feature::Spin, false), ("ante", Feature::Spin, true), ("chance", Feature::Chance, false), (m.special_name, Feature::Wild, false)] {
            let p = parts(m, feature, ante, n, 11 + machine as u64);
            let rtp = rtp_of(machine, m, &p, feature, ante);
            let nn = p.n as f64;
            println!(
                "{:>2} {:<18} {:<15} rtp {:>7.3}%  plain {:.4} hot {:.4},{:.4},{:.4},{:.4} natural {:.5} start {:.5} coins {:.4}  bonus 1 in {:>6.1}  holds 1 in {:>7.1}  grand 1 in {:.0}",
                machine,
                reels::machine_name(machine),
                what,
                rtp * 100.0,
                p.plain / nn,
                p.hot[0] / nn,
                p.hot[1] / nn,
                p.hot[2] / nn,
                p.hot[3] / nn,
                p.natural / nn,
                p.start / nn,
                p.coins / nn,
                nn / p.bonuses.max(1) as f64,
                nn / p.holds.max(1) as f64,
                nn / p.grands.max(1) as f64,
            );
        }
    }
}

/// What a way to play has to pay back: 99% of its price.
fn target(machine: u8, feature: Feature, ante: bool) -> f64 {
    0.99 * price(machine, feature, ante) as f64 / 100.0
}

fn mini(machine: u8) -> f64 {
    worth(machine) / 100.0 + top_worth(machine)
}

/// The value `x` in `lo..hi` where `f` crosses zero (bisection; `f` goes one way).
fn solve(lo: f64, hi: f64, f: impl Fn(f64) -> f64) -> f64 {
    let (mut a, mut b) = (lo, hi);
    let fa = f(a);
    for _ in 0..60 {
        let c = (a + b) / 2.0;
        if (f(c) > 0.0) == (fa > 0.0) { a = c } else { b = c }
    }
    (a + b) / 2.0
}

/// Works out the numbers the Info pages don't show, machine by machine (`PYR_RR_MACHINE`,
/// `PYR_RR_ROUNDS`; `cargo test -p pyr-poker remaster_tune -- --ignored
/// --nocapture`), and prints them.
#[test]
#[ignore]
fn remaster_tune() {
    let n = rounds();
    let only: Option<u8> = std::env::var("PYR_RR_MACHINE").ok().and_then(|v| v.parse().ok());
    println!("mini games' worth: {:?}", (0..12u8).map(|m| format!("{:.6}", mini(m))).collect::<Vec<_>>());
    for machine in 0..12u8 {
        if only.is_some_and(|o| o != machine) {
            continue;
        }
        let base = MACHINES[machine as usize];
        let w = mini(machine);
        let seed = 101 + machine as u64;
        match base.family {
            Hot => {
                // For a wild weight: E[x] so a spin pays 99%, and then what a Hot Spin pays.
                let at = |wild: u32| {
                    let m = Machine { wild, later_wild: wild, ..base };
                    let s = parts(&m, Feature::Spin, false, n, seed);
                    let h = parts(&m, Feature::Wild, false, n, seed + 1);
                    let e = solve(1.0, 6.0, |e| s.reels(e, 0.0, 0.0, 0.0) + s.bonus_rate() * w - target(machine, Feature::Spin, false));
                    let hot = (h.reels(e, 0.0, 0.0, 0.0) + h.bonus_rate() * w) / target(machine, Feature::Wild, false);
                    println!("  wild {wild}: E[x] {e:.4}, Hot Spin {:.3}%, bonus 1 in {:.1}", hot * 99.0, 1.0 / s.bonus_rate());
                    (e, hot)
                };
                let (mut a, mut b) = (1_190u32, 1_250u32);
                let (mut fa, mut fb) = (at(a).1 - 1.0, at(b).1 - 1.0);
                for _ in 0..6 {
                    if (fb - fa).abs() < 1e-9 {
                        break;
                    }
                    let c = (b as f64 - fb * (b as f64 - a as f64) / (fb - fa)).round().clamp(500.0, 4_000.0) as u32;
                    if c == a || c == b {
                        break;
                    }
                    let fc = at(c).1 - 1.0;
                    (a, fa, b, fb) = (b, fb, c, fc);
                }
                let (e, hot) = at(b);
                println!("{machine} {}: wild {b}, E[x] {e:.4} (×3 {:.1}%, ×5 {:.1}%), Hot Spin {:.3}%", reels::machine_name(machine), (3.2 - e) / 2.0 * 100.0, (e - 2.4) / 2.0 * 100.0, hot * 99.0);
            }
            Frozen | Spotlight | Espresso => {
                // Two weights (the spin's wild, and the wild on what's drawn after), so the
                // spin and the feature spin both pay 99%: Newton's, a step at a time.
                let eval = |wild: f64, later: f64| {
                    let m = Machine { wild: wild.round() as u32, later_wild: later.round() as u32, ..base };
                    let s = parts(&m, Feature::Spin, false, n, seed);
                    let f = parts(&m, Feature::Wild, false, n, seed + 1);
                    let rs = (s.reels(0.0, 0.0, 0.0, 0.0) + s.bonus_rate() * w) / target(machine, Feature::Spin, false) - 1.0;
                    let rf = (f.reels(0.0, 0.0, 0.0, 0.0) + f.bonus_rate() * w) / target(machine, Feature::Wild, false) - 1.0;
                    println!("  wild {wild:.0}, later {later:.0}: spin {:.3}%, {} {:.3}%", (rs + 1.0) * 99.0, base.special_name, (rf + 1.0) * 99.0);
                    (rs, rf)
                };
                let (mut x, mut y) = (base.wild as f64, base.later_wild as f64);
                for _ in 0..5 {
                    let (s0, f0) = eval(x, y);
                    if s0.abs() < 0.0005 && f0.abs() < 0.0005 {
                        break;
                    }
                    let h = 80.0;
                    let (s1, f1) = eval(x + h, y);
                    let (s2, f2) = eval(x, y + h);
                    let (a, b, c, d) = ((s1 - s0) / h, (s2 - s0) / h, (f1 - f0) / h, (f2 - f0) / h);
                    let det = a * d - b * c;
                    if det.abs() < 1e-12 {
                        break;
                    }
                    x -= (d * s0 - b * f0) / det;
                    y -= (-c * s0 + a * f0) / det;
                    x = x.clamp(100.0, 15_000.0);
                    y = y.clamp(100.0, 15_000.0);
                }
                println!("{machine} {}: wild {:.0}, later {:.0}", reels::machine_name(machine), x, y);
            }
            Tumble => {
                // The scatter's weight so Bonus Chance pays 99% (the bonus as often as the
                // tumbles bring it); then what a bomb by itself and a Bomb Spin's are worth on
                // average, so a spin and a Bomb Spin pay 99%.
                let solve_at = |scatter: u32| {
                    let m = Machine { scatter, ..base };
                    let s = parts(&m, Feature::Spin, false, n, seed);
                    let b = parts(&m, Feature::Wild, false, n, seed + 1);
                    let c = parts(&m, Feature::Chance, false, n, seed + 2);
                    let nn = s.n as f64;
                    let en = (target(machine, Feature::Spin, false) - s.plain / nn - s.bonus_rate() * w) / (s.natural / nn);
                    let nb = b.n as f64;
                    let es = (target(machine, Feature::Wild, false) - b.plain / nb - b.natural / nb * en - b.bonus_rate() * w) / (b.start / nb);
                    let chance = (c.reels(0.0, en, 0.0, 0.0) + c.bonus_rate() * w) / target(machine, Feature::Chance, false);
                    println!("  scatter {scatter}: a bomb {en:.3}, a Bomb Spin's {es:.3}, Bonus Chance {:.3}%, bonus 1 in {:.1}", chance * 99.0, 1.0 / s.bonus_rate());
                    (en, es, chance - 1.0, s, b)
                };
                let (mut a, mut c) = (2_300u32, 2_500u32);
                let (mut fa, mut fc) = (solve_at(a).2, solve_at(c).2);
                for _ in 0..5 {
                    if (fc - fa).abs() < 1e-9 {
                        break;
                    }
                    let x = (c as f64 - fc * (c as f64 - a as f64) / (fc - fa)).round().clamp(1_500.0, 3_500.0) as u32;
                    if x == a || x == c {
                        break;
                    }
                    let fx = solve_at(x).2;
                    (a, fa, c, fc) = (c, fc, x, fx);
                }
                let (en, es, ch, s, b) = solve_at(c);
                let (nn, nb) = (s.n as f64, b.n as f64);
                println!(
                    "{machine} {}: scatter {c}: a bomb {en:.3} on average (now {:.3}), a Bomb Spin's {es:.3} (now {:.3}); Bonus Chance {:.3}%; bonus 1 in {:.1}, chain {:.4} with no bomb, {:.4} per natural bomb, {:.4} per Bomb Spin bomb",
                    reels::machine_name(machine),
                    mean(base.bombs),
                    mean(base.start_bombs),
                    (ch + 1.0) * 99.0,
                    1.0 / s.bonus_rate(),
                    s.plain / nn,
                    s.natural / nn,
                    b.start / nb,
                );
            }
            Hold => {
                // Buy Hold & Win: the Grand about 1 in 1,550 (a coin's chance on an empty place
                // each respin), and its coins 35.49 times the bet, what's left of 36.50 at 99%;
                // then the coins' weight so a spin pays 99%.
                let grand_at = |hold_coin: u32| {
                    let m = Machine { hold_coin, ..base };
                    let b = parts(&m, Feature::Wild, false, n, seed + 1);
                    let rate = b.grand_rate();
                    println!("  hold coin {hold_coin}: the Grand 1 in {:.0}, {:.3} coins' worth", 1.0 / rate.max(1e-12), b.coins / b.n as f64 * 100.0);
                    (rate, b)
                };
                let want = 1.0 / 1_550.0;
                let (mut lo, mut hi) = (2_000u32, 6_000u32);
                while hi - lo > 40 {
                    let mid = (lo + hi) / 2;
                    if grand_at(mid).0 < want { lo = mid } else { hi = mid }
                }
                let hold_coin = (lo + hi) / 2;
                let (rate, b) = grand_at(hold_coin);
                let base = Machine { hold_coin, ..base };
                let nb = b.n as f64;
                // (Coins counted at a hundredth of the bet each: what one's worth, in hundredths.)
                let coin = 35.49 / (b.coins / nb);
                let at = |special: u32| {
                    let m = Machine { special, ..base };
                    let s = parts(&m, Feature::Spin, false, n, seed);
                    let grand = if reelrow::progressive(machine) { 0.0 } else { GRAND_X as f64 };
                    let r = (s.reels(0.0, 0.0, 0.0, coin) + s.bonus_rate() * w + s.grand_rate() * grand) / target(machine, Feature::Spin, false);
                    println!("  coin weight {special}: spin {:.3}%, Hold & Win 1 in {:.1}, lines {:.4}, coins {:.4}", r * 99.0, s.n as f64 / s.holds.max(1) as f64, s.plain / s.n as f64, s.coins / s.n as f64 * coin);
                    r
                };
                let (mut a, mut c) = (11_000u32, 13_000u32);
                let (mut fa, mut fc) = (at(a) - 1.0, at(c) - 1.0);
                for _ in 0..6 {
                    if (fc - fa).abs() < 1e-9 {
                        break;
                    }
                    let x = (c as f64 - fc * (c as f64 - a as f64) / (fc - fa)).round().clamp(1_000.0, 25_000.0) as u32;
                    if x == a || x == c {
                        break;
                    }
                    let fx = at(x) - 1.0;
                    (a, fa, c, fc) = (c, fc, x, fx);
                }
                println!(
                    "{machine} {}: hold coin {hold_coin} (the Grand 1 in {:.0}), a coin {:.4} times the bet on average (now {:.4}), coin weight {c}",
                    reels::machine_name(machine),
                    1.0 / rate,
                    coin / 100.0,
                    mean(base.coins) / 100.0,
                );
            }
        }
    }
}

/// Ante's scatter weight, machine by machine, so a spin a quarter dearer pays 99%
/// (`PYR_RR_MACHINE`, `PYR_RR_ROUNDS`; `cargo test -p pyr-poker remaster_ante -- --ignored
/// --nocapture`).
#[test]
#[ignore]
fn remaster_ante() {
    let n = rounds();
    let only: Option<u8> = std::env::var("PYR_RR_MACHINE").ok().and_then(|v| v.parse().ok());
    for machine in 0..12u8 {
        if only.is_some_and(|o| o != machine) {
            continue;
        }
        let base = MACHINES[machine as usize];
        let at = |ante_scatter: u32| {
            let m = Machine { ante_scatter, ..base };
            let p = parts(&m, Feature::Spin, true, n, 300 + machine as u64);
            rtp_of(machine, &m, &p, Feature::Spin, true)
        };
        let (mut lo, mut hi) = (base.scatter, base.scatter + 1_500);
        while hi - lo > 1 {
            let mid = (lo + hi) / 2;
            if at(mid) < 1.0 { lo = mid } else { hi = mid }
        }
        println!("{machine} {}: ante scatter {lo} ({:.3}%) or {hi} ({:.3}%)", reels::machine_name(machine), at(lo) * 99.0, at(hi) * 99.0);
    }
}

/// Every machine, every way to play it on the reels (a spin, with Ante or without, Bonus Chance,
/// its feature spin) pays back 99%, worked out by playing it (bombs, coins and Hot Wilds at
/// what they're worth on average: see [`Parts`]); and the rest of Play Feature is Reel Row's,
/// at Reel Row's prices.
#[test]
fn every_way_to_play_pays_back_99_percent() {
    let n = std::env::var("PYR_RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1_500_000);
    for machine in 0..12u8 {
        let m = &MACHINES[machine as usize];
        for (what, feature, ante) in [("a spin", Feature::Spin, false), ("Ante", Feature::Spin, true), ("Bonus Chance", Feature::Chance, false), (m.special_name, Feature::Wild, false)] {
            let p = parts(m, feature, ante, n, 1_000 + machine as u64 * 10 + ante as u64);
            let rtp = rtp_of(machine, m, &p, feature, ante);
            assert!((0.985..0.995).contains(&rtp), "{} ({machine}), {what}: {:.3}%", reels::machine_name(machine), rtp * 100.0);
        }
        for f in [Feature::Bonus, Feature::Super, Feature::Mystery, Feature::AllIn] {
            assert_eq!(price(machine, f, false), reelrow::price(f));
        }
    }
}

/// What 0.13.28's Info pages and Play Feature say, machine by machine.
#[test]
fn the_numbers_on_the_info_pages() {
    let points = [267, 300, 209, 222, 304, 223, 300, 387, 223, 308, 234, 227];
    let specials = [753, 3_650, 175, 169, 3_585, 169, 3_650, 563, 170, 3_587, 1_597, 511];
    let caps = [10_000, 50_000, 500_000, 5_000, 0, 20_000, 50_000, 7_500, 10_000, 0, 5_000, 25_000];
    for machine in 0..12u8 {
        let m = &MACHINES[machine as usize];
        assert_eq!(m.point, points[machine as usize]);
        assert_eq!(price(machine, Feature::Wild, false), specials[machine as usize]);
        assert_eq!(price(machine, Feature::Chance, false), if m.family == Tumble { 189 } else { 200 });
        assert_eq!(price(machine, Feature::Spin, true), 125);
        assert_eq!(m.cap_x, caps[machine as usize]);
        // A round's top is the top prize's.
        if let reelrow::Top::Fixed(x) = reelrow::TOPS[machine as usize].top {
            assert_eq!(m.cap_x, x);
        }
        // Bombs land about a cell in seventy; their worth from 2× to 100× (Cosmic's to 500×).
        if m.family == Tumble {
            let w = weights(m, 0, false, When::Spin);
            let p = w[BOMB as usize] as f64 / OUT_OF as f64;
            assert!((1.0 / p - 70.0).abs() < 1.0, "{p}");
            let most = m.bombs.iter().map(|b| b.0).max().unwrap();
            assert_eq!(most, if machine == 2 { 500 } else { 100 });
            assert_eq!(m.bombs.iter().map(|b| b.0).min(), Some(2));
            assert!(m.start_bombs.iter().all(|b| b.0 >= 2 && b.0 <= most));
        }
    }
    // Hot Wilds: ×2 six times in ten, else ×3 or ×5.
    let total: u32 = HOT_X.iter().map(|h| h.1).sum();
    assert_eq!(HOT_X[0], (2, total * 6 / 10));
    assert_eq!(HOT_X.iter().map(|h| h.0).collect::<Vec<_>>(), vec![2, 3, 5]);
    // The jackpot coins: 20×, 50×, 200× the bet; the Grand 1,000×.
    assert_eq!([MINI, MINOR, MAJOR], [2_000, 5_000, 20_000]);
    assert!(COINS.iter().any(|c| c.0 == MINI) && COINS.iter().any(|c| c.0 == MINOR) && COINS.iter().any(|c| c.0 == MAJOR));
    assert_eq!(GRAND_X, 1_000);
}

/// Rounds played out by the rules: tumbles burst their wins and drop, bombs add up; Hold & Win
/// holds coins for three respins, back to three with each new one, collectors gathering what's
/// held; Hot Wilds fill their reel with a multiplier; frozen wilds stay; spotlight wilds walk;
/// Espresso Shots stay one more spin. Each round pays what its parts add up to, costs what its
/// feature does, plays again the same from its seed, and checks as the player's Pyr checks it.
#[test]
fn rounds_play_out_by_the_rules() {
    let mut s = Stream::new([5; 32]);
    let mut seen = std::collections::HashSet::new();
    for i in 0..24_000u32 {
        let machine = (i % 12) as u8;
        let m = &MACHINES[machine as usize];
        let feature = [Feature::Spin, Feature::Spin, Feature::Chance, Feature::Wild][(i / 12 % 4) as usize];
        let ante = feature == Feature::Spin && i / 48 % 2 == 1;
        let bet = [20, 100, 1_000][(i / 96 % 3) as usize];
        let seed = s.next_u32();
        let mut st = Stream::new(crate::fair::seed_from(seed as u64));
        let o = play_with(machine, bet, feature, ante, &mut st, 123_456_789);
        assert_eq!(o.cost, feature::cost(bet, price(machine, feature, ante)));
        // Played again from the seed: the same.
        let mut st = Stream::new(crate::fair::seed_from(seed as u64));
        assert_eq!(play_with(machine, bet, feature, ante, &mut st, 123_456_789), o);
        // Written and read back: the same.
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(serde_json::from_str::<Outcome>(&json).unwrap(), o);
        // Pays what its parts add up to (up to the machine's most).
        let parts = o.ways_paid + o.collected + o.hold_paid + o.grand + o.bonus.as_ref().map(|b| b.paid()).unwrap_or(0);
        let cap = if m.cap_x > 0 { bet * m.cap_x } else { crate::casino::MAX_PAYOUT };
        assert_eq!(o.paid, parts.min(cap), "{o:?}");
        assert_eq!(o.capped, parts > cap);
        assert_eq!(o.bonus.is_some(), o.scatters >= 3);
        // Every screen's wins are its ways.
        for st in &o.steps {
            assert_eq!(st.wins, ways(&st.grid, &st.x, bet, m.point));
        }
        match m.family {
            Tumble => {
                // Each screen bursts its wins; what's left drops; the last has no win.
                for w in o.steps.windows(2) {
                    let (a, b) = (&w[0], &w[1]);
                    assert!(!a.wins.is_empty());
                    let gone = burst(&a.grid, &a.wins);
                    for r in 0..REELS {
                        let kept: Vec<(u8, u32)> = (0..ROWS).rev().filter(|&row| gone & bit(r, row) == 0).map(|row| (a.grid[r][row], a.values[r][row])).collect();
                        let now: Vec<(u8, u32)> = (0..ROWS).rev().map(|row| (b.grid[r][row], b.values[r][row])).take(kept.len()).collect();
                        assert_eq!(kept, now);
                        for row in 0..ROWS {
                            if a.grid[r][row] == BOMB || a.grid[r][row] == SCATTER {
                                assert_eq!(gone & bit(r, row), 0);
                            }
                        }
                    }
                }
                let last = o.steps.last().unwrap();
                assert!(last.wins.is_empty() || o.steps.len() == MAX_STEPS);
                let points: u64 = o.steps.iter().flat_map(|s| &s.wins).map(|w| w.points).sum();
                let bombs: u32 = (0..PLACES).filter(|&k| last.grid[k / ROWS][k % ROWS] == BOMB).map(|k| last.values[k / ROWS][k % ROWS]).sum();
                let mult = if points > 0 { bombs.max(1) } else { 1 };
                assert_eq!(o.ways_paid, share(bet, points * m.point * mult as u64, 10_000));
                assert_eq!(o.mult, if points > 0 && bombs > 1 { bombs } else { 0 });
                for st in &o.steps {
                    for k in 0..PLACES {
                        let (sym, v) = (st.grid[k / ROWS][k % ROWS], st.values[k / ROWS][k % ROWS]);
                        assert_eq!(sym == BOMB, v > 0);
                        if sym == BOMB {
                            assert!(m.bombs.iter().chain(m.start_bombs.iter()).any(|b| b.0 == v));
                        }
                    }
                }
                if feature == Feature::Wild {
                    assert!(count(&o.steps[0].grid, BOMB) >= 3);
                }
                if o.mult > 1 {
                    seen.insert("bombs");
                }
                if o.steps.len() >= 3 {
                    seen.insert("tumbles");
                }
            }
            Hold => {
                if feature == Feature::Wild {
                    assert!(o.steps.is_empty());
                    assert_eq!(count(&o.hold[0].grid, COIN), 6);
                } else {
                    let g = &o.steps[0];
                    let coins = count(&g.grid, COIN) as usize;
                    assert_eq!(o.hold.is_empty(), coins < HOLD_START);
                    let worth: u32 = (0..PLACES).filter(|&k| g.grid[k / ROWS][k % ROWS] == COIN).map(|k| g.values[k / ROWS][k % ROWS]).sum();
                    let collectors = (0..ROWS).filter(|&row| g.grid[REELS - 1][row] == COLLECTOR).count() as u64;
                    assert_eq!(o.collected, share(bet, worth as u64 * collectors, 100));
                    if collectors > 0 && worth > 0 {
                        seen.insert("collected");
                    }
                }
                if !o.hold.is_empty() {
                    seen.insert("hold");
                    // Coins held stay as they were; three respins without a new one end it.
                    let mut quiet = 0;
                    for w in o.hold.windows(2) {
                        let (a, b) = (&w[0], &w[1]);
                        let mut new = false;
                        for k in 0..PLACES {
                            let (r, row) = (k / ROWS, k % ROWS);
                            if a.grid[r][row] != BLANK {
                                assert_eq!((a.grid[r][row], a.values[r][row]), (b.grid[r][row], b.values[r][row]));
                                assert_ne!(b.held & bit(r, row), 0);
                            } else if b.grid[r][row] != BLANK {
                                new = true;
                                assert!(b.grid[r][row] == COIN || r == REELS - 1);
                                if b.grid[r][row] == COLLECTOR {
                                    let coins: u32 = (0..PLACES).filter(|&j| b.grid[j / ROWS][j % ROWS] == COIN).map(|j| b.values[j / ROWS][j % ROWS]).sum();
                                    assert_eq!(b.values[r][row], coins);
                                    seen.insert("gathered");
                                }
                            }
                        }
                        quiet = if new { 0 } else { quiet + 1 };
                        assert!(quiet <= RESPINS as usize);
                    }
                    let last = o.hold.last().unwrap();
                    let full = last.grid.iter().flatten().all(|&c| c != BLANK);
                    assert!(full || quiet == RESPINS as usize, "{:?}", o.hold);
                    let worth: u64 = last.values.iter().flatten().map(|&x| x as u64).sum();
                    assert_eq!(o.hold_paid, share(bet, worth, 100));
                    assert_eq!(o.grand > 0, full);
                    if full {
                        seen.insert("grand");
                        assert_eq!(o.grand, if reelrow::progressive(machine) { reelrow::top_win(machine, bet, 123_456_789) } else { bet * GRAND_X });
                    }
                }
            }
            Hot => {
                let g = &o.steps[0];
                for r in 0..REELS {
                    let full = g.grid[r].iter().all(|&c| c == WILD);
                    assert_eq!(g.x[r] > 0, full && (1..REELS - 1).contains(&r));
                    assert!(g.grid[r].iter().all(|&c| c != WILD) || full);
                    if g.x[r] > 0 {
                        assert!([2, 3, 5].contains(&g.x[r]));
                    }
                }
                if feature == Feature::Wild {
                    assert!(g.x[REELS / 2] > 0);
                }
                if g.x.iter().filter(|&&x| x > 0).count() >= 2 && !g.wins.is_empty() {
                    seen.insert("hot twice");
                }
            }
            Frozen => {
                assert!(o.steps.len() <= MAX_FROZEN + 1);
                for w in o.steps.windows(2) {
                    let (a, b) = (&w[0], &w[1]);
                    assert_eq!(b.held, mask(&a.grid, WILD));
                    assert_eq!(mask(&b.grid, WILD) & b.held, b.held);
                }
                // A respin follows every screen that brought a new wild (up to twenty).
                for (i, st) in o.steps.iter().enumerate() {
                    let new = mask(&st.grid, WILD) & !st.held != 0;
                    assert_eq!(i + 1 < o.steps.len(), new && i < MAX_FROZEN, "{i} {:?}", o.steps);
                }
                if feature == Feature::Wild {
                    assert!(count(&o.steps[0].grid, WILD) >= 2);
                }
                if o.steps.len() >= 3 {
                    seen.insert("frozen");
                }
            }
            Spotlight => {
                for w in o.steps.windows(2) {
                    let (a, b) = (&w[0], &w[1]);
                    assert_eq!(b.held, mask(&a.grid, WILD) >> ROWS);
                    assert_ne!(b.held, 0);
                }
                assert_eq!(mask(&o.steps.last().unwrap().grid, WILD) >> ROWS, 0);
                if feature == Feature::Wild {
                    assert!(o.steps.len() >= 5);
                }
                if o.steps.len() >= 4 {
                    seen.insert("walk");
                }
            }
            Espresso => {
                for (i, st) in o.steps.iter().enumerate() {
                    for r in 0..REELS {
                        assert!(st.grid[r].iter().all(|&c| c == WILD) || !st.grid[r].contains(&WILD));
                    }
                    if i > 0 {
                        // What stayed is what landed new on the screen before.
                        let prev = &o.steps[i - 1];
                        let landed: u16 = (1..REELS - 1).filter(|&r| prev.grid[r][0] == WILD && prev.held & bit(r, 0) == 0).map(|r| 0b111 << (r * ROWS)).sum();
                        assert_eq!(st.held, landed);
                    }
                }
                if feature == Feature::Wild {
                    assert!(o.steps[0].grid[1][0] == WILD && o.steps[0].grid[3][0] == WILD && o.steps.len() >= 2);
                }
                if o.steps.len() >= 3 {
                    seen.insert("shots");
                }
            }
        }
        // The house's check, as the player's Pyr does it.
        let commit = crate::casino::commit(&[1; 32]);
        let mut st2 = crate::casino::stream(&[1; 32], &[2; 32], crate::casino::Game::Reels, i as u64);
        let round = crate::casino::Outcome::Remaster(play_with(machine, bet, feature, ante, &mut st2, 0));
        crate::casino::verify(&commit, &[1; 32], &[2; 32], i as u64, &round).unwrap();
    }
    for what in ["bombs", "tumbles", "collected", "hold", "gathered", "hot twice", "frozen", "walk", "shots"] {
        assert!(seen.contains(what), "never saw {what}");
    }
}

/// Bonus Chance starts the bonus 1 spin in 44.5 (nothing after the spin brings a scatter); Ante
/// makes it about 1.7 times as likely (1.5 on the tumbling machines), and is for spins only.
#[test]
fn bonus_chance_and_ante() {
    for machine in 0..12u8 {
        let m = &MACHINES[machine as usize];
        let c = parts(m, Feature::Chance, false, 400_000, 77);
        assert!((1.0 / c.bonus_rate() - 44.5).abs() < 2.0, "{machine}: 1 in {}", 1.0 / c.bonus_rate());
        let s = parts(m, Feature::Spin, false, 2_000_000, 78);
        let a = parts(m, Feature::Spin, true, 2_000_000, 78);
        let k = a.bonus_rate() / s.bonus_rate();
        let want = if m.family == Tumble { 1.5 } else { 1.7 };
        assert!((k - want).abs() < 0.1, "{machine}: {k}");
    }
    assert!(check(0, 100, Feature::Chance, true).is_err());
    assert!(check(0, 100, Feature::Spin, true).is_ok());
}
