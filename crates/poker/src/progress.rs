//! What a player builds up: chips (play money only: they can't be bought or cashed out),
//! experience and levels, achievements, statistics, and the looks they unlock by playing
//! (never by paying). A record is kept by the server a player plays on.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::eval::Category;
use crate::game::{Action, Event, Posting, Street};

use crate::levels::{self, Counts, Play};

/// Chips a new player starts with (100,000 since 0.13.6).
pub const STARTING_CHIPS: u64 = 100_000;
/// The daily gift at Spark on the first day of a streak (see `levels::daily`).
pub const DAILY_BASE: u64 = 2_000;

/// A casino record: the fields are 0.13.28's, in its order, so that a server moved from 0.13.28
/// onto this Pyr reads and writes everyone's as it was (and anything else it holds comes through
/// in `extra`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Record {
    pub chips: u64,
    pub xp: u64,
    pub hands: u64,
    pub hands_won: u64,
    pub showdowns: u64,
    pub showdowns_won: u64,
    /// Hands they put chips in before the flop by choice (calling or raising, not just the blinds).
    pub vpip: u64,
    /// Hands they raised before the flop.
    pub pfr: u64,
    pub biggest_pot: u64,
    /// Their best hand ever shown (eval's number), and what it's called.
    pub best_hand: u32,
    pub best_hand_name: String,
    pub achievements: BTreeSet<String>,
    /// The last day (days since 1970) the daily gift was taken, and how many days in a row.
    pub daily_day: u64,
    pub daily_streak: u32,
    /// Looks they've chosen (ids from [`COSMETICS`]).
    pub card_back: String,
    pub felt: String,
    pub chip_set: String,
    pub frame: String,
    /// Looks given to them (prize boxes, Game Masters' gifts).
    pub gifts: BTreeSet<String>,
    /// Their level's title kept off their name at tables (0.13.6).
    pub hide_title: bool,
    /// Pyr Casino, by game (its id).
    pub casino: BTreeMap<String, CasinoStats>,
    /// Poker's MMR.
    pub rating: crate::rating::Rating,
    /// The level curve `xp` is on: 2 since 0.13.5 (`levels`); before, 0.13.0's (moved over
    /// where it was when the record is read).
    pub curve: u8,
    /// Experience from play, in tenths, not yet a whole point.
    pub xp_tenths: u64,
    /// The day (days since 1970) `xp_today` counts, and the XP from play that day (half past
    /// 4,000).
    pub xp_day: u64,
    pub xp_today: u64,
    /// Ember Back waiting to be claimed, in hundredths of a chip; and all claimed so far.
    pub ember_x100: u64,
    pub ember_claimed: u64,
    /// Today's missions.
    pub missions: Missions,
    /// Ember Stars paid for.
    pub stars_paid: u64,
    /// The week's Ember Back so far (hundredths), and which week (`levels::week`): the weekly
    /// VIP bonus is a share of it.
    pub ember_week_x100: u64,
    pub ember_week: u64,
    /// The weekly VIP bonus waiting (chips), and the week it was last claimed in.
    pub weekly_ready: u64,
    pub weekly_claimed: u64,
    /// What a record holds that this Pyr doesn't know (a newer server's): kept as it is and
    /// written back with the rest, so a server moved onto this Pyr loses none of it.
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Today's three missions, as 0.13.28's records keep them (ids into `levels::MISSIONS`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Missions {
    pub day: u64,
    pub ids: Vec<u8>,
    pub progress: Vec<u32>,
    pub done: Vec<bool>,
    /// The day's chest (all three done) paid.
    pub all_paid: bool,
    #[serde(flatten, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Missions {
    /// Today's, as they stand (the day's three, new, when the day's changed).
    pub fn of(&self, today: u64) -> Missions {
        if self.day == today && self.ids.len() == 3 && self.progress.len() == 3 && self.done.len() == 3 {
            return self.clone();
        }
        Missions { day: today, ids: levels::missions_for(today).to_vec(), progress: vec![0; 3], done: vec![false; 3], all_paid: false, extra: Default::default() }
    }

    /// As the app shows them.
    pub fn view(&self) -> Vec<crate::net::MissionView> {
        self.ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| {
                let m = levels::MISSIONS.get(*id as usize)?;
                Some(crate::net::MissionView {
                    text: m.what.to_string(),
                    progress: self.progress.get(i).copied().unwrap_or(0),
                    target: m.goal,
                    done: self.done.get(i).copied().unwrap_or(false),
                    xp: m.xp,
                    chips: m.chips,
                })
            })
            .collect()
    }
}

/// How someone's done at one of the casino's games.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CasinoStats {
    pub rounds: u64,
    pub wagered: u64,
    /// Chips back (stakes of winners included).
    pub paid: u64,
    pub best_win: u64,
    /// The best round's multiple of its bet, in hundredths.
    pub best_multiple_x100: u64,
}

impl Record {
    pub fn new() -> Record {
        Record { chips: STARTING_CHIPS, curve: levels::CURVE, ..Default::default() }
    }

    /// A record as read: one from before levels (0.13.0's curve) has its experience moved onto
    /// today's where it was (the same level, as far into it).
    pub fn read(mut self) -> Record {
        if self.curve < levels::CURVE {
            self.xp = levels::moved_from_013(self.xp);
            self.curve = levels::CURVE;
        }
        self
    }

    pub fn level(&self) -> u32 {
        levels::level_for(self.xp)
    }

    pub fn tier(&self) -> usize {
        levels::tier(self.level())
    }

    /// The daily gift, if it hasn't been taken today: 2,000 at Spark, 500 more for each day in
    /// a row (up to a week), a quarter more for each tier; a broke player gets at least enough
    /// to have 5,000. Returns the chips given.
    pub fn claim_daily(&mut self, today: u64) -> Option<u64> {
        if self.daily_day == today && self.daily_streak > 0 {
            return None;
        }
        self.daily_streak = if self.daily_day + 1 == today { (self.daily_streak + 1).min(7) } else { 1 };
        self.daily_day = today;
        let mut gift = levels::daily(self.daily_streak, self.tier());
        if self.chips + gift < levels::BROKE_FLOOR {
            gift = levels::BROKE_FLOOR - self.chips;
        }
        self.chips += gift;
        Some(gift)
    }

