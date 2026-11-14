//! Pyr Casino's games against the house, for play chips: slots (Pyr Sevens and Reel Row's
//! machines), blackjack, roulette, baccarat, craps, dice and keno.
//!
//! Every round is fair the way Pyr's poker shuffles are. Before it, the house (the Pyr server)
//! shows the fingerprint of a secret seed. The player's Pyr adds a seed of its own and bets.
//! The round is played from both seeds and the round's number. After it, the house shows its
//! seed: the player's Pyr checks it matches the fingerprint and plays the round again itself,
//! with the code here, to see every reel, card, ball and die land the same.
//!
//! Nothing here has randomness of its own, so the server and the app run the same rounds.

pub mod baccarat;
pub mod bonfire;
pub mod bjtable;
pub mod blackjack;
pub mod craps;
pub mod dragon;
pub mod dice;
pub mod domino;
pub mod feature;
pub mod keno;
pub mod plinko;
pub mod reels;
pub mod reelrow;
pub mod remaster;
pub mod roulette;
pub mod slots;

#[cfg(test)]
mod tests;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cards::Card;
use crate::fair::Stream;

/// The games.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Game {
    Slots,
    Blackjack,
    Roulette,
    Baccarat,
    Craps,
    Dice,
    Keno,
    /// Reel Row's twelve machines (0.12.0; which one is in the bet).
    Reels,
    /// Ember Dragon (0.12.0).
    Dragon,
    /// Plinko (0.13.5), one of Pyr's Originals.
    Plinko,
    /// Dominó Cubano's tables (0.13.11): its hands counted as rounds (statistics, experience).
    Domino,
}

impl Game {
    pub const ALL: [Game; 11] = [Game::Slots, Game::Blackjack, Game::Roulette, Game::Baccarat, Game::Craps, Game::Dice, Game::Keno, Game::Reels, Game::Dragon, Game::Plinko, Game::Domino];

    /// Games apps before 0.12.0 don't know (they're never told of them: see `CASINO_012_CAP`).
    pub fn is_new(self) -> bool {
        matches!(self, Game::Reels | Game::Dragon)
    }

    /// Its name on the casino floor.
    pub fn name(self) -> &'static str {
        match self {
            Game::Slots => "Pyr Sevens",
            Game::Blackjack => "Blackjack",
            Game::Roulette => "Roulette",
            Game::Baccarat => "Baccarat",
            Game::Craps => "Craps",
            Game::Dice => "Dice",
            Game::Keno => "Keno",
            Game::Reels => "Reel Row",
            Game::Dragon => "Ember Dragon",
            Game::Plinko => "Plinko",
            Game::Domino => "Dominó Cubano",
        }
    }

    /// A short id (records, settings).
    pub fn id(self) -> &'static str {
        match self {
            Game::Slots => "slots",
            Game::Blackjack => "blackjack",
            Game::Roulette => "roulette",
            Game::Baccarat => "baccarat",
            Game::Craps => "craps",
            Game::Dice => "dice",
            Game::Keno => "keno",
            Game::Reels => "reel_row",
            Game::Dragon => "ember_dragon",
            Game::Plinko => "plinko",
            Game::Domino => "domino",
        }
    }

    pub fn from_id(id: &str) -> Option<Game> {
        Game::ALL.into_iter().find(|g| g.id() == id)
    }

    /// One line to sell it.
    pub fn pitch(self) -> &'static str {
        match self {
            Game::Slots => "Five reels, twenty lines, wild flames and free spins. Five wilds win the Phoenix Jackpot.",
            Game::Blackjack => "Beat the dealer to 21. Blackjack pays 3 to 2; double, split and insurance.",
            Game::Roulette => "A single-zero wheel. Straight up pays 35 to 1.",
            Game::Baccarat => "Player, Banker or Tie: the high rollers' game, and the simplest at the table.",
            Game::Craps => "The loudest table in the house: pass line, odds, place bets, hardways.",
            Game::Dice => "Pick your chance, from 2% to 98%. The lower it is, the more it pays.",
            Game::Keno => "Pick up to ten numbers; twenty balls come out. Ten for ten pays 100,000 to 1.",
            Game::Reels => "Twelve machines, 243 ways, wilds, a pick-and-dodge bonus or a prize wheel.",
            Game::Dragon => "243 ways. Feed the dragon eggs to grow its wins to 5×; six fireballs for Ember Respins and four jackpots.",
            Game::Plinko => "Drop a ball through the pegs. Pick the difficulty and the rows: up to 10,000 times the bet on Expert.",
            Game::Domino => "Double-nine Cuban dominoes for chips, at tables of four: in pairs or each for yourself.",
        }
    }

    /// What the house keeps over time, in hundredths of a percent (for the floor's "fair play"
    /// notes): the rest of every chip bet comes back to the players.
    pub fn house_edge_bp(self) -> u32 {
        match self {
            Game::Slots => 100,
            Game::Blackjack => 50,
            Game::Roulette => 270,
            Game::Baccarat => 106,
            Game::Craps => 141,
            Game::Dice => 100,
            Game::Keno => 500,
            Game::Reels => 100,
            Game::Dragon => 100,
            Game::Plinko => 100,
            // (The house's 3% of every pot.)
            Game::Domino => 300,
        }
    }
}

