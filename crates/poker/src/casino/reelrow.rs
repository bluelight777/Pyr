//! Reel Row from 0.13.0: the twelve machines as Stake's slots are, each with a mini game of its
//! own and Play Feature (see [`super::feature`]).
//!
//! The reels are Reel Row's (`reels`): five by three, 243 ways, the same symbols and weights,
//! three or more scatters for the bonus (about 1 spin in 176). What's changed is where the money
//! is: the base game pays back about 65% of the bet and the bonus is worth about 59.4 times it
//! (it was 95% and 7 times), so a bonus is an event, and buying one for 60 times the bet pays back
//! 99% like everything else. Every machine's bonus is its own game:
//!
//! - Scoville Slots, **Heat Ladder**: eight peppers, hotter and richer up the Scoville scale. Each
//!   might be too hot (the run's over, unless there's a glass of milk to drink); top it for the
//!   Pepper X.
//! - Deep Sea Treasure, **Abyss Dive**: five depths of three clams: pearls, an air tank (a second
//!   chance) or a shark. The chest is at the bottom.
//! - Cosmic Jackpot, **Cargo Run**: the rocket climbs, its cargo multiplying, till it blows.
//! - Sugar Rush, **Candy Grab**: three goes with the claw; a golden ticket is another go.
//! - Gold Rush, **Mine Shaft**: the original pick-and-dodge: nine nuggets, a lantern (a life),
//!   the mother lode and four cave-ins.
//! - Neon Nights, **Neon Drop**: five balls down ten rows of pegs into the bins.
//! - Tomb of Ra, **Curse of the Tomb**: four chambers of three sarcophagi: treasure, an amulet or
//!   a mummy. Ra's mask waits past the fourth.
//! - Frostbite Fortune, **Ice Fishing**: seven casts: fish, a boot, hot cocoa (a life) or thin ice.
//! - Moonlit Cauldron, **Potion Brew**: three of nine jars (three each of three colours). All one
//!   colour brews four times its herbs, all different twice.
//! - Dragon's Hoard, **Treasure Match**: twelve chests, three each of four treasures, and the
//!   dragon's egg: open till three match; the egg first doubles it.
//! - The Daily Grind, **Cup Shuffle**: five rounds of find the bean, one spilt cup forgiven.
//! - Starlight Cabaret, **Champagne Tower**: six tiers of glasses, each poured one multiplying the
//!   tower, till it topples (an encore saves one) or stands complete.
//!
//! No choice in a mini game changes its odds: what's under what comes out of the round's random
//! numbers in the order things are opened, so every pick has the same chances (as drawing each
//! from what's left as it's opened). The tests work out every game's worth exactly.

use serde::{Deserialize, Serialize};

use super::feature::{self, Feature};
use super::reels::{self, BLANK, Grid, Pick, ROWS, SCATTER, WILD, WayWin, share};
use crate::fair::Stream;

/// One point (see `reels::PAY`) is this many ten-thousandths of the bet (0.0637×).
pub const POINT_TENTHOUSANDTHS: u64 = 637;
/// The middle reel, wild all the way down on a Wild spin.
pub const WILD_REEL: usize = 2;
/// A Chance spin starts the bonus this often (per million): about 1 in 44.5, four times what a
/// spin does.
pub const CHANCE_PPM: u32 = 22_460;
/// Mystery: a Super Bonus 3 times in 10 (a Bonus the rest).
pub const MYSTERY_SUPER_IN_10: u32 = 3;
/// All or nothing: 198 in 10,000 (1 in 50.5) win this many times the bet.
pub const ALL_IN_X: u64 = 2_500;
pub const ALL_IN_PER_10K: u32 = 198;
/// A Super Bonus pays every prize this many times over.
pub const SUPER_X: u64 = 3;

/// A machine's top prize: a fixed multiple of the bet, or its progressive jackpot's pool (its
/// share of the pool by the bet: all of it at the biggest bet).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Top {
    Fixed(u64),
    Progressive,
}

/// What can happen at the end of a machine's bonus, once in a long while: its name, what it
/// pays, and how often (per billion bonuses). A fixed one is worth a tenth of the bet a bonus
/// on average (paid for by the mini games being worth that much less), so its odds follow its
/// size; the progressive jackpots are fed apart, by their share of every round (`POOL_SHARE_BP`).
#[derive(Clone, Copy, Debug)]
pub struct TopPrize {
    pub name: &'static str,
    pub top: Top,
    pub per_billion: u32,
}

const fn fixed(name: &'static str, x: u64) -> TopPrize {
    TopPrize { name, top: Top::Fixed(x), per_billion: (100_000_000 / x) as u32 }
}

/// Each machine's top prize, so no two have the same reach: the Cosmic Jackpot's Supernova at
/// 500,000 times the bet; 50,000 times in the deep and the tomb; down to 5,000 on the gentle
/// ones; and two progressive jackpots, Gold Rush's Mother Lode and the Dragon's Hoard.
pub const TOPS: [TopPrize; 12] = [
    fixed("The Carolina Reaper", 10_000),
    fixed("Poseidon's Trident", 50_000),
    fixed("Supernova", 500_000),
    fixed("The Sugar Crown", 5_000),
    TopPrize { name: "The Mother Lode", top: Top::Progressive, per_billion: PROGRESSIVE_PER_BILLION },
    fixed("Neon Jackpot", 20_000),
    fixed("Ra's Gold", 50_000),
    fixed("The Golden Fish", 7_500),
    fixed("The Philosopher's Stone", 10_000),
    TopPrize { name: "The Hoard", top: Top::Progressive, per_billion: PROGRESSIVE_PER_BILLION },
    fixed("The Golden Bean", 5_000),
    fixed("Standing Ovation", 25_000),
];

/// A progressive jackpot: won at the end of 1 bonus in 100,000; started at this many chips, and
/// grown by this share of every round played on its machine (hundredths of a percent).
pub const PROGRESSIVE_PER_BILLION: u32 = 10_000;
pub const POOL_SEED: u64 = 100_000_000;
pub const POOL_SHARE_BP: u64 = 50;

pub fn progressive(machine: u8) -> bool {
    TOPS.get(machine as usize).is_some_and(|t| t.top == Top::Progressive)
}

/// A round's share for its machine's progressive jackpot (none on the others).
pub fn pool_share(machine: u8, cost: u64) -> u64 {
    if progressive(machine) { cost * POOL_SHARE_BP / 10_000 } else { 0 }
}