    /// The looks they've unlocked so far.
    pub fn unlocked(&self) -> Vec<&'static Cosmetic> {
        COSMETICS.iter().filter(|c| c.unlock.met(self) || self.gifts.contains(c.id)).collect()
    }

    /// Experience from play (in tenths, before a long day halves it), counted on `today`.
    /// Returns the whole points it made.
    fn add_play(&mut self, tenths: u64, today: u64) -> u64 {
        if self.xp_day != today {
            self.xp_day = today;
            self.xp_today = 0;
        }
        self.xp_tenths += levels::day_tenths(tenths, self.xp_today);
        let whole = self.xp_tenths / 10;
        self.xp_tenths %= 10;
        self.xp_today += whole;
        whole
    }

    /// A new week (as the VIP Club counts them): last week's bonus gets ready (20% of last
    /// week's Ember Back, from Ember tier up; one not claimed by then is gone), and this week's
    /// starts from nothing.
    pub fn roll_week(&mut self, today: u64) {
        let w = levels::week(today);
        if self.ember_week != w {
            self.weekly_ready = if self.tier() >= 1 { self.ember_week_x100 * levels::WEEKLY_PERCENT / 100 / 100 } else { 0 };
            self.ember_week_x100 = 0;
            self.ember_week = w;
        }
    }

    /// Ember Back earned (hundredths of a chip).
    fn add_ember(&mut self, x100: u64, today: u64) {
        self.roll_week(today);
        self.ember_x100 += x100;
        self.ember_week_x100 += x100;
    }

    /// Ember Back claimed: the whole chips of it (hundredths stay). Returns the chips.
    pub fn claim_ember(&mut self) -> u64 {
        let c = self.ember_x100 / 100;
        self.chips += c;
        self.ember_claimed += c;
        self.ember_x100 %= 100;
        c
    }

    /// The weekly VIP bonus claimed. Returns the chips (0: none waiting).
    pub fn claim_weekly(&mut self, today: u64) -> u64 {
        self.roll_week(today);
        let c = self.weekly_ready;
        if c > 0 {
            self.chips += c;
            self.weekly_ready = 0;
            self.weekly_claimed = levels::week(today);
        }
        c
    }

    /// What this week's Ember Back has built of next week's bonus so far (chips; nothing below
    /// Ember tier).
    pub fn weekly_building(&self, today: u64) -> u64 {
        if self.tier() == 0 || self.ember_week != levels::week(today) {
            return 0;
        }
        self.ember_week_x100 * levels::WEEKLY_PERCENT / 100 / 100
    }

    /// The bonus waiting as of `today` (one from last week, ready once the week's turned).
    pub fn weekly_waiting(&self, today: u64) -> u64 {
        let mut r = self.clone();
        r.roll_week(today);
        r.weekly_ready
    }

    /// Counts a round or hand toward today's missions (`n`: how much it counts for each kind):
    /// each mission done pays its XP and chips, and all three the day's chest. Returns the XP.
    fn count_missions(&mut self, today: u64, n: impl Fn(Counts) -> u32, g: &mut Gains) -> u64 {
        self.missions = self.missions.of(today);
        let mut xp = 0;
        for i in 0..3 {
            let Some(m) = levels::MISSIONS.get(self.missions.ids[i] as usize) else { continue };
            if self.missions.done[i] {
                continue;
            }
            let k = n(m.counts);
            if k == 0 {
                continue;
            }
            self.missions.progress[i] = (self.missions.progress[i] + k).min(m.goal);
            if self.missions.progress[i] >= m.goal {
                self.missions.done[i] = true;
                xp += m.xp;
                self.chips += m.chips;
                g.chips += m.chips;
                g.missions.push(self.missions.ids[i]);
            }
        }
        if self.missions.done.iter().all(|d| *d) && !self.missions.all_paid {
            self.missions.all_paid = true;
            xp += levels::MISSIONS_CHEST_XP;
            self.chips += levels::MISSIONS_CHEST_CHIPS;
            g.chips += levels::MISSIONS_CHEST_CHIPS;
            g.all_missions = true;
        }
        xp
    }

    /// Experience added (`whole` points): every level it reaches pays (a new tier's chest
    /// too), and so does a new Ember Star past level 100.
    fn gain(&mut self, whole: u64, g: &mut Gains) {
        let before = self.xp;
        self.xp += whole;
        g.xp += whole;
        let (l0, l1) = (levels::level_for(before), levels::level_for(self.xp));
        for l in l0 + 1..=l1 {
            let p = levels::pays(l);
            self.chips += p;
            g.chips += p;
            if levels::starts_tier(l) {
                g.tier_up = Some(l);
            }
        }
        if l1 > l0 {
            g.level_up = Some(l1);
        }
        let s = levels::stars(self.xp);
        if s > self.stars_paid {
            // (One star's pay, however many one round crosses: as 0.13.28.)
            self.chips += levels::STAR_PAYS;
            g.chips += levels::STAR_PAYS;
            g.star = Some(s);
            self.stars_paid = s;
        }
    }
}

/// Experience needed to reach each level (see `levels`).
pub fn xp_for_level(level: u32) -> u64 {
    levels::xp_for(level)
}

pub fn level_for(xp: u64) -> u32 {
    levels::level_for(xp)
}

/// An achievement: its id, name and what it takes.
pub struct Achievement {
    pub id: &'static str,
    pub name: &'static str,
    pub about: &'static str,
    pub xp: u64,
}