/// The least and most a round may stake, by game (chips; for slots, the line bet; for keno, the
/// bet per draw). As 0.13.28 has them: blackjack, baccarat and keno to 100,000 a round (0.13.2),
/// a roulette spin to 1,000,000 (0.13.6), Pyr Sevens to 100,000 a spin (0.13.6), Reel Row and
/// Ember Dragon to 100,000 (0.13.4).
pub fn limits(game: Game) -> (u64, u64) {
    match game {
        Game::Slots => (1, slots::MAX_LINE_BET),
        Game::Blackjack => (10, 100_000),
        Game::Roulette => (1, 1_000_000),
        Game::Baccarat => (10, 100_000),
        Game::Craps => (5, 50_000),
        Game::Dice => (1, 100_000),
        Game::Keno => (1, 100_000),
        Game::Reels => (reels::MIN_BET, reels::MAX_BET),
        Game::Dragon => (dragon::MIN_BET, dragon::MAX_BET),
        Game::Plinko => (plinko::MIN_BET, plinko::MAX_BET),
        Game::Domino => (10, 10_000_000),
    }
}

/// The most one round may pay back (chips): a Supernova (500,000 times the biggest Reel Row bet,
/// 100,000 since 0.13.4).
pub const MAX_PAYOUT: u64 = 50_000_000_000;

/// What the house shows before a round: the fingerprint of its secret seed.
pub fn commit(server_seed: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr-casino commit v1");
    h.update(server_seed);
    h.finalize().into()
}

/// The seed a round is played from: the house's, the player's, the game and the round's number.
pub fn round_seed(server_seed: &[u8; 32], client_seed: &[u8; 32], game: Game, nonce: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr-casino round v1");
    h.update(server_seed);
    h.update(client_seed);
    h.update(game.id().as_bytes());
    h.update(nonce.to_le_bytes());
    h.finalize().into()
}

/// A slot round's seed from the player's side when they've set a seed of their own for the slot
/// (Fairness, 0.13.0): their seed (any words), the slot's statistics key and the round's number,
/// so the same seed plays differently every round, and anyone with the house's seed after it can
/// play the round again from the three (as Stake's client seed and nonce work).
pub fn client_seed(seed: &str, slot: &str, nonce: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr client seed v1");
    h.update((seed.len() as u64).to_le_bytes());
    h.update(seed.as_bytes());
    h.update((slot.len() as u64).to_le_bytes());
    h.update(slot.as_bytes());
    h.update(nonce.to_le_bytes());
    h.finalize().into()
}

/// The round's random numbers.
pub fn stream(server_seed: &[u8; 32], client_seed: &[u8; 32], game: Game, nonce: u64) -> Stream {
    Stream::new(round_seed(server_seed, client_seed, game, nonce))
}

