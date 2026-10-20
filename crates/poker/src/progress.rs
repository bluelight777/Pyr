//! What a player builds up: chips (play money only: they can't be bought or cashed out),
//! experience and levels, achievements, statistics, and the looks they unlock by playing
//! (never by paying). A record is kept by the server a player plays on.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::eval::Category;
use crate::game::{Action, Event, Posting, Street};

/// Chips a new player starts with, and the most the daily gift tops a broke player up to.
pub const STARTING_CHIPS: u64 = 20_000;
/// The daily gift: this much, plus this much more for each day in a row (up to a week).
pub const DAILY_BASE: u64 = 2_000;
pub const DAILY_STREAK_STEP: u64 = 500;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub gifts: BTreeSet<String>,
    /// Pyr Casino, by game (its id).
    #[serde(default)]
    pub casino: BTreeMap<String, CasinoStats>,
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
        Record { chips: STARTING_CHIPS, ..Default::default() }
    }

    pub fn level(&self) -> u32 {
        level_for(self.xp)
    }

    /// The daily gift, if it hasn't been taken today. Returns the chips given.
    pub fn claim_daily(&mut self, today: u64) -> Option<u64> {
        if self.daily_day == today && self.daily_streak > 0 {
            return None;
        }
        self.daily_streak = if self.daily_day + 1 == today { (self.daily_streak + 1).min(7) } else { 1 };
        self.daily_day = today;
        let mut gift = DAILY_BASE + DAILY_STREAK_STEP * (self.daily_streak as u64 - 1);
        // Broke: topped up to the starting chips at least.
        if self.chips + gift < STARTING_CHIPS / 4 {
            gift = STARTING_CHIPS / 4 - self.chips;
        }
        self.chips += gift;
        Some(gift)
    }

    /// The looks they've unlocked so far.
    pub fn unlocked(&self) -> Vec<&'static Cosmetic> {
        COSMETICS.iter().filter(|c| c.unlock.met(self) || self.gifts.contains(c.id)).collect()
    }
}

/// Experience needed to reach each level: 250 for level 2, rising.
pub fn xp_for_level(level: u32) -> u64 {
    if level <= 1 {
        return 0;
    }
    let l = (level - 1) as f64;
    (250.0 * l.powf(1.45)).round() as u64
}

pub fn level_for(xp: u64) -> u32 {
    let mut l = 1;
    while xp_for_level(l + 1) <= xp && l < 200 {
        l += 1;
    }
    l
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
    Achievement { id: "big_win", name: "Big Win", about: "Win 50 times your bet or more in one round.", xp: 300 },
    Achievement { id: "whale", name: "Whale", about: "Bet 25,000 or more on one round.", xp: 200 },
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
}

/// Reads a hand's events for one seat. `values` gives the eval number and category of each
/// hand shown (the server works them out; see [`crate::game::Table`]).
pub fn read_hand(events: &[Event], seat: usize, values: &dyn Fn(usize) -> Option<(u32, Category, String)>) -> HandResult {
    let mut r = HandResult::default();
    let mut street = Street::Preflop;
    let mut river_aggressor = None;
    let mut anyone_shown = false;
    let mut folded = false;
    for e in events {
        match e {
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
                if let Some(i) = seats.iter().position(|s| *s == seat) {
                    r.won += amounts[i];
                }
            }
            _ => {}
        }
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
    let before = rec.level();
    let entrants = entrants.max(2) as u64;
    let place = (place.max(1) as u64).min(entrants);
    let beaten = entrants - place;
    let xp = 50 + beaten * 400 / (entrants - 1) * (entrants.min(50)) / 10 + if place == 1 { 250 } else { 0 };
    rec.xp += xp;
    let after = rec.level();
    Gains { xp, new_achievements: Vec::new(), level_up: (after > before).then_some(after) }
}

/// Adds a casino round to a record: its game's numbers, a little experience (a point for every
/// 500 chips bet, ten at most) and any achievement it earned.
pub fn apply_casino(rec: &mut Record, o: &crate::casino::Outcome) -> Gains {
    use crate::casino::{Outcome, blackjack};
    let before = rec.level();
    let mut g = Gains::default();
    let wagered = o.wagered();
    let back = match o {
        Outcome::Craps(c) => c.paid + c.kept,
        _ => o.paid(),
    };
    let st = rec.casino.entry(o.game().id().to_string()).or_default();
    st.rounds += 1;
    st.wagered += wagered;
    st.paid += back;
    st.best_win = st.best_win.max(back.saturating_sub(wagered));
    let mult = crate::casino::multiple_x100(back, wagered);
    if back > wagered {
        st.best_multiple_x100 = st.best_multiple_x100.max(mult);
    }
    g.xp += (wagered / 500).min(10);
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
    }
    rec.xp += g.xp;
    let after = rec.level();
    g.level_up = (after > before).then_some(after);
    g
}

/// What a hand added: experience and newly earned achievements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gains {
    pub xp: u64,
    pub new_achievements: Vec<&'static str>,
    pub level_up: Option<u32>,
}

/// Adds a hand to a record. `big_blind` sizes "big pot"; `beat_shark` is true when they won a
/// showdown with a Shark bot in it.
pub fn apply(rec: &mut Record, h: &HandResult, big_blind: u64, beat_shark: bool) -> Gains {
    let mut g = Gains::default();
    if !h.played {
        return g;
    }
    let before = rec.level();
    rec.hands += 1;
    g.xp += 5;
    if h.vpip {
        rec.vpip += 1;
    }
    if h.pfr {
        rec.pfr += 1;
    }
    let net_won = h.won > h.put_in;
    if h.won > 0 {
        rec.hands_won += 1;
        g.xp += 15;
        rec.biggest_pot = rec.biggest_pot.max(h.won);
    }
    if h.showdown {
        rec.showdowns += 1;
        if h.won > 0 {
            rec.showdowns_won += 1;
            g.xp += 10;
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
    rec.xp += g.xp;
    let after = rec.level();
    if after > before {
        g.level_up = Some(after);
    }
    g
}

/// The daily streak achievement (checked when the gift is taken).
pub fn after_daily(rec: &mut Record) -> Gains {
    let mut g = Gains::default();
    if rec.daily_streak >= 7 && rec.achievements.insert("streak_7".into()) {
        g.new_achievements.push("streak_7");
        g.xp += 300;
        let before = rec.level();
        rec.xp += g.xp;
        if rec.level() > before {
            g.level_up = Some(rec.level());
        }
    }
    g
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::parse_cards;
    use crate::eval;
    use crate::game::{Rules, Table};

    #[test]
    fn levels_rise_slower_and_slower() {
        assert_eq!(level_for(0), 1);
        assert_eq!(level_for(249), 1);
        assert_eq!(level_for(250), 2);
        assert!(xp_for_level(10) - xp_for_level(9) > xp_for_level(3) - xp_for_level(2));
        for l in 1..60 {
            assert_eq!(level_for(xp_for_level(l)), l);
        }
    }

    #[test]
    fn daily_chips_grow_with_a_streak_and_rescue_the_broke() {
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
        assert_eq!(broke.claim_daily(5), Some(STARTING_CHIPS / 4));
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
        let g = apply(&mut rec, &h, 10, false);
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
