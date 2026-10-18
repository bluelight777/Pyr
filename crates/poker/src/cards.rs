//! Cards: 52 of them, each a number from 0 to 51 (rank * 4 + suit), so a hand fits in a few
//! bytes and a set of cards in one `u64`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// A card: rank 0 (two) to 12 (ace), suit 0 to 3 (clubs, diamonds, hearts, spades).
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Card(u8);

pub const RANKS: &str = "23456789TJQKA";
pub const SUITS: &str = "cdhs";

impl Card {
    pub fn new(rank: u8, suit: u8) -> Card {
        assert!(rank < 13 && suit < 4);
        Card(rank * 4 + suit)
    }

    pub fn from_index(i: u8) -> Card {
        assert!(i < 52);
        Card(i)
    }

    pub fn index(self) -> u8 {
        self.0
    }

    /// 0 (two) to 12 (ace).
    pub fn rank(self) -> u8 {
        self.0 / 4
    }

    /// 0 clubs, 1 diamonds, 2 hearts, 3 spades.
    pub fn suit(self) -> u8 {
        self.0 % 4
    }

    /// The card's bit in a [`CardSet`].
    pub fn bit(self) -> u64 {
        1 << self.0
    }

    /// "Ah", "Td", "2c".
    pub fn parse(s: &str) -> Option<Card> {
        let mut c = s.chars();
        let r = RANKS.find(c.next()?.to_ascii_uppercase())? as u8;
        let s = SUITS.find(c.next()?.to_ascii_lowercase())? as u8;
        if c.next().is_some() {
            return None;
        }
        Some(Card::new(r, s))
    }

    /// The rank's name for people: "Ace", "Ten", "Two".
    pub fn rank_name(rank: u8) -> &'static str {
        ["Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten", "Jack", "Queen", "King", "Ace"][rank as usize]
    }

    /// Plural: "Aces", "Sixes".
    pub fn rank_plural(rank: u8) -> &'static str {
        ["Twos", "Threes", "Fours", "Fives", "Sixes", "Sevens", "Eights", "Nines", "Tens", "Jacks", "Queens", "Kings", "Aces"][rank as usize]
    }

    /// The suit's symbol: ♣ ♦ ♥ ♠.
    pub fn suit_symbol(self) -> char {
        ['♣', '♦', '♥', '♠'][self.suit() as usize]
    }

    /// Every card, in order.
    pub fn all() -> impl Iterator<Item = Card> {
        (0..52).map(Card)
    }
}

impl fmt::Display for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let r = RANKS.as_bytes()[self.rank() as usize] as char;
        let s = SUITS.as_bytes()[self.suit() as usize] as char;
        write!(f, "{r}{s}")
    }
}

impl fmt::Debug for Card {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl TryFrom<String> for Card {
    type Error = String;
    fn try_from(s: String) -> Result<Card, String> {
        Card::parse(&s).ok_or_else(|| format!("{s:?} isn't a card"))
    }
}

impl From<Card> for String {
    fn from(c: Card) -> String {
        c.to_string()
    }
}

/// Some cards, as a bit set.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Debug)]
pub struct CardSet(pub u64);

impl CardSet {
    pub fn of(cards: &[Card]) -> CardSet {
        CardSet(cards.iter().fold(0, |m, c| m | c.bit()))
    }

    pub fn contains(self, c: Card) -> bool {
        self.0 & c.bit() != 0
    }

    pub fn add(&mut self, c: Card) {
        self.0 |= c.bit();
    }

    pub fn len(self) -> u32 {
        self.0.count_ones()
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn cards(self) -> impl Iterator<Item = Card> {
        let mut m = self.0;
        std::iter::from_fn(move || {
            if m == 0 {
                return None;
            }
            let i = m.trailing_zeros() as u8;
            m &= m - 1;
            Some(Card(i))
        })
    }
}

/// Cards written as "Ah Kd 7c" (spaces or none).
pub fn parse_cards(s: &str) -> Option<Vec<Card>> {
    let t: String = s.chars().filter(|c| !c.is_whitespace() && *c != ',').collect();
    if t.len() % 2 != 0 {
        return None;
    }
    t.as_bytes().chunks(2).map(|p| Card::parse(std::str::from_utf8(p).ok()?)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cards_read_and_write_the_same() {
        for c in Card::all() {
            assert_eq!(Card::parse(&c.to_string()), Some(c));
        }
        assert_eq!(Card::parse("ah"), Some(Card::new(12, 2)));
        assert_eq!(Card::parse("1h"), None);
        assert_eq!(Card::parse("Ahh"), None);
        assert_eq!(parse_cards("Ah Kd, 7c").unwrap().len(), 3);
        let json = serde_json::to_string(&Card::parse("Td").unwrap()).unwrap();
        assert_eq!(json, "\"Td\"");
        assert!(serde_json::from_str::<Card>("\"Xx\"").is_err());
    }

    #[test]
    fn a_set_holds_cards() {
        let cs = parse_cards("AhKd2c").unwrap();
        let s = CardSet::of(&cs);
        assert_eq!(s.len(), 3);
        assert!(s.contains(cs[1]));
        let mut back: Vec<Card> = s.cards().collect();
        back.sort();
        let mut want = cs.clone();
        want.sort();
        assert_eq!(back, want);
    }
}
