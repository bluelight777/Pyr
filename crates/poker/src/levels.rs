//! Levels (0.13.5): a hundred levels in ten tiers, each with its own crest (Spark, Ember,
//! Flame, Blaze, Bonfire, Wildfire, Inferno, Sunfire, Starfire and Eternal Flame), and past
//! level 100 an Ember Star for every 10,000 XP. Experience comes from playing, about the same a
//! minute at every game (a little more for bigger stakes, and half once the day's play passes
//! 4,000 XP). Every level pays chips (three times as much every fifth), every new tier opens a
//! chest of 100,000, and the tier is your VIP Club status: Ember Back (a share of the house's
//! edge on your play), more daily chips, and from Ember up a weekly bonus.
//!
//! The numbers are 0.13.28's, read off its own screens and records (see the rebuild's
//! `measured/levels.md`): the XP each level takes and what each pays, the XP a round of each game
//! earns, the house edge Ember Back is a share of, the tiers' shares, the daily chips, the weekly
//! bonus, the missions. A record from before (on 0.13.0's curve) keeps its level: its experience
//! moves onto this curve where it was (`curve` 2).

/// The XP each level takes to the next (level 1 to 2 first), levels 1 to 99: 0.13.28's.
pub const LEVEL_XP: [u64; 99] = [
    80, 90, 100, 110, 140, 160, 190, 210, 250, 280, 320, 360, 400, 440, 490, 540, 590, 640, 690, 750, 800, 860, 920, 990, 1050, 1110, 1180, 1250, 1320, 1390, 1470, 1540, 1620, 1690,
    1770, 1850, 1930, 2020, 2100, 2190, 2280, 2360, 2450, 2540, 2640, 2730, 2830, 2920, 3020, 3120, 3220, 3320, 3420, 3520, 3630, 3730, 3840, 3950, 4060, 4170, 4280, 4390, 4510, 4620,
    4740, 4850, 4970, 5090, 5210, 5330, 5450, 5580, 5700, 5830, 5950, 6080, 6210, 6340, 6470, 6600, 6730, 6870, 7000, 7140, 7270, 7410, 7550, 7690, 7830, 7970, 8110, 8260, 8400, 8550,
    8690, 8840, 8990, 9140, 9290,
];

pub const MAX_LEVEL: u32 = 100;
/// Past level 100: an Ember Star for every this much XP. Crossing one pays `STAR_PAYS` (once,
/// however many a single round crosses: 0.13.28 did the same).
pub const STAR_XP: u64 = 10_000;
pub const STAR_PAYS: u64 = 25_000;
/// The curve records are on (`Record::curve`): 2, this one.
pub const CURVE: u8 = 2;

/// The tiers: name, the level each starts at.
pub const TIERS: [(&str, u32); 10] =
    [("Spark", 1), ("Ember", 10), ("Flame", 20), ("Blaze", 30), ("Bonfire", 40), ("Wildfire", 50), ("Inferno", 60), ("Sunfire", 70), ("Starfire", 80), ("Eternal Flame", 100)];

/// What a new tier's chest holds.
pub const CHEST: u64 = 100_000;

/// The XP it takes to reach `level` from nothing (level 1 at 0).
pub fn xp_for(level: u32) -> u64 {
    LEVEL_XP.iter().take(level.clamp(1, MAX_LEVEL) as usize - 1).sum()
}

/// The level `xp` reaches (1 to 100).
pub fn level_for(xp: u64) -> u32 {
    let mut total = 0;
    for (i, c) in LEVEL_XP.iter().enumerate() {
        total += c;
        if xp < total {
            return i as u32 + 1;
        }
    }
    MAX_LEVEL
}

/// Ember Stars: one for every 10,000 XP past level 100.
pub fn stars(xp: u64) -> u64 {
    xp.saturating_sub(xp_for(MAX_LEVEL)) / STAR_XP
}

/// The XP still to go to the next level (or, at 100, the next star).
pub fn to_next(xp: u64) -> u64 {
    let l = level_for(xp);
    if l < MAX_LEVEL {
        xp_for(l + 1) - xp
    } else {
        xp_for(MAX_LEVEL) + (stars(xp) + 1) * STAR_XP - xp
    }
}

