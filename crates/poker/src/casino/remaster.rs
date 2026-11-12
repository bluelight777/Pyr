//! Reel Row Remastered (0.13.2): Reel Row's twelve machines, each with a feature of its own on
//! the reels, as 0.13.28 has them. The rules, the pay tables and the prices are 0.13.28's, off
//! its Info pages and Play Feature; what they don't show (how often a wild, a bomb or a coin
//! lands, what bombs and coins are worth) is the rebuild's own, set so that every way to play
//! pays back 99% (see the tests).
//!
//! - Tumbles (Cosmic Jackpot, Sugar Rush, Neon Nights, Moonlit Cauldron): a win's symbols burst,
//!   the rest drop and new ones fall in from the top, again while there are wins. Bombs land now
//!   and then (about a cell in seventy), worth 2× to 100× (Cosmic's star orbs to 500×): when the
//!   tumbling's over, every bomb on the reels adds up and the whole chain's win is multiplied by
//!   the total. Three scatters after the tumbles start the bonus.
//! - Hold & Win (Deep Sea Treasure, Gold Rush, Tomb of Ra, Dragon's Hoard): coins land with
//!   their worth on them (the Mini, Minor and Major among them: 20×, 50× and 200× the bet), now
//!   and then a collector on the last reel, which pays every coin on the reels. Six coins or more
//!   start Hold & Win: three respins, every new coin held and the count back to three, a
//!   collector landing gathering everything held. All fifteen places: the Grand, 1,000× the bet
//!   (on Gold Rush and Dragon's Hoard, their progressive jackpot).
//! - Hot Wilds (Scoville Slots): a wild on reels 2 to 4 fills its reel and puts a ×2 (six times
//!   in ten), a ×3 or a ×5 on it; a win through two or three Hot Wilds multiplies by all of them.
//! - Frozen Wilds (Frostbite Fortune): wilds freeze where they land and every other cell spins
//!   again, again for every new wild, up to twenty respins, paying each time.
//! - Spotlight Wilds (Starlight Cabaret): a wild steps one reel left each respin while the rest
//!   spins again, paying every time, till it walks off the first reel; new wilds join the walk.
//! - Espresso Shots (The Daily Grind): a wild on reels 2 to 4 fills its reel and stays for one
//!   more spin while the rest spins again; new shots keep it going.
//!
//! Every machine: five reels by three rows, 243 ways, Reel Row's symbols and pay table at the
//! machine's own point of the bet; three scatters start its mini game (Reel Row's, with its top
//! prize). Ante, beside the bet: a spin a quarter dearer, the bonus about 1.7 times as likely (1.5
//! on the tumbling machines). Play Feature: Bonus Chance (the bonus 1 spin in 45), the machine's
//! feature spin (a Bomb Spin, Buy Hold & Win, a Hot Spin, a Frozen Spin, an Encore Spin, a Double
//! Shot), the mini game bought, its Super, the Mystery Bonus and All or Nothing, as in Reel Row.
//! A round pays up to the machine's top prize.
//!
//! A round is played from the round's random numbers alone, in a fixed order, so anyone can play
//! it again from the seeds and get the same screens.

use serde::{Deserialize, Serialize};

use super::feature::{self, Feature};
use super::reelrow::{self, Bonus};
use super::reels::{self, BLANK, Grid, PAY, PAY_SYMBOLS, REELS, ROWS, SCATTER, WILD, share};
use crate::fair::Stream;

/// The symbols besides Reel Row's (0 to 5 pay, then the wild, the scatter and a blank): a bomb
/// (tumbling machines), a coin and a collector (Hold & Win machines).
pub const BOMB: u8 = 9;
pub const COIN: u8 = 10;
pub const COLLECTOR: u8 = 11;
const KINDS: usize = 12;
/// A cell's weights are out of this many.
const OUT_OF: u32 = 100_000;
/// The places on the reels.
pub const PLACES: usize = REELS * ROWS;
/// Frozen Wilds: the most respins.
pub const MAX_FROZEN: usize = 20;
/// Hold & Win: the respins, back to this many with every new coin; how many coins start it.
pub const RESPINS: u8 = 3;
pub const HOLD_START: usize = 6;
/// Hold & Win: the Grand, times the bet (a fixed one).
pub const GRAND_X: u64 = 1_000;
/// The jackpot coins, in hundredths of the bet: the Mini, the Minor and the Major.
pub const MINI: u32 = 2_000;
pub const MINOR: u32 = 5_000;
pub const MAJOR: u32 = 20_000;
/// Ante: a spin costs this many hundredths of the bet.
pub const ANTE_PRICE: u64 = 125;
/// Bonus Chance: the bonus this often (per million), 1 spin in 44.5 (Reel Row's).
pub const CHANCE_PPM: u32 = reelrow::CHANCE_PPM;
/// A Bomb Spin's bombs; a Frozen Spin's wilds.
pub const BOMB_SPIN_BOMBS: usize = 3;
pub const FROZEN_SPIN_WILDS: usize = 2;
/// The longest a round's tumbles or walks can go on (they end long before, all but never).
const MAX_STEPS: usize = 100;