/// A bet on a game that's over in one go (blackjack's first bet starts a hand; craps has its own
/// table, see [`craps`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "game", rename_all = "snake_case")]
pub enum Bet {
    /// Every one of the twenty lines, at this much each.
    Slots { line_bet: u64 },
    Roulette { chips: Vec<roulette::Chip> },
    Baccarat(baccarat::Bets),
    /// `chance` in hundredths of a percent (200 to 9,800); over: the roll has to come in at the
    /// top of the range instead of the bottom.
    Dice { chips: u64, chance: u16, over: bool },
    Keno { chips: u64, picks: Vec<u8> },
    Blackjack { chips: u64 },
    /// A spin on one of Reel Row's machines (0 to 11), or its bonus bought (`reels::BUY_X`
    /// times the bet).
    Reels { machine: u8, bet: u64, buy: bool },
    /// A spin of Ember Dragon, with the dragon grown to `growth` for this bet (as the house
    /// last said: the spin is played from it).
    Dragon { bet: u64, growth: u32 },
    /// A round on one of Reel Row's machines from 0.13.0 (`reelrow`): a spin, or a Play
    /// Feature.
    Row { machine: u8, bet: u64, feature: feature::Feature },
    /// A round of Pyr Sevens with a Play Feature (0.13.0).
    SlotsPlus { line_bet: u64, feature: feature::Feature },
    /// A round of Ember Dragon with a Play Feature (0.13.0).
    DragonPlus { bet: u64, growth: u32, feature: feature::Feature },
    /// A ball dropped at Plinko (0.13.5): `rows` from 8 to 16.
    Plinko { chips: u64, risk: plinko::Risk, rows: u8 },
    /// A round on one of Reel Row Remastered's machines (`remaster`, 0.13.2): a spin, with Ante
    /// or without, or a Play Feature.
    Remaster {
        machine: u8,
        bet: u64,
        feature: feature::Feature,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ante: bool,
    },
}

impl Bet {
    pub fn game(&self) -> Game {
        match self {
            Bet::Slots { .. } => Game::Slots,
            Bet::Roulette { .. } => Game::Roulette,
            Bet::Baccarat(_) => Game::Baccarat,
            Bet::Dice { .. } => Game::Dice,
            Bet::Keno { .. } => Game::Keno,
            Bet::Blackjack { .. } => Game::Blackjack,
            Bet::Reels { .. } => Game::Reels,
            Bet::Dragon { .. } => Game::Dragon,
            Bet::Row { .. } => Game::Reels,
            Bet::SlotsPlus { .. } => Game::Slots,
            Bet::DragonPlus { .. } => Game::Dragon,
            Bet::Plinko { .. } => Game::Plinko,
            Bet::Remaster { .. } => Game::Reels,
        }
    }

    /// Whether it's a bet from 0.13.0 on (an older server doesn't know it).
    pub fn is_013(&self) -> bool {
        matches!(self, Bet::Row { .. } | Bet::SlotsPlus { .. } | Bet::DragonPlus { .. } | Bet::Plinko { .. } | Bet::Remaster { .. })
    }

    /// All the chips it puts down.
    pub fn total(&self) -> u64 {
        match self {
            Bet::Slots { line_bet } => line_bet.saturating_mul(slots::LINES as u64),
            Bet::Roulette { chips } => chips.iter().map(|c| c.chips).fold(0u64, |a, b| a.saturating_add(b)),
            Bet::Baccarat(b) => b.total(),
            Bet::Dice { chips, .. } | Bet::Keno { chips, .. } | Bet::Blackjack { chips } => *chips,
            Bet::Reels { bet, buy, .. } => if *buy { bet.saturating_mul(reels::BUY_X) } else { *bet },
            Bet::Dragon { bet, .. } => *bet,
            Bet::Row { bet, feature, .. } => feature::cost(*bet, reelrow::price(*feature)),
            Bet::SlotsPlus { line_bet, feature } => feature::cost(line_bet.saturating_mul(slots::LINES as u64), slots::price(*feature)),
            Bet::DragonPlus { bet, feature, .. } => feature::cost(*bet, dragon::price(*feature).unwrap_or(100)),
            Bet::Plinko { chips, .. } => *chips,
            Bet::Remaster { machine, bet, feature, ante } => feature::cost(*bet, remaster::price(*machine, *feature, *ante)),
        }
    }

