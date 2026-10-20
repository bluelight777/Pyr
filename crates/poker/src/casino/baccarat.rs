//! Baccarat (punto banco), eight decks: Player or Banker, closest to nine, drawing by the
//! tableau below. Player pays even money, Banker pays 19 to 20 (5% commission), Tie 8 to 1
//! (Player and Banker bets push), either pair 11 to 1.

use serde::{Deserialize, Serialize};

use super::Shoe;
use crate::cards::Card;
use crate::fair::Stream;

pub const DECKS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Player,
    Banker,
    Tie,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Player => "Player",
            Side::Banker => "Banker",
            Side::Tie => "Tie",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Bets {
    #[serde(default)]
    pub player: u64,
    #[serde(default)]
    pub banker: u64,
    #[serde(default)]
    pub tie: u64,
    #[serde(default)]
    pub player_pair: u64,
    #[serde(default)]
    pub banker_pair: u64,
}

impl Bets {
    pub fn total(&self) -> u64 {
        [self.player, self.banker, self.tie, self.player_pair, self.banker_pair].iter().fold(0u64, |a, b| a.saturating_add(*b))
    }

    pub fn check(&self) -> Result<(), String> {
        if self.total() == 0 {
            return Err("put chips on Player, Banker, Tie or a pair".into());
        }
        if self.tie > 5_000 || self.player_pair > 5_000 || self.banker_pair > 5_000 {
            return Err("the most on Tie or a pair is 5,000".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub bets: Bets,
    /// The cards in the order dealt to each side (two or three).
    pub player: Vec<Card>,
    pub banker: Vec<Card>,
    pub player_total: u8,
    pub banker_total: u8,
    pub winner: Side,
    pub player_pair: bool,
    pub banker_pair: bool,
    pub paid: u64,
}

/// A card's count: aces 1, twos to nines their number, tens and faces 0.
pub fn value(c: Card) -> u8 {
    match c.rank() {
        12 => 1,
        r if r <= 7 => r + 2,
        _ => 0,
    }
}

pub fn total(cards: &[Card]) -> u8 {
    cards.iter().map(|c| value(*c)).sum::<u8>() % 10
}

/// Whether the banker draws, holding `banker` with the player's third card `p3` (if they drew).
pub fn banker_draws(banker: u8, p3: Option<u8>) -> bool {
    match p3 {
        None => banker <= 5,
        Some(p) => match banker {
            0..=2 => true,
            3 => p != 8,
            4 => (2..=7).contains(&p),
            5 => (4..=7).contains(&p),
            6 => (6..=7).contains(&p),
            _ => false,
        },
    }
}

/// Deals a coup: (player's cards, banker's cards).
pub fn deal(s: &mut Stream) -> (Vec<Card>, Vec<Card>) {
    let mut shoe = Shoe::new(DECKS);
    let mut player = vec![shoe.draw(s)];
    let mut banker = vec![shoe.draw(s)];
    player.push(shoe.draw(s));
    banker.push(shoe.draw(s));
    let (pt, bt) = (total(&player), total(&banker));
    if pt >= 8 || bt >= 8 {
        return (player, banker);
    }
    let mut p3 = None;
    if pt <= 5 {
        let c = shoe.draw(s);
        p3 = Some(value(c));
        player.push(c);
    }
    if banker_draws(bt, p3) {
        banker.push(shoe.draw(s));
    }
    (player, banker)
}

pub fn settle(bets: &Bets, player: &[Card], banker: &[Card]) -> (Side, bool, bool, u64) {
    let (pt, bt) = (total(player), total(banker));
    let winner = if pt > bt {
        Side::Player
    } else if bt > pt {
        Side::Banker
    } else {
        Side::Tie
    };
    let pp = player[0].rank() == player[1].rank();
    let bp = banker[0].rank() == banker[1].rank();
    let mut paid = 0;
    match winner {
        Side::Player => paid += bets.player * 2,
        Side::Banker => paid += bets.banker + bets.banker * 95 / 100,
        Side::Tie => paid += bets.player + bets.banker + bets.tie * 9,
    }
    if pp {
        paid += bets.player_pair * 12;
    }
    if bp {
        paid += bets.banker_pair * 12;
    }
    (winner, pp, bp, paid)
}

pub fn play(bets: &Bets, s: &mut Stream) -> Outcome {
    let (player, banker) = deal(s);
    let (winner, player_pair, banker_pair, paid) = settle(bets, &player, &banker);
    Outcome { bets: bets.clone(), player_total: total(&player), banker_total: total(&banker), player, banker, winner, player_pair, banker_pair, paid }
}