pub const ACHIEVEMENTS: &[Achievement] = &[
    Achievement { id: "first_win", name: "First Blood", about: "Win your first pot.", xp: 100 },
    Achievement { id: "showdown_10", name: "Card Shark", about: "Win 10 showdowns.", xp: 200 },
    Achievement { id: "straight", name: "Straight Shooter", about: "Win with a straight.", xp: 100 },
    Achievement { id: "flush", name: "Flush With Cash", about: "Win with a flush.", xp: 100 },
    Achievement { id: "full_house", name: "Full House", about: "Win with a full house.", xp: 150 },
    Achievement { id: "quads", name: "Four of a Kind", about: "Win with four of a kind.", xp: 300 },
    Achievement { id: "straight_flush", name: "Straight Flush", about: "Win with a straight flush.", xp: 600 },
    Achievement { id: "royal", name: "Royalty", about: "Win with a royal flush.", xp: 1500 },
    Achievement { id: "big_pot", name: "High Roller", about: "Win a pot of 100 big blinds or more.", xp: 250 },
    Achievement { id: "bluff", name: "Poker Face", about: "Bet the river and win without showing.", xp: 200 },
    Achievement { id: "all_in_win", name: "All In", about: "Go all-in and win.", xp: 150 },
    Achievement { id: "hands_100", name: "Regular", about: "Play 100 hands.", xp: 200 },
    Achievement { id: "hands_1000", name: "Marathon", about: "Play 1,000 hands.", xp: 1000 },
    Achievement { id: "shark_hunter", name: "Shark Hunter", about: "Win a showdown against a Shark bot.", xp: 300 },
    Achievement { id: "streak_7", name: "Every Day", about: "Take the daily chips seven days in a row.", xp: 300 },
    // Pyr Casino.
    Achievement { id: "casino_first", name: "Bright Lights", about: "Play your first game in Pyr Casino.", xp: 50 },
    Achievement { id: "free_spins", name: "Rising Phoenix", about: "Start free spins on Pyr Sevens.", xp: 150 },
    Achievement { id: "jackpot", name: "Phoenix Jackpot", about: "Win the Phoenix Jackpot: five flames on a line.", xp: 2500 },
    Achievement { id: "natural", name: "Natural", about: "Get a blackjack.", xp: 100 },
    Achievement { id: "split_sweep", name: "Double Trouble", about: "Split at blackjack and win every hand.", xp: 200 },
    Achievement { id: "straight_up", name: "Straight Up", about: "Hit a single number at roulette.", xp: 200 },
    Achievement { id: "dead_heat", name: "Dead Heat", about: "Win a Tie bet at baccarat.", xp: 200 },
    Achievement { id: "hot_shooter", name: "Hot Shooter", about: "Make four points in a row at craps.", xp: 400 },
    Achievement { id: "long_shot", name: "Long Shot", about: "Win a dice roll at a 5% chance or less.", xp: 300 },
    Achievement { id: "seer", name: "Seer", about: "Catch 7 or more numbers in one keno draw.", xp: 800 },
    // (0.13.5 to 0.13.12: Plinko, and Dominó's; 0.13.28's words.)
    Achievement { id: "edge_of_the_board", name: "Edge of the Board", about: "Land a Plinko ball in an edge bucket.", xp: 600 },
    Achievement { id: "capicua", name: "¡Capicúa!", about: "Win a Dominó hand with a tile that fits both ends.", xp: 500 },
    Achievement { id: "all_the_way", name: "All the Way", about: "Win a Dominó hand doubled to 8 times the ante.", xp: 800 },
    Achievement { id: "clean_sweep", name: "Clean Sweep", about: "Find every safe tile in a Reel Row pick-and-dodge.", xp: 400 },
    Achievement { id: "upgrade", name: "Upgraded", about: "Spin the Upgrade on a Reel Row prize wheel.", xp: 300 },
    Achievement { id: "elder", name: "Dragon Tamer", about: "Raise an Ember Dragon to an Elder.", xp: 300 },
    Achievement { id: "full_board", name: "Wall of Fire", about: "Fill all fifteen spots in Ember Respins.", xp: 1_000 },
    Achievement { id: "dragon_jackpot", name: "Dragon's Gold", about: "Win a jackpot on Ember Dragon.", xp: 400 },
    Achievement { id: "big_win", name: "Big Win", about: "Win 50 times your bet or more in one round.", xp: 300 },
    Achievement { id: "whale", name: "Whale", about: "Bet 25,000 or more on one round.", xp: 200 },
    // (Reel Row Remastered's, 0.13.2.)
    Achievement { id: "chain_reaction", name: "Chain Reaction", about: "Tumble ten times in one spin.", xp: 300 },
    Achievement { id: "kaboom", name: "Kaboom", about: "Bombs adding up to ×100 or more on a win.", xp: 500 },
    Achievement { id: "grand", name: "Grand", about: "Fill every place in Hold & Win for the Grand.", xp: 2000 },
    Achievement { id: "wild_ride", name: "Wild Ride", about: "Ten respins of wilds in one spin.", xp: 400 },
];

pub fn achievement(id: &str) -> Option<&'static Achievement> {
    ACHIEVEMENTS.iter().find(|a| a.id == id)
}