    /// Whether it's a bet the house takes.
    pub fn check(&self) -> Result<(), String> {
        let (min, max) = limits(self.game());
        let fits = |n: u64, what: &str| -> Result<(), String> {
            if n < min {
                return Err(format!("the least {what} is {min}"));
            }
            if n > max {
                return Err(format!("the most {what} is {max}"));
            }
            Ok(())
        };
        match self {
            Bet::Slots { line_bet } => fits(*line_bet, "line bet"),
            Bet::Roulette { chips } => {
                if chips.is_empty() || chips.len() > 60 {
                    return Err("place 1 to 60 bets on the layout".into());
                }
                for c in chips {
                    c.spot.check()?;
                    if c.chips == 0 {
                        return Err("a bet of nothing".into());
                    }
                    if c.chips > roulette::max_on(&c.spot) {
                        return Err(format!("the most on {} is {}", c.spot.name(), roulette::max_on(&c.spot)));
                    }
                }
                fits(self.total(), "for one spin")
            }
            Bet::Baccarat(b) => {
                b.check()?;
                fits(self.total(), "for one hand")
            }
            Bet::Dice { chips, chance, .. } => {
                if !(dice::MIN_CHANCE..=dice::MAX_CHANCE).contains(chance) {
                    return Err("pick a chance from 2% to 98%".into());
                }
                fits(*chips, "bet")
            }
            Bet::Keno { chips, picks } => {
                keno::check_picks(picks)?;
                fits(*chips, "bet")
            }
            Bet::Blackjack { chips } => fits(*chips, "bet"),
            Bet::Reels { machine, bet, .. } => reels::check(*machine, *bet),
            Bet::Dragon { bet, growth } => dragon::check(*bet, *growth),
            Bet::Row { feature: feature::Feature::Builds, .. } => Err("Reel Row hasn't that feature".into()),
            Bet::Row { machine, bet, .. } => reelrow::check(*machine, *bet),
            Bet::SlotsPlus { feature: feature::Feature::Builds, .. } => Err("Pyr Sevens hasn't that feature".into()),
            Bet::SlotsPlus { line_bet, .. } => fits(*line_bet, "line bet"),
            Bet::DragonPlus { bet, growth, feature } => {
                if dragon::price(*feature).is_none() {
                    return Err("Ember Dragon hasn't that feature".into());
                }
                dragon::check(*bet, *growth)
            }
            // (0.13.28's words.)
            Bet::Plinko { chips, rows, .. } => {
                if !(plinko::MIN_ROWS..=plinko::MAX_ROWS).contains(rows) {
                    return Err("Plinko has 8 to 16 rows".into());
                }
                if *chips == 0 {
                    return Err("a bet of nothing".into());
                }
                fits(*chips, "bet")
            }
            Bet::Remaster { machine, bet, feature, ante } => remaster::check(*machine, *bet, *feature, *ante),
        }
    }
}

/// How a round came out.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "game", rename_all = "snake_case")]
pub enum Outcome {
    Slots(slots::Outcome),
    Blackjack(blackjack::Outcome),
    Roulette(roulette::Outcome),
    Baccarat(baccarat::Outcome),
    Craps(craps::Outcome),
    Dice(dice::Outcome),
    Keno(keno::Outcome),
    Reels(reels::Outcome),
    Dragon(dragon::Outcome),
    /// Reel Row from 0.13.0.
    Row(reelrow::Outcome),
    /// Plinko (0.13.5).
    Plinko(plinko::Outcome),
    /// Reel Row Remastered (0.13.2).
    Remaster(remaster::Outcome),
    /// A Dominó hand (0.13.11).
    Domino(domino::Outcome),
}

impl Outcome {
    /// What its statistics are kept under: the game's id, or for Reel Row the machine's
    /// (`reel_row:3`).
    pub fn stats_key(&self) -> String {
        match self {
            Outcome::Reels(o) => reels_key(o.machine),
            Outcome::Row(o) => reels_key(o.machine),
            Outcome::Remaster(o) => reels_key(o.machine),
            _ => self.game().id().to_string(),
        }
    }

