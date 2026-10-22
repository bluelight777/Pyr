//! Reel Row: twelve themed machines, five reels by three rows, 243 ways. The machines and their
//! math are Chimbita's Reel Row (MIT), brought into Pyr Casino for its chips and its fair rounds.
//!
//! Every cell is drawn on its own from the same weights; wilds land on reels 2 to 4 only. A win
//! is three or more of a symbol on adjacent reels from the left, in any row (wilds stand in for
//! the six pay symbols), and pays its points times the number of ways it's made. Three or more
//! scatters anywhere start the machine's bonus: most machines play pick-and-dodge, some spin a
//! prize wheel. Either can be bought for [`BUY_X`] times the bet.
//!
//! Pick-and-dodge is fifteen covered tiles: nine hold chips, one gold, one an extra life, four
//! a hazard. A hazard uses up a life; with none left it ends the bonus. Find all eleven safe
//! tiles for the finale. What the tiles hold comes out of the round's random numbers in the
//! order they're opened, so whichever tile is clicked, every pick has the same odds (the same
//! as drawing each tile's contents from what's left as it's picked, the way Reel Row did).
//!
//! The wheel has twenty slices of prizes and one Upgrade, which spins a smaller wheel of bigger
//! ones.
//!
//! Over a long run the base game pays back about 95% and the bonus about 4%: 99% in all, and
//! the same for a bought bonus (see the tests, which work it out exactly). (Reel Row paid back
//! 92.5% of its own play money; Pyr Casino's slots are set to about 99%.)

use serde::{Deserialize, Serialize};

use crate::fair::Stream;

pub const REELS: usize = 5;
pub const ROWS: usize = 3;
/// Symbols: 0 to 5 pay (low to high), then the wild, the scatter and a blank.
pub const PAY_SYMBOLS: u8 = 6;
pub const WILD: u8 = 6;
pub const SCATTER: u8 = 7;
pub const BLANK: u8 = 8;
/// Cell weights out of 1,000 (pay symbols, wild, scatter, blank); reels 1 and 5 have no wild
/// (its weight goes to blanks).
const WEIGHTS: [u32; 9] = [200, 170, 140, 100, 60, 30, 25, 25, 250];
/// Points per way for three, four and five of a kind.
pub const PAY: [[u64; 3]; 6] = [[5, 10, 25], [6, 12, 30], [8, 16, 40], [10, 25, 60], [15, 40, 100], [25, 75, 250]];
/// One point is this many ten-thousandths of the bet (0.0928×); pays round to the nearest chip.
pub const POINT_TENTHOUSANDTHS: u64 = 928;
/// Buying the bonus costs this many times the bet.
pub const BUY_X: u64 = 7;
/// The bets a spin can be.
pub const BETS: [u64; 9] = [20, 50, 100, 200, 500, 1_000, 2_000, 5_000, 10_000];
pub const MIN_BET: u64 = 20;
pub const MAX_BET: u64 = 10_000;

/// The pick-and-dodge board: tiles, hazards, the life and gold tiles in it, lives to start with.
pub const TILES: usize = 15;
pub const HAZARDS: u8 = 4;
const LIVES_IN_BAG: u8 = 1;
const GOLDS: u8 = 1;
const START_LIVES: u8 = 1;
/// Chip prizes in hundredths of the bet, with their weights; gold prizes too; and the finale.
const CHIPS: [(u64, u32); 5] = [(30, 30), (50, 30), (80, 22), (119, 12), (199, 6)];
const GOLD: [(u64, u32); 3] = [(500, 50), (800, 35), (1_500, 15)];
pub const FINALE_X: u64 = 15;

/// The prize wheel: twenty slices (times the bet), `UPGRADE` spinning the inner wheel.
pub const UPGRADE: u64 = 0;
pub const OUTER: [u64; 20] = [3, 3, 2, 4, 3, 5, 3, 6, 2, 8, 3, 4, 3, 10, 3, 5, 2, 10, UPGRADE, 6];
pub const INNER: [u64; 8] = [15, 20, 25, 30, 40, 50, 100, 150];

