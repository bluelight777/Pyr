//! Ember Dragon: Pyr Casino's own slot. Five reels by three rows, 243 ways, a dragon that grows
//! as it's fed, and Ember Respins (hold and spin) with four jackpots.
//!
//! - Every cell is drawn on its own from the same weights (wilds on reels 2 to 4 only). A win is
//!   three or more of a symbol on adjacent reels from the left, in any row (wilds stand in for
//!   the six pay symbols); it pays its points times the ways it's made, times the dragon's age.
//! - The dragon: every egg that lands feeds it. It grows from a Hatchling (wins ×1) through Young
//!   (×2), Adult (×3) and Ancient (×4) to an Elder (×5). Its growth is kept for each bet size (so
//!   growing it on small bets can't make big ones pay more), shown before every spin and played
//!   from: a spin's wins are at the age the dragon had as it began. When Ember Respins are over
//!   it has spent its fire, and starts again from an egg.
//! - Ember Respins: six or more fireballs anywhere. They're held; the other spots spin again,
//!   three times, and each new fireball sticks and gives three more. Every fireball holds a
//!   prize (1 to 25 times the bet) or a jackpot. Fill all fifteen spots for the Grand as well.
//! - The jackpots: Mini, Minor, Major and Grand, 10, 25, 100 and 1,000 times the bet, each with
//!   a pool on top that grows with every spin on the server (a share of each bet) and pays its
//!   share for the bet (all of it at the most bet). A pool is won once a round.
//!
//! Over a long run the reels and the respins pay back about 98.5% and the pools the 0.5% put
//! into them: about 99% (the tests work it out exactly). The cells, the fireballs' prizes and
//! the respins all come out of the round's random numbers, in that order, so anyone can play
//! a round again from its seeds, its bet, the dragon's growth and the pools as it began.

use serde::{Deserialize, Serialize};

use crate::fair::Stream;

pub const REELS: usize = 5;
pub const ROWS: usize = 3;
pub const SPOTS: usize = REELS * ROWS;
/// Symbols: 0 to 5 pay (low to high: sapphire, ruby, sword, chalice, crown, chest), then the
/// wild (the dragon coin), the fireball and the egg.
pub const PAY_SYMBOLS: u8 = 6;
pub const WILD: u8 = 6;
pub const FIRE: u8 = 7;
pub const EGG: u8 = 8;
/// Cell weights out of 1,000 (pay symbols, wild, fireball, egg). Reels 1 and 5 have no wild:
/// its weight goes to the two lowest.
const WEIGHTS: [u32; 9] = [250, 185, 140, 105, 75, 50, 40, 120, 35];
/// Points per way for three, four and five of a kind.
pub const PAY: [[u64; 3]; 6] = [[5, 10, 25], [6, 12, 30], [8, 16, 40], [10, 25, 60], [15, 40, 100], [25, 75, 250]];
/// One point is this many ten-thousandths of the bet (0.0085×); a spin's reel wins round to the
/// nearest chip.
pub const POINT_TENTHOUSANDTHS: u64 = 85;
/// The bets a spin can be.
pub const BETS: [u64; 9] = [20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000];
pub const MIN_BET: u64 = 20;
pub const MAX_BET: u64 = 10_000;

/// The dragon's ages: the growth each takes, its name, and what it multiplies reel wins by.
pub const AGES: [(u32, &str, u64); 5] = [(0, "Hatchling", 1), (20, "Young", 2), (45, "Adult", 3), (80, "Ancient", 4), (125, "Elder", 5)];
/// Growth is kept up to here (an Elder stays an Elder until its fire's spent).
pub const GROWTH_CAP: u32 = 125;

/// Ember Respins: the fireballs that start them, the respins (given again by each new
/// fireball), and the chance each empty spot lands one on a respin (ten-thousandths).
pub const TRIGGER: usize = 6;
pub const RESPINS: u8 = 3;
const RESPIN_FIRE: u32 = 800;

/// The four jackpots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Jackpot {
    Mini,
    Minor,
    Major,
    Grand,
}

impl Jackpot {
    pub const ALL: [Jackpot; 4] = [Jackpot::Mini, Jackpot::Minor, Jackpot::Major, Jackpot::Grand];