    pub fn game(&self) -> Game {
        match self {
            Outcome::Slots(_) => Game::Slots,
            Outcome::Blackjack(_) => Game::Blackjack,
            Outcome::Roulette(_) => Game::Roulette,
            Outcome::Baccarat(_) => Game::Baccarat,
            Outcome::Craps(_) => Game::Craps,
            Outcome::Dice(_) => Game::Dice,
            Outcome::Keno(_) => Game::Keno,
            Outcome::Reels(_) => Game::Reels,
            Outcome::Dragon(_) => Game::Dragon,
            Outcome::Plinko(_) => Game::Plinko,
            Outcome::Row(_) => Game::Reels,
            Outcome::Remaster(_) => Game::Reels,
            Outcome::Domino(_) => Game::Domino,
        }
    }

    /// Chips staked in the round (all of them: doubles, splits and insurance too).
    pub fn wagered(&self) -> u64 {
        match self {
            Outcome::Slots(o) => o.wagered(),
            Outcome::Blackjack(o) => o.wagered,
            Outcome::Roulette(o) => o.chips.iter().map(|c| c.chips).sum(),
            Outcome::Baccarat(o) => o.bets.total(),
            Outcome::Craps(o) => o.wagered,
            Outcome::Dice(o) => o.chips,
            Outcome::Keno(o) => o.chips,
            Outcome::Reels(o) => o.wagered(),
            Outcome::Dragon(o) => o.wagered(),
            Outcome::Plinko(o) => o.chips,
            Outcome::Row(o) => o.wagered(),
            Outcome::Remaster(o) => o.wagered(),
            Outcome::Domino(o) => o.wagered,
        }
    }

    /// Chips back to the player from the round (stakes of winning bets included).
    pub fn paid(&self) -> u64 {
        match self {
            Outcome::Slots(o) => o.paid,
            Outcome::Blackjack(o) => o.paid,
            Outcome::Roulette(o) => o.paid,
            Outcome::Baccarat(o) => o.paid,
            Outcome::Craps(o) => o.paid,
            Outcome::Dice(o) => o.paid,
            Outcome::Keno(o) => o.paid,
            Outcome::Reels(o) => o.paid,
            Outcome::Dragon(o) => o.paid,
            Outcome::Plinko(o) => o.paid,
            Outcome::Row(o) => o.paid,
            Outcome::Remaster(o) => o.paid,
            Outcome::Domino(o) => o.paid,
        }
    }

    /// The round in a few words, for the history and the big-win feed.
    pub fn headline(&self) -> String {
        match self {
            Outcome::Slots(o) if o.feature == feature::Feature::AllIn => if o.lucky { "all or nothing, and won it".into() } else { "all or nothing".into() },
            Outcome::Slots(o) if o.free_x > slots::FREE_SPIN_MULTIPLIER => format!("{} Super Free Spins", o.spins.len().saturating_sub(1)),
            Outcome::Slots(o) => {
                if o.jackpot_won > 0 {
                    "the Phoenix Jackpot".into()
                } else if o.spins.len() > 1 {
                    format!("{} free spins", o.spins.len() - 1)
                } else {
                    "a spin".into()
                }
            }
            Outcome::Blackjack(o) => o.headline(),
            Outcome::Roulette(o) => format!("{} {}", o.number, roulette::color_name(o.number)),
            Outcome::Baccarat(o) => format!("{} wins, {} to {}", o.winner.name(), o.player_total, o.banker_total),
            Outcome::Craps(o) => format!("{} ({} and {})", o.dice.0 + o.dice.1, o.dice.0, o.dice.1),
            Outcome::Dice(o) => format!("rolled {}", dice::show(o.roll)),
            Outcome::Keno(o) => format!("caught {} of {}", o.hits.len(), o.picks.len()),
            Outcome::Reels(o) => {
                let on = reels::machine_name(o.machine);
                match &o.bonus {
                    Some(reels::Bonus::Wheel { inner: Some(_), .. }) => format!("the wheel's Upgrade on {on}"),
                    Some(reels::Bonus::Wheel { .. }) => format!("the prize wheel on {on}"),
                    Some(reels::Bonus::Pick { finale: true, .. }) => format!("the finale on {on}"),
                    Some(reels::Bonus::Pick { .. }) => format!("the bonus on {on}"),
                    None => format!("a spin on {on}"),
                }
            }
            Outcome::Row(o) => {
                let on = reels::machine_name(o.machine);
                let game = reelrow::GAMES.get(o.machine as usize).map(|g| g.0).unwrap_or("the bonus");
                match (o.feature, &o.bonus) {
                    (feature::Feature::AllIn, _) if o.lucky => format!("all or nothing on {on}, and won it"),
                    (feature::Feature::AllIn, _) => format!("all or nothing on {on}"),
                    (_, Some(b)) if b.times > 1 => format!("the Super {game} on {on}"),
                    (_, Some(_)) => format!("the {game} on {on}"),
                    (_, None) => format!("a spin on {on}"),
                }
            }
            Outcome::Dragon(o) if o.feature == feature::Feature::AllIn => if o.lucky { "all or nothing on Ember Dragon, and won it".into() } else { "all or nothing on Ember Dragon".into() },
            Outcome::Dragon(o) if o.super_respins && o.jackpots().is_empty() => "Super Ember Respins".into(),
            Outcome::Dragon(o) => {
                let jackpots = o.jackpots();
                match (jackpots.iter().max(), &o.respins) {
                    (Some(j), _) => format!("the {} jackpot on Ember Dragon", j.name()),
                    (None, Some(r)) => format!("Ember Respins, {} fireballs", r.board.len()),
                    (None, None) => format!("a spin with a{} {} dragon", if dragon::age(o.growth) == 2 || dragon::age(o.growth) == 4 { "n" } else { "" }, dragon::AGES[dragon::age(o.growth)].1),
                }
            }
            // (0.13.28's words: "114× on Expert with 9 rows", "3.7× on Low with 9 rows".)
            Outcome::Plinko(o) => format!("{} on {} with {} rows", plinko::label(o.multiplier_x100), o.risk.name(), o.rows),
            Outcome::Remaster(o) => o.headline(),
            Outcome::Domino(o) => o.headline(),
        }
    }
}