/// What a machine's top prize pays at `bet` (`pool`: its progressive jackpot's, as the round
/// began).
pub fn top_win(machine: u8, bet: u64, pool: u64) -> u64 {
    match TOPS[machine as usize].top {
        Top::Fixed(x) => bet.saturating_mul(x),
        Top::Progressive => (pool as u128 * bet as u128 / reels::MAX_BET as u128) as u64,
    }
}

/// The top prize at the end of a bonus: drawn once, after the mini game.
fn roll_top(machine: u8, bet: u64, pool: u64, s: &mut Stream) -> u64 {
    if s.below(1_000_000_000) < TOPS[machine as usize].per_billion { top_win(machine, bet, pool) } else { 0 }
}

fn is_zero(x: &u64) -> bool {
    *x == 0
}

/// What each Play Feature costs, in hundredths of the bet (each pays back about 99%).
pub fn price(f: Feature) -> u64 {
    match f {
        Feature::Spin => 100,
        Feature::Chance => 200,
        Feature::Wild => 383,
        Feature::Bonus => 6_000,
        Feature::Super => 18_000,
        Feature::Mystery => 9_600,
        Feature::AllIn => 5_000,
    }
}

/// Whether a spin is one the house takes.
pub fn check(machine: u8, bet: u64) -> Result<(), String> {
    reels::check(machine, bet)
}

/// Each machine's mini game: its name, and what it's about in a few words.
pub const GAMES: [(&str, &str); 12] = [
    ("Heat Ladder", "Climb the Scoville scale, pepper by pepper. Too hot ends it, unless you've milk. Top it for the Pepper X."),
    ("Abyss Dive", "Five depths, three clams each: pearls, an air tank, or a shark. The chest waits at the bottom."),
    ("Cargo Run", "The rocket climbs and the cargo multiplies, till it blows."),
    ("Candy Grab", "Three goes with the claw. A golden ticket is another go."),
    ("Mine Shaft", "Fifteen tiles: nuggets, a lantern, the mother lode, and four cave-ins. Clear the shaft for the finale."),
    ("Neon Drop", "Five balls down ten rows of pegs. The edges pay the most."),
    ("Curse of the Tomb", "Four chambers, three sarcophagi each: treasure, an amulet, or a mummy. Ra's mask waits past the fourth."),
    ("Ice Fishing", "Seven casts: fish, a boot, hot cocoa, or thin ice."),
    ("Potion Brew", "Pick three of nine jars. All one colour brews 4×, all different 2×."),
    ("Treasure Match", "Open chests till three match. Find the dragon's egg first to double it."),
    ("Cup Shuffle", "Five rounds of find the bean. One spilt cup is forgiven; find them all for the finale."),
    ("Champagne Tower", "Six tiers of glasses, each multiplying the tower, till it topples or stands complete."),
];

// The games' prizes, in hundredths of the bet.

/// The Heat Ladder, the Abyss Dive, the Curse of the Tomb, Ice Fishing and the Champagne Tower
/// start with a second chance in hand (a glass of milk, an air tank, an amulet, a cocoa, an
/// encore), so a bonus is never over at its first step.
pub const START_EXTRA: u8 = 1;

/// Heat Ladder: each rung's prize, the chance (percent) it's too hot, the chance a cleared rung
/// comes with milk, and the Pepper X at the top.
pub const RUNG: [u64; 8] = [135, 185, 252, 336, 471, 673, 1_009, 1_513];
pub const TOO_HOT: [u32; 8] = [10, 12, 15, 18, 22, 26, 30, 35];
pub const MILK: u32 = 12;
pub const PEPPER_X: u64 = 5_044;
/// Abyss Dive: each depth's clams (pearls, tanks, sharks) and its pearl; the chest.
pub const DEPTHS: [(u8, u8, u8); 5] = [(2, 0, 1), (1, 1, 1), (2, 0, 1), (1, 1, 1), (1, 0, 2)];
pub const PEARL: [u64; 5] = [616, 923, 1_334, 1_949, 3_078];
pub const CHEST: u64 = 6_155;
/// Cargo Run: the cargo's `CARGO_A`×10,000 / (u + 1) for u below 10,000, at most `CARGO_CAP`.
pub const CARGO_A: u64 = 1_159;
pub const CARGO_CAP: u64 = 72_000;
/// Candy Grab: what the claw can come up with (0: nothing; `TICKET`: a golden ticket) and how
/// often; the goes to start with, and the most there can be.
pub const TICKET: u64 = u64::MAX;
pub const CANDY: [(u64, u32); 7] = [(0, 20), (1_125, 26), (2_025, 20), (3_376, 14), (6_188, 6), (10_126, 2), (TICKET, 12)];
pub const GRABS: u8 = 3;
pub const MAX_GRABS: u8 = 6;
/// Mine Shaft: the nuggets, the mother lode and the finale.
pub const NUGGETS: [(u64, u32); 5] = [(257, 30), (428, 30), (684, 22), (1_018, 12), (1_702, 6)];
pub const LODE: [(u64, u32); 3] = [(4_277, 50), (6_844, 35), (12_832, 15)];
pub const MINE_FINALE: u64 = 12_832;
/// Neon Drop: the bins (by how many times the ball went right), and the balls.
pub const BINS: [u64; 11] = [12_335, 6_168, 3_084, 1_542, 771, 385, 771, 1_542, 3_084, 6_168, 12_335];
pub const BALLS: usize = 5;
pub const PEG_ROWS: usize = 10;
/// Curse of the Tomb: each chamber's sarcophagi (treasure, amulets, mummies) and its treasure;
/// Ra's mask.
pub const CHAMBERS: [(u8, u8, u8); 4] = [(2, 0, 1), (1, 1, 1), (2, 0, 1), (1, 0, 2)];
pub const TREASURE: [u64; 4] = [587, 979, 1_567, 3_133];
pub const MASK: u64 = 6_266;
/// Ice Fishing: what a cast brings up (kind 0 a boot, 1 to 5 fish, small to the big one; then
/// cocoa and thin ice) and how often; the casts.
pub const COCOA: u64 = u64::MAX - 1;
pub const ICE: u64 = u64::MAX;
pub const CATCH: [(u64, u32); 8] = [(0, 16), (557, 24), (1_044, 18), (1_948, 12), (3_479, 6), (5_567, 2), (COCOA, 8), (ICE, 12)];
pub const CASTS: u8 = 7;
/// Potion Brew: each jar's herb and how often; what a brew of one colour and of three multiply
/// it by.
pub const HERB: [(u64, u32); 5] = [(521, 30), (868, 30), (1_563, 22), (2_606, 12), (5_211, 6)];
pub const SAME_X: u64 = 4;
pub const RAINBOW_X: u64 = 2;
/// Treasure Match: each treasure's prize (the egg doubles it).
pub const TREASURES: [u64; 4] = [1_822, 2_733, 4_252, 6_681];
/// Cup Shuffle: each round's prize, the finale, and the spills forgiven.
pub const CUP_ROUND: [u64; 5] = [2_415, 3_865, 6_038, 9_662, 15_700];
pub const CUP_FINALE: u64 = 28_984;
pub const NAPKINS: u8 = 1;
/// Champagne Tower: the tower to start, what each tier poured multiplies it by (tenths), the
/// chance (percent) each topples, and of an encore on a tier poured.
pub const TOWER_BASE: u64 = 518;
pub const GROW_TENTHS: [u64; 6] = [15, 15, 16, 16, 18, 20];
pub const TOPPLE: [u32; 6] = [12, 15, 18, 22, 26, 30];
pub const ENCORE: u32 = 10;