    pub fn name(self) -> &'static str {
        match self {
            Jackpot::Mini => "Mini",
            Jackpot::Minor => "Minor",
            Jackpot::Major => "Major",
            Jackpot::Grand => "Grand",
        }
    }

    /// Its fixed part, times the bet.
    pub fn base_x(self) -> u64 {
        match self {
            Jackpot::Mini => 10,
            Jackpot::Minor => 25,
            Jackpot::Major => 100,
            Jackpot::Grand => 1_000,
        }
    }

    /// Its pool's share of every spin's bet (hundredths of a percent), and what the house puts
    /// in to start it again once it's won.
    pub fn share_bp(self) -> u64 {
        match self {
            Jackpot::Mini => 5,
            Jackpot::Minor => 10,
            Jackpot::Major => 15,
            Jackpot::Grand => 20,
        }
    }

    pub fn seed(self) -> u64 {
        match self {
            Jackpot::Mini => 2_000,
            Jackpot::Minor => 10_000,
            Jackpot::Major => 50_000,
            Jackpot::Grand => 250_000,
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

/// The jackpots' pools (chips), Mini to Grand.
pub type Pools = [u64; 4];

/// What the pools start at.
pub fn seed_pools() -> Pools {
    Jackpot::ALL.map(|j| j.seed())
}

/// Each pool's share of a spin's bet (chips).
pub fn pool_shares(bet: u64) -> Pools {
    Jackpot::ALL.map(|j| (bet * j.share_bp()).div_ceil(10_000))
}

/// What a pool pays at `bet`: its share for the bet (all of it at the most bet).
pub fn pool_part(pool: u64, bet: u64) -> u64 {
    (pool as u128 * bet.min(MAX_BET) as u128 / MAX_BET as u128) as u64
}

/// A jackpot's value at `bet` with its pool at `pool`: its fixed part and its pool's share.
pub fn jackpot_value(j: Jackpot, pool: u64, bet: u64) -> u64 {
    j.base_x() * bet + pool_part(pool, bet)
}

/// What a fireball holds: a prize (times the bet), or a jackpot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum Fire {
    Prize { x: u64 },
    Jackpot { which: Jackpot },
}

impl Fire {
    /// Its worth, times the bet (a jackpot's fixed part).
    pub fn x(self) -> u64 {
        match self {
            Fire::Prize { x } => x,
            Fire::Jackpot { which } => which.base_x(),
        }
    }
}

/// What fireballs hold, with their weights.
const FIRES: [(Fire, u32); 12] = [
    (Fire::Prize { x: 1 }, 1_500),
    (Fire::Prize { x: 2 }, 1_250),
    (Fire::Prize { x: 3 }, 800),
    (Fire::Prize { x: 5 }, 600),
    (Fire::Prize { x: 8 }, 350),
    (Fire::Prize { x: 10 }, 225),
    (Fire::Prize { x: 15 }, 125),
    (Fire::Prize { x: 25 }, 60),
    (Fire::Jackpot { which: Jackpot::Mini }, 50),
    (Fire::Jackpot { which: Jackpot::Minor }, 15),
    (Fire::Jackpot { which: Jackpot::Major }, 4),
    (Fire::Jackpot { which: Jackpot::Grand }, 1),
];

/// grid[reel][row]
pub type Grid = [[u8; ROWS]; REELS];

/// A spot's number: reel by reel, top to bottom (reel 1's rows are 0 to 2).
pub fn spot(reel: usize, row: usize) -> u8 {
    (reel * ROWS + row) as u8
}

/// The dragon's age at `growth`: its index in `AGES`.
pub fn age(growth: u32) -> usize {
    AGES.iter().rposition(|a| growth >= a.0).unwrap_or(0)
}

pub fn age_mult(growth: u32) -> u64 {
    AGES[age(growth)].2
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WayWin {
    pub symbol: u8,
    /// Reels it runs across (3 to 5), the ways it's made, and its points.
    pub count: u8,
    pub ways: u32,
    pub points: u64,
}

/// Ember Respins, as they went.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Respins {
    /// Each respin: the fireballs it brought (spot, what it holds); none, and a respin's gone.
    pub spins: Vec<Vec<(u8, Fire)>>,
    /// Every fireball held at the end (spot, what it holds), and whether they filled the board.
    pub board: Vec<(u8, Fire)>,
    pub full: bool,
    /// The prizes and jackpots' fixed parts (and the Grand's, for a full board), times the bet.
    pub x: u64,
    /// The pools' shares won (Mini to Grand: each once a round), and the chips it all paid.
    pub pooled: Pools,
    pub paid: u64,
}

/// How a spin came out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub bet: u64,
    /// The dragon's growth for this bet as the spin began (its age multiplies the reel wins),
    /// and after it.
    pub growth: u32,
    pub growth_after: u32,
    pub grid: Grid,
    /// The fireballs the reels landed (spot, what it holds).
    pub fires: Vec<(u8, Fire)>,
    /// Eggs landed (each feeds the dragon).
    pub eggs: u32,
    pub wins: Vec<WayWin>,
    /// The reel wins, at the dragon's age.
    pub reels_paid: u64,
    pub respins: Option<Respins>,
    /// The jackpots' pools as the spin began (with this spin's shares in).
    pub pools: Pools,
    pub paid: u64,
}