/// A Reel Row machine's statistics key.
pub fn reels_key(machine: u8) -> String {
    format!("{}:{machine}", Game::Reels.id())
}

/// The slots, by statistics key: their names and what they pay back over time (hundredths of a
/// percent), for their descriptions.
pub fn slot_rtp_bp(key: &str) -> Option<u32> {
    if key == Game::Slots.id() {
        // (98% from the reels, and the jackpot's 1% back.)
        return Some(9_900);
    }
    if key == Game::Dragon.id() {
        // (98.5% from the reels and respins, and the jackpots' pools' half a percent back.)
        return Some(9_900);
    }
    key.strip_prefix("reel_row:").and_then(|m| m.parse::<u8>().ok()).filter(|m| (*m as usize) < reels::MACHINES.len()).map(|_| 9_900)
}

/// Plays a one-go round (everything but blackjack and craps, which go a step at a time).
/// `jackpot` is the Phoenix Jackpot's pool as the round starts (slots), or a progressive Reel
/// Row machine's (`reelrow::progressive`, Remastered too).
pub fn play(bet: &Bet, s: &mut Stream, jackpot: u64) -> Result<Outcome, String> {
    bet.check()?;
    Ok(match bet {
        Bet::Slots { line_bet } => Outcome::Slots(slots::play(*line_bet, s, jackpot)),
        Bet::Roulette { chips } => Outcome::Roulette(roulette::play(chips, s)),
        Bet::Baccarat(b) => Outcome::Baccarat(baccarat::play(b, s)),
        Bet::Dice { chips, chance, over } => Outcome::Dice(dice::play(*chips, *chance, *over, s)),
        Bet::Keno { chips, picks } => Outcome::Keno(keno::play(*chips, picks, s)),
        Bet::Reels { machine, bet, buy } => Outcome::Reels(reels::play(*machine, *bet, *buy, s)),
        Bet::Row { machine, bet, feature } => Outcome::Row(reelrow::play_with(*machine, *bet, *feature, s, jackpot)),
        Bet::Remaster { machine, bet, feature, ante } => Outcome::Remaster(remaster::play_with(*machine, *bet, *feature, *ante, s, jackpot)),
        Bet::SlotsPlus { line_bet, feature } => Outcome::Slots(slots::play_feature(*line_bet, *feature, s, jackpot)),
        Bet::DragonPlus { .. } => return Err("Ember Dragon is played with its jackpots' pools (dragon::play_feature)".into()),
        Bet::Blackjack { .. } => return Err("blackjack is dealt a step at a time".into()),
        Bet::Dragon { .. } => return Err("Ember Dragon is played with its jackpots' pools (dragon::play)".into()),
        Bet::Plinko { chips, risk, rows } => Outcome::Plinko(plinko::play(*chips, *risk, *rows, s)),
    })
}