/// How a look is unlocked.
pub enum Unlock {
    Always,
    Level(u32),
    Achievement(&'static str),
    /// Only from a prize box or a Game Master's gift.
    Prize,
}

impl Unlock {
    pub fn met(&self, r: &Record) -> bool {
        match self {
            Unlock::Always => true,
            Unlock::Level(l) => r.level() >= *l,
            Unlock::Achievement(a) => r.achievements.contains(*a),
            Unlock::Prize => false,
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Unlock::Always => "Yours from the start".into(),
            Unlock::Level(l) => format!("Reach level {l}"),
            Unlock::Achievement(a) => format!("Earn \u{201c}{}\u{201d}", achievement(a).map(|a| a.name).unwrap_or(a)),
            Unlock::Prize => "Only from a prize box or a Game Master".into(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    CardBack,
    Felt,
    Chips,
    Frame,
}

pub struct Cosmetic {
    pub id: &'static str,
    pub kind: Kind,
    pub name: &'static str,
    pub unlock: Unlock,
}

pub const COSMETICS: &[Cosmetic] = &[
    Cosmetic { id: "back_ember", kind: Kind::CardBack, name: "Ember", unlock: Unlock::Always },
    Cosmetic { id: "back_midnight", kind: Kind::CardBack, name: "Midnight", unlock: Unlock::Level(3) },
    Cosmetic { id: "back_jade", kind: Kind::CardBack, name: "Jade", unlock: Unlock::Level(8) },
    Cosmetic { id: "back_gold", kind: Kind::CardBack, name: "Gilded", unlock: Unlock::Achievement("royal") },
    Cosmetic { id: "felt_campfire", kind: Kind::Felt, name: "Campfire", unlock: Unlock::Always },
    Cosmetic { id: "felt_classic", kind: Kind::Felt, name: "Classic Green", unlock: Unlock::Always },
    Cosmetic { id: "felt_royal", kind: Kind::Felt, name: "Royal Blue", unlock: Unlock::Level(5) },
    Cosmetic { id: "felt_crimson", kind: Kind::Felt, name: "Crimson", unlock: Unlock::Level(12) },
    Cosmetic { id: "felt_obsidian", kind: Kind::Felt, name: "Obsidian", unlock: Unlock::Achievement("hands_1000") },
    Cosmetic { id: "chips_classic", kind: Kind::Chips, name: "Classic", unlock: Unlock::Always },
    Cosmetic { id: "chips_neon", kind: Kind::Chips, name: "Neon", unlock: Unlock::Level(10) },
    Cosmetic { id: "chips_gold", kind: Kind::Chips, name: "Gold", unlock: Unlock::Achievement("big_pot") },
    Cosmetic { id: "frame_none", kind: Kind::Frame, name: "None", unlock: Unlock::Always },
    Cosmetic { id: "frame_bronze", kind: Kind::Frame, name: "Bronze", unlock: Unlock::Level(4) },
    Cosmetic { id: "frame_silver", kind: Kind::Frame, name: "Silver", unlock: Unlock::Level(15) },
    Cosmetic { id: "frame_gold", kind: Kind::Frame, name: "Gold", unlock: Unlock::Level(30) },
    Cosmetic { id: "frame_shark", kind: Kind::Frame, name: "Shark Hunter", unlock: Unlock::Achievement("shark_hunter") },
    // Each tier's frame (0.13.5), and a felt at Blaze, Wildfire and Starfire.
    Cosmetic { id: "tier_ember", kind: Kind::Frame, name: "Ember tier", unlock: Unlock::Level(10) },
    Cosmetic { id: "tier_flame", kind: Kind::Frame, name: "Flame tier", unlock: Unlock::Level(20) },
    Cosmetic { id: "tier_blaze", kind: Kind::Frame, name: "Blaze tier", unlock: Unlock::Level(30) },
    Cosmetic { id: "tier_bonfire", kind: Kind::Frame, name: "Bonfire tier", unlock: Unlock::Level(40) },
    Cosmetic { id: "tier_wildfire", kind: Kind::Frame, name: "Wildfire tier", unlock: Unlock::Level(50) },
    Cosmetic { id: "tier_inferno", kind: Kind::Frame, name: "Inferno tier", unlock: Unlock::Level(60) },
    Cosmetic { id: "tier_sunfire", kind: Kind::Frame, name: "Sunfire tier", unlock: Unlock::Level(70) },
    Cosmetic { id: "tier_starfire", kind: Kind::Frame, name: "Starfire tier", unlock: Unlock::Level(80) },
    Cosmetic { id: "tier_eternal", kind: Kind::Frame, name: "Eternal Flame", unlock: Unlock::Level(100) },
    Cosmetic { id: "felt_blaze", kind: Kind::Felt, name: "Blaze", unlock: Unlock::Level(30) },
    Cosmetic { id: "felt_wildfire", kind: Kind::Felt, name: "Wildfire", unlock: Unlock::Level(50) },
    Cosmetic { id: "felt_starfire", kind: Kind::Felt, name: "Starfire", unlock: Unlock::Level(80) },
    // Prize boxes' and Game Masters' own.
    Cosmetic { id: "back_inferno", kind: Kind::CardBack, name: "Inferno", unlock: Unlock::Prize },
    Cosmetic { id: "back_aurora", kind: Kind::CardBack, name: "Aurora", unlock: Unlock::Prize },
    Cosmetic { id: "felt_amethyst", kind: Kind::Felt, name: "Amethyst", unlock: Unlock::Prize },
    Cosmetic { id: "felt_blackgold", kind: Kind::Felt, name: "Black & Gold", unlock: Unlock::Prize },
    Cosmetic { id: "frame_ember", kind: Kind::Frame, name: "Ember Crown", unlock: Unlock::Prize },
    Cosmetic { id: "frame_diamond", kind: Kind::Frame, name: "Diamond", unlock: Unlock::Prize },
];

/// A look by its id.
pub fn cosmetic(id: &str) -> Option<&'static Cosmetic> {
    COSMETICS.iter().find(|c| c.id == id)
}

/// What happened to one seat in a hand, worked out from the hand's events.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HandResult {
    pub played: bool,
    pub vpip: bool,
    pub pfr: bool,
    pub won: u64,
    pub put_in: u64,
    pub showdown: bool,
    /// Their hand at the showdown (eval number and category), if shown.
    pub shown: Option<(u32, Category, String)>,
    pub went_all_in: bool,
    /// They bet or raised the river and won with nobody calling.
    pub river_bluff: bool,
    /// Their share of the hand's rake (as much of it as of the chips won): Ember Back is a
    /// share of the rake you pay.
    pub rake_paid: u64,
}

/// Reads a hand's events for one seat. `values` gives the eval number and category of each
/// hand shown (the server works them out; see [`crate::game::Table`]).
pub fn read_hand(events: &[Event], seat: usize, values: &dyn Fn(usize) -> Option<(u32, Category, String)>) -> HandResult {
    let mut r = HandResult::default();
    let mut street = Street::Preflop;
    let mut river_aggressor = None;
    let mut anyone_shown = false;
    let mut folded = false;
    let (mut all_won, mut rake) = (0u64, 0u64);
    for e in events {
        match e {
            Event::HandEnded { rake: r, .. } => rake += r,
            Event::HandStarted { seats, .. } => r.played = seats.contains(&seat),
            Event::Posted { seat: s, what, amount } if *s == seat => {
                r.put_in += amount;
                let _ = matches!(what, Posting::Ante);
            }
            Event::Street { street: st, .. } => street = *st,
            Event::Acted { seat: s, action, amount, stack } => {
                if street == Street::River && matches!(action, Action::Raise { .. }) {
                    river_aggressor = Some(*s);
                }
                if *s == seat {
                    folded |= matches!(action, Action::Fold);
                    r.put_in += amount;
                    if *stack == 0 && *amount > 0 {
                        r.went_all_in = true;
                    }
                    if street == Street::Preflop {
                        match action {
                            Action::Call => r.vpip = true,
                            Action::Raise { .. } | Action::AllIn => {
                                r.vpip = true;
                                r.pfr = true;
                            }
                            _ => {}
                        }
                    }
                }
            }
            Event::Returned { seat: s, amount } if *s == seat => {
                r.put_in = r.put_in.saturating_sub(*amount);
            }
            Event::Shown { seat: s, .. } => {
                anyone_shown = true;
                if *s == seat {
                    r.showdown = true;
                    r.shown = values(seat);
                }
            }
            Event::Won { seats, amounts, .. } => {
                all_won += amounts.iter().sum::<u64>();
                if let Some(i) = seats.iter().position(|s| *s == seat) {
                    r.won += amounts[i];
                }
            }
            _ => {}
        }
    }
    if all_won > 0 {
        r.rake_paid = (rake as u128 * r.won as u128 / all_won as u128) as u64;
    }
    r.river_bluff = r.won > 0 && !anyone_shown && river_aggressor == Some(seat);
    // Held to the river without showing (everyone else folded): the hand they made still
    // counts as theirs (best hand, the made-a-straight kind of achievement).
    if r.shown.is_none() && r.played && !folded && street == Street::River {
        r.shown = values(seat);
    }
    r
}

/// A tournament finished: experience for how far they got (more for bigger fields), and
/// the wins counted.
pub fn apply_placed(rec: &mut Record, place: usize, entrants: usize) -> Gains {
    let entrants = entrants.max(2) as u64;
    let place = (place.max(1) as u64).min(entrants);
    let beaten = entrants - place;
    let xp = 50 + beaten * 400 / (entrants - 1) * (entrants.min(50)) / 10 + if place == 1 { 250 } else { 0 };
    let mut g = Gains::default();
    rec.gain(xp, &mut g);
    g
}

/// What kind of round a casino outcome is, for its experience and Ember Back.
pub fn play_of(o: &crate::casino::Outcome) -> Play {
    use crate::casino::Outcome;
    match o {
        Outcome::Slots(_) | Outcome::Reels(_) | Outcome::Row(_) | Outcome::Dragon(_) => Play::Slots,
        Outcome::Blackjack(_) => Play::Blackjack,
        Outcome::Roulette(_) => Play::Roulette,
        Outcome::Baccarat(_) => Play::Baccarat,
        Outcome::Craps(_) => Play::Craps,
        Outcome::Dice(_) => Play::Dice,
        Outcome::Keno(_) => Play::Keno,
        Outcome::Plinko(_) => Play::Plinko,
    }
}

/// Counts a casino round in its game's numbers (the house's record, and the app's copy of it,
/// the same way).
pub fn count_casino(st: &mut CasinoStats, o: &crate::casino::Outcome) {
    let (wagered, back) = casino_back(o);
    st.rounds += 1;
    st.wagered += wagered;
    st.paid += back;
    st.best_win = st.best_win.max(back.saturating_sub(wagered));
    if back > wagered {
        st.best_multiple_x100 = st.best_multiple_x100.max(crate::casino::multiple_x100(back, wagered));
    }
}

/// A round's stake and the chips back from it (at craps, bets left riding count as back).
fn casino_back(o: &crate::casino::Outcome) -> (u64, u64) {
    let back = match o {
        crate::casino::Outcome::Craps(c) => c.paid + c.kept,
        _ => o.paid(),
    };
    (o.wagered(), back)
}

/// Adds a casino round to a record: its game's numbers; experience from play (the game's own
/// for a round, a little more for a bigger bet, half after a long day) and from any
/// achievement it earned; Ember Back (the tier's share of the house's edge on the bet); today's
/// missions; and the chips the levels, a new tier, the missions or a star pay.
pub fn apply_casino(rec: &mut Record, o: &crate::casino::Outcome, today: u64) -> Gains {
    use crate::casino::{Outcome, blackjack};
    let mut g = Gains::default();
    let (wagered, back) = casino_back(o);
    count_casino(rec.casino.entry(o.stats_key()).or_default(), o);
    let mult = crate::casino::multiple_x100(back, wagered);
    // (Achievements' experience gathers in `g.xp` first.)
    let earn = |id: &'static str, rec: &mut Record, g: &mut Gains| {
        if rec.achievements.insert(id.to_string()) {
            g.new_achievements.push(id);
            g.xp += achievement(id).map(|a| a.xp).unwrap_or(0);
        }
    };
    earn("casino_first", rec, &mut g);
    if wagered >= 25_000 {
        earn("whale", rec, &mut g);
    }
    if wagered > 0 && mult >= 5_000 {
        earn("big_win", rec, &mut g);
    }
    match o {
        Outcome::Slots(s) => {
            if s.spins.len() > 1 {
                earn("free_spins", rec, &mut g);
            }
            if s.jackpot_won > 0 {
                earn("jackpot", rec, &mut g);
            }
        }
        Outcome::Blackjack(b) => {
            if b.hands.iter().any(|h| h.result == Some(blackjack::Result::Blackjack)) {
                earn("natural", rec, &mut g);
            }
            if b.hands.len() > 1 && b.hands.iter().all(|h| h.result == Some(blackjack::Result::Win)) {
                earn("split_sweep", rec, &mut g);
            }
        }
        Outcome::Roulette(r) => {
            if r.chips.iter().zip(&r.returns).any(|(c, back)| *back > 0 && matches!(c.spot, crate::casino::roulette::Spot::Straight { .. })) {
                earn("straight_up", rec, &mut g);
            }
        }
        Outcome::Baccarat(b) => {
            if b.winner == crate::casino::baccarat::Side::Tie && b.bets.tie > 0 {
                earn("dead_heat", rec, &mut g);
            }
        }
        Outcome::Craps(c) => {
            if c.streak >= 4 {
                earn("hot_shooter", rec, &mut g);
            }
        }
        Outcome::Dice(d) => {
            if d.won && d.chance <= 500 {
                earn("long_shot", rec, &mut g);
            }
        }
        Outcome::Keno(k) => {
            if k.hits.len() >= 7 {
                earn("seer", rec, &mut g);
            }
        }
        Outcome::Plinko(p) => {
            if p.bucket == 0 || p.bucket == p.rows {
                earn("edge_of_the_board", rec, &mut g);
            }
        }
        Outcome::Reels(r) => match &r.bonus {
            Some(crate::casino::reels::Bonus::Pick { finale: true, .. }) => earn("clean_sweep", rec, &mut g),
            Some(crate::casino::reels::Bonus::Wheel { inner: Some(_), .. }) => earn("upgrade", rec, &mut g),
            _ => {}
        },
        Outcome::Row(r) => {
            use crate::casino::reelrow::{Mini, Tier};
            // (A mini game played to its very end: the Pepper X, the chest, the mask, the
            // finale, a tower standing complete.)
            let top = r.bonus.as_ref().is_some_and(|b| match &b.game {
                Mini::Ladder { top, .. } => *top,
                Mini::Dive { chest, .. } => *chest,
                Mini::Tomb { mask, .. } => *mask,
                Mini::Mine { finale, .. } | Mini::Cups { finale, .. } => *finale,
                Mini::Tower { tiers, .. } => tiers.len() == 6 && tiers.iter().all(|t| matches!(t, Tier::Poured { .. })),
                _ => false,
            });
            if top {
                earn("clean_sweep", rec, &mut g);
            }
        }
        Outcome::Dragon(d) => {
            use crate::casino::dragon;
            if dragon::age(d.growth_after) == dragon::AGES.len() - 1 || dragon::age(d.growth) == dragon::AGES.len() - 1 {
                earn("elder", rec, &mut g);
            }
            if d.respins.as_ref().is_some_and(|r| r.full) {
                earn("full_board", rec, &mut g);
            }
            if !d.jackpots().is_empty() {
                earn("dragon_jackpot", rec, &mut g);
            }
        }
    }
    let achieved = std::mem::take(&mut g.xp);
    let (mut played, mut missions) = (0, 0);
    if wagered > 0 {
        let play = play_of(o);
        played = rec.add_play(play.xp_tenths(wagered), today);
        rec.add_ember(play.ember_x100(wagered, rec.tier()), today);
        let won = back > wagered;
        missions = rec.count_missions(
            today,
            |c| match c {
                Counts::Plays(p) => (p == play) as u32,
                Counts::SlotTenX => (play == Play::Slots && mult >= 1_000) as u32,
                Counts::CasinoWins => won as u32,
                _ => 0,
            },
            &mut g,
        );
    }
    rec.gain(played + achieved + missions, &mut g);
    g
}

/// What a hand or round added: experience, achievements newly earned, the level reached, and
/// what came with them (0.13.5): the chips paid by levels, a new tier's chest, missions, the
/// day's chest and Ember Stars.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gains {
    pub xp: u64,
    pub new_achievements: Vec<&'static str>,
    pub level_up: Option<u32>,
    /// Chips paid besides the round's own winnings.
    pub chips: u64,
    /// A new tier reached: the level it starts at (10, Ember, to 100, Eternal Flame).
    pub tier_up: Option<u32>,
    /// Missions done (their ids, `levels::MISSIONS`).
    pub missions: Vec<u8>,
    /// All three of today's missions done: the day's chest paid.
    pub all_missions: bool,
    /// A new Ember Star: how many now.
    pub star: Option<u64>,
}