/// A cell's number: a bomb's multiple, a coin's worth (hundredths of the bet), what a collector
/// gathered (hundredths of the bet). values[reel][row].
pub type Values = [[u32; ROWS]; REELS];

/// Where a round's random numbers come from: the round's [`Stream`] (and, in the tests, a quicker
/// generator, to play millions of rounds).
pub trait Draw {
    /// A number from 0 to `n - 1`, each as likely.
    fn below(&mut self, n: u32) -> u32;
}

impl Draw for Stream {
    fn below(&mut self, n: u32) -> u32 {
        Stream::below(self, n)
    }
}

/// What a machine's reels do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Tumble,
    Hold,
    Hot,
    Frozen,
    Spotlight,
    Espresso,
}

/// A machine: what its reels do, its numbers, and what its Play Feature costs.
#[derive(Clone, Copy, Debug)]
pub struct Machine {
    pub family: Family,
    /// Its feature (the Info page's name), and its feature spin's (Play Feature).
    pub feature: &'static str,
    pub special_name: &'static str,
    /// What a point of the pay table is worth, in ten-thousandths of the bet.
    pub point: u64,
    /// Bonus Chance's price and the feature spin's, in hundredths of the bet.
    pub chance_price: u64,
    pub special_price: u64,
    /// The most a round pays, times the bet (its top prize; 0 for a progressive one).
    pub cap_x: u64,
    // What the Info page doesn't show. Weights out of 100,000 (Reel Row's, 1,000, times 100): a
    // wild on reels 2 to 4, on the spin and on what's drawn after it (respins, tumbles); a
    // scatter, and with Ante.
    wild: u32,
    later_wild: u32,
    scatter: u32,
    ante_scatter: u32,
    /// Bombs (tumbling machines) or coins (Hold & Win), on every reel; collectors on the last.
    special: u32,
    collector: u32,
    /// Hold & Win: a coin's chance on each empty place each respin, and a collector's on the
    /// last reel's (out of 100,000).
    hold_coin: u32,
    hold_collector: u32,
    /// What a bomb landing by itself is worth and how often, and a Bomb Spin's three; what a
    /// coin is worth (hundredths of the bet) and how often; Hot Wilds' multipliers.
    bombs: &'static [(u32, u32)],
    start_bombs: &'static [(u32, u32)],
    coins: &'static [(u32, u32)],
    hot: &'static [(u32, u32)],
}

const fn machine(family: Family, feature: &'static str, special_name: &'static str, point: u64, chance_price: u64, special_price: u64, cap_x: u64) -> Machine {
    Machine { family, feature, special_name, point, chance_price, special_price, cap_x, wild: 2_500, later_wild: 2_500, scatter: 2_500, ante_scatter: 3_000, special: 0, collector: 0, hold_coin: 0, hold_collector: 0, bombs: &[], start_bombs: &[], coins: &[], hot: &HOT_X }
}

impl Machine {
    const fn wilds(mut self, wild: u32, later_wild: u32) -> Machine {
        self.wild = wild;
        self.later_wild = later_wild;
        self
    }
    const fn scatters(mut self, scatter: u32, ante_scatter: u32) -> Machine {
        self.scatter = scatter;
        self.ante_scatter = ante_scatter;
        self
    }
    const fn bombs(mut self, special: u32, bombs: &'static [(u32, u32)], start_bombs: &'static [(u32, u32)]) -> Machine {
        self.special = special;
        self.bombs = bombs;
        self.start_bombs = start_bombs;
        self
    }
    const fn coins(mut self, special: u32, collector: u32, hold_coin: u32, hold_collector: u32) -> Machine {
        self.special = special;
        self.collector = collector;
        self.hold_coin = hold_coin;
        self.hold_collector = hold_collector;
        self.coins = &COINS;
        self
    }
}

use Family::*;

/// The twelve, in Reel Row's order. The points and prices are 0.13.28's.
pub const MACHINES: [Machine; 12] = [
    machine(Hot, "Hot Wilds", "Hot Spin", 267, 200, 753, 10_000).wilds(1_217, 1_217).scatters(2_500, 3_069),
    machine(Hold, "Pearl Hold & Win", "Buy Hold & Win", 300, 200, 3_650, 50_000).coins(12_085, 600, 4_077, 1_200).scatters(2_500, 3_053),
    machine(Tumble, "Cosmic Tumbles", "Bomb Spin", 209, 189, 175, 500_000).bombs(1_430, &COSMIC_BOMBS, &COSMIC_START).scatters(2_466, 2_897),
    machine(Tumble, "Candy Tumbles", "Bomb Spin", 222, 189, 169, 5_000).bombs(1_430, &SUGAR_BOMBS, &SUGAR_START).scatters(2_471, 2_907),
    machine(Hold, "Nugget Hold & Win", "Buy Hold & Win", 304, 200, 3_585, 0).coins(12_123, 600, 4_077, 1_200).scatters(2_500, 3_054),
    machine(Tumble, "Neon Tumbles", "Bomb Spin", 223, 189, 169, 20_000).bombs(1_430, &NEON_BOMBS, &NEON_START).scatters(2_467, 2_907),
    machine(Hold, "Scarab Hold & Win", "Buy Hold & Win", 300, 200, 3_650, 50_000).coins(12_106, 600, 4_077, 1_200).scatters(2_500, 3_053),
    machine(Frozen, "Frozen Wilds", "Frozen Spin", 387, 200, 563, 7_500).wilds(2_027, 2_584).scatters(2_500, 3_054),
    machine(Tumble, "Bubbling Tumbles", "Bomb Spin", 223, 189, 170, 10_000).bombs(1_430, &MOONLIT_BOMBS, &MOONLIT_START).scatters(2_472, 2_904),
    machine(Hold, "Hoard Hold & Win", "Buy Hold & Win", 308, 200, 3_587, 0).coins(12_085, 600, 4_077, 1_200).scatters(2_500, 3_054),
    machine(Espresso, "Espresso Shots", "Double Shot", 234, 200, 1_597, 5_000).wilds(1_804, 1_948).scatters(2_500, 3_077),
    machine(Spotlight, "Spotlight Wilds", "Encore Spin", 227, 200, 511, 25_000).wilds(2_234, 2_431).scatters(2_500, 3_053),
];