/// Where `xp` is between this level (or star) and the next: (from, to).
pub fn span(xp: u64) -> (u64, u64) {
    let l = level_for(xp);
    if l < MAX_LEVEL {
        (xp_for(l), xp_for(l + 1))
    } else {
        let from = xp_for(MAX_LEVEL) + stars(xp) * STAR_XP;
        (from, from + STAR_XP)
    }
}

/// Which tier a level is in (0: Spark, to 9: Eternal Flame).
pub fn tier(level: u32) -> usize {
    TIERS.iter().rposition(|t| level >= t.1).unwrap_or(0)
}

pub fn tier_name(level: u32) -> &'static str {
    TIERS[tier(level)].0
}

/// The levels a tier spans (first, last).
pub fn tier_levels(t: usize) -> (u32, u32) {
    let first = TIERS[t.min(TIERS.len() - 1)].1;
    let last = TIERS.get(t + 1).map(|n| n.1 - 1).unwrap_or(MAX_LEVEL);
    (first, last)
}

/// Whether `level` is the first of its tier (a chest, a frame, maybe a felt).
pub fn starts_tier(level: u32) -> bool {
    level > 1 && TIERS.iter().any(|t| t.1 == level)
}

fn roman(mut n: u32) -> String {
    let mut s = String::new();
    for (v, r) in [(10, "X"), (9, "IX"), (5, "V"), (4, "IV"), (1, "I")] {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// A level as it's called: its tier and its place in it ("Spark I", "Ember IV", "Starfire
/// XIV"), or just "Eternal Flame".
pub fn title(level: u32) -> String {
    let t = tier(level);
    if t == TIERS.len() - 1 {
        return TIERS[t].0.to_string();
    }
    format!("{} {}", TIERS[t].0, roman(level - TIERS[t].1 + 1))
}

/// What reaching `level` pays: 1,000 + 400 a level, three times that every fifth level, and a
/// new tier's chest.
pub fn pays(level: u32) -> u64 {
    let base = 1_000 + 400 * level as u64;
    let base = if level % 5 == 0 { base * 3 } else { base };
    base + if starts_tier(level) { CHEST } else { 0 }
}

/// Ember Back: the tier's share of the house's edge, in hundredths of a percent (5% at Spark,
/// then 2.5% more a tier to 25% at Starfire; 30% at Eternal Flame).
pub fn ember_bp(tier: usize) -> u64 {
    if tier >= TIERS.len() - 1 { 3_000 } else { 500 + 250 * tier as u64 }
}

/// Daily chips grow a quarter for each tier: the multiple, in hundredths.
pub fn daily_x100(tier: usize) -> u64 {
    100 + 25 * tier.min(TIERS.len() - 1) as u64
}

/// The daily chips: 2,000, 500 more for each day in a row (up to a week), times the tier's
/// multiple. (Broke: at least enough to have `BROKE_FLOOR`.)
pub fn daily(streak: u32, tier: usize) -> u64 {
    (2_000 + 500 * streak.clamp(1, 7) as u64 - 500) * daily_x100(tier) / 100
}

/// The daily chips top a broke player up to this much at least.
pub const BROKE_FLOOR: u64 = 5_000;

/// The weekly VIP bonus: this share (percent) of the week's Ember Back, from Ember up.
pub const WEEKLY_PERCENT: u64 = 20;

/// Weeks as the VIP Club counts them (from Mondays: days since 1970 + 3, over 7).
pub fn week(day: u64) -> u64 {
    (day + 3) / 7
}

/// Full XP from play until the day's reaches this; half after.
pub const DAY_FULL_XP: u64 = 4_000;

/// What a round or a hand is, for the experience it earns and the house's edge on it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Play {
    Dice,
    Plinko,
    /// Any slot machine: Pyr Sevens, Reel Row's, Ember Dragon.
    Slots,
    Keno,
    Blackjack,
    Roulette,
    Baccarat,
    Craps,
    /// A Dominó hand (0.13.11).
    Domino,
    Poker,
}

impl Play {
    /// XP for one round at `bet`, in tenths, before a long day halves it: the game's own (so
    /// that a minute of any game earns about the same) and a little more for each power of ten
    /// bet. 0.13.28's, round by round (`measured/levels.md`).
    pub fn xp_tenths(self, bet: u64) -> u64 {
        // (In hundredths of XP: the base, and what each power of ten bet adds.)
        let (base, step) = match self {
            Play::Dice | Play::Plinko => (100, 10),
            Play::Slots => (110, 10),
            Play::Keno => (300, 30),
            Play::Blackjack => (260, 40),
            Play::Roulette => (510, 40),
            Play::Baccarat => (370, 40),
            Play::Craps => (193, 15),
            // (A hand of dominoes, at its stake: 6.8 XP at 500, as 0.13.28 gives.)
            Play::Domino => (550, 50),
            Play::Poker => return 80,
        };
        let l = (bet.max(1) as f64).log10();
        ((base as f64 + step as f64 * l) / 10.0 + 1e-9).floor() as u64
    }

    /// The house's edge Ember Back is a share of, in hundredths of a percent.
    pub fn edge_bp(self) -> u64 {
        match self {
            Play::Dice | Play::Plinko | Play::Slots => 100,
            Play::Keno => 500,
            Play::Blackjack => 50,
            Play::Roulette => 270,
            Play::Baccarat => 106,
            Play::Craps => 140,
            // (The house's 3% of a pot.)
            Play::Domino => 300,
            // (At poker, Ember Back is a share of the rake paid.)
            Play::Poker => 0,
        }
    }

    /// Ember Back from a round at `bet` at `tier`, in hundredths of a chip.
    pub fn ember_x100(self, bet: u64, tier: usize) -> u64 {
        (bet as u128 * self.edge_bp() as u128 * ember_bp(tier) as u128 / 1_000_000) as u64
    }

    pub fn is_slot(self) -> bool {
        self == Play::Slots
    }
}

/// XP from play after a long day: half once the day's passes `DAY_FULL_XP`.
pub fn day_tenths(tenths: u64, xp_today: u64) -> u64 {
    if xp_today >= DAY_FULL_XP { tenths / 2 } else { tenths }
}

/// Moves 0.13.0's experience onto this curve where it was: the same level, as far into it.
pub fn moved_from_013(xp: u64) -> u64 {
    // (0.13.0's: 250 × (level - 1)^1.45 to reach a level.)
    let old = |level: u32| -> u64 { if level <= 1 { 0 } else { (250.0 * ((level - 1) as f64).powf(1.45)).round() as u64 } };
    let mut l = 1;
    while old(l + 1) <= xp && l < 200 {
        l += 1;
    }
    if l >= MAX_LEVEL {
        return xp_for(MAX_LEVEL);
    }
    let into = (xp - old(l)) as f64 / (old(l + 1) - old(l)).max(1) as f64;
    xp_for(l) + (into * LEVEL_XP[l as usize - 1] as f64) as u64
}

/// Today's missions: one at poker, one at the tables, one at the slots and Pyr's Originals.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Corner {
    Poker,
    Tables,
    Slots,
}

