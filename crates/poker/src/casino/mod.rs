//! Pyr Casino's games against the house, for play chips: slots, blackjack, roulette, baccarat,
//! craps, dice and keno.
//!
//! Every round is fair the way Pyr's poker shuffles are. Before it, the house (the Pyr server)
//! shows the fingerprint of a secret seed. The player's Pyr adds a seed of its own and bets.
//! The round is played from both seeds and the round's number. After it, the house shows its
//! seed: the player's Pyr checks it matches the fingerprint and plays the round again itself,
//! with the code here, to see every reel, card, ball and die land the same.
//!
//! Nothing here has randomness of its own, so the server and the app run the same rounds.

pub mod baccarat;
pub mod blackjack;
pub mod craps;
pub mod dice;
pub mod keno;
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
}

impl Game {
    pub const ALL: [Game; 7] = [Game::Slots, Game::Blackjack, Game::Roulette, Game::Baccarat, Game::Craps, Game::Dice, Game::Keno];

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
        }
    }

    /// What the house keeps over time, in hundredths of a percent (for the floor's "fair play"
    /// notes): the rest of every chip bet comes back to the players.
    pub fn house_edge_bp(self) -> u32 {
        match self {
            Game::Slots => 450,
            Game::Blackjack => 50,
            Game::Roulette => 270,
            Game::Baccarat => 106,
            Game::Craps => 141,
            Game::Dice => 100,
            Game::Keno => 500,
        }
    }
}

/// The least and most a round may stake, by game (chips; for slots, the line bet; for keno, the
/// bet per draw).
pub fn limits(game: Game) -> (u64, u64) {
    match game {
        Game::Slots => (1, slots::MAX_LINE_BET),
        Game::Blackjack => (10, 50_000),
        Game::Roulette => (1, 50_000),
        Game::Baccarat => (10, 50_000),
        Game::Craps => (5, 50_000),
        Game::Dice => (1, 100_000),
        Game::Keno => (1, 1_000),
    }
}

/// The most one round may pay back (chips), so a lucky round can't flood the play-chip economy.
pub const MAX_PAYOUT: u64 = 25_000_000;

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
        }
    }

    /// All the chips it puts down.
    pub fn total(&self) -> u64 {
        match self {
            Bet::Slots { line_bet } => line_bet.saturating_mul(slots::LINES as u64),
            Bet::Roulette { chips } => chips.iter().map(|c| c.chips).fold(0u64, |a, b| a.saturating_add(b)),
            Bet::Baccarat(b) => b.total(),
            Bet::Dice { chips, .. } | Bet::Keno { chips, .. } | Bet::Blackjack { chips } => *chips,
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
}

impl Outcome {
    pub fn game(&self) -> Game {
        match self {
            Outcome::Slots(_) => Game::Slots,
            Outcome::Blackjack(_) => Game::Blackjack,
            Outcome::Roulette(_) => Game::Roulette,
            Outcome::Baccarat(_) => Game::Baccarat,
            Outcome::Craps(_) => Game::Craps,
            Outcome::Dice(_) => Game::Dice,
            Outcome::Keno(_) => Game::Keno,
        }
    }

    /// Chips staked in the round (all of them: doubles, splits and insurance too).
    pub fn wagered(&self) -> u64 {
        match self {
            Outcome::Slots(o) => o.line_bet * slots::LINES as u64,
            Outcome::Blackjack(o) => o.wagered,
            Outcome::Roulette(o) => o.chips.iter().map(|c| c.chips).sum(),
            Outcome::Baccarat(o) => o.bets.total(),
            Outcome::Craps(o) => o.wagered,
            Outcome::Dice(o) => o.chips,
            Outcome::Keno(o) => o.chips,
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
        }
    }

    /// The round in a few words, for the history and the big-win feed.
    pub fn headline(&self) -> String {
        match self {
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
        }
    }
}

/// Plays a one-go round (everything but blackjack and craps, which go a step at a time).
/// `jackpot` is the Phoenix Jackpot's pool as the round starts (slots).
pub fn play(bet: &Bet, s: &mut Stream, jackpot: u64) -> Result<Outcome, String> {
    bet.check()?;
    Ok(match bet {
        Bet::Slots { line_bet } => Outcome::Slots(slots::play(*line_bet, s, jackpot)),
        Bet::Roulette { chips } => Outcome::Roulette(roulette::play(chips, s)),
        Bet::Baccarat(b) => Outcome::Baccarat(baccarat::play(b, s)),
        Bet::Dice { chips, chance, over } => Outcome::Dice(dice::play(*chips, *chance, *over, s)),
        Bet::Keno { chips, picks } => Outcome::Keno(keno::play(*chips, picks, s)),
        Bet::Blackjack { .. } => return Err("blackjack is dealt a step at a time".into()),
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
        Outcome::Slots(o) => slots::play(o.line_bet, &mut s, o.jackpot_pool) == *o,
        Outcome::Roulette(o) => roulette::play(&o.chips, &mut s) == *o,
        Outcome::Baccarat(o) => baccarat::play(&o.bets, &mut s) == *o,
        Outcome::Dice(o) => dice::play(o.chips, o.chance, o.over, &mut s) == *o,
        Outcome::Keno(o) => keno::play(o.chips, &o.picks, &mut s) == *o,
        Outcome::Blackjack(o) => blackjack::replay(o.base, s, &o.actions).is_ok_and(|again| again == *o),
        Outcome::Craps(o) => {
            let dice = craps::roll(&mut s);
            let before = craps::Table { point: o.point_before, bets: o.before.clone(), streak: 0 };
            let (after, again) = craps::settle(&before, dice);
            dice == o.dice && after.point == o.point_after && after.bets == o.after && again.paid == o.paid && again.decided == o.decided
        }
    };
    if same { Ok(()) } else { Err(format!("{} came out differently when this Pyr played it again", outcome.game().name())) }
}