/// Hot Wilds' multipliers and how often each comes (out of 10,000): ×2 six times in ten.
pub const HOT_X: [(u32, u32); 3] = [(2, 6_000), (3, 2_975), (5, 1_025)];

/// What a bomb landing by itself can be worth, and how often (out of 100,000): Cosmic Jackpot's
/// star orbs to 500×, the others' to 100×; and what a Bomb Spin's three are worth.
pub const COSMIC_BOMBS: [(u32, u32); 13] = [(2, 37_901), (3, 21_618), (4, 14_507), (5, 10_648), (8, 5_552), (10, 4_075), (15, 2_323), (20, 1_560), (25, 1_145), (50, 438), (100, 168), (250, 47), (500, 18)];
pub const COSMIC_START: [(u32, u32); 10] = [(2, 32_823), (3, 20_607), (4, 14_848), (5, 11_504), (8, 6_722), (10, 5_208), (15, 3_276), (20, 2_358), (25, 1_827), (50, 827)];
pub const SUGAR_BOMBS: [(u32, u32); 11] = [(2, 39_674), (3, 21_825), (4, 14_344), (5, 10_342), (8, 5_192), (10, 3_743), (15, 2_066), (20, 1_355), (25, 977), (50, 354), (100, 128)];
pub const SUGAR_START: [(u32, u32); 10] = [(2, 35_996), (3, 21_291), (4, 14_706), (5, 11_026), (8, 6_012), (10, 4_508), (15, 2_671), (20, 1_843), (25, 1_382), (50, 565)];
pub const NEON_BOMBS: [(u32, u32); 11] = [(2, 40_031), (3, 21_930), (4, 14_297), (5, 10_263), (8, 5_105), (10, 3_665), (15, 2_006), (20, 1_309), (25, 939), (50, 335), (100, 120)];
pub const NEON_START: [(u32, u32); 10] = [(2, 36_042), (3, 21_318), (4, 14_702), (5, 11_016), (8, 5_998), (10, 4_495), (15, 2_660), (20, 1_834), (25, 1_374), (50, 561)];
pub const MOONLIT_BOMBS: [(u32, u32); 11] = [(2, 39_884), (3, 21_887), (4, 14_317), (5, 10_295), (8, 5_141), (10, 3_697), (15, 2_031), (20, 1_327), (25, 955), (50, 343), (100, 123)];
pub const MOONLIT_START: [(u32, u32); 10] = [(2, 35_960), (3, 21_296), (4, 14_707), (5, 11_031), (8, 6_018), (10, 4_513), (15, 2_676), (20, 1_847), (25, 1_385), (50, 567)];

/// What a coin can be worth (hundredths of the bet; the last three the Mini, the Minor and the
/// Major), and how often (out of 100,000).
pub const COINS: [(u32, u32); 9] = [(50, 36_735), (100, 22_556), (200, 13_849), (300, 10_411), (500, 7_267), (1_000, 4_462), (MINI, 2_740), (MINOR, 1_438), (MAJOR, 542)];

pub fn family(machine: u8) -> Family {
    MACHINES[(machine as usize).min(11)].family
}

/// Whether a spin is one the house takes.
pub fn check(machine: u8, bet: u64, feature: Feature, ante: bool) -> Result<(), String> {
    reelrow::check(machine, bet)?;
    if ante && feature != Feature::Spin {
        return Err("Ante is for spins".into());
    }
    Ok(())
}

/// What a round costs, in hundredths of the bet.
pub fn price(machine: u8, feature: Feature, ante: bool) -> u64 {
    let m = &MACHINES[(machine as usize).min(11)];
    match feature {
        Feature::Spin => if ante { ANTE_PRICE } else { 100 },
        Feature::Chance => m.chance_price,
        Feature::Wild => m.special_price,
        _ => reelrow::price(feature),
    }
}