/// A rung of the Heat Ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum Rung {
    /// Eaten: its prize (chips), and a glass of milk with it.
    Cleared { chips: u64, milk: bool },
    /// Too hot: a glass of milk drunk (on up), or the end.
    Hot { saved: bool },
}

/// What turned up: a prize (`kind`: which), a second chance (an air tank, an amulet, cocoa, a
/// golden ticket), or the danger (a shark, a mummy, thin ice; `saved` if a second chance was
/// used up on it, else the end).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum Find {
    Prize { kind: u8, chips: u64 },
    Extra,
    Danger { saved: bool },
}

/// A depth or a chamber: the one opened, and what the other two held (0 a prize, 1 the second
/// chance, 2 the danger).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trio {
    pub found: Find,
    pub others: Vec<u8>,
}

/// A ball down the pegs: right (1) or left (0) at each row, lowest bit first; its bin; its prize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ball {
    pub path: u16,
    pub bin: u8,
    pub chips: u64,
}

/// A jar: its colour (0 to 2), its herb (which, 0 to 4), the herb's worth.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Jar {
    pub colour: u8,
    pub herb: u8,
    pub chips: u64,
}

/// A round of the Cup Shuffle: the bean found or not (a napkin used up, or the end); and which
/// of the other two cups it was under, if not (for the show).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum Cup {
    Found { chips: u64 },
    Missed { saved: bool, other: u8 },
}

/// A tier of the Champagne Tower.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum Tier {
    /// Poured: the tower after it (hundredths of the bet), and an encore with it.
    Poured { tower: u64, encore: bool },
    /// Toppled: an encore used up (the tier's lost, on to the next), or the end.
    Toppled { saved: bool },
}

/// A machine's mini game, played out (its prizes in chips, before a Super Bonus's tripling).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "game", rename_all = "snake_case")]
pub enum Mini {
    Ladder { rungs: Vec<Rung>, top: bool },
    Dive { depths: Vec<Trio>, chest: bool },
    /// The cargo's multiple when it blew (hundredths of the bet).
    Cargo { x100: u64 },
    Claw { grabs: Vec<Find> },
    Mine { picks: Vec<Pick>, finale: bool },
    Drop { balls: Vec<Ball> },
    Tomb { chambers: Vec<Trio>, mask: bool },
    Fishing { casts: Vec<Find> },
    Potion { jars: Vec<Jar> },
    /// The chests in the order opened (0 to 3 a treasure, 4 the egg), the treasure that made
    /// three, and whether the egg came first.
    Match { opened: Vec<u8>, treasure: u8, egg: bool },
    Cups { rounds: Vec<Cup>, finale: bool },
    Tower { tiers: Vec<Tier>, tower: u64 },
}

/// The bonus: the game, what it won, and the multiple it's paid at (3 for a Super Bonus).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bonus {
    pub game: Mini,
    pub won: u64,
    pub times: u64,
    /// The machine's top prize, when it came at the end: what it paid, in place of the rest.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub top: u64,
}