impl Corner {
    /// As the home page heads its card.
    pub fn heading(self) -> &'static str {
        match self {
            Corner::Poker => "TODAY · POKER",
            Corner::Tables => "TODAY · TABLES",
            Corner::Slots => "TODAY · SLOTS & ORIGINALS",
        }
    }
}

/// What counts toward a mission.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Counts {
    PokerHands,
    PokerPots,
    Showdowns,
    Plays(Play),
    /// A slot round that paid 10 times its bet or more.
    SlotTenX,
    /// Any casino round that paid back more than its bet.
    CasinoWins,
}

/// A mission: what it asks, how many, and what it pays.
pub struct Mission {
    pub corner: Corner,
    pub what: &'static str,
    pub goal: u32,
    pub counts: Counts,
    pub xp: u64,
    pub chips: u64,
}

/// Every mission there is (its place here is its id, as 0.13.28's records keep it).
pub const MISSIONS: [Mission; 12] = [
    Mission { corner: Corner::Poker, what: "Play 20 hands of poker", goal: 20, counts: Counts::PokerHands, xp: 200, chips: 5_000 },
    Mission { corner: Corner::Poker, what: "Win 5 pots at poker", goal: 5, counts: Counts::PokerPots, xp: 250, chips: 6_000 },
    Mission { corner: Corner::Poker, what: "Win 2 showdowns", goal: 2, counts: Counts::Showdowns, xp: 250, chips: 6_000 },
    Mission { corner: Corner::Tables, what: "Play 25 hands of blackjack", goal: 25, counts: Counts::Plays(Play::Blackjack), xp: 200, chips: 5_000 },
    Mission { corner: Corner::Tables, what: "Spin roulette 20 times", goal: 20, counts: Counts::Plays(Play::Roulette), xp: 200, chips: 5_000 },
    Mission { corner: Corner::Tables, what: "Play 15 coups of baccarat", goal: 15, counts: Counts::Plays(Play::Baccarat), xp: 200, chips: 5_000 },
    Mission { corner: Corner::Tables, what: "Roll 20 times at craps", goal: 20, counts: Counts::Plays(Play::Craps), xp: 200, chips: 5_000 },
    Mission { corner: Corner::Slots, what: "Spin any slot 50 times", goal: 50, counts: Counts::Plays(Play::Slots), xp: 200, chips: 5_000 },
    Mission { corner: Corner::Slots, what: "Land a 10× win on a slot", goal: 1, counts: Counts::SlotTenX, xp: 300, chips: 8_000 },
    Mission { corner: Corner::Slots, what: "Play 30 keno draws", goal: 30, counts: Counts::Plays(Play::Keno), xp: 200, chips: 5_000 },
    Mission { corner: Corner::Slots, what: "Roll the dice 40 times", goal: 40, counts: Counts::Plays(Play::Dice), xp: 150, chips: 4_000 },
    Mission { corner: Corner::Slots, what: "Win 10 rounds in the casino", goal: 10, counts: Counts::CasinoWins, xp: 250, chips: 6_000 },
];