/// One screen of a round: the spin, a tumble, a respin, a Hold & Win respin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub grid: Grid,
    /// What a bomb, a coin or a collector on it is worth (see [`Values`]).
    #[serde(default, skip_serializing_if = "no_values")]
    pub values: Values,
    /// The places that stayed from the screen before (bit 3 × reel + row): frozen and walking
    /// wilds, Espresso Shots, coins held.
    #[serde(default, skip_serializing_if = "is_zero16")]
    pub held: u16,
    /// Hot Wilds: each reel's multiplier (0: none).
    #[serde(default, skip_serializing_if = "no_x")]
    pub x: [u8; REELS],
    /// What the ways paid on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wins: Vec<Win>,
}

fn no_values(v: &Values) -> bool {
    v.iter().flatten().all(|&x| x == 0)
}

fn is_zero16(x: &u16) -> bool {
    *x == 0
}

fn no_x(x: &[u8; REELS]) -> bool {
    x.iter().all(|&v| v == 0)
}

fn is_zero(x: &u64) -> bool {
    *x == 0
}

fn is_zero32(x: &u32) -> bool {
    *x == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A ways win: the symbol, the reels it runs across (3 to 5), the ways it's made, what Hot Wilds
/// multiplied it by (1: none), and its points (the pay table's times the ways and the multiplier).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Win {
    pub symbol: u8,
    pub count: u8,
    pub ways: u32,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub x: u32,
    pub points: u64,
    /// In chips (on its own; a tumbling chain's is multiplied by the bombs at the end).
    pub pay: u64,
}

fn one() -> u32 {
    1
}

fn is_one(x: &u32) -> bool {
    *x == 1
}

/// A round on a Remastered machine.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub machine: u8,
    pub bet: u64,
    pub feature: Feature,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ante: bool,
    /// What it cost.
    pub cost: u64,
    /// The spin and what followed it, screen by screen: tumbles, respins (none when a bonus
    /// was bought).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// Tumbling machines: the bombs' total that multiplied the chain's win (0: no win, or no
    /// bombs).
    #[serde(default, skip_serializing_if = "is_zero32")]
    pub mult: u32,
    /// What the ways paid, all screens together (and multiplied by the bombs).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub ways_paid: u64,
    /// Hold & Win machines: what collectors in the spin paid.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub collected: u64,
    /// Hold & Win: its board as it started, then after each respin.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hold: Vec<Step>,
    /// What Hold & Win paid: its coins and collectors, and the Grand (all fifteen places).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub hold_paid: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub grand: u64,
    pub scatters: u8,
    pub bonus: Option<Bonus>,
    /// Mystery: whether it turned out the Super Bonus. All or nothing: whether it won.
    #[serde(default, skip_serializing_if = "is_false")]
    pub lucky: bool,
    /// Whether the round hit the most a round pays (its top prize).
    #[serde(default, skip_serializing_if = "is_false")]
    pub capped: bool,
    pub paid: u64,
    /// A progressive machine's jackpot pool as the round began (to play it again by).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pool: u64,
}

impl Outcome {
    pub fn wagered(&self) -> u64 {
        self.cost
    }

    /// What its machine's progressive jackpot paid (the bonus's top prize, Hold & Win's Grand).
    pub fn pool_won(&self) -> u64 {
        if !reelrow::progressive(self.machine) {
            return 0;
        }
        self.bonus.as_ref().map(|b| b.top).unwrap_or(0).saturating_add(self.grand)
    }

    /// The round in a few words, for the history and the big-win feed.
    pub fn headline(&self) -> String {
        let m = (self.machine as usize).min(11);
        let on = reels::machine_name(self.machine);
        let game = reelrow::GAMES[m].0;
        let top = reelrow::TOPS[m].name;
        match (self.feature, &self.bonus) {
            (Feature::AllIn, _) if self.lucky => format!("all or nothing on {on}, and won it"),
            (Feature::AllIn, _) => format!("all or nothing on {on}"),
            (_, Some(b)) if b.top > 0 => format!("{top} on {on}"),
            _ if self.grand > 0 && reelrow::progressive(self.machine) => format!("{top} on {on}"),
            _ if self.grand > 0 => format!("the Grand on {on}"),
            (_, Some(b)) if b.times > 1 => format!("the Super {game} on {on}"),
            (_, Some(_)) => format!("the {game} on {on}"),
            _ if !self.hold.is_empty() => format!("Hold & Win on {on}"),
            _ if self.mult > 1 => format!("a ×{} tumble on {on}", self.mult),
            _ => format!("a spin on {on}"),
        }
    }
}

/// What the reels did, before the bonus: exact, in fractions of the bet (the tests add these up
/// over millions of rounds).
#[derive(Clone, Debug, Default)]
pub struct Reels {
    pub steps: Vec<Step>,
    pub hold: Vec<Step>,
    /// The ways' points, all screens (a tumbling chain's before the bombs).
    pub points: u64,
    /// Tumbling: the bombs' total (1 with none); what Hold & Win and collectors paid
    /// (hundredths of the bet); whether all fifteen places filled.
    pub mult: u32,
    pub collected: u64,
    pub held: u64,
    pub grand: bool,
    pub scatters: u8,
}