impl Outcome {
    /// The jackpots it won.
    pub fn jackpots(&self) -> Vec<Jackpot> {
        let Some(r) = &self.respins else { return vec![] };
        let mut out: Vec<Jackpot> = r.board.iter().filter_map(|(_, f)| if let Fire::Jackpot { which } = f { Some(*which) } else { None }).collect();
        if r.full {
            out.push(Jackpot::Grand);
        }
        out
    }
}

/// Whether a spin can be bet.
pub fn check(bet: u64, growth: u32) -> Result<(), String> {
    if !BETS.contains(&bet) {
        return Err(format!("Ember Dragon's bets are {}", BETS.iter().map(|b| b.to_string()).collect::<Vec<_>>().join(", ")));
    }
    if growth > GROWTH_CAP {
        return Err("the dragon can't be that grown".into());
    }
    Ok(())
}

fn weights(reel: usize) -> [u32; 9] {
    let mut w = WEIGHTS;
    if reel == 0 || reel == REELS - 1 {
        w[0] += w[WILD as usize] / 2;
        w[1] += w[WILD as usize] - w[WILD as usize] / 2;
        w[WILD as usize] = 0;
    }
    w
}

fn draw_cell(reel: usize, s: &mut Stream) -> u8 {
    let w = weights(reel);
    let total: u32 = w.iter().sum();
    let mut x = s.below(total);
    for (i, wt) in w.iter().enumerate() {
        if x < *wt {
            return i as u8;
        }
        x -= wt;
    }
    0
}

fn draw_fire(s: &mut Stream) -> Fire {
    let total: u32 = FIRES.iter().map(|f| f.1).sum();
    let mut x = s.below(total);
    for (f, w) in FIRES {
        if x < w {
            return f;
        }
        x -= w;
    }
    FIRES[0].0
}

/// The reel wins in a grid: three or more of a kind from the left, wilds standing in.
pub fn ways(grid: &Grid) -> Vec<WayWin> {
    let mut out = Vec::new();
    for sym in 0..PAY_SYMBOLS {
        let mut ways = 1u32;
        let mut count = 0u8;
        for reel in grid {
            let n = reel.iter().filter(|c| **c == sym || **c == WILD).count() as u32;
            if n == 0 {
                break;
            }
            ways *= n;
            count += 1;
        }
        if count >= 3 {
            out.push(WayWin { symbol: sym, count, ways, points: PAY[sym as usize][count as usize - 3] * ways as u64 });
        }
    }
    out
}

/// Plays Ember Respins from the fireballs the reels landed.
fn respins(bet: u64, fires: &[(u8, Fire)], pools: &Pools, s: &mut Stream) -> Respins {
    let mut board: Vec<(u8, Fire)> = fires.to_vec();
    let mut left = RESPINS;
    let mut spins = Vec::new();
    while left > 0 && board.len() < SPOTS {
        let mut caught = Vec::new();
        for sp in 0..SPOTS as u8 {
            if board.iter().any(|(b, _)| *b == sp) {
                continue;
            }
            if s.below(10_000) < RESPIN_FIRE {
                caught.push((sp, draw_fire(s)));
            }
        }
        if caught.is_empty() {
            left -= 1;
        } else {
            left = RESPINS;
            board.extend(caught.iter().copied());
        }
        spins.push(caught);
    }
    board.sort_by_key(|(sp, _)| *sp);
    let full = board.len() == SPOTS;
    let mut x: u64 = board.iter().map(|(_, f)| f.x()).sum();
    // The jackpots' pools: each won once a round.
    let mut pooled = [0u64; 4];
    let won = |j: Jackpot, pooled: &mut Pools| {
        if pooled[j.index()] == 0 {
            pooled[j.index()] = pool_part(pools[j.index()], bet);
        }
    };
    for (_, f) in &board {
        if let Fire::Jackpot { which } = f {
            won(*which, &mut pooled);
        }
    }
    if full {
        x += Jackpot::Grand.base_x();
        won(Jackpot::Grand, &mut pooled);
    }
    let paid = x.saturating_mul(bet).saturating_add(pooled.iter().sum::<u64>());
    Respins { spins, board, full, x, pooled, paid }
}