/// A shoe of `decks` decks, dealt from the top, each card picked as it's dealt (the same as
/// shuffling the whole shoe first: every order is equally likely).
#[derive(Clone)]
pub struct Shoe {
    cards: Vec<Card>,
    next: usize,
}

impl Shoe {
    pub fn new(decks: usize) -> Shoe {
        let mut cards = Vec::with_capacity(52 * decks);
        for _ in 0..decks {
            cards.extend(Card::all());
        }
        Shoe { cards, next: 0 }
    }

    /// The next card off the shoe.
    pub fn draw(&mut self, s: &mut Stream) -> Card {
        let n = self.cards.len();
        // (A shoe never runs out in one round: a hand of blackjack uses at most a few dozen.)
        let i = self.next % n;
        let j = i + s.below((n - i) as u32) as usize;
        self.cards.swap(i, j);
        self.next += 1;
        self.cards[i]
    }
}

/// A round's multiple of the bet, in hundredths ("x12.50").
pub fn multiple_x100(paid: u64, wagered: u64) -> u64 {
    if wagered == 0 {
        0
    } else {
        ((paid as u128 * 100) / wagered as u128) as u64
    }
}

/// Checks a round that's over, as the player's Pyr does: the house's seed is the one it promised
/// (`commit`, shown before the round), and playing the round again from the seeds gives exactly
/// what the house said.
pub fn verify(commit_shown: &[u8; 32], server_seed: &[u8; 32], client_seed: &[u8; 32], nonce: u64, outcome: &Outcome) -> Result<(), String> {
    if &commit(server_seed) != commit_shown {
        return Err("the house's seed isn't the one it promised before the round".into());
    }
    let mut s = stream(server_seed, client_seed, outcome.game(), nonce);
    let same = match outcome {
        Outcome::Slots(o) => slots::play_feature(o.line_bet, o.feature, &mut s, o.jackpot_pool) == *o,
        Outcome::Roulette(o) => roulette::play(&o.chips, &mut s) == *o,
        Outcome::Baccarat(o) => baccarat::play(&o.bets, &mut s) == *o,
        Outcome::Dice(o) => dice::play(o.chips, o.chance, o.over, &mut s) == *o,
        Outcome::Keno(o) => keno::play(o.chips, &o.picks, &mut s) == *o,
        Outcome::Reels(o) => reels::check(o.machine, o.bet).is_ok() && reels::play(o.machine, o.bet, o.bought, &mut s) == *o,
        Outcome::Dragon(o) => dragon::check(o.bet, o.growth).is_ok() && dragon::play_feature(o.bet, o.growth, o.pools, o.feature, &mut s) == *o,
        Outcome::Plinko(o) => (plinko::MIN_ROWS..=plinko::MAX_ROWS).contains(&o.rows) && plinko::play(o.chips, o.risk, o.rows, &mut s) == *o,
        Outcome::Row(o) => reelrow::check(o.machine, o.bet).is_ok() && reelrow::play_with(o.machine, o.bet, o.feature, &mut s, o.pool) == *o,
        Outcome::Remaster(o) => remaster::check(o.machine, o.bet, o.feature, o.ante).is_ok() && remaster::play_with(o.machine, o.bet, o.feature, o.ante, &mut s, o.pool) == *o,
        Outcome::Blackjack(o) => blackjack::replay(o.base, s, &o.actions).is_ok_and(|again| again == *o),
        // (A table's hand: checked from its record, `domroom::check`.)
        Outcome::Domino(_) => false,
        Outcome::Craps(o) => {
            let dice = craps::roll(&mut s);
            let before = craps::Table { point: o.point_before, bets: o.before.clone(), streak: 0 };
            let (after, again) = craps::settle(&before, dice);
            dice == o.dice && after.point == o.point_after && after.bets == o.after && again.paid == o.paid && again.decided == o.decided
        }
    };
    if same { Ok(()) } else { Err(format!("{} came out differently when this Pyr played it again", outcome.game().name())) }
}