impl Reels {
    /// What it all paid, times the bet (`point`: the machine's), the Grand apart.
    pub fn x(&self, point: u64) -> f64 {
        (self.points * point * self.mult.max(1) as u64) as f64 / 10_000.0 + (self.collected + self.held) as f64 / 100.0
    }
}

fn bit(reel: usize, row: usize) -> u16 {
    1 << (reel * ROWS + row)
}

/// A value drawn from a table of (value, weight).
fn value<D: Draw>(t: &[(u32, u32)], d: &mut D) -> u32 {
    let total: u32 = t.iter().map(|x| x.1).sum();
    let mut r = d.below(total);
    for &(v, w) in t {
        if r < w {
            return v;
        }
        r -= w;
    }
    t[t.len() - 1].0
}

fn weighted<D: Draw>(d: &mut D, w: &[u32]) -> usize {
    let total: u32 = w.iter().sum();
    let mut r = d.below(total);
    for (i, &x) in w.iter().enumerate() {
        if r < x {
            return i;
        }
        r -= x;
    }
    w.len() - 1
}

/// When a cell is drawn: on the spin, or after it (a respin's, a tumble's new cells), and
/// whether a scatter can come (not after the spin on a Bonus Chance, nor on a respin).
#[derive(Clone, Copy, PartialEq, Eq)]
enum When {
    Spin,
    Later { scatters: bool },
}

/// The weights of a cell on `reel`.
fn weights(m: &Machine, reel: usize, ante: bool, when: When) -> [u32; KINDS] {
    let mut w = [0u32; KINDS];
    for (k, x) in reels::WEIGHTS.iter().take(PAY_SYMBOLS as usize).enumerate() {
        w[k] = x * 100;
    }
    if (1..REELS - 1).contains(&reel) {
        w[WILD as usize] = if when == When::Spin { m.wild } else { m.later_wild };
    }
    if !matches!(when, When::Later { scatters: false }) {
        w[SCATTER as usize] = if ante { m.ante_scatter } else { m.scatter };
    }
    match m.family {
        Tumble => w[BOMB as usize] = m.special,
        Hold => {
            w[COIN as usize] = m.special;
            if reel == REELS - 1 {
                w[COLLECTOR as usize] = m.collector;
            }
        }
        _ => {}
    }
    let used: u32 = w.iter().sum();
    w[BLANK as usize] = OUT_OF - used;
    w
}

/// A cell drawn, with its number (a bomb's multiple, a coin's worth).
fn cell<D: Draw>(m: &Machine, reel: usize, ante: bool, when: When, d: &mut D) -> (u8, u32) {
    let k = weighted(d, &weights(m, reel, ante, when)) as u8;
    let v = match k {
        BOMB => value(m.bombs, d),
        COIN => value(m.coins, d),
        _ => 0,
    };
    (k, v)
}

fn spin_cells<D: Draw>(m: &Machine, ante: bool, d: &mut D) -> (Grid, Values) {
    let mut g = [[BLANK; ROWS]; REELS];
    let mut v = [[0; ROWS]; REELS];
    for r in 0..REELS {
        for row in 0..ROWS {
            (g[r][row], v[r][row]) = cell(m, r, ante, When::Spin, d);
        }
    }
    (g, v)
}

fn count(g: &Grid, sym: u8) -> u8 {
    g.iter().flatten().filter(|&&x| x == sym).count() as u8
}

fn mask(g: &Grid, sym: u8) -> u16 {
    let mut m = 0;
    for r in 0..REELS {
        for row in 0..ROWS {
            if g[r][row] == sym {
                m |= bit(r, row);
            }
        }
    }
    m
}

/// `n` different places drawn from `from` (each as likely, in the order drawn).
fn places<D: Draw>(from: &[(usize, usize)], n: usize, d: &mut D) -> Vec<(usize, usize)> {
    let mut left = from.to_vec();
    (0..n).map(|_| left.remove(d.below(left.len() as u32) as usize)).collect()
}

fn all_places() -> Vec<(usize, usize)> {
    (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).collect()
}

/// The ways wins on a screen: three or more of a symbol on reels side by side from the left, in
/// any row; a wild stands in for every pay symbol, on whatever reel it is. A win through a reel
/// with a Hot Wild's multiplier (`x`) is multiplied by it.
pub fn ways(g: &Grid, x: &[u8; REELS], bet: u64, point: u64) -> Vec<Win> {
    let mut wins = Vec::new();
    for sym in 0..PAY_SYMBOLS {
        let (mut ways, mut n, mut mult) = (1u32, 0usize, 1u32);
        for (r, col) in g.iter().enumerate() {
            let c = col.iter().filter(|&&v| v == sym || v == WILD).count() as u32;
            if c == 0 {
                break;
            }
            ways *= c;
            n += 1;
            if x[r] > 0 {
                mult *= x[r] as u32;
            }
        }
        if n >= 3 {
            let points = PAY[sym as usize][n - 3] * ways as u64 * mult as u64;
            wins.push(Win { symbol: sym, count: n as u8, ways, x: mult, points, pay: share(bet, points * point, 10_000) });
        }
    }
    wins
}

fn points(wins: &[Win]) -> u64 {
    wins.iter().map(|w| w.points).sum()
}