/// A spin at `bet`, the dragon grown to `growth` for it and the jackpots' pools at `pools`.
pub fn play(bet: u64, growth: u32, pools: Pools, s: &mut Stream) -> Outcome {
    let mut grid: Grid = [[0; ROWS]; REELS];
    for (r, reel) in grid.iter_mut().enumerate() {
        for cell in reel.iter_mut() {
            *cell = draw_cell(r, s);
        }
    }
    let mut fires = Vec::new();
    let mut eggs = 0;
    for (r, reel) in grid.iter().enumerate() {
        for (row, c) in reel.iter().enumerate() {
            match *c {
                FIRE => fires.push((spot(r, row), draw_fire(s))),
                EGG => eggs += 1,
                _ => {}
            }
        }
    }
    let wins = ways(&grid);
    let points: u64 = wins.iter().map(|w| w.points).sum();
    let reels_paid = (points * POINT_TENTHOUSANDTHS * bet * age_mult(growth) + 5_000) / 10_000;
    let respins = (fires.len() >= TRIGGER).then(|| respins(bet, &fires, &pools, s));
    // Fed by its eggs; its fire spent by the respins.
    let growth_after = if respins.is_some() { 0 } else { (growth + eggs).min(GROWTH_CAP) };
    let paid = reels_paid.saturating_add(respins.as_ref().map(|r| r.paid).unwrap_or(0)).min(super::MAX_PAYOUT);
    Outcome { bet, growth, growth_after, grid, fires, eggs, wins, reels_paid, respins, pools, paid }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fair::seed_from;

    fn binom(n: usize, k: usize, p: f64) -> f64 {
        let mut c = 1.0;
        for i in 0..k {
            c = c * (n - i) as f64 / (i + 1) as f64;
        }
        c * p.powi(k as i32) * (1.0 - p).powi((n - k) as i32)
    }

    /// The reel wins' points a spin, exactly: for each symbol and run, the ways each reel makes
    /// (its cells that are the symbol or wild) multiplied out, and the next reel making none.
    fn ways_points() -> f64 {
        let mut ev = 0.0;
        for sym in 0..PAY_SYMBOLS as usize {
            let (mut e, mut p0) = (Vec::new(), Vec::new());
            for r in 0..REELS {
                let w = weights(r);
                let p = (w[sym] + w[WILD as usize]) as f64 / w.iter().sum::<u32>() as f64;
                e.push(ROWS as f64 * p);
                p0.push((1.0 - p).powi(ROWS as i32));
            }
            for k in 3..=5 {
                let prod: f64 = e[..k].iter().product();
                let tail = if k < 5 { p0[k] } else { 1.0 };
                ev += PAY[sym][k - 3] as f64 * prod * tail;
            }
        }
        ev
    }

    /// Ember Respins from `n` fireballs, exactly: the chance of each count at the end, by
    /// respins left.
    fn respins_from(n: usize, left: u8, memo: &mut std::collections::HashMap<(usize, u8), (f64, f64)>) -> (f64, f64) {
        if n == SPOTS {
            return (SPOTS as f64, 1.0);
        }
        if left == 0 {
            return (n as f64, 0.0);
        }
        if let Some(v) = memo.get(&(n, left)) {
            return *v;
        }
        let q = RESPIN_FIRE as f64 / 10_000.0;
        let m = SPOTS - n;
        let (mut e, mut full) = (0.0, 0.0);
        for k in 0..=m {
            let pk = binom(m, k, q);
            let (a, b) = if k == 0 { respins_from(n, left - 1, memo) } else { respins_from(n + k, RESPINS, memo) };
            e += pk * a;
            full += pk * b;
        }
        memo.insert((n, left), (e, full));
        (e, full)
    }

    struct Rtp {
        trigger: f64,
        respins_x: f64,
        mult: f64,
        reels: f64,
        respins: f64,
    }

    /// What it pays back over a long run, worked out exactly: the reel wins times the dragon's
    /// average age (its growth over spins, from egg to Elder and back once the respins spend
    /// its fire), and the respins (the fireballs held at the end, each worth its prize on
    /// average, and the Grand's fixed part for a full board). (Rounding to the chip aside, and
    /// the pools' shares, which pay back what's put into them.)
    fn rtp() -> Rtp {
        let w = WEIGHTS;
        let total = w.iter().sum::<u32>() as f64;
        let (f, e) = (w[FIRE as usize] as f64 / total, w[EGG as usize] as f64 / total);
        // Fireballs and eggs over the fifteen cells (each cell one or the other or neither).
        let mut joint: Vec<(u32, bool, f64)> = Vec::new();
        let fact = |n: usize| (1..=n).map(|x| x as f64).product::<f64>();
        for ne in 0..=SPOTS {
            for nf in 0..=SPOTS - ne {
                let no = SPOTS - ne - nf;
                let p = fact(SPOTS) / (fact(ne) * fact(nf) * fact(no)) * e.powi(ne as i32) * f.powi(nf as i32) * (1.0 - e - f).powi(no as i32);
                joint.push((ne as u32, nf >= TRIGGER, p));
            }
        }
        let trigger: f64 = joint.iter().filter(|j| j.1).map(|j| j.2).sum();
        // Respins: from how many fireballs, to how many.
        let mut memo = std::collections::HashMap::new();
        let (mut fin, mut full) = (0.0, 0.0);
        for n in TRIGGER..=SPOTS {
            let p = binom(SPOTS, n, f) / trigger;
            let (a, b) = respins_from(n, RESPINS, &mut memo);
            fin += p * a;
            full += p * b;
        }
        let fw: f64 = FIRES.iter().map(|x| x.1 as f64).sum();
        let per_fire: f64 = FIRES.iter().map(|(fi, wt)| fi.x() as f64 * *wt as f64).sum::<f64>() / fw;
        let respins_x = fin * per_fire + full * Jackpot::Grand.base_x() as f64;
        // The dragon's growth over spins (eggs feed it; the respins start it again).
        let cap = GROWTH_CAP as usize;
        let mut pi = vec![0.0; cap + 1];
        pi[0] = 1.0;
        for _ in 0..3_000 {
            let mut next = vec![0.0; cap + 1];
            for (g, pg) in pi.iter().enumerate() {
                if *pg == 0.0 {
                    continue;
                }
                for (ne, trig, p) in &joint {
                    if *trig {
                        next[0] += pg * p;
                    } else {
                        next[(g + *ne as usize).min(cap)] += pg * p;
                    }
                }
            }
            pi = next;
        }
        let mult: f64 = pi.iter().enumerate().map(|(g, p)| p * age_mult(g as u32) as f64).sum();
        let reels = ways_points() * POINT_TENTHOUSANDTHS as f64 / 10_000.0 * mult;
        Rtp { trigger, respins_x, mult, reels, respins: trigger * respins_x }
    }

    #[test]
    fn it_pays_back_about_99_percent() {
        let r = rtp();
        let total = r.reels + r.respins;
        eprintln!(
            "Ember Dragon: respins 1 in {:.0} spins worth {:.1}x the bet ({:.2}%), reel wins {:.2}% (the dragon's average age multiplies by {:.2}), {:.2}% in all, and the pools' 0.5%",
            1.0 / r.trigger,
            r.respins_x,
            r.respins * 100.0,
            r.reels * 100.0,
            r.mult,
            total * 100.0
        );
        assert!((0.983..0.988).contains(&total), "{total}");
        let pools: u64 = Jackpot::ALL.iter().map(|j| j.share_bp()).sum();
        assert_eq!(pools, 50, "the pools take half a percent");
        // Respins come often enough to look forward to, and pay well when they do.
        assert!((150.0..220.0).contains(&(1.0 / r.trigger)), "{}", 1.0 / r.trigger);
        assert!(r.respins_x > 40.0);
        // The dragon is seen at every age.
        assert!((2.5..3.6).contains(&r.mult), "{}", r.mult);
    }

    /// Played for real, at one bet, the dragon growing and starting again: what comes back is
    /// what the sums say, every outcome adds up, and a round plays again the same from its seed.
    #[test]
    fn spins_add_up_and_play_again_the_same() {
        let mut s = Stream::new(seed_from(42));
        let bet = 100;
        let pools = seed_pools();
        let (mut growth, mut wagered, mut paid, mut respins, mut elders) = (0u32, 0u64, 0u64, 0u32, 0u32);
        let mut ages = [0u32; 5];
        let n = 400_000;
        for i in 0..n {
            let seed = s.below(u32::MAX) as u64;
            let mut st = Stream::new(seed_from(seed ^ i));
            let o = play(bet, growth, pools, &mut st);
            // The same seed plays the same spin.
            if i % 10_000 == 0 {
                assert_eq!(play(bet, growth, pools, &mut Stream::new(seed_from(seed ^ i))), o);
            }
            assert_eq!(o.growth, growth);
            ages[age(growth)] += 1;
            if age(growth) == 4 {
                elders += 1;
            }
            let fires = o.grid.iter().flatten().filter(|c| **c == FIRE).count();
            assert_eq!(fires, o.fires.len());
            assert_eq!(o.respins.is_some(), fires >= TRIGGER);
            if let Some(r) = &o.respins {
                respins += 1;
                assert_eq!(o.growth_after, 0);
                assert!(r.board.len() >= TRIGGER && r.board.len() <= SPOTS);
                assert_eq!(r.full, r.board.len() == SPOTS);
                let x: u64 = r.board.iter().map(|(_, f)| f.x()).sum::<u64>() + if r.full { Jackpot::Grand.base_x() } else { 0 };
                assert_eq!(r.x, x);
                // Each spot held once.
                let mut spots: Vec<u8> = r.board.iter().map(|b| b.0).collect();
                spots.dedup();
                assert_eq!(spots.len(), r.board.len());
            } else {
                assert_eq!(o.growth_after, (growth + o.eggs).min(GROWTH_CAP));
            }
            assert_eq!(o.paid, o.reels_paid + o.respins.as_ref().map(|r| r.paid).unwrap_or(0));
            growth = o.growth_after;
            wagered += bet;
            paid += o.paid;
        }
        let rtp = paid as f64 / wagered as f64;
        eprintln!("{n} spins: {rtp:.4} back, {respins} respins, ages {ages:?}");
        // (A long run's luck: within a few percent of the sums.)
        assert!((0.93..1.05).contains(&rtp), "{rtp}");
        assert!(respins > 1_500 && elders > 0);
        assert!(ages.iter().all(|a| *a > n as u32 / 20), "every age is seen: {ages:?}");
    }

    /// Ten million spins against the sums, the reels and the respins each (a long run: `cargo
    /// test -- --ignored dragon_long_run --nocapture`).
    #[test]
    #[ignore]
    fn dragon_long_run() {
        let r = rtp();
        let mut s = Stream::new(seed_from(7));
        let (bet, pools) = (100u64, seed_pools());
        let (mut growth, mut reels, mut fixed, mut n_resp) = (0u32, 0u64, 0u64, 0u64);
        let n = 10_000_000u64;
        for i in 0..n {
            let seed = s.below(u32::MAX) as u64;
            let o = play(bet, growth, pools, &mut Stream::new(seed_from(seed ^ i)));
            reels += o.reels_paid;
            if let Some(rs) = &o.respins {
                fixed += rs.x * bet;
                n_resp += 1;
            }
            growth = o.growth_after;
        }
        let w = (n * bet) as f64;
        eprintln!(
            "{n} spins: reels {:.4} (sums {:.4}), respins {:.4} (sums {:.4}), respins 1 in {:.1} (sums {:.1})",
            reels as f64 / w,
            r.reels,
            fixed as f64 / w,
            r.respins,
            n as f64 / n_resp as f64,
            1.0 / r.trigger
        );
        assert!((reels as f64 / w - r.reels).abs() < 0.01);
        assert!((fixed as f64 / w - r.respins).abs() < 0.03);
    }

    #[test]
    fn ways_count_wilds_and_runs_from_the_left() {
        // Crowns on reels 1 to 3 (two on reel 2), a wild on reel 3, nothing on 4.
        let g: Grid = [[4, 0, 1], [4, 4, 2], [WILD, 3, 3], [0, 1, 2], [4, 4, 4]];
        let w = ways(&g);
        let crown = w.iter().find(|x| x.symbol == 4).unwrap();
        assert_eq!((crown.count, crown.ways, crown.points), (3, 2, 15 * 2));
        // Sapphires: reel 1, then a wild on reel 3 isn't next to it: no run.
        assert!(w.iter().all(|x| x.symbol != 0));
        assert_eq!(age(0), 0);
        assert_eq!(age(19), 0);
        assert_eq!(age(20), 1);
        assert_eq!(age(125), 4);
        assert_eq!(age_mult(80), 4);
        assert!(check(100, 0).is_ok() && check(99, 0).is_err() && check(100, 126).is_err());
        assert_eq!(pool_part(250_000, MAX_BET), 250_000);
        assert_eq!(pool_part(250_000, 100), 2_500);
        assert_eq!(jackpot_value(Jackpot::Grand, 250_000, 100), 100_000 + 2_500);
    }
}