/// Which bonus a machine plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BonusKind {
    Pick,
    Wheel,
}

/// The twelve machines: their names (for the big-win ticker) and bonuses. How they look is the
/// app's (`casino::reels` there).
pub const MACHINES: [(&str, BonusKind); 12] = [
    ("Scoville Slots", BonusKind::Pick),
    ("Deep Sea Treasure", BonusKind::Pick),
    ("Cosmic Jackpot", BonusKind::Wheel),
    ("Sugar Rush", BonusKind::Wheel),
    ("Gold Rush", BonusKind::Pick),
    ("Neon Nights", BonusKind::Wheel),
    ("Tomb of Ra", BonusKind::Pick),
    ("Frostbite Fortune", BonusKind::Pick),
    ("Moonlit Cauldron", BonusKind::Pick),
    ("Dragon's Hoard", BonusKind::Pick),
    ("The Daily Grind", BonusKind::Pick),
    ("Starlight Cabaret", BonusKind::Wheel),
];

pub fn machine_name(m: u8) -> &'static str {
    MACHINES.get(m as usize).map(|x| x.0).unwrap_or("Reel Row")
}

pub fn bonus_kind(m: u8) -> BonusKind {
    MACHINES.get(m as usize).map(|x| x.1).unwrap_or(BonusKind::Pick)
}

/// grid[reel][row]
pub type Grid = [[u8; ROWS]; REELS];

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WayWin {
    pub symbol: u8,
    /// Reels it runs across (3 to 5), and the ways it's made.
    pub count: u8,
    pub ways: u32,
    pub pay: u64,
}

/// What a pick turned up, in the order the tiles are opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum Pick {
    Chips { chips: u64 },
    Gold { chips: u64 },
    /// An extra life.
    Life,
    /// A hazard, and a life used up.
    Saved,
    /// A hazard with no life left: the end.
    Busted,
}

/// The bonus, played out.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Bonus {
    /// Every pick, in order; whether every safe tile was found (the finale, `FINALE_X` times
    /// the bet more).
    Pick { picks: Vec<Pick>, finale: bool, won: u64 },
    /// The slice the wheel stopped on, and the inner wheel's when it was the Upgrade.
    Wheel { slice: u8, inner: Option<u8>, won: u64 },
}