/// The places a tumble's wins burst: every cell of each win's symbol (and every wild) on the
/// reels it runs across.
pub fn burst(g: &Grid, wins: &[Win]) -> u16 {
    let mut m = 0;
    for w in wins {
        for r in 0..w.count as usize {
            for row in 0..ROWS {
                if g[r][row] == w.symbol || g[r][row] == WILD {
                    m |= bit(r, row);
                }
            }
        }
    }
    m
}

/// The feature a round plays on the reels: none, Bonus Chance (with or without the bonus
/// decided), or the machine's feature spin.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Play {
    Spin,
    Chance(bool),
    Special,
}

/// The spin's screen (Hot Wilds and Espresso Shots already filling their reels, covering what
/// was under them), for Bonus Chance drawn again till it has the bonus or not, as decided.
fn first<D: Draw>(m: &Machine, ante: bool, play: Play, d: &mut D) -> (Grid, Values) {
    loop {
        let (mut g, v) = spin_cells(m, ante, d);
        if m.family == Hot || m.family == Espresso {
            for r in 1..REELS - 1 {
                if g[r].contains(&WILD) {
                    g[r] = [WILD; ROWS];
                }
            }
        }
        match play {
            Play::Chance(bonus) if (count(&g, SCATTER) >= 3) != bonus => continue,
            _ => return (g, v),
        }
    }
}

/// The reels of a round (a spin, Bonus Chance or the machine's feature spin), before the bonus.
pub fn spin<D: Draw>(machine: u8, ante: bool, feature: Feature, bet: u64, d: &mut D) -> Reels {
    spin_on(&MACHINES[(machine as usize).min(11)], ante, feature, bet, d)
}

/// `spin`, on a machine as given (the tests try others).
fn spin_on<D: Draw>(m: &Machine, ante: bool, feature: Feature, bet: u64, d: &mut D) -> Reels {
    let play = match feature {
        Feature::Chance => Play::Chance(d.below(1_000_000) < CHANCE_PPM),
        Feature::Wild => Play::Special,
        _ => Play::Spin,
    };
    // Bonus Chance decides the bonus with the spin: nothing after it brings a scatter.
    let later = When::Later { scatters: m.family == Tumble && !matches!(play, Play::Chance(_)) };
    match m.family {
        Hot => hot(m, ante, play, bet, d),
        Frozen => frozen(m, ante, play, bet, d),
        Spotlight => spotlight(m, ante, play, bet, d),
        Espresso => espresso(m, ante, play, bet, d),
        Tumble => tumble(m, ante, play, later, bet, d),
        Hold => hold(m, ante, play, bet, d),
    }
}

fn hot<D: Draw>(m: &Machine, ante: bool, play: Play, bet: u64, d: &mut D) -> Reels {
    let (mut g, _) = first(m, ante, play, d);
    if play == Play::Special {
        g[REELS / 2] = [WILD; ROWS];
    }
    let mut x = [0u8; REELS];
    for r in 1..REELS - 1 {
        if g[r][0] == WILD && g[r].iter().all(|&c| c == WILD) {
            x[r] = value(m.hot, d) as u8;
        }
    }
    let wins = ways(&g, &x, bet, m.point);
    let scatters = count(&g, SCATTER);
    Reels { points: points(&wins), steps: vec![Step { grid: g, x, wins, ..Step::default() }], mult: 1, scatters, ..Reels::default() }
}

fn frozen<D: Draw>(m: &Machine, ante: bool, play: Play, bet: u64, d: &mut D) -> Reels {
    let (mut g, _) = first(m, ante, play, d);
    if play == Play::Special {
        let middle: Vec<_> = (1..REELS - 1).flat_map(|r| (0..ROWS).map(move |row| (r, row))).collect();
        for (r, row) in places(&middle, FROZEN_SPIN_WILDS, d) {
            g[r][row] = WILD;
        }
    }
    let scatters = count(&g, SCATTER);
    let wins = ways(&g, &[0; REELS], bet, m.point);
    let mut out = Reels { points: points(&wins), mult: 1, scatters, ..Reels::default() };
    out.steps.push(Step { grid: g, wins, ..Step::default() });
    let mut frozen = mask(&g, WILD);
    let mut new = frozen != 0;
    while new && out.steps.len() <= MAX_FROZEN {
        for r in 0..REELS {
            for row in 0..ROWS {
                if frozen & bit(r, row) == 0 {
                    g[r][row] = cell(m, r, ante, When::Later { scatters: false }, d).0;
                }
            }
        }
        let now = mask(&g, WILD);
        new = now & !frozen != 0;
        let wins = ways(&g, &[0; REELS], bet, m.point);
        out.points += points(&wins);
        out.steps.push(Step { grid: g, held: frozen, wins, ..Step::default() });
        frozen = now;
    }
    out
}