impl Gains {
    /// As the app is told (0.13.28's way): `net`, the chips won or lost in the round itself.
    pub fn view(&self, net: i64) -> crate::net::GainsView {
        crate::net::GainsView {
            xp: self.xp,
            achievements: self.new_achievements.iter().map(|s| s.to_string()).collect(),
            level_up: self.level_up,
            net,
            chips: self.chips,
            tier_up: self.tier_up,
            missions: self.missions.iter().filter_map(|id| levels::MISSIONS.get(*id as usize)).map(|m| m.what.to_string()).collect(),
            all_missions: self.all_missions,
            star: self.star,
        }
    }

    /// Whether there's anything to celebrate.
    pub fn any(&self) -> bool {
        !self.new_achievements.is_empty() || self.level_up.is_some() || self.chips > 0 || !self.missions.is_empty() || self.all_missions || self.star.is_some()
    }
}

/// Adds a hand to a record. `big_blind` sizes "big pot"; `beat_shark` is true when they won a
/// showdown with a Shark bot in it. A hand earns 8 XP however it went (0.13.5: about what a
/// minute at any game earns), Ember Back from the rake they paid, and counts toward today's
/// missions.
pub fn apply(rec: &mut Record, h: &HandResult, big_blind: u64, beat_shark: bool, today: u64) -> Gains {
    let mut g = Gains::default();
    if !h.played {
        return g;
    }
    rec.hands += 1;
    if h.vpip {
        rec.vpip += 1;
    }
    if h.pfr {
        rec.pfr += 1;
    }
    let net_won = h.won > h.put_in;
    if h.won > 0 {
        rec.hands_won += 1;
        rec.biggest_pot = rec.biggest_pot.max(h.won);
    }
    if h.showdown {
        rec.showdowns += 1;
        if h.won > 0 {
            rec.showdowns_won += 1;
        }
    }
    if let Some((v, _, name)) = &h.shown
        && *v > rec.best_hand
    {
        rec.best_hand = *v;
        rec.best_hand_name = name.clone();
    }
    let earn = |id: &'static str, rec: &mut Record, g: &mut Gains| {
        if rec.achievements.insert(id.to_string()) {
            g.new_achievements.push(id);
            g.xp += achievement(id).map(|a| a.xp).unwrap_or(0);
        }
    };
    if h.won > 0 {
        earn("first_win", rec, &mut g);
    }
    if rec.showdowns_won >= 10 {
        earn("showdown_10", rec, &mut g);
    }
    if h.won > 0
        && let Some((v, cat, _)) = &h.shown
    {
        let id = match cat {
            Category::Straight => Some("straight"),
            Category::Flush => Some("flush"),
            Category::FullHouse => Some("full_house"),
            Category::Quads => Some("quads"),
            Category::StraightFlush if (*v >> 16) & 0xf == 12 => Some("royal"),
            Category::StraightFlush => Some("straight_flush"),
            _ => None,
        };
        if let Some(id) = id {
            earn(id, rec, &mut g);
        }
    }
    if net_won && h.won >= 100 * big_blind {
        earn("big_pot", rec, &mut g);
    }
    if h.river_bluff {
        earn("bluff", rec, &mut g);
    }
    if h.went_all_in && net_won {
        earn("all_in_win", rec, &mut g);
    }
    if rec.hands >= 100 {
        earn("hands_100", rec, &mut g);
    }
    if rec.hands >= 1000 {
        earn("hands_1000", rec, &mut g);
    }
    if beat_shark && h.showdown && h.won > 0 {
        earn("shark_hunter", rec, &mut g);
    }
    let achieved = std::mem::take(&mut g.xp);
    let played = rec.add_play(Play::Poker.xp_tenths(big_blind), today);
    if h.rake_paid > 0 {
        rec.add_ember(h.rake_paid * levels::ember_bp(rec.tier()) / 100, today);
    }
    let showdown_won = h.showdown && h.won > 0;
    let missions = rec.count_missions(
        today,
        |c| match c {
            Counts::PokerHands => 1,
            Counts::PokerPots => (h.won > 0) as u32,
            Counts::Showdowns => showdown_won as u32,
            _ => 0,
        },
        &mut g,
    );
    rec.gain(played + achieved + missions, &mut g);
    g
}