impl Bonus {
    pub fn won(&self) -> u64 {
        match self {
            Bonus::Pick { won, .. } | Bonus::Wheel { won, .. } => *won,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub machine: u8,
    pub bet: u64,
    /// The bonus was bought (no spin).
    pub bought: bool,
    /// The spin (none when the bonus was bought).
    pub grid: Option<Grid>,
    pub wins: Vec<WayWin>,
    pub scatters: u8,
    pub bonus: Option<Bonus>,
    pub paid: u64,
}

impl Outcome {
    /// Chips staked.
    pub fn wagered(&self) -> u64 {
        if self.bought { self.bet * BUY_X } else { self.bet }
    }
}

/// Whether a bet is one the house takes.
pub fn check(machine: u8, bet: u64) -> Result<(), String> {
    if machine as usize >= MACHINES.len() {
        return Err("there's no such machine".into());
    }
    if !BETS.contains(&bet) {
        return Err(format!("a spin is {} to {} chips", MIN_BET, MAX_BET));
    }
    Ok(())
}

fn draw_cell(s: &mut Stream, reel: usize) -> u8 {
    let mut w = WEIGHTS;
    if reel == 0 || reel == REELS - 1 {
        w[BLANK as usize] += w[WILD as usize];
        w[WILD as usize] = 0;
    }
    let total: u32 = w.iter().sum();
    let mut r = s.below(total);
    for (i, &x) in w.iter().enumerate() {
        if r < x {
            return i as u8;
        }
        r -= x;
    }
    BLANK
}

pub fn spin_grid(s: &mut Stream) -> Grid {
    let mut g = [[BLANK; ROWS]; REELS];
    for (r, col) in g.iter_mut().enumerate() {
        for c in col.iter_mut() {
            *c = draw_cell(s, r);
        }
    }
    g
}

/// `chips` times `x`/`per`, rounded to the nearest chip.
fn share(chips: u64, x: u64, per: u64) -> u64 {
    (chips as u128 * x as u128 + per as u128 / 2).checked_div(per as u128).unwrap_or(0).min(u64::MAX as u128) as u64
}

/// The ways wins on a grid (left to right; wilds stand in for the pay symbols).
pub fn score(g: &Grid, bet: u64) -> (Vec<WayWin>, u8) {
    let mut wins = Vec::new();
    for sym in 0..PAY_SYMBOLS {
        let mut ways = 1u32;
        let mut n = 0usize;
        for col in g.iter() {
            let c = col.iter().filter(|&&x| x == sym || x == WILD).count() as u32;
            if c == 0 {
                break;
            }
            ways *= c;
            n += 1;
        }
        if n >= 3 {
            // (No wild on reel 1: every run starts with the symbol itself.)
            let points = PAY[sym as usize][n - 3] * ways as u64;
            let pay = share(bet, points * POINT_TENTHOUSANDTHS, 10_000);
            if pay > 0 {
                wins.push(WayWin { symbol: sym, count: n as u8, ways, pay });
            }
        }
    }
    let scatters = g.iter().flatten().filter(|&&x| x == SCATTER).count() as u8;
    (wins, scatters)
}

fn weighted(s: &mut Stream, t: &[(u64, u32)]) -> u64 {
    let total: u32 = t.iter().map(|x| x.1).sum();
    let mut r = s.below(total);
    for &(v, w) in t {
        if r < w {
            return v;
        }
        r -= w;
    }
    t[t.len() - 1].0
}

/// Pick-and-dodge, played out: each pick draws from what's left under the tiles.
pub fn play_pick(bet: u64, s: &mut Stream) -> Bonus {
    let (mut hazards, mut lives_bag, mut golds, mut chips) = (HAZARDS, LIVES_IN_BAG, GOLDS, TILES as u8 - HAZARDS - LIVES_IN_BAG - GOLDS);
    let mut lives = START_LIVES;
    let mut picks = Vec::new();
    let mut won = 0u64;
    loop {
        let bag = [hazards, lives_bag, golds, chips];
        let total: u32 = bag.iter().map(|&x| x as u32).sum();
        let mut r = s.below(total);
        let mut kind = 3;
        for (k, &n) in bag.iter().enumerate() {
            if r < n as u32 {
                kind = k;
                break;
            }
            r -= n as u32;
        }
        match kind {
            0 => {
                hazards -= 1;
                if lives > 0 {
                    lives -= 1;
                    picks.push(Pick::Saved);
                } else {
                    picks.push(Pick::Busted);
                    return Bonus::Pick { picks, finale: false, won };
                }
            }
            1 => {
                lives_bag -= 1;
                lives += 1;
                picks.push(Pick::Life);
            }
            2 => {
                golds -= 1;
                let v = share(bet, weighted(s, &GOLD), 100);
                won += v;
                picks.push(Pick::Gold { chips: v });
            }
            _ => {
                chips -= 1;
                let v = share(bet, weighted(s, &CHIPS), 100);
                won += v;
                picks.push(Pick::Chips { chips: v });
            }
        }
        if lives_bag + golds + chips == 0 {
            won += bet * FINALE_X;
            return Bonus::Pick { picks, finale: true, won };
        }
    }
}

/// The prize wheel, spun (and the inner one, on the Upgrade).
pub fn play_wheel(bet: u64, s: &mut Stream) -> Bonus {
    let slice = s.below(OUTER.len() as u32) as u8;
    let (inner, x) = match OUTER[slice as usize] {
        UPGRADE => {
            let i = s.below(INNER.len() as u32) as u8;
            (Some(i), INNER[i as usize])
        }
        x => (None, x),
    };
    Bonus::Wheel { slice, inner, won: bet * x }
}

/// Plays a round: a spin (and its bonus, when three scatters start it), or a bought bonus.
pub fn play(machine: u8, bet: u64, buy: bool, s: &mut Stream) -> Outcome {
    let bonus = |s: &mut Stream| match bonus_kind(machine) {
        BonusKind::Pick => play_pick(bet, s),
        BonusKind::Wheel => play_wheel(bet, s),
    };
    if buy {
        let b = bonus(s);
        let paid = b.won().min(super::MAX_PAYOUT);
        return Outcome { machine, bet, bought: true, grid: None, wins: Vec::new(), scatters: 0, bonus: Some(b), paid };
    }
    let grid = spin_grid(s);
    let (wins, scatters) = score(&grid, bet);
    let b = (scatters >= 3).then(|| bonus(s));
    let paid = (wins.iter().map(|w| w.pay).sum::<u64>() + b.as_ref().map(|b| b.won()).unwrap_or(0)).min(super::MAX_PAYOUT);
    Outcome { machine, bet, bought: false, grid: Some(grid), wins, scatters, bonus: b, paid }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The chance of each count (0 to 3) of a symbol or wild in a reel's three cells.
    fn counts(reel: usize, sym: u8) -> [f64; 4] {
        let mut w = WEIGHTS;
        if reel == 0 || reel == REELS - 1 {
            w[BLANK as usize] += w[WILD as usize];
            w[WILD as usize] = 0;
        }
        let total: u32 = w.iter().sum();
        let p = (w[sym as usize] + w[WILD as usize]) as f64 / total as f64;
        let c = [1.0, 3.0, 3.0, 1.0];
        std::array::from_fn(|k| c[k] * p.powi(k as i32) * (1.0 - p).powi(3 - k as i32))
    }

    /// The base game's exact return at a bet (every count of every symbol on every reel).
    fn base_return(bet: u64) -> f64 {
        let mut ev = 0.0;
        for sym in 0..PAY_SYMBOLS {
            let d: Vec<[f64; 4]> = (0..REELS).map(|r| counts(r, sym)).collect();
            for code in 0..4usize.pow(5) {
                let cs: Vec<usize> = (0..5).map(|r| (code / 4usize.pow(r as u32)) % 4).collect();
                let pr: f64 = (0..5).map(|r| d[r][cs[r]]).product();
                let (mut n, mut ways) = (0, 1u64);
                for &c in &cs {
                    if c == 0 {
                        break;
                    }
                    ways *= c as u64;
                    n += 1;
                }
                if n >= 3 {
                    ev += pr * share(bet, PAY[sym as usize][n - 3] * ways * POINT_TENTHOUSANDTHS, 10_000) as f64;
                }
            }
        }
        ev / bet as f64
    }

    /// The chance three or more of the fifteen cells are scatters.
    fn bonus_chance() -> f64 {
        let p = WEIGHTS[SCATTER as usize] as f64 / 1_000.0;
        let c = |n: u64, k: u64| (0..k).fold(1.0, |a, i| a * (n - i) as f64 / (i + 1) as f64);
        1.0 - (0..3).map(|k| c(15, k) * p.powi(k as i32) * (1.0 - p).powi(15 - k as i32)).sum::<f64>()
    }

    /// Pick-and-dodge's exact worth (times the bet), over every order the tiles can come out in.
    fn pick_worth() -> f64 {
        let mean = |t: &[(u64, u32)]| t.iter().map(|&(v, w)| v as f64 * w as f64).sum::<f64>() / t.iter().map(|x| x.1 as f64).sum::<f64>() / 100.0;
        let (ce, ge) = (mean(&CHIPS), mean(&GOLD));
        fn f(h: u8, l: u8, g: u8, c: u8, lives: u8, ce: f64, ge: f64) -> f64 {
            let total = (h + l + g + c) as f64;
            if l + g + c == 0 {
                return 0.0;
            }
            let after = |l: u8, g: u8, c: u8, lives: u8| if l + g + c == 0 { FINALE_X as f64 } else { f(h, l, g, c, lives, ce, ge) };
            let mut ev = 0.0;
            if h > 0 && lives > 0 {
                ev += h as f64 / total * f(h - 1, l, g, c, lives - 1, ce, ge);
            }
            if l > 0 {
                ev += l as f64 / total * after(l - 1, g, c, lives + 1);
            }
            if g > 0 {
                ev += g as f64 / total * (ge + after(l, g - 1, c, lives));
            }
            if c > 0 {
                ev += c as f64 / total * (ce + after(l, g, c - 1, lives));
            }
            ev
        }
        f(HAZARDS, LIVES_IN_BAG, GOLDS, TILES as u8 - HAZARDS - LIVES_IN_BAG - GOLDS, START_LIVES, ce, ge)
    }

    fn wheel_worth() -> f64 {
        let inner = INNER.iter().sum::<u64>() as f64 / INNER.len() as f64;
        OUTER.iter().map(|&x| if x == UPGRADE { inner } else { x as f64 }).sum::<f64>() / OUTER.len() as f64
    }

    #[test]
    fn it_pays_back_about_99_percent_whichever_bonus() {
        let (pick, wheel) = (pick_worth(), wheel_worth());
        assert!((pick - wheel).abs() < 0.01, "the two bonuses are worth the same: {pick} and {wheel}");
        let chance = bonus_chance();
        for bet in BETS {
            let base = base_return(bet);
            for bonus in [pick, wheel] {
                let rtp = base + chance * bonus;
                assert!((0.988..0.992).contains(&rtp), "bet {bet}: {rtp}");
            }
        }
        // A bought bonus too.
        for bonus in [pick, wheel] {
            let rtp = bonus / BUY_X as f64;
            assert!((0.988..0.993).contains(&rtp), "bought: {rtp}");
        }
    }

    #[test]
    fn rounds_played_out_pay_what_they_say_and_land_near_the_math() {
        // Many rounds from one stream: what's paid is what the wins and bonus add up to, and the
        // bonus is worth about what the exact sum says.
        let mut s = Stream::new([7; 32]);
        let (mut bonuses, mut won) = (0u64, 0u64);
        for i in 0..200_000u64 {
            let machine = (i % 12) as u8;
            let o = play(machine, 100, false, &mut s);
            let wins: u64 = o.wins.iter().map(|w| w.pay).sum();
            assert_eq!(o.paid, wins + o.bonus.as_ref().map(|b| b.won()).unwrap_or(0));
            assert_eq!(o.bonus.is_some(), o.scatters >= 3);
            if let Some(b) = &o.bonus {
                bonuses += 1;
                won += b.won();
                match (b, bonus_kind(machine)) {
                    (Bonus::Pick { picks, finale, .. }, BonusKind::Pick) => {
                        assert!(picks.len() <= TILES);
                        assert_eq!(*finale, !matches!(picks.last(), Some(Pick::Busted)));
                    }
                    (Bonus::Wheel { slice, inner, .. }, BonusKind::Wheel) => {
                        assert_eq!(inner.is_some(), OUTER[*slice as usize] == UPGRADE);
                    }
                    _ => panic!("the wrong bonus for machine {machine}"),
                }
            }
        }
        let per = won as f64 / bonuses as f64 / 100.0;
        assert!((6.4..7.5).contains(&per), "a bonus is worth {per}x");
        // A bought one: no spin, the bonus straight away, its cost the bet times BUY_X.
        let o = play(5, 200, true, &mut s);
        assert!(o.bought && o.grid.is_none() && o.bonus.is_some() && o.wagered() == 1_400);
    }

    #[test]
    fn wilds_extend_a_run_and_bets_are_checked() {
        let mut g = [[BLANK; ROWS]; REELS];
        g[0][0] = 0;
        g[1][1] = WILD;
        g[2][2] = WILD;
        let (wins, _) = score(&g, 100);
        assert_eq!(wins.len(), 1);
        assert_eq!((wins[0].symbol, wins[0].count, wins[0].ways), (0, 3, 1));
        // 5 points of 0.0928× a 100 bet: 46.4, rounded.
        assert_eq!(wins[0].pay, 46);
        assert!(check(12, 100).is_err() && check(0, 30).is_err() && check(11, 10_000).is_ok());
    }
}