impl Bonus {
    pub fn paid(&self) -> u64 {
        if self.top > 0 { self.top } else { self.won.saturating_mul(self.times) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub machine: u8,
    pub bet: u64,
    pub feature: Feature,
    /// What it cost (the bet times the feature's price).
    pub cost: u64,
    /// The spin (none for a feature that goes straight to its bonus or prize).
    pub grid: Option<Grid>,
    pub wins: Vec<WayWin>,
    pub scatters: u8,
    pub bonus: Option<Bonus>,
    /// Mystery: whether it turned out the Super Bonus. All or nothing: whether it won.
    pub lucky: bool,
    pub paid: u64,
    /// A progressive machine's jackpot pool as the round began (to play it again by).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub pool: u64,
}

impl Outcome {
    pub fn wagered(&self) -> u64 {
        self.cost
    }
}

/// A random permutation's first item and the rest: `items` drawn one by one from what's left.
fn shuffle3(s: &mut Stream, mut items: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(items.len());
    while !items.is_empty() {
        let i = s.below(items.len() as u32) as usize;
        out.push(items.remove(i));
    }
    out
}

/// A trio (a depth, a chamber) opened: one of its three, and the others.
fn open_trio(s: &mut Stream, (prizes, extras, dangers): (u8, u8, u8)) -> (u8, Vec<u8>) {
    let mut items = Vec::new();
    items.extend(std::iter::repeat_n(0u8, prizes as usize));
    items.extend(std::iter::repeat_n(1u8, extras as usize));
    items.extend(std::iter::repeat_n(2u8, dangers as usize));
    let order = shuffle3(s, items);
    (order[0], order[1..].to_vec())
}

/// A draw from a weighted table: its index.
fn weighted_index(s: &mut Stream, t: &[(u64, u32)]) -> usize {
    let total: u32 = t.iter().map(|x| x.1).sum();
    let mut r = s.below(total);
    for (i, &(_, w)) in t.iter().enumerate() {
        if r < w {
            return i;
        }
        r -= w;
    }
    t.len() - 1
}

fn ladder(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let (mut rungs, mut lives, mut won) = (Vec::new(), START_EXTRA, 0u64);
    for r in 0..RUNG.len() {
        if s.below(100) < TOO_HOT[r] {
            if lives == 0 {
                rungs.push(Rung::Hot { saved: false });
                return (Mini::Ladder { rungs, top: false }, won);
            }
            lives -= 1;
            rungs.push(Rung::Hot { saved: true });
        } else {
            let chips = share(bet, RUNG[r], 100);
            let milk = s.below(100) < MILK;
            lives += milk as u8;
            won += chips;
            rungs.push(Rung::Cleared { chips, milk });
        }
    }
    won += share(bet, PEPPER_X, 100);
    (Mini::Ladder { rungs, top: true }, won)
}

/// Abyss Dive and the Curse of the Tomb: trios of prize, second chance and danger, deeper and
/// richer, the last prize past the last.
fn trios(bet: u64, s: &mut Stream, layout: &[(u8, u8, u8)], prize: &[u64], last: u64) -> (Vec<Trio>, bool, u64) {
    let (mut out, mut extra, mut won) = (Vec::new(), START_EXTRA, 0u64);
    for (i, &t) in layout.iter().enumerate() {
        let (first, others) = open_trio(s, t);
        let found = match first {
            0 => {
                let chips = share(bet, prize[i], 100);
                won += chips;
                Find::Prize { kind: i as u8, chips }
            }
            1 => {
                extra += 1;
                Find::Extra
            }
            _ if extra > 0 => {
                extra -= 1;
                Find::Danger { saved: true }
            }
            _ => {
                out.push(Trio { found: Find::Danger { saved: false }, others });
                return (out, false, won);
            }
        };
        out.push(Trio { found, others });
    }
    won += share(bet, last, 100);
    (out, true, won)
}

fn cargo(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let u = s.below(10_000) as u64;
    let x100 = (CARGO_A * 10_000 / (u + 1)).min(CARGO_CAP);
    (Mini::Cargo { x100 }, share(bet, x100, 100))
}

fn claw(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let (mut grabs, mut left, mut won) = (Vec::new(), GRABS, 0u64);
    while left > 0 && grabs.len() < MAX_GRABS as usize {
        let i = weighted_index(s, &CANDY);
        match CANDY[i].0 {
            TICKET => grabs.push(Find::Extra),
            v => {
                left -= 1;
                let chips = share(bet, v, 100);
                won += chips;
                grabs.push(Find::Prize { kind: i as u8, chips });
            }
        }
    }
    (Mini::Claw { grabs }, won)
}

fn mine(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let (picks, finale, won) = reels::play_pick_with(bet, s, &NUGGETS, &LODE, MINE_FINALE);
    (Mini::Mine { picks, finale }, won)
}

fn drop(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let mut balls = Vec::new();
    let mut won = 0;
    for _ in 0..BALLS {
        let mut path = 0u16;
        for row in 0..PEG_ROWS {
            path |= (s.below(2) as u16) << row;
        }
        let bin = path.count_ones() as u8;
        let chips = share(bet, BINS[bin as usize], 100);
        won += chips;
        balls.push(Ball { path, bin, chips });
    }
    (Mini::Drop { balls }, won)
}

fn fishing(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let (mut casts, mut cocoa, mut won) = (Vec::new(), START_EXTRA, 0u64);
    for _ in 0..CASTS {
        let i = weighted_index(s, &CATCH);
        let f = match CATCH[i].0 {
            COCOA => {
                cocoa += 1;
                Find::Extra
            }
            ICE if cocoa > 0 => {
                cocoa -= 1;
                Find::Danger { saved: true }
            }
            ICE => {
                casts.push(Find::Danger { saved: false });
                return (Mini::Fishing { casts }, won);
            }
            v => {
                let chips = share(bet, v, 100);
                won += chips;
                Find::Prize { kind: i as u8, chips }
            }
        };
        casts.push(f);
    }
    (Mini::Fishing { casts }, won)
}

fn potion(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let mut left = [3u32; 3];
    let mut jars = Vec::new();
    let mut herbs = 0u64;
    for _ in 0..3 {
        let total: u32 = left.iter().sum();
        let mut r = s.below(total);
        let mut colour = 0;
        for (c, &n) in left.iter().enumerate() {
            if r < n {
                colour = c;
                break;
            }
            r -= n;
        }
        left[colour] -= 1;
        let herb = weighted_index(s, &HERB);
        herbs += HERB[herb].0;
        jars.push(Jar { colour: colour as u8, herb: herb as u8, chips: share(bet, HERB[herb].0, 100) });
    }
    let x = brew_x(&jars);
    (Mini::Potion { jars }, share(bet, herbs * x, 100))
}

/// What a brew multiplies its herbs by: all one colour, all different, or neither.
pub fn brew_x(jars: &[Jar]) -> u64 {
    let c: Vec<u8> = jars.iter().map(|j| j.colour).collect();
    if c.len() == 3 && c[0] == c[1] && c[1] == c[2] {
        SAME_X
    } else if c.len() == 3 && c[0] != c[1] && c[1] != c[2] && c[0] != c[2] {
        RAINBOW_X
    } else {
        1
    }
}

fn treasure_match(bet: u64, s: &mut Stream) -> (Mini, u64) {
    // Twelve chests (three of each treasure) and the egg, opened one by one till three match.
    let mut left = [3u32, 3, 3, 3, 1];
    let mut seen = [0u8; 4];
    let mut opened = Vec::new();
    let mut egg = false;
    loop {
        let total: u32 = left.iter().sum();
        let mut r = s.below(total);
        let mut k = 0;
        for (i, &n) in left.iter().enumerate() {
            if r < n {
                k = i;
                break;
            }
            r -= n;
        }
        left[k] -= 1;
        opened.push(k as u8);
        if k == 4 {
            egg = true;
            continue;
        }
        seen[k] += 1;
        if seen[k] == 3 {
            let x = if egg { 2 } else { 1 };
            return (Mini::Match { opened, treasure: k as u8, egg }, share(bet, TREASURES[k] * x, 100));
        }
    }
}

fn cups(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let (mut rounds, mut napkins, mut won) = (Vec::new(), NAPKINS, 0u64);
    for &prize in &CUP_ROUND {
        if s.below(3) == 0 {
            let chips = share(bet, prize, 100);
            won += chips;
            rounds.push(Cup::Found { chips });
        } else {
            let other = s.below(2) as u8;
            if napkins == 0 {
                rounds.push(Cup::Missed { saved: false, other });
                return (Mini::Cups { rounds, finale: false }, won);
            }
            napkins -= 1;
            rounds.push(Cup::Missed { saved: true, other });
        }
    }
    won += share(bet, CUP_FINALE, 100);
    (Mini::Cups { rounds, finale: true }, won)
}

fn tower(bet: u64, s: &mut Stream) -> (Mini, u64) {
    let (mut tiers, mut encores, mut value) = (Vec::new(), START_EXTRA, TOWER_BASE);
    for t in 0..GROW_TENTHS.len() {
        if s.below(100) < TOPPLE[t] {
            if encores == 0 {
                tiers.push(Tier::Toppled { saved: false });
                return (Mini::Tower { tiers, tower: value }, share(bet, value, 100));
            }
            encores -= 1;
            tiers.push(Tier::Toppled { saved: true });
        } else {
            value = (value * GROW_TENTHS[t] + 5) / 10;
            let encore = s.below(100) < ENCORE;
            encores += encore as u8;
            tiers.push(Tier::Poured { tower: value, encore });
        }
    }
    (Mini::Tower { tiers, tower: value }, share(bet, value, 100))
}

/// A machine's mini game, played out: the game and what it won.
pub fn play_mini(machine: u8, bet: u64, s: &mut Stream) -> (Mini, u64) {
    match machine {
        0 => ladder(bet, s),
        1 => {
            let (depths, chest, won) = trios(bet, s, &DEPTHS, &PEARL, CHEST);
            (Mini::Dive { depths, chest }, won)
        }
        2 => cargo(bet, s),
        3 => claw(bet, s),
        4 => mine(bet, s),
        5 => drop(bet, s),
        6 => {
            let (chambers, mask, won) = trios(bet, s, &CHAMBERS, &TREASURE, MASK);
            (Mini::Tomb { chambers, mask }, won)
        }
        7 => fishing(bet, s),
        8 => potion(bet, s),
        9 => treasure_match(bet, s),
        10 => cups(bet, s),
        _ => tower(bet, s),
    }
}

/// The spin's grid: as ever; for a Chance spin, a grid with the bonus (about 1 in 44.5) or one
/// without, each as likely as any other of its kind (drawn till one is); for a Wild spin, the
/// middle reel wild.
fn grid_for(feature: Feature, s: &mut Stream) -> Grid {
    match feature {
        Feature::Chance => {
            let bonus = s.below(1_000_000) < CHANCE_PPM;
            loop {
                let g = reels::spin_grid(s);
                let scatters = g.iter().flatten().filter(|&&x| x == SCATTER).count();
                if (scatters >= 3) == bonus {
                    return g;
                }
            }
        }
        Feature::Wild => {
            let mut g = reels::spin_grid(s);
            g[WILD_REEL] = [WILD; ROWS];
            g
        }
        _ => reels::spin_grid(s),
    }
}

/// Plays a round on a machine: a spin (its bonus with three scatters), or a feature.
pub fn play(machine: u8, bet: u64, feature: Feature, s: &mut Stream) -> Outcome {
    play_with(machine, bet, feature, s, 0)
}

/// `play`, on a progressive machine with its jackpot's `pool` as the round begins.
pub fn play_with(machine: u8, bet: u64, feature: Feature, s: &mut Stream, pool: u64) -> Outcome {
    let cost = feature::cost(bet, price(feature));
    let pool = if progressive(machine) { pool } else { 0 };
    let mut o = Outcome { machine, bet, feature, cost, grid: None, wins: Vec::new(), scatters: 0, bonus: None, lucky: false, paid: 0, pool };
    let bonus = |times: u64, s: &mut Stream| {
        let (game, won) = play_mini(machine, bet, s);
        let top = roll_top(machine, bet, pool, s);
        Bonus { game, won, times, top }
    };
    match feature {
        Feature::Spin | Feature::Chance | Feature::Wild => {
            let g = grid_for(feature, s);
            let (wins, scatters) = reels::score_at(&g, bet, POINT_TENTHOUSANDTHS);
            o.grid = Some(g);
            o.wins = wins;
            o.scatters = scatters;
            if scatters >= 3 {
                o.bonus = Some(bonus(1, s));
            }
        }
        Feature::Bonus => o.bonus = Some(bonus(1, s)),
        Feature::Super => o.bonus = Some(bonus(SUPER_X, s)),
        Feature::Mystery => {
            o.lucky = s.below(10) < MYSTERY_SUPER_IN_10;
            o.bonus = Some(bonus(if o.lucky { SUPER_X } else { 1 }, s));
        }
        Feature::AllIn => {
            o.lucky = s.below(10_000) < ALL_IN_PER_10K;
            o.paid = if o.lucky { bet * ALL_IN_X } else { 0 };
            return o;
        }
    }
    o.paid = (o.wins.iter().map(|w| w.pay).sum::<u64>() + o.bonus.as_ref().map(|b| b.paid()).unwrap_or(0)).min(super::MAX_PAYOUT);
    o
}

/// The cells' symbols for the app's reels (BLANK cells drawn as nothing).
pub fn is_blank(sym: u8) -> bool {
    sym == BLANK
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::casino::reels::{BETS, PAY, PAY_SYMBOLS, REELS, WEIGHTS};

    fn mean(t: &[(u64, u32)]) -> f64 {
        let w: f64 = t.iter().map(|x| x.1 as f64).sum();
        t.iter().map(|&(v, n)| v as f64 * n as f64).sum::<f64>() / w
    }

    /// Each mini game's exact worth, in hundredths of the bet (every way it can go, weighed by
    /// how likely it is).
    fn worth(machine: u8) -> f64 {
        match machine {
            0 => {
                fn f(r: usize, lives: u32, memo: &mut HashMap<(usize, u32), f64>) -> f64 {
                    if r == RUNG.len() {
                        return PEPPER_X as f64;
                    }
                    if let Some(v) = memo.get(&(r, lives)) {
                        return *v;
                    }
                    let q = TOO_HOT[r] as f64 / 100.0;
                    let m = MILK as f64 / 100.0;
                    let mut ev = (1.0 - q) * (RUNG[r] as f64 + m * f(r + 1, lives + 1, memo) + (1.0 - m) * f(r + 1, lives, memo));
                    if lives > 0 {
                        ev += q * f(r + 1, lives - 1, memo);
                    }
                    memo.insert((r, lives), ev);
                    ev
                }
                f(0, START_EXTRA as u32, &mut HashMap::new())
            }
            1 | 6 => {
                let (layout, prize, last): (&[(u8, u8, u8)], &[u64], u64) = if machine == 1 { (&DEPTHS, &PEARL, CHEST) } else { (&CHAMBERS, &TREASURE, MASK) };
                fn f(i: usize, extra: u32, layout: &[(u8, u8, u8)], prize: &[u64], last: u64) -> f64 {
                    if i == layout.len() {
                        return last as f64;
                    }
                    let (p, e, d) = layout[i];
                    let n = (p + e + d) as f64;
                    let mut ev = p as f64 / n * (prize[i] as f64 + f(i + 1, extra, layout, prize, last)) + e as f64 / n * f(i + 1, extra + 1, layout, prize, last);
                    if extra > 0 {
                        ev += d as f64 / n * f(i + 1, extra - 1, layout, prize, last);
                    }
                    ev
                }
                f(0, START_EXTRA as u32, layout, prize, last)
            }
            2 => (0..10_000u64).map(|u| (CARGO_A * 10_000 / (u + 1)).min(CARGO_CAP) as f64).sum::<f64>() / 10_000.0,
            3 => {
                let total: f64 = CANDY.iter().map(|x| x.1 as f64).sum();
                let pt = CANDY.iter().filter(|x| x.0 == TICKET).map(|x| x.1 as f64).sum::<f64>() / total;
                let ev_val = CANDY.iter().filter(|x| x.0 != TICKET).map(|&(v, w)| v as f64 * w as f64).sum::<f64>() / total;
                fn f(left: u8, used: u8, ev_val: f64, pt: f64) -> f64 {
                    if left == 0 || used == MAX_GRABS {
                        return 0.0;
                    }
                    ev_val + (1.0 - pt) * f(left - 1, used + 1, ev_val, pt) + pt * f(left, used + 1, ev_val, pt)
                }
                f(GRABS, 0, ev_val, pt)
            }
            4 => {
                let (ce, ge) = (mean(&NUGGETS), mean(&LODE));
                fn f(h: u8, l: u8, g: u8, c: u8, lives: u8, ce: f64, ge: f64) -> f64 {
                    let total = (h + l + g + c) as f64;
                    if l + g + c == 0 {
                        return 0.0;
                    }
                    let after = |l: u8, g: u8, c: u8, lives: u8| if l + g + c == 0 { MINE_FINALE as f64 } else { f(h, l, g, c, lives, ce, ge) };
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
                let safe = reels::TILES as u8 - reels::HAZARDS - reels::LIVES_IN_BAG - reels::GOLDS;
                f(reels::HAZARDS, reels::LIVES_IN_BAG, reels::GOLDS, safe, reels::START_LIVES, ce, ge)
            }
            5 => {
                let c = |n: u64, k: u64| (0..k).fold(1.0, |a, i| a * (n - i) as f64 / (i + 1) as f64);
                BALLS as f64 * (0..=PEG_ROWS as u64).map(|k| c(PEG_ROWS as u64, k) / 1024.0 * BINS[k as usize] as f64).sum::<f64>()
            }
            7 => {
                let total: f64 = CATCH.iter().map(|x| x.1 as f64).sum();
                fn f(left: u8, cocoa: u8, total: f64) -> f64 {
                    if left == 0 {
                        return 0.0;
                    }
                    let mut ev = 0.0;
                    for &(v, w) in &CATCH {
                        let p = w as f64 / total;
                        match v {
                            COCOA => ev += p * f(left - 1, cocoa + 1, total),
                            ICE if cocoa > 0 => ev += p * f(left - 1, cocoa - 1, total),
                            ICE => {}
                            v => ev += p * (v as f64 + f(left - 1, cocoa, total)),
                        }
                    }
                    ev
                }
                f(CASTS, START_EXTRA, total)
            }
            8 => {
                // Three of nine (three of each colour): all one colour 3/84, all different 27/84.
                let (same, rainbow) = (3.0 / 84.0, 27.0 / 84.0);
                3.0 * mean(&HERB) * (same * SAME_X as f64 + rainbow * RAINBOW_X as f64 + (1.0 - same - rainbow))
            }
            9 => {
                // The chance the egg's out before a treasure makes three, over every order.
                fn g(counts: [u8; 4], egg: bool, memo: &mut HashMap<([u8; 4], bool), f64>) -> f64 {
                    if let Some(v) = memo.get(&(counts, egg)) {
                        return *v;
                    }
                    let n = counts.iter().map(|c| (3 - c) as f64).sum::<f64>() + if egg { 0.0 } else { 1.0 };
                    let mut p = 0.0;
                    if !egg {
                        p += g(counts, true, memo) / n;
                    }
                    for k in 0..4 {
                        if counts[k] < 3 {
                            let q = (3 - counts[k]) as f64 / n;
                            let mut c = counts;
                            c[k] += 1;
                            p += q * if c[k] == 3 { if egg { 1.0 } else { 0.0 } } else { g(c, egg, memo) };
                        }
                    }
                    memo.insert((counts, egg), p);
                    p
                }
                let pe = g([0; 4], false, &mut HashMap::new());
                TREASURES.iter().sum::<u64>() as f64 / 4.0 * (1.0 + pe)
            }
            10 => {
                fn f(r: usize, napkins: u8) -> f64 {
                    if r == CUP_ROUND.len() {
                        return CUP_FINALE as f64;
                    }
                    let mut ev = (CUP_ROUND[r] as f64 + f(r + 1, napkins)) / 3.0;
                    if napkins > 0 {
                        ev += 2.0 / 3.0 * f(r + 1, napkins - 1);
                    }
                    ev
                }
                f(0, NAPKINS)
            }
            _ => {
                fn f(t: usize, enc: u8, value: u64) -> f64 {
                    if t == GROW_TENTHS.len() {
                        return value as f64;
                    }
                    let q = TOPPLE[t] as f64 / 100.0;
                    let e = ENCORE as f64 / 100.0;
                    let grown = (value * GROW_TENTHS[t] + 5) / 10;
                    let mut ev = (1.0 - q) * (e * f(t + 1, enc + 1, grown) + (1.0 - e) * f(t + 1, enc, grown));
                    ev += q * if enc > 0 { f(t + 1, enc - 1, value) } else { value as f64 };
                    ev
                }
                f(0, START_EXTRA, TOWER_BASE)
            }
        }
    }

    /// The chance of each (count of a symbol or wild, count of scatters) in a reel's three cells.
    fn reel_counts(reel: usize, sym: u8, wild_reel: bool) -> Vec<((usize, usize), f64)> {
        if wild_reel {
            return vec![((3, 0), 1.0)];
        }
        let mut w = WEIGHTS;
        if reel == 0 || reel == REELS - 1 {
            w[BLANK as usize] += w[WILD as usize];
            w[WILD as usize] = 0;
        }
        let total: f64 = w.iter().map(|&x| x as f64).sum();
        let pm = (w[sym as usize] + w[WILD as usize]) as f64 / total;
        let ps = w[SCATTER as usize] as f64 / total;
        let po = 1.0 - pm - ps;
        let fact = |n: usize| (1..=n).product::<usize>() as f64;
        let mut out = Vec::new();
        for m in 0..=3 {
            for sc in 0..=(3 - m) {
                let o = 3 - m - sc;
                out.push(((m, sc), fact(3) / (fact(m) * fact(sc) * fact(o)) * pm.powi(m as i32) * ps.powi(sc as i32) * po.powi(o as i32)));
            }
        }
        out
    }

    /// The base game exactly, at a bet: the chance of the bonus, and what the reels pay (times the
    /// bet) on grids with it and on grids without.
    fn base(bet: u64, wild: bool) -> (f64, f64, f64) {
        let (mut p_bonus, mut with, mut without) = (0.0, 0.0, 0.0);
        for sym in 0..PAY_SYMBOLS {
            let d: Vec<_> = (0..REELS).map(|r| reel_counts(r, sym, wild && r == WILD_REEL)).collect();
            let mut stack = vec![(0usize, Vec::<usize>::new(), 0usize, 1.0f64)];
            while let Some((r, ms, sc, pr)) = stack.pop() {
                if r == REELS {
                    let (mut n, mut ways) = (0, 1u64);
                    for &m in &ms {
                        if m == 0 {
                            break;
                        }
                        ways *= m as u64;
                        n += 1;
                    }
                    if sym == 0 && sc >= 3 {
                        p_bonus += pr;
                    }
                    if n >= 3 {
                        let win = share(bet, PAY[sym as usize][n - 3] * ways * POINT_TENTHOUSANDTHS, 10_000) as f64 / bet as f64;
                        if sc >= 3 {
                            with += pr * win;
                        } else {
                            without += pr * win;
                        }
                    }
                    continue;
                }
                for &((m, s), p) in &d[r] {
                    let mut ms = ms.clone();
                    ms.push(m);
                    stack.push((r + 1, ms, (sc + s).min(3), pr * p));
                }
            }
        }
        (p_bonus, with, without)
    }

    /// Every machine's mini game is worth about 59.4 times the bet (so a bonus bought for 60
    /// pays back 99%), and none pays more than 833 times (a Super Bonus's three times that, at
    /// the biggest bet, is the most a round pays).
    #[test]
    fn every_mini_game_is_worth_about_59_times_the_bet() {
        for m in 0..12u8 {
            let w = worth(m) / 100.0 + top_worth(m);
            assert!((59.2..59.6).contains(&w), "machine {m}: {w}");
        }
        let most = [
            RUNG.iter().sum::<u64>() + PEPPER_X,
            PEARL.iter().sum::<u64>() + CHEST,
            CARGO_CAP,
            MAX_GRABS as u64 * CANDY.iter().filter(|x| x.0 != TICKET).map(|x| x.0).max().unwrap(),
            9 * NUGGETS.iter().map(|x| x.0).max().unwrap() + LODE.iter().map(|x| x.0).max().unwrap() + MINE_FINALE,
            BALLS as u64 * BINS.iter().max().unwrap(),
            TREASURE.iter().sum::<u64>() + MASK,
            CASTS as u64 * CATCH.iter().filter(|x| x.0 < COCOA).map(|x| x.0).max().unwrap(),
            3 * HERB.iter().map(|x| x.0).max().unwrap() * SAME_X,
            2 * TREASURES.iter().max().unwrap(),
            CUP_ROUND.iter().sum::<u64>() + CUP_FINALE,
            GROW_TENTHS.iter().fold(TOWER_BASE, |v, g| (v * g + 5) / 10),
        ];
        for (m, x) in most.iter().enumerate() {
            assert!(*x <= 83_300, "machine {m} can pay {}x", x / 100);
            assert!(x / 100 * SUPER_X * reels::MAX_BET <= super::super::MAX_PAYOUT);
        }
    }

    /// The whole game pays back about 99% at every bet on every machine, and so does every Play
    /// Feature: a Chance spin, a Wild spin, the bonus bought, the Super Bonus, the Mystery Bonus,
    /// and all or nothing.
    #[test]
    fn every_way_to_play_pays_back_about_99_percent() {
        for bet in BETS {
            let (p, with, without) = base(bet, false);
            assert!((p - 0.005674).abs() < 0.000001, "{p}");
            let (pw, with_w, without_w) = base(bet, true);
            let chance = CHANCE_PPM as f64 / 1e6;
            for m in 0..12u8 {
                let w = worth(m) / 100.0 + top_worth(m);
                let cost = |f: Feature| feature::cost(bet, price(f)) as f64 / bet as f64;
                let spin = with + without + p * w;
                let chance_rtp = (chance * (with / p + w) + (1.0 - chance) * without / (1.0 - p)) / cost(Feature::Chance);
                let wild = (with_w + without_w + pw * w) / cost(Feature::Wild);
                let bonus = w / cost(Feature::Bonus);
                let sup = w * SUPER_X as f64 / cost(Feature::Super);
                let mystery = w * (1.0 + (SUPER_X - 1) as f64 * MYSTERY_SUPER_IN_10 as f64 / 10.0) / cost(Feature::Mystery);
                let all_in = ALL_IN_X as f64 * ALL_IN_PER_10K as f64 / 10_000.0 / cost(Feature::AllIn);
                for (what, rtp) in [("a spin", spin), ("Chance", chance_rtp), ("Wild", wild), ("Bonus", bonus), ("Super", sup), ("Mystery", mystery), ("All or nothing", all_in)] {
                    // (At the smallest bets, a feature's cost rounds to the chip: within 1.5%.)
                    let ok = if bet >= 100 { 0.985..0.995 } else { 0.98..0.999 };
                    assert!(ok.contains(&rtp), "bet {bet}, machine {m}, {what}: {rtp}");
                }
            }
        }
    }

    /// Played out, rounds pay what their parts add up to, cost what their feature does, and
    /// each mini game comes out near its worth; a Chance spin starts the bonus about four times
    /// as often; a Wild spin's middle reel is wild.
    #[test]
    fn rounds_played_out_add_up_and_land_near_the_math() {
        let mut s = Stream::new([9; 32]);
        for m in 0..12u8 {
            let n = 20_000;
            let mut won = 0u64;
            for _ in 0..n {
                let o = play(m, 100, Feature::Bonus, &mut s);
                let b = o.bonus.as_ref().unwrap();
                assert_eq!(o.cost, 6_000);
                assert_eq!(o.paid, b.paid());
                won += b.won;
                // Its parts add up to what it says it won.
                let parts: u64 = match &b.game {
                    Mini::Ladder { rungs, top } => rungs.iter().map(|r| if let Rung::Cleared { chips, .. } = r { *chips } else { 0 }).sum::<u64>() + if *top { share(100, PEPPER_X, 100) } else { 0 },
                    Mini::Dive { depths, chest } => depths.iter().map(|t| if let Find::Prize { chips, .. } = t.found { chips } else { 0 }).sum::<u64>() + if *chest { share(100, CHEST, 100) } else { 0 },
                    Mini::Tomb { chambers, mask } => chambers.iter().map(|t| if let Find::Prize { chips, .. } = t.found { chips } else { 0 }).sum::<u64>() + if *mask { share(100, MASK, 100) } else { 0 },
                    Mini::Cargo { x100 } => share(100, *x100, 100),
                    Mini::Claw { grabs } | Mini::Fishing { casts: grabs } => grabs.iter().map(|g| if let Find::Prize { chips, .. } = g { *chips } else { 0 }).sum(),
                    Mini::Mine { picks, finale } => picks.iter().map(|p| match p { Pick::Chips { chips } | Pick::Gold { chips } => *chips, _ => 0 }).sum::<u64>() + if *finale { share(100, MINE_FINALE, 100) } else { 0 },
                    Mini::Drop { balls } => {
                        assert_eq!(balls.len(), BALLS);
                        balls.iter().map(|b| b.chips).sum()
                    }
                    Mini::Potion { jars } => jars.iter().map(|j| j.chips).sum::<u64>() * brew_x(jars),
                    Mini::Match { opened, treasure, egg } => {
                        assert_eq!(opened.iter().filter(|k| **k == *treasure).count(), 3);
                        assert_eq!(*egg, opened.contains(&4));
                        share(100, TREASURES[*treasure as usize] * if *egg { 2 } else { 1 }, 100)
                    }
                    Mini::Cups { rounds, finale } => rounds.iter().map(|r| if let Cup::Found { chips } = r { *chips } else { 0 }).sum::<u64>() + if *finale { share(100, CUP_FINALE, 100) } else { 0 },
                    Mini::Tower { tower, .. } => share(100, *tower, 100),
                };
                assert_eq!(parts, b.won, "machine {m}: {:?}", b.game);
            }
            let per = won as f64 / n as f64 / 100.0;
            assert!((50.0..70.0).contains(&per), "machine {m}: worth {per} played out");
        }
        // Chance spins: the bonus about 1 in 44.5; Wild spins: the middle reel wild.
        let (mut bonuses, n) = (0, 40_000);
        for i in 0..n {
            let o = play((i % 12) as u8, 200, Feature::Chance, &mut s);
            assert_eq!(o.cost, 400);
            assert_eq!(o.bonus.is_some(), o.scatters >= 3);
            bonuses += o.bonus.is_some() as u32;
        }
        let rate = bonuses as f64 / n as f64;
        assert!((0.018..0.027).contains(&rate), "{rate}");
        let o = play(3, 100, Feature::Wild, &mut s);
        assert_eq!(o.grid.unwrap()[WILD_REEL], [WILD; ROWS]);
        assert_eq!(o.cost, 383);
        // A Super Bonus pays three times what it won; all or nothing is the one or the other.
        let o = play(7, 50, Feature::Super, &mut s);
        assert_eq!(o.paid, o.bonus.as_ref().unwrap().won * 3);
        assert_eq!(o.cost, 9_000);
        for _ in 0..200 {
            let o = play(0, 10_000, Feature::AllIn, &mut s);
            assert!(o.paid == 0 || o.paid == 25_000_000);
            assert_eq!(o.paid > 0, o.lucky);
        }
    }

    /// A fixed top prize's worth a bonus, in times the bet (a progressive one's is fed apart).
    fn top_worth(m: u8) -> f64 {
        match TOPS[m as usize].top {
            Top::Fixed(x) => x as f64 * TOPS[m as usize].per_billion as f64 / 1e9,
            Top::Progressive => 0.0,
        }
    }

    /// The machines reach differently (a Supernova at 500,000 times the bet, and two
    /// progressive jackpots), each top prize comes about as often as it should, and it pays in
    /// place of the bonus's own prizes, the Super Bonus's included.
    #[test]
    fn top_prizes_reach_far_and_come_as_often_as_they_should() {
        let reach: Vec<u64> = (0..12u8).map(|m| top_win(m, 1, 0)).collect();
        assert_eq!(reach.iter().max(), Some(&500_000));
        assert!(reach.iter().filter(|x| **x == 50_000).count() >= 2);
        let mut kinds = reach.clone();
        kinds.sort();
        kinds.dedup();
        assert!(kinds.len() >= 7, "machines differ: {kinds:?}");
        assert_eq!((0..12u8).filter(|m| progressive(*m)).count(), 2);
        for m in 0..12u8 {
            assert!((0.09..0.11).contains(&top_worth(m)) || progressive(m), "machine {m}");
        }
        // The biggest a round can pay is within what the house pays at all.
        assert!(top_win(2, reels::MAX_BET, 0) <= super::super::MAX_PAYOUT);
        // About as often as it should: The Sugar Crown, 1 bonus in 50,000.
        let mut s = Stream::new([4; 32]);
        let n = 2_000_000;
        let hits = (0..n).filter(|_| roll_top(3, 100, 0, &mut s) > 0).count();
        assert!((25..60).contains(&hits), "{hits} in {n}");
        // A progressive jackpot pays its pool's share by the bet; with no pool, nothing.
        assert_eq!(top_win(4, reels::MAX_BET, 123_456_789), 123_456_789);
        assert_eq!(top_win(9, reels::MAX_BET / 10, 1_000_000), 100_000);
        assert_eq!(pool_share(4, 10_000), 50);
        assert_eq!(pool_share(0, 10_000), 0);
        // Paid in place of the rest (a Super Bonus's three times included).
        let b = Bonus { game: Mini::Cargo { x100: 500 }, won: 5_000, times: SUPER_X, top: 7_000_000 };
        assert_eq!(b.paid(), 7_000_000);
        let b = Bonus { top: 0, ..b };
        assert_eq!(b.paid(), 15_000);
        // Played with a pool, a progressive round keeps it to be checked by; others don't.
        assert_eq!(play_with(4, 100, Feature::Bonus, &mut s, 5_000_000).pool, 5_000_000);
        assert_eq!(play_with(0, 100, Feature::Bonus, &mut s, 5_000_000).pool, 0);
    }

    #[test]
    fn a_round_plays_the_same_again() {
        for f in Feature::ALL {
            for m in 0..12u8 {
                let seed = [m + 3 * f as u8; 32];
                assert_eq!(play(m, 500, f, &mut Stream::new(seed)), play(m, 500, f, &mut Stream::new(seed)));
            }
        }
    }
}
