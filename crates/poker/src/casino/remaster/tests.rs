use super::*;
use crate::casino::reelrow::tests::{top_worth, worth};

mod exact;

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

/// Another sample (`PYR_RR_SEED`, added to the seeds): to check a tuning on rounds it wasn't
/// tuned on.
fn seed_from_env() -> u64 {
    std::env::var("PYR_RR_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0) * 1_000
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
    /// Bonuses the spin's own screen didn't start (tumbles brought the third scatter).
    pub later: u64,
    pub grands: u64,
    pub holds: u64,
    /// The way played (for the bonus's exact part).
    pub way: Option<(Feature, bool)>,
    pub first_p: f64,
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
        self.later += o.later;
        self.grands += o.grands;
        self.holds += o.holds;
    }

    /// What the reels pay on average (times the bet), bombs, coins and Hot Wilds worth this
    /// much on average.
    pub(crate) fn reels(&self, hot_x: f64, natural: f64, start: f64, coin: f64) -> f64 {
        let n = self.n as f64;
        (self.plain + (0..4).map(|k| self.hot[k] * hot_x.powi(k as i32)).sum::<f64>() + self.natural * natural + self.start * start + self.coins * coin) / n
    }

    /// How often the bonus comes: the spin's own screen's chance worked out exactly, and what
    /// the tumbles add, played.
    pub(crate) fn bonus_rate(&self) -> f64 {
        self.first_p + self.later as f64 / self.n as f64
    }

    /// As played (to check the exact part by).
    pub(crate) fn bonus_rate_played(&self) -> f64 {
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
                        p.later += (r.scatters >= 3 && r.steps.first().is_some_and(|s| count(&s.grid, SCATTER) < 3)) as u64;
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
    p.way = Some((feature, ante));
    p.first_p = first_bonus_p(m, feature, ante);
    p
}

fn binom(n: usize, k: usize, p: f64) -> f64 {
    let c = (0..k).fold(1.0, |a, i| a * (n - i) as f64 / (i + 1) as f64);
    c * p.powi(k as i32) * (1.0 - p).powi((n - k) as i32)
}

/// The chance the spin's own screen shows three scatters or more (before any tumble), worked out
/// exactly: Bonus Chance's is its own; a Hot Wild or an Espresso Shot covers its reel's; a Bomb
/// Spin's bombs, a Frozen Spin's wilds and an Encore's cover cells.
pub(crate) fn first_bonus_p(m: &Machine, feature: Feature, ante: bool) -> f64 {
    match feature {
        Feature::Chance => return m.chance_ppm as f64 / 1e6,
        Feature::Wild if m.family == Hold => return 0.0,
        _ => {}
    }
    let sc = (if ante { m.ante_scatter } else { m.scatter }) as f64 / OUT_OF as f64;
    let pw = m.wild as f64 / OUT_OF as f64;
    let special = feature == Feature::Wild;
    let mut dist = vec![1.0f64];
    let covering = matches!(m.family, Hot | Espresso);
    for r in 0..REELS {
        let d: Vec<f64> = if covering && (1..REELS - 1).contains(&r) {
            let forced = special && ((m.family == Hot && r == REELS / 2) || (m.family == Espresso && (r == 1 || r == REELS - 2)));
            if forced {
                vec![1.0]
            } else {
                let h = 1.0 - (1.0 - pw).powi(3);
                let c = sc / (1.0 - pw);
                let mut d: Vec<f64> = (0..=ROWS).map(|k| (1.0 - h) * binom(ROWS, k, c)).collect();
                d[0] += h;
                d
            }
        } else {
            (0..=ROWS).map(|k| binom(ROWS, k, sc)).collect()
        };
        let mut new = vec![0.0; dist.len() + ROWS];
        for (i, a) in dist.iter().enumerate() {
            for (j, b) in d.iter().enumerate() {
                new[i + j] += a * b;
            }
        }
        dist = new;
    }
    // Cells covered after the spin's drawn (each drawn cell as likely a scatter as any other).
    let covered = match (m.family, special) {
        (Frozen, true) => FROZEN_SPIN_WILDS,
        (Spotlight, true) => 1,
        (Tumble, true) => BOMB_SPIN_BOMBS,
        _ => 0,
    };
    if covered > 0 {
        let cells = PLACES - covered;
        return 1.0 - (0..3).map(|k| binom(cells, k, sc)).sum::<f64>();
    }
    1.0 - dist.iter().take(3).sum::<f64>()
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
            let p = parts(m, feature, ante, n, 11 + machine as u64 + seed_from_env());
            let rtp = rtp_of(machine, m, &p, feature, ante);
            let nn = p.n as f64;
            println!(
                "{:>2} {:<18} {:<15} rtp {:>7.3}%  plain {:.4} hot {:.4},{:.4},{:.4},{:.4} natural {:.5} start {:.5} coins {:.4}  bonus 1 in {:>6.1} (played {:.1})  holds 1 in {:>7.1}  grand 1 in {:.0}",
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
                1.0 / p.bonus_rate(),
                1.0 / p.bonus_rate_played().max(1e-12),
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
                // A bomb's worth so Bonus Chance pays 99% (its bonus 1 in 44.5 exactly); then the
                // scatter's weight so a spin does (the tumbles bringing some of its bonuses); then
                // a Bomb Spin's bombs' worth; then Ante's scatter.
                let c = parts(&base, Feature::Chance, false, n, seed + 2);
                let nc = c.n as f64;
                let en = (target(machine, Feature::Chance, false) - c.plain / nc - c.bonus_rate() * w) / (c.natural / nc);
                let spin_at = |scatter: u32| {
                    let m = Machine { scatter, ..base };
                    let s = parts(&m, Feature::Spin, false, n, seed);
                    let r = (s.reels(0.0, en, 0.0, 0.0) + s.bonus_rate() * w) / target(machine, Feature::Spin, false);
                    println!("  scatter {scatter}: spin {:.3}%, bonus 1 in {:.1} (the spin's own screen 1 in {:.1})", r * 99.0, 1.0 / s.bonus_rate(), 1.0 / s.first_p);
                    (r - 1.0, s)
                };
                let (mut a, mut cc) = (2_400u32, 2_550u32);
                let (mut fa, mut fc) = (spin_at(a).0, spin_at(cc).0);
                for _ in 0..5 {
                    if (fc - fa).abs() < 1e-9 {
                        break;
                    }
                    let x = (cc as f64 - fc * (cc as f64 - a as f64) / (fc - fa)).round().clamp(1_500.0, 3_500.0) as u32;
                    if x == a || x == cc {
                        break;
                    }
                    let fx = spin_at(x).0;
                    (a, fa, cc, fc) = (cc, fc, x, fx);
                }
                let scatter = cc;
                let (_, s) = spin_at(scatter);
                let m = Machine { scatter, ..base };
                let b = parts(&m, Feature::Wild, false, n, seed + 1);
                let nb = b.n as f64;
                let es = (target(machine, Feature::Wild, false) - b.plain / nb - b.natural / nb * en - b.bonus_rate() * w) / (b.start / nb);
                let ante_at = |ante_scatter: u32| {
                    let m = Machine { ante_scatter, ..m };
                    let a = parts(&m, Feature::Spin, true, n, seed + 3);
                    (a.reels(0.0, en, 0.0, 0.0) + a.bonus_rate() * w) / target(machine, Feature::Spin, true)
                };
                let (mut lo, mut hi) = (scatter, scatter + 1_000);
                while hi - lo > 1 {
                    let mid = (lo + hi) / 2;
                    if ante_at(mid) < 1.0 { lo = mid } else { hi = mid }
                }
                let ns = s.n as f64;
                println!(
                    "{machine} {}: scatter {scatter}, ante {lo}/{hi} ({:.3}%/{:.3}%); a bomb {en:.4} on average (now {:.4}), a Bomb Spin's {es:.4} (now {:.4}); bonus 1 in {:.1}; chain {:.4} with no bomb, {:.5} per natural bomb, {:.5} per Bomb Spin bomb",
                    reels::machine_name(machine),
                    ante_at(lo) * 99.0,
                    ante_at(hi) * 99.0,
                    mean(base.bombs),
                    mean(base.start_bombs),
                    1.0 / s.bonus_rate(),
                    s.plain / ns,
                    s.natural / ns,
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
/// its feature spin) pays back 99%; and the rest of Play Feature is Reel Row's, at Reel Row's
/// prices. The wild machines are worked out exactly (to a hundredth of a percent), and played to
/// check the reels play as worked out. The tumbling and Hold & Win machines are played, values
/// averaged (see [`Parts`]): a million and a half rounds pin them only to about ±0.3%, so here
/// they're held to 98-100%; how close to 99% they are is what `remaster_parts` says at twenty
/// million rounds a way (as 0.13.x worked it out), which is how they were tuned: within ±0.1%
/// (rebuild/notes-reelrow.md).
#[test]
fn every_way_to_play_pays_back_99_percent() {
    let n = std::env::var("PYR_RR_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(1_500_000);
    for machine in 0..12u8 {
        let m = &MACHINES[machine as usize];
        for (what, feature, ante) in [("a spin", Feature::Spin, false), ("Ante", Feature::Spin, true), ("Bonus Chance", Feature::Chance, false), (m.special_name, Feature::Wild, false)] {
            let name = reels::machine_name(machine);
            // The wild machines' spins worked out exactly (and played, to check the reels play
            // as worked out: within five standard errors); the rest played, values averaged.
            if let Some((rtp, _)) = exact::chance_rtp(machine, 40_000, 3_000 + machine as u64).filter(|_| feature == Feature::Chance) {
                assert!((0.989..0.991).contains(&rtp), "{name} ({machine}), {what}: {:.3}%", rtp * 100.0);
                continue;
            }
            if let Some(rtp) = exact::rtp(machine, feature, ante) {
                assert!((0.989..0.991).contains(&rtp), "{name} ({machine}), {what}: {:.3}% exactly", rtp * 100.0);
                let f = figures(machine, feature, ante, n / 3, 2_000 + machine as u64 * 10 + ante as u64);
                let (played, se) = f.rtp(machine, feature, ante);
                assert!((played - rtp).abs() < 5.0 * se, "{name} ({machine}), {what}: played {:.3}% ±{:.3}, worked out {:.3}%", played * 100.0, se * 100.0, rtp * 100.0);
                continue;
            }
            let p = parts(m, feature, ante, n, 1_000 + machine as u64 * 10 + ante as u64);
            let rtp = rtp_of(machine, m, &p, feature, ante);
            assert!((0.98..1.0).contains(&rtp), "{name} ({machine}), {what}: {:.3}%", rtp * 100.0);
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

/// The Hold & Win machines' coin weights, each so a spin pays 99% (the coins' worth and the
/// respins as they are), and Buy Hold & Win as it comes out (`PYR_RR_ROUNDS`; `cargo test -p
/// pyr-poker remaster_hold -- --ignored --nocapture`).
#[test]
#[ignore]
fn remaster_hold() {
    let n = rounds();
    for machine in [1u8, 4, 6, 9] {
        let base = MACHINES[machine as usize];
        let w = mini(machine);
        let coin = mean(base.coins);
        let b = parts(&base, Feature::Wild, false, n, 500 + machine as u64);
        let buy = rtp_of(machine, &base, &b, Feature::Wild, false);
        let at = |special: u32| {
            let m = Machine { special, ..base };
            let s = parts(&m, Feature::Spin, false, n, 600 + machine as u64);
            let grand = if reelrow::progressive(machine) { 0.0 } else { GRAND_X as f64 };
            (s.reels(0.0, 0.0, 0.0, coin) + s.bonus_rate() * w + s.grand_rate() * grand) / target(machine, Feature::Spin, false)
        };
        let (mut a, mut c) = (base.special - 150, base.special + 150);
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
        println!("{machine} {}: coin weight {c} ({:.3}%; now {}); Buy Hold & Win {:.3}%, the Grand 1 in {:.0}", reels::machine_name(machine), (fc + 1.0) * 99.0, base.special, buy * 100.0, 1.0 / b.grand_rate());
    }
}

/// The wild machines worked out exactly (`cargo test -p pyr-poker remaster_exact -- --ignored
/// --nocapture`).
#[test]
#[ignore]
fn remaster_exact() {
    for machine in 0..12u8 {
        let m = &MACHINES[machine as usize];
        for (what, feature, ante) in [("spin", Feature::Spin, false), ("ante", Feature::Spin, true), (m.special_name, Feature::Wild, false)] {
            if let Some(rtp) = exact::rtp(machine, feature, ante) {
                println!("{machine:>2} {:<18} {what:<15} {:.4}%", reels::machine_name(machine), rtp * 100.0);
            }
            if feature == Feature::Spin
                && !ante
                && let Some((rtp, ppm)) = exact::chance_rtp(machine, 400_000, 77)
            {
                println!("{machine:>2} {:<18} {:<15} {:.4}% (its screens with the bonus played): 99% at {ppm:.0} per million, 1 in {:.2}", reels::machine_name(machine), "Bonus Chance", rtp * 100.0, 1e6 / ppm);
            }
        }
    }
}

/// Bonus Chance's bonuses, counted three ways (a check).
#[test]
#[ignore]
fn remaster_chance_count() {
    let m = &MACHINES[2];
    let mut d = Quick::new(5);
    let (mut first, mut last, n) = (0u64, 0u64, 2_000_000u64);
    for _ in 0..n {
        let r = spin_on(m, false, Feature::Chance, 100, &mut d);
        first += (count(&r.steps[0].grid, SCATTER) >= 3) as u64;
        last += (r.scatters >= 3) as u64;
    }
    println!("decided 1 in {:.2}; first screen 1 in {:.2}, after the tumbles 1 in {:.2}", 1e6 / m.chance_ppm as f64, n as f64 / first as f64, n as f64 / last as f64);
}

/// Bomb Spins on the machine as given, `n` rounds, its bombs counted (each worth 1): what a
/// round pays on average is `a + b × μ` times the bet, μ being what a Bomb Spins bomb's worth on
/// average (a spin's win times 1 till a bomb's been on a win, then times the bombs' total).
/// Returns (a, b, spins a round).
pub(crate) fn bomb_spins_parts(m: &Machine, n: u64, seed: u64) -> (f64, f64, f64) {
    let threads = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(2).min(8) as u64;
    let c = Machine { free_bombs: &[(1, 1)], ..*m };
    let point = m.point as f64 / 10_000.0;
    let all: Vec<(f64, f64, u64)> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                let c = &c;
                sc.spawn(move || {
                    let mut d = Quick::new(seed.wrapping_mul(1_000_003).wrapping_add(t));
                    let (mut a, mut b, mut spins) = (0.0, 0.0, 0u64);
                    let rounds = n / threads + if t < n % threads { 1 } else { 0 };
                    for _ in 0..rounds {
                        for f in bomb_spins(c, 100_000, &mut d) {
                            let p: u64 = f.steps.iter().flat_map(|s| s.wins.iter()).map(|w| w.points).sum();
                            spins += 1;
                            if f.mult == 0 {
                                a += p as f64 * point;
                            } else {
                                b += (p * f.mult as u64) as f64 * point;
                            }
                        }
                    }
                    (a, b, spins)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (a, b, spins) = all.iter().fold((0.0, 0.0, 0u64), |x, y| (x.0 + y.0, x.1 + y.1, x.2 + y.2));
    (a / n as f64, b / n as f64, spins as f64 / n as f64)
}

/// Super Hold & Win on the machine as given, `n` rounds: what a round pays on average, times
/// the bet (the Grand at 1,000×; on a progressive machine, its pool apart), and how often the
/// Grand comes.
pub(crate) fn super_hold_x(m: &Machine, machine: u8, n: u64, seed: u64) -> (f64, f64) {
    let threads = std::thread::available_parallelism().map(|x| x.get()).unwrap_or(2).min(8) as u64;
    let grand = if reelrow::progressive(machine) { 0.0 } else { GRAND_X as f64 };
    let all: Vec<(f64, u64)> = std::thread::scope(|sc| {
        let hs: Vec<_> = (0..threads)
            .map(|t| {
                sc.spawn(move || {
                    let mut d = Quick::new(seed.wrapping_mul(1_000_003).wrapping_add(t));
                    let (mut x, mut grands) = (0.0, 0u64);
                    let rounds = n / threads + if t < n % threads { 1 } else { 0 };
                    for _ in 0..rounds {
                        let r = hold(m, false, Play::Builds, 100, &mut d);
                        x += r.held as f64 / 100.0;
                        if r.grand {
                            x += grand;
                            grands += 1;
                        }
                    }
                    (x, grands)
                })
            })
            .collect();
        hs.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let (x, g) = all.iter().fold((0.0, 0u64), |a, b| (a.0 + b.0, a.1 + b.1));
    (x / n as f64, g as f64 / n as f64)
}

/// The rounds that build: Bomb Spins' bombs' average worth for 99% (`a + b μ` at the price), and
/// Super Hold & Win's Payers and Hoarders scaled for 99% (`PYR_RR_ROUNDS`, `PYR_RR_MACHINE`;
/// `cargo test -p pyr-poker remaster_builds -- --ignored --nocapture`).
#[test]
#[ignore]
fn remaster_builds() {
    let n = rounds();
    let only: Option<u8> = std::env::var("PYR_RR_MACHINE").ok().and_then(|v| v.parse().ok());
    for machine in 0..12u8 {
        if only.is_some_and(|o| o != machine) {
            continue;
        }
        let mut m = MACHINES[machine as usize];
        // (To try other reels: PYR_RR_FREE="pay per thousand,bomb weight".)
        if let Some((pay, bomb)) = std::env::var("PYR_RR_FREE").ok().and_then(|v| v.split_once(',').and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))) {
            m.free_pay = pay;
            m.free_bomb = bomb;
        }
        let want = 0.99 * m.builds_price as f64 / 100.0;
        match m.family {
            Tumble => {
                let (a, b, spins) = bomb_spins_parts(&m, n, 500 + machine as u64 + seed_from_env());
                println!(
                    "{machine} {}: Bomb Spins at pay {} and bomb weight {}: a {a:.4}, b {b:.4}, {spins:.2} spins a round; a bomb's worth {:.4} now ({:.3}%), {:.4} for 99%",
                    reels::machine_name(machine),
                    m.free_pay,
                    m.free_bomb,
                    mean(m.free_bombs),
                    (a + b * mean(m.free_bombs)) / want * 99.0,
                    (want - a) / b
                );
            }
            Hold => {
                let at = |k: f64| {
                    let t = Machine { payer: (m.payer as f64 * k).round() as u32, hoarder: (m.hoarder as f64 * k).round() as u32, ..m };
                    super_hold_x(&t, machine, n, 600 + machine as u64 + seed_from_env())
                };
                let (now, g) = at(1.0);
                println!("{machine} {}: Super Hold & Win now {:.3}% (the Grand 1 in {:.0})", reels::machine_name(machine), now / want * 99.0, 1.0 / g.max(1e-12));
                let (mut lo, mut hi) = (0.2f64, 4.0f64);
                for _ in 0..14 {
                    let mid = (lo * hi).sqrt();
                    if at(mid).0 < want { lo = mid } else { hi = mid }
                }
                let k = (lo * hi).sqrt();
                println!("   for 99%: Payers and Hoarders ×{k:.4}: payer {}, hoarder {} ({:.3}%)", (m.payer as f64 * k).round(), (m.hoarder as f64 * k).round(), at(k).0 / want * 99.0);
            }
            _ => {}
        }
    }
}

/// Bomb Spins and Super Hold & Win play by their rules, the same again from the seed and
/// written and read back.
#[test]
fn the_rounds_that_build_play_by_the_rules() {
    let mut s = Stream::new([9; 32]);
    let (mut more_seen, mut payers, mut hoarders, mut built) = (0, 0, 0, 0);
    for i in 0..1_600u32 {
        let machine = [2u8, 3, 5, 8, 1, 4, 6, 9][(i % 8) as usize];
        let m = &MACHINES[machine as usize];
        let bet = [20, 100, 1_000][(i / 8 % 3) as usize];
        let seed = s.next_u32();
        let mut st = Stream::new(crate::fair::seed_from(seed as u64));
        let o = play_with(machine, bet, Feature::Builds, false, &mut st, 123_456_789);
        assert!(check(machine, bet, Feature::Builds, false).is_ok());
        assert_eq!(o.cost, feature::cost(bet, m.builds_price));
        let mut st = Stream::new(crate::fair::seed_from(seed as u64));
        assert_eq!(play_with(machine, bet, Feature::Builds, false, &mut st, 123_456_789), o);
        let json = serde_json::to_string(&o).unwrap();
        assert_eq!(serde_json::from_str::<Outcome>(&json).unwrap(), o);
        assert!(o.bonus.is_none() && o.steps.is_empty());
        let cap = if m.cap_x > 0 { bet * m.cap_x } else { crate::casino::MAX_PAYOUT };
        let parts = o.hold_paid + o.grand + o.free_paid();
        assert_eq!(o.paid, parts.min(cap));
        if m.family == Tumble {
            // Ten spins, and five more for every three scatters.
            let more = o.free.iter().filter(|f| f.more).count();
            assert_eq!(o.free.len(), (FREE_SPINS + MORE_SPINS * more).min(MAX_FREE_SPINS));
            more_seen += more;
            let mut mult = 0u32;
            for f in &o.free {
                let last = f.steps.last().unwrap();
                let points: u64 = f.steps.iter().flat_map(|s| s.wins.iter()).map(|w| w.points).sum();
                // Bombs on a win add to the multiplier; it stays.
                if points > 0 {
                    mult += (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).filter(|&(r, row)| last.grid[r][row] == BOMB).map(|(r, row)| last.values[r][row]).sum::<u32>();
                }
                assert_eq!(f.mult, mult);
                assert_eq!(f.paid, share(bet, points * m.point * mult.max(1) as u64, 10_000));
                assert_eq!(f.more, count(&last.grid, SCATTER) >= 3);
                for st in &f.steps {
                    assert_eq!(st.wins, ways(&st.grid, &st.x, bet, m.point));
                }
            }
            built += (mult > 0) as u32;
        } else {
            // Six coins to start; then as Hold & Win, with Payers and Hoarders.
            assert_eq!(o.hold[0].grid.iter().flatten().filter(|&&c| c == COIN).count(), HOLD_START);
            for w in o.hold.windows(2) {
                let (a, b) = (&w[0], &w[1]);
                let new = |r: usize, row: usize| a.grid[r][row] == BLANK && b.grid[r][row] != BLANK;
                let any_new = (0..REELS).any(|r| (0..ROWS).any(|row| new(r, row)));
                let paid: u32 = (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).filter(|&(r, row)| new(r, row) && b.grid[r][row] == PAYER).map(|(r, row)| b.values[r][row]).sum();
                let coins: u32 = (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).filter(|&(r, row)| matches!(b.grid[r][row], COIN | PAYER)).map(|(r, row)| b.values[r][row]).sum();
                let all: u32 = (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).filter(|&(r, row)| matches!(b.grid[r][row], COIN | PAYER | COLLECTOR)).map(|(r, row)| b.values[r][row]).sum();
                for r in 0..REELS {
                    for row in 0..ROWS {
                        match b.grid[r][row] {
                            // A coin held: what it was, and every new Payer's worth.
                            COIN if !new(r, row) => assert_eq!(b.values[r][row], a.values[r][row] + paid),
                            COLLECTOR if new(r, row) => assert_eq!(b.values[r][row], coins),
                            COLLECTOR => assert_eq!(b.values[r][row], a.values[r][row]),
                            // A Hoarder: everything else, as of the last respin that brought
                            // something new.
                            HOARDER => {
                                assert_eq!(b.values[r][row], if any_new { all } else { a.values[r][row] });
                                hoarders += new(r, row) as u32;
                            }
                            PAYER => payers += new(r, row) as u32,
                            _ => {}
                        }
                    }
                }
            }
            let last = o.hold.last().unwrap();
            assert_eq!(o.hold_paid, share(bet, last.values.iter().flatten().map(|&v| v as u64).sum(), 100));
        }
    }
    // (Seen in these rounds: five spins more, multipliers built, Payers and Hoarders.)
    assert!(more_seen > 0 && built > 0 && payers > 0 && hoarders > 0, "{more_seen} {built} {payers} {hoarders}");
}

/// The rounds that build pay back 99%: Bomb Spins worked out with its bombs counted (a round's
/// worth is `a + b μ`, see [`bomb_spins_parts`]), Super Hold & Win played. (A sample of a few
/// hundred thousand rounds pins Bomb Spins to about ±1.5%: `remaster_builds` at millions of rounds
/// is how they were set, within ±0.2%.)
#[test]
fn the_rounds_that_build_pay_back_99_percent() {
    for machine in [2u8, 3, 5, 8, 1, 4, 6, 9] {
        let m = &MACHINES[machine as usize];
        let price = m.builds_price as f64 / 100.0;
        let rtp = if m.family == Tumble {
            let (a, b, _) = bomb_spins_parts(m, 300_000, 40 + machine as u64);
            (a + b * mean(m.free_bombs)) / price
        } else {
            super_hold_x(m, machine, 3_000_000, 50 + machine as u64).0 / price
        };
        // (A few hundred thousand rounds of Bomb Spins pin it to about ±1.5%; three million
        // of Super Hold & Win to about ±0.2%.)
        let band = if m.family == Tumble { 0.965..1.015 } else { 0.982..0.998 };
        assert!(band.contains(&rtp), "{} ({machine}): {:.3}%", reels::machine_name(machine), rtp * 100.0);
    }
}

/// Super Hold & Win's Payers and Hoarders scaled for 99%, from two long samples (a round's
/// worth varies a lot: a million rounds pin it only to about ±0.35%). `PYR_RR_ROUNDS` a sample;
/// `cargo test -p pyr-poker remaster_super_hold -- --ignored --nocapture`.
#[test]
#[ignore]
fn remaster_super_hold() {
    let n = rounds();
    let only: Option<u8> = std::env::var("PYR_RR_MACHINE").ok().and_then(|v| v.parse().ok());
    for machine in [1u8, 4, 6, 9] {
        if only.is_some_and(|o| o != machine) {
            continue;
        }
        let m = MACHINES[machine as usize];
        let want = 0.99 * m.builds_price as f64 / 100.0;
        let at = |k: f64| {
            let t = Machine { payer: (m.payer as f64 * k).round() as u32, hoarder: (m.hoarder as f64 * k).round() as u32, ..m };
            super_hold_x(&t, machine, n, 700 + machine as u64 + seed_from_env()).0 / want * 0.99
        };
        let (a, b) = (at(1.0), at(1.04));
        let k = 1.0 + 0.04 * (0.99 - a) / (b - a);
        println!(
            "{machine} {}: now {:.3}%, at ×1.04 {:.3}%; for 99% ×{k:.4}: payer {}, hoarder {}",
            reels::machine_name(machine),
            a * 100.0,
            b * 100.0,
            (m.payer as f64 * k).round(),
            (m.hoarder as f64 * k).round()
        );
    }
}