fn spotlight<D: Draw>(m: &Machine, ante: bool, play: Play, bet: u64, d: &mut D) -> Reels {
    let (mut g, _) = first(m, ante, play, d);
    if play == Play::Special {
        g[REELS - 1][d.below(ROWS as u32) as usize] = WILD;
    }
    let scatters = count(&g, SCATTER);
    let wins = ways(&g, &[0; REELS], bet, m.point);
    let mut out = Reels { points: points(&wins), mult: 1, scatters, ..Reels::default() };
    out.steps.push(Step { grid: g, wins, ..Step::default() });
    let mut walkers = mask(&g, WILD);
    while out.steps.len() < MAX_STEPS {
        // Every walking wild a reel to the left; off the first reel, gone.
        let moved = walkers >> ROWS;
        if moved == 0 {
            break;
        }
        for r in 0..REELS {
            for row in 0..ROWS {
                g[r][row] = if moved & bit(r, row) != 0 { WILD } else { cell(m, r, ante, When::Later { scatters: false }, d).0 };
            }
        }
        walkers = mask(&g, WILD);
        let wins = ways(&g, &[0; REELS], bet, m.point);
        out.points += points(&wins);
        out.steps.push(Step { grid: g, held: moved, wins, ..Step::default() });
    }
    out
}

fn espresso<D: Draw>(m: &Machine, ante: bool, play: Play, bet: u64, d: &mut D) -> Reels {
    let (mut g, _) = first(m, ante, play, d);
    let mut shots = [false; REELS];
    for r in 1..REELS - 1 {
        shots[r] = g[r][0] == WILD || (play == Play::Special && (r == 1 || r == REELS - 2));
        if shots[r] {
            g[r] = [WILD; ROWS];
        }
    }
    let scatters = count(&g, SCATTER);
    let wins = ways(&g, &[0; REELS], bet, m.point);
    let mut out = Reels { points: points(&wins), mult: 1, scatters, ..Reels::default() };
    out.steps.push(Step { grid: g, wins, ..Step::default() });
    // Each shot stays for one more spin; the shots that land on it keep it going.
    let mut staying = shots;
    while staying.iter().any(|&s| s) && out.steps.len() < MAX_STEPS {
        let mut held = 0;
        let mut new = [false; REELS];
        for r in 0..REELS {
            if staying[r] {
                g[r] = [WILD; ROWS];
                held |= 0b111 << (r * ROWS);
                continue;
            }
            for row in 0..ROWS {
                g[r][row] = cell(m, r, ante, When::Later { scatters: false }, d).0;
            }
            if g[r].contains(&WILD) {
                new[r] = true;
                g[r] = [WILD; ROWS];
            }
        }
        let wins = ways(&g, &[0; REELS], bet, m.point);
        out.points += points(&wins);
        out.steps.push(Step { grid: g, held, wins, ..Step::default() });
        staying = new;
    }
    out
}

fn tumble<D: Draw>(m: &Machine, ante: bool, play: Play, later: When, bet: u64, d: &mut D) -> Reels {
    let (mut g, mut v) = first(m, ante, play, d);
    if play == Play::Special {
        for (r, row) in places(&all_places(), BOMB_SPIN_BOMBS, d) {
            g[r][row] = BOMB;
            v[r][row] = value(m.start_bombs, d);
        }
    }
    let mut out = Reels { mult: 1, ..Reels::default() };
    loop {
        let wins = ways(&g, &[0; REELS], bet, m.point);
        let done = wins.is_empty() || out.steps.len() + 1 >= MAX_STEPS;
        out.points += points(&wins);
        let gone = burst(&g, &wins);
        out.steps.push(Step { grid: g, values: v, wins, ..Step::default() });
        if done {
            break;
        }
        // What's left drops; new cells fall in from the top, the lowest first.
        for r in 0..REELS {
            let kept: Vec<(u8, u32)> = (0..ROWS).rev().filter(|&row| gone & bit(r, row) == 0).map(|row| (g[r][row], v[r][row])).collect();
            for row in (0..ROWS).rev() {
                let i = ROWS - 1 - row;
                (g[r][row], v[r][row]) = if i < kept.len() { kept[i] } else { cell(m, r, ante, later, d) };
            }
        }
    }
    if out.points > 0 {
        out.mult = (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).filter(|&(r, row)| g[r][row] == BOMB).map(|(r, row)| v[r][row]).sum::<u32>().max(1);
    }
    out.scatters = count(&g, SCATTER);
    out
}