/// All three done: the day's chest.
pub const MISSIONS_CHEST_CHIPS: u64 = 25_000;
pub const MISSIONS_CHEST_XP: u64 = 500;

/// The day's three missions (one from each corner): the same for everyone that day, as
/// 0.13.28 had them.
pub fn missions_for(day: u64) -> [u8; 3] {
    let pick = |corner: Corner, n: u64| -> u8 {
        let ids: Vec<u8> = MISSIONS.iter().enumerate().filter(|(_, m)| m.corner == corner).map(|(i, _)| i as u8).collect();
        let mut h = day.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ n.wrapping_mul(0x94d0_49bb_1331_11eb);
        h ^= h >> 31;
        h = h.wrapping_mul(0xd6e8_feb8_6659_fd93);
        h ^= h >> 32;
        ids[(h % ids.len() as u64) as usize]
    };
    [pick(Corner::Poker, 1), pick(Corner::Tables, 2), pick(Corner::Slots, 3)]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 0.13.28's own numbers, as its screen said them.
    #[test]
    fn the_curve_is_0_13_28s() {
        assert_eq!((xp_for(2), xp_for(3), xp_for(6), xp_for(9), xp_for(18)), (80, 170, 520, 1080, 4750));
        assert_eq!((xp_for(10), xp_for(20), xp_for(30), xp_for(40), xp_for(50)), (1_330, 6_080, 16_310, 33_690, 59_650));
        assert_eq!((xp_for(36), xp_for(48), xp_for(62), xp_for(80), xp_for(100)), (25_790, 53_710, 103_910, 201_230, 359_560));
        assert_eq!((level_for(0), level_for(79), level_for(80), level_for(500), level_for(359_559), level_for(9_000_000)), (1, 1, 2, 5, 99, 100));
        assert_eq!((stars(359_560), stars(450_000), stars(1_000_000)), (0, 9, 64));
        assert_eq!((to_next(0), to_next(81), to_next(500), to_next(730), to_next(1_330)), (80, 89, 20, 140, 280));
        assert_eq!(span(730), (680, 870));
        assert_eq!((title(1), title(7), title(10), title(46), title(93), title(100)), ("Spark I".into(), "Spark VII".into(), "Ember I".into(), "Bonfire VII".into(), "Starfire XIV".into(), "Eternal Flame".into()));
        assert_eq!((pays(2), pays(5), pays(6), pays(8), pays(10), pays(11), pays(15)), (1_800, 9_000, 3_400, 4_200, 115_000, 5_400, 21_000));
        assert_eq!((pays(70), pays(80), pays(94), pays(100)), (187_000, 199_000, 38_600, 223_000));
        assert_eq!((ember_bp(0), ember_bp(1), ember_bp(8), ember_bp(9)), (500, 750, 2_500, 3_000));
        assert_eq!((tier_levels(0), tier_levels(1), tier_levels(8), tier_levels(9)), ((1, 9), (10, 19), (80, 99), (100, 100)));
    }

    #[test]
    fn rounds_earn_as_0_13_28s_did() {
        let t = |p: Play, bets: &[u64]| bets.iter().map(|b| p.xp_tenths(*b)).collect::<Vec<_>>();
        assert_eq!(t(Play::Dice, &[1, 10, 10_240]), [10, 11, 14]);
        assert_eq!(t(Play::Plinko, &[100]), [12]);
        assert_eq!(t(Play::Slots, &[20, 100]), [12, 13]);
        assert_eq!(t(Play::Keno, &[10, 10_240]), [33, 42]);
        assert_eq!(t(Play::Blackjack, &[10, 25, 100, 500, 1_000]), [30, 31, 34, 36, 38]);
        assert_eq!(t(Play::Roulette, &[15, 1_000]), [55, 63]);
        assert_eq!(t(Play::Baccarat, &[15, 1_000]), [41, 49]);
        assert_eq!(t(Play::Craps, &[10, 15, 25, 100, 1_000]), [20, 21, 21, 22, 23]);
        assert_eq!(t(Play::Poker, &[20, 200]), [80, 80]);
        assert_eq!((day_tenths(14, 3_500), day_tenths(14, 4_000)), (14, 7));
        // Ember Back (Spark's 5%): dice 10,240 512; keno 10 2 and 10,240 2,560; blackjack 1,000
        // 25; roulette 15 2 and 1,000 135; baccarat 1,000 53; craps 15 1, 100 7, 1,000 70;
        // slots 20 1, 100 5; plinko 100 5.
        let e = |p: Play, b: u64| p.ember_x100(b, 0);
        assert_eq!((e(Play::Dice, 10_240), e(Play::Keno, 10), e(Play::Keno, 10_240)), (512, 2, 2_560));
        assert_eq!((e(Play::Blackjack, 1_000), e(Play::Roulette, 15), e(Play::Roulette, 1_000), e(Play::Baccarat, 1_000)), (25, 2, 135, 53));
        assert_eq!((e(Play::Craps, 15), e(Play::Craps, 100), e(Play::Craps, 1_000), e(Play::Slots, 20), e(Play::Slots, 100), e(Play::Plinko, 100)), (1, 7, 70, 1, 5, 5));
    }

    #[test]
    fn daily_chips_by_tier_and_streak_as_0_13_28s() {
        let by_tier: Vec<u64> = [0, 1, 2, 3, 4, 5, 8, 9].iter().map(|t| daily(1, *t)).collect();
        assert_eq!(by_tier, [2_000, 2_500, 3_000, 3_500, 4_000, 4_500, 6_000, 6_500]);
        assert_eq!((daily(4, 0), daily(7, 2), daily(9, 0)), (3_500, 7_500, 5_000));
        // (Day 20,766 was Monday, November 9, 2026: a new week; the day before, the last one.)
        assert_eq!((week(20_768), week(20_766), week(20_765), week(20_759)), (2_967, 2_967, 2_966, 2_966));
    }

    #[test]
    fn a_record_keeps_its_level_on_the_new_curve() {
        // 0.13.0's level 12 halfway: level 12 halfway here.
        let old = |l: u32| (250.0 * ((l - 1) as f64).powf(1.45)).round() as u64;
        let xp = (old(12) + old(13)) / 2;
        let moved = moved_from_013(xp);
        assert_eq!(level_for(moved), 12);
        assert!((moved - xp_for(12)) as f64 / LEVEL_XP[11] as f64 > 0.4);
        assert_eq!(level_for(moved_from_013(10_000_000)), 100);
    }

    #[test]
    fn three_missions_a_day_one_from_each_corner() {
        let mut seen = std::collections::BTreeSet::new();
        for day in 20_000..20_200 {
            let m = missions_for(day);
            assert_eq!([MISSIONS[m[0] as usize].corner, MISSIONS[m[1] as usize].corner, MISSIONS[m[2] as usize].corner], [Corner::Poker, Corner::Tables, Corner::Slots]);
            seen.extend(m);
        }
        assert_eq!(seen.len(), MISSIONS.len(), "every mission comes up");
    }
}