/// The daily streak achievement (checked when the gift is taken).
pub fn after_daily(rec: &mut Record) -> Gains {
    let mut g = Gains::default();
    if rec.daily_streak >= 7 && rec.achievements.insert("streak_7".into()) {
        g.new_achievements.push("streak_7");
        rec.gain(300, &mut g);
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::parse_cards;
    use crate::eval;
    use crate::game::{Rules, Table};

    /// A record as a 0.13.28 server wrote it (a real one, from 0.13.28 run here) reads, and
    /// writes back the same, with whatever a newer Pyr added kept as it was.
    #[test]
    fn a_0_13_28_record_reads_and_writes_back_as_it_was() {
        let written = r#"{"chips":79890,"xp":738,"hands":4,"hands_won":0,"showdowns":0,"showdowns_won":0,"vpip":0,"pfr":0,"biggest_pot":0,"best_hand":0,"best_hand_name":"","achievements":["casino_first","streak_7"],"daily_day":20768,"daily_streak":1,"card_back":"","felt":"","chip_set":"","frame":"","gifts":[],"hide_title":false,"casino":{"dice":{"rounds":71,"wagered":542828,"paid":506937,"best_win":10035,"best_multiple_x100":197},"plinko":{"rounds":1,"wagered":100,"paid":400,"best_win":300,"best_multiple_x100":400}},"rating":{"mmr":999.6971507379536,"peak":1000.0,"tourneys":0,"hands":8},"curve":2,"xp_tenths":0,"xp_day":20768,"xp_today":42,"ember_x100":0,"ember_claimed":123,"missions":{"day":20768,"ids":[0,3,10],"progress":[20,25,40],"done":[true,true,true],"all_paid":true},"stars_paid":0,"ember_week_x100":2960,"ember_week":2967,"weekly_ready":200,"weekly_claimed":0,"from_later":{"a":1}}"#;
        let rec: Record = serde_json::from_str(written).unwrap();
        let rec = rec.read();
        assert_eq!((rec.chips, rec.xp, rec.level(), rec.weekly_ready, rec.ember_claimed), (79_890, 738, 7, 200, 123));
        assert_eq!(rec.missions.ids, [0, 3, 10]);
        let back: serde_json::Value = serde_json::to_value(&rec).unwrap();
        assert_eq!(back, serde_json::from_str::<serde_json::Value>(written).unwrap());
        // A record of this Pyr's own has nothing extra.
        assert!(!serde_json::to_string(&Record::new()).unwrap().contains("extra"));
    }

    /// A record from before levels (0.13.0's curve) keeps its level when it's read.
    #[test]
    fn a_record_from_before_levels_keeps_its_level() {
        let old: Record = serde_json::from_str(r#"{"chips":5000,"xp":30000}"#).unwrap();
        // (0.13.0's level 18: 250 × 17^1.45 = 15,155 to 250 × 18^1.45 = 16,497... 30,000 is 0.13.0's level 28.)
        let lvl_013 = (1..200).take_while(|l| (250.0 * (*l as f64).powf(1.45)).round() as u64 <= 30_000).count() as u32 + 1;
        let rec = old.read();
        assert_eq!((rec.curve, rec.level()), (levels::CURVE, lvl_013));
        assert_eq!(Record::new().curve, levels::CURVE);
    }

    #[test]
    fn daily_chips_grow_with_a_streak_and_a_tier_and_rescue_the_broke() {
        let mut r = Record::new();
        assert_eq!(r.claim_daily(100), Some(2000));
        assert_eq!(r.claim_daily(100), None, "once a day");
        assert_eq!(r.claim_daily(101), Some(2500));
        assert_eq!(r.claim_daily(103), Some(2000), "a missed day starts over");
        for d in 104..112 {
            r.claim_daily(d);
        }
        assert_eq!(r.daily_streak, 7);
        assert_eq!(after_daily(&mut r).new_achievements, vec!["streak_7"]);
        let mut broke = Record { chips: 0, ..Record::new() };
        assert_eq!(broke.claim_daily(5), Some(5_000));
        let mut flame = Record { chips: 0, xp: levels::xp_for(20), daily_day: 5, daily_streak: 6, ..Record::new() };
        assert_eq!(flame.claim_daily(6), Some(7_500));
    }

    /// 0.13.28's dice round by round: experience in tenths, Ember Back, the long day's half,
    /// and what reaching levels, a tier, missions, the day's chest and a star pay.
    #[test]
    fn rounds_earn_and_pay_as_0_13_28s() {
        use crate::casino::{Outcome, dice};
        let roll = |chips: u64, won: bool| Outcome::Dice(dice::Outcome { chips, chance: 5_000, over: false, roll: if won { 100 } else { 9_000 }, won, paid: if won { chips * 2 - chips / 50 } else { 0 } });
        let day = 20_768;
        let mut rec = Record { xp: 730, ..Record::new() };
        rec.achievements.insert("casino_first".into());
        let g = apply_casino(&mut rec, &roll(10_240, false), day);
        assert_eq!((rec.xp, rec.xp_tenths, rec.ember_x100, rec.ember_week_x100), (731, 4, 512, 512));
        assert_eq!(g.xp, 1);
        // A long day: half.
        rec.xp_today = 4_000;
        apply_casino(&mut rec, &roll(10_240, false), day);
        assert_eq!((rec.xp, rec.xp_tenths), (732, 1));
        // Ember Back claimed: the whole chips (hundredths stay).
        let chips = rec.chips;
        rec.ember_x100 = 12_345;
        assert_eq!(rec.claim_ember(), 123);
        assert_eq!((rec.chips - chips, rec.ember_x100, rec.ember_claimed), (123, 45, 123));
        // Levels: one round from 79 XP through a mission and the day's chest pays every level
        // reached (2 to 7: 22,800), the mission's 4,000 and the chest's 25,000.
        let mut rec = Record { xp: 79, ..Record::new() };
        rec.achievements.insert("casino_first".into());
        rec.missions = Missions { day: 20_768, ids: vec![0, 3, 10], progress: vec![20, 25, 39], done: vec![true, true, false], all_paid: false, ..Default::default() };
        let chips = rec.chips;
        let g = apply_casino(&mut rec, &roll(1, false), day);
        assert_eq!(rec.xp, 730);
        assert_eq!(rec.chips - chips, 22_800 + 4_000 + 25_000);
        assert_eq!((g.level_up, g.missions.clone(), g.all_missions, g.chips), (Some(7), vec![10u8], true, 51_800));
        // A new tier: its chest.
        let mut rec = Record { xp: levels::xp_for(10) - 1, ..Record::new() };
        rec.achievements.insert("casino_first".into());
        let g = apply_casino(&mut rec, &roll(1, false), day);
        assert_eq!((g.level_up, g.tier_up, g.chips), (Some(10), Some(10), 115_000));
        // Stars: one pays 25,000; two crossed at once still 25,000 (as 0.13.28).
        let mut rec = Record { xp: 399_559, stars_paid: 3, ..Record::new() };
        rec.achievements.insert("casino_first".into());
        let g = apply_casino(&mut rec, &roll(1, false), day);
        assert_eq!((g.star, g.chips, rec.stars_paid), (Some(4), 25_000, 4));
        rec.xp = 419_559;
        let g = apply_casino(&mut rec, &roll(1, false), day);
        assert_eq!((g.star, g.chips, rec.stars_paid), (Some(6), 25_000, 6));
    }

    /// Plinko counts like every other round, and a ball in an edge bucket is Edge of the Board.
    #[test]
    fn a_plinko_ball_counts_and_the_edge_is_an_achievement() {
        use crate::casino::{Outcome, plinko};
        let ball = |bucket: u8| {
            let m = plinko::table(plinko::Risk::Low, 8)[bucket as usize];
            Outcome::Plinko(plinko::Outcome { chips: 100, risk: plinko::Risk::Low, rows: 8, path: (0..8).map(|i| i < bucket).collect(), bucket, multiplier_x100: m, paid: m })
        };
        let mut rec = Record::new();
        let g = apply_casino(&mut rec, &ball(4), 20_768);
        assert!(!g.new_achievements.contains(&"edge_of_the_board"));
        assert_eq!((rec.xp_tenths + rec.xp * 10) as i64, 500 + 12, "Bright Lights, and a ball at 100 (1.2 XP)");
        let g = apply_casino(&mut rec, &ball(8), 20_768);
        assert!(g.new_achievements.contains(&"edge_of_the_board"));
        assert_eq!(rec.casino.get("plinko").map(|c| c.rounds), Some(2));
    }

    /// The weekly VIP bonus: 20% of last week's Ember Back once the week turns, from Ember tier
    /// up; one not claimed is gone the next week.
    #[test]
    fn the_weekly_bonus_as_0_13_28s() {
        let mut rec = Record { xp: levels::xp_for(10), ember_week: 2_966, ember_week_x100: 100_000, weekly_ready: 500, ..Record::new() };
        assert_eq!(rec.weekly_waiting(20_768), 200);
        rec.roll_week(20_768);
        assert_eq!((rec.weekly_ready, rec.ember_week_x100, rec.ember_week), (200, 0, 2_967));
        let chips = rec.chips;
        assert_eq!(rec.claim_weekly(20_768), 200);
        assert_eq!((rec.chips - chips, rec.weekly_ready, rec.weekly_claimed), (200, 0, 2_967));
        assert_eq!(rec.claim_weekly(20_768), 0);
        // Spark: none.
        let mut spark = Record { ember_week: 2_966, ember_week_x100: 100_000, ..Record::new() };
        assert_eq!(spark.claim_weekly(20_768), 0);
    }

    #[test]
    fn a_hand_counts_and_earns() {
        // Heads-up: seat 0 (button) raises, seat 1 calls; seat 0 makes a flush.
        let mut t = Table::new(Rules::holdem(5, 10, 2));
        t.sit(0, "a", "A", 1000).unwrap();
        t.sit(1, "b", "B", 1000).unwrap();
        let mut deck = parse_cards("AhKc QhQd").unwrap();
        // Dealt: a1 b1 a2 b2 → a = Ah Qh, b = Kc Qd.
        deck = vec![deck[0], deck[1], deck[2], deck[3]];
        let board = parse_cards("2s 9h 4h Jc 7h").unwrap();
        let mut rest: Vec<_> = crate::cards::Card::all().filter(|c| !deck.contains(c) && !board.contains(c)).collect();
        deck.push(rest.remove(0));
        deck.extend(&board[..3]);
        deck.push(rest.remove(0));
        deck.push(board[3]);
        deck.push(rest.remove(0));
        deck.push(board[4]);
        deck.extend(rest);
        let mut events: Vec<Event> = t.start_hand(1, deck).unwrap().into_iter().map(|(_, e)| e).collect();
        events.extend(t.act(0, Action::Raise { to: 30 }).unwrap().into_iter().map(|(_, e)| e));
        events.extend(t.act(1, Action::Call).unwrap().into_iter().map(|(_, e)| e));
        for _ in 0..3 {
            events.extend(t.act(1, Action::Check).unwrap().into_iter().map(|(_, e)| e));
            events.extend(t.act(0, Action::Check).unwrap().into_iter().map(|(_, e)| e));
        }
        let board = parse_cards("2s 9h 4h Jc 7h").unwrap();
        let holes = [parse_cards("AhQh").unwrap(), parse_cards("KcQd").unwrap()];
        let values = |s: usize| {
            let v = eval::evaluate(&[holes[s].clone(), board.clone()].concat());
            Some((v.0, v.category(), v.describe()))
        };
        let h = read_hand(&events, 0, &values);
        assert!(h.played && h.vpip && h.pfr && h.showdown);
        assert_eq!((h.won, h.put_in), (60, 30));
        let mut rec = Record::new();
        let g = apply(&mut rec, &h, 10, false, 20_768);
        assert!(g.new_achievements.contains(&"first_win") && g.new_achievements.contains(&"flush"), "{g:?}");
        assert_eq!(rec.best_hand_name, "Flush, Ace high");
        let other = read_hand(&events, 1, &values);
        assert!(other.vpip && !other.pfr && other.won == 0);
        // Unlocks follow levels and achievements.
        rec.xp = xp_for_level(5);
        let ids: Vec<&str> = rec.unlocked().iter().map(|c| c.id).collect();
        assert!(ids.contains(&"felt_royal") && !ids.contains(&"felt_crimson"));
    }

    #[test]
    fn a_hand_held_to_the_river_counts_without_a_showdown() {
        let mut t = Table::new(Rules::holdem(5, 10, 2));
        t.sit(0, "a", "A", 1000).unwrap();
        t.sit(1, "b", "B", 1000).unwrap();
        let deck0 = parse_cards("AhKc QhQd").unwrap();
        let mut deck = vec![deck0[0], deck0[1], deck0[2], deck0[3]];
        let board = parse_cards("2s 9h 4h Jc 7h").unwrap();
        let mut rest: Vec<_> = crate::cards::Card::all().filter(|c| !deck.contains(c) && !board.contains(c)).collect();
        deck.push(rest.remove(0));
        deck.extend(&board[..3]);
        deck.push(rest.remove(0));
        deck.push(board[3]);
        deck.push(rest.remove(0));
        deck.push(board[4]);
        deck.extend(rest);
        let mut events: Vec<Event> = t.start_hand(1, deck).unwrap().into_iter().map(|(_, e)| e).collect();
        events.extend(t.act(0, Action::Call).unwrap().into_iter().map(|(_, e)| e));
        events.extend(t.act(1, Action::Check).unwrap().into_iter().map(|(_, e)| e));
        for _ in 0..2 {
            events.extend(t.act(1, Action::Check).unwrap().into_iter().map(|(_, e)| e));
            events.extend(t.act(0, Action::Check).unwrap().into_iter().map(|(_, e)| e));
        }
        // The river: B checks, A bets, B folds.
        events.extend(t.act(1, Action::Check).unwrap().into_iter().map(|(_, e)| e));
        events.extend(t.act(0, Action::Raise { to: 20 }).unwrap().into_iter().map(|(_, e)| e));
        events.extend(t.act(1, Action::Fold).unwrap().into_iter().map(|(_, e)| e));
        assert!(!events.iter().any(|e| matches!(e, Event::Shown { .. })));
        let holes = [parse_cards("AhQh").unwrap(), parse_cards("KcQd").unwrap()];
        let values = |s: usize| {
            let v = eval::evaluate(&[holes[s].clone(), board.clone()].concat());
            Some((v.0, v.category(), v.describe()))
        };
        let a = read_hand(&events, 0, &values);
        assert!(!a.showdown && a.won > 0);
        assert_eq!(a.shown.as_ref().map(|x| x.2.as_str()), Some("Flush, Ace high"));
        let b = read_hand(&events, 1, &values);
        assert!(b.shown.is_none(), "folded: not counted");
    }
}