fn hold<D: Draw>(m: &Machine, ante: bool, play: Play, bet: u64, d: &mut D) -> Reels {
    let mut out = Reels { mult: 1, ..Reels::default() };
    // The board Hold & Win starts with: the spin's coins, or six to start when it's bought.
    let mut board = [[BLANK; ROWS]; REELS];
    let mut values = [[0u32; ROWS]; REELS];
    let start = if play == Play::Special {
        for (r, row) in places(&all_places(), HOLD_START, d) {
            board[r][row] = COIN;
            values[r][row] = value(m.coins, d);
        }
        true
    } else {
        let (g, mut v) = first(m, ante, play, d);
        out.scatters = count(&g, SCATTER);
        let wins = ways(&g, &[0; REELS], bet, m.point);
        out.points = points(&wins);
        // A collector pays every coin on the reels.
        let coins: u32 = (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).filter(|&(r, row)| g[r][row] == COIN).map(|(r, row)| v[r][row]).sum();
        for row in 0..ROWS {
            if g[REELS - 1][row] == COLLECTOR {
                v[REELS - 1][row] = coins;
                out.collected += coins as u64;
            }
        }
        out.steps.push(Step { grid: g, values: v, wins, ..Step::default() });
        for r in 0..REELS {
            for row in 0..ROWS {
                if g[r][row] == COIN {
                    board[r][row] = COIN;
                    values[r][row] = v[r][row];
                }
            }
        }
        count(&g, COIN) as usize >= HOLD_START
    };
    if !start {
        return out;
    }
    out.hold.push(Step { grid: board, values, ..Step::default() });
    let mut left = RESPINS;
    let full = |b: &Grid| b.iter().flatten().all(|&c| c != BLANK);
    while left > 0 && !full(&board) {
        left -= 1;
        let held = !mask(&board, BLANK) & ((1 << PLACES) - 1);
        let mut collectors = Vec::new();
        for r in 0..REELS {
            for row in 0..ROWS {
                if board[r][row] != BLANK {
                    continue;
                }
                let x = d.below(OUT_OF);
                if x < m.hold_coin {
                    board[r][row] = COIN;
                    values[r][row] = value(m.coins, d);
                    left = RESPINS;
                } else if r == REELS - 1 && x < m.hold_coin + m.hold_collector {
                    board[r][row] = COLLECTOR;
                    collectors.push(row);
                    left = RESPINS;
                }
            }
        }
        // A collector landing gathers every coin held.
        let coins: u32 = (0..REELS).flat_map(|r| (0..ROWS).map(move |row| (r, row))).filter(|&(r, row)| board[r][row] == COIN).map(|(r, row)| values[r][row]).sum();
        for row in collectors {
            values[REELS - 1][row] = coins;
        }
        out.hold.push(Step { grid: board, values, held, ..Step::default() });
    }
    out.held = values.iter().flatten().map(|&x| x as u64).sum();
    out.grand = full(&board);
    out
}

/// Plays a round: a spin (with Ante or without), or a Play Feature.
pub fn play(machine: u8, bet: u64, feature: Feature, ante: bool, s: &mut Stream) -> Outcome {
    play_with(machine, bet, feature, ante, s, 0)
}

/// `play`, on a progressive machine with its jackpot's `pool` as the round begins.
pub fn play_with(machine: u8, bet: u64, feature: Feature, ante: bool, s: &mut Stream, pool: u64) -> Outcome {
    let m = &MACHINES[(machine as usize).min(11)];
    let ante = ante && feature == Feature::Spin;
    let cost = feature::cost(bet, price(machine, feature, ante));
    let pool = if reelrow::progressive(machine) { pool } else { 0 };
    let mut o = Outcome { machine, bet, feature, ante, cost, steps: Vec::new(), mult: 0, ways_paid: 0, collected: 0, hold: Vec::new(), hold_paid: 0, grand: 0, scatters: 0, bonus: None, lucky: false, capped: false, paid: 0, pool };
    let bonus = |times: u64, s: &mut Stream| {
        let (game, won) = reelrow::play_mini(machine, bet, s);
        let top = reelrow::roll_top(machine, bet, pool, s);
        Bonus { game, won, times, top }
    };
    match feature {
        Feature::Spin | Feature::Chance | Feature::Wild => {
            let r = spin(machine, ante, feature, bet, s);
            o.ways_paid = share(bet, r.points * m.point * r.mult as u64, 10_000);
            o.mult = if r.points > 0 && r.mult > 1 { r.mult } else { 0 };
            o.collected = share(bet, r.collected, 100);
            o.hold_paid = share(bet, r.held, 100);
            if r.grand {
                o.grand = if reelrow::progressive(machine) { reelrow::top_win(machine, bet, pool) } else { bet.saturating_mul(GRAND_X) };
            }
            o.scatters = r.scatters;
            o.steps = r.steps;
            o.hold = r.hold;
            if o.scatters >= 3 {
                o.bonus = Some(bonus(1, s));
            }
        }
        Feature::Bonus => o.bonus = Some(bonus(1, s)),
        Feature::Super => o.bonus = Some(bonus(reelrow::SUPER_X, s)),
        Feature::Mystery => {
            o.lucky = s.below(10) < reelrow::MYSTERY_SUPER_IN_10;
            o.bonus = Some(bonus(if o.lucky { reelrow::SUPER_X } else { 1 }, s));
        }
        Feature::AllIn => {
            o.lucky = s.below(10_000) < reelrow::ALL_IN_PER_10K;
            o.paid = if o.lucky { bet * reelrow::ALL_IN_X } else { 0 };
            return o;
        }
    }
    let total = o.ways_paid.saturating_add(o.collected).saturating_add(o.hold_paid).saturating_add(o.grand).saturating_add(o.bonus.as_ref().map(|b| b.paid()).unwrap_or(0));
    let cap = if m.cap_x > 0 { bet.saturating_mul(m.cap_x) } else { super::MAX_PAYOUT };
    o.capped = total > cap.min(super::MAX_PAYOUT);
    o.paid = total.min(cap).min(super::MAX_PAYOUT);
    o
}

#[cfg(test)]
mod tests;
