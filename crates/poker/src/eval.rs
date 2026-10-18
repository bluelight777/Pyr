//! What a hand is worth: the best five cards from five to seven, as one number that sorts the
//! way poker does (a higher number wins; equal numbers split the pot).

use crate::cards::{Card, CardSet};

/// A hand's worth: its category in the top bits, then the ranks that break ties.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Value(pub u32);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum Category {
    HighCard = 0,
    Pair = 1,
    TwoPair = 2,
    Trips = 3,
    Straight = 4,
    Flush = 5,
    FullHouse = 6,
    Quads = 7,
    StraightFlush = 8,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Category::HighCard => "High Card",
            Category::Pair => "Pair",
            Category::TwoPair => "Two Pair",
            Category::Trips => "Three of a Kind",
            Category::Straight => "Straight",
            Category::Flush => "Flush",
            Category::FullHouse => "Full House",
            Category::Quads => "Four of a Kind",
            Category::StraightFlush => "Straight Flush",
        }
    }

    fn from_u32(v: u32) -> Category {
        [
            Category::HighCard,
            Category::Pair,
            Category::TwoPair,
            Category::Trips,
            Category::Straight,
            Category::Flush,
            Category::FullHouse,
            Category::Quads,
            Category::StraightFlush,
        ][v as usize]
    }
}

fn make(cat: Category, ranks: &[u8]) -> Value {
    let mut v = (cat as u32) << 20;
    for (i, r) in ranks.iter().take(5).enumerate() {
        v |= (*r as u32) << (16 - 4 * i);
    }
    Value(v)
}

/// The highest rank of a five-in-a-row in `mask` (bit = rank), counting the ace as low too.
fn straight_top(mask: u16) -> Option<u8> {
    // Bit 0 is the ace played low; bit r + 1 is rank r.
    let ext: u32 = ((mask as u32) << 1) | if mask & (1 << 12) != 0 { 1 } else { 0 };
    for top in (4..=13u32).rev() {
        let run = 0b11111u32 << (top - 4);
        if ext & run == run {
            return Some((top - 1) as u8);
        }
    }
    None
}

/// The ranks set in `mask`, highest first.
fn top_ranks(mask: u16, n: usize) -> Vec<u8> {
    (0..13u8).rev().filter(|r| mask & (1 << r) != 0).take(n).collect()
}

/// The worth of the best five cards in `cards` (five to seven of them; fewer are valued as
/// they are, which is what's shown before the river).
pub fn evaluate(cards: &[Card]) -> Value {
    evaluate_set(CardSet::of(cards))
}

pub fn evaluate_set(set: CardSet) -> Value {
    let mut suit_mask = [0u16; 4];
    let mut counts = [0u8; 13];
    let mut any = 0u16;
    for c in set.cards() {
        suit_mask[c.suit() as usize] |= 1 << c.rank();
        counts[c.rank() as usize] += 1;
        any |= 1 << c.rank();
    }
    let flush = suit_mask.iter().copied().find(|m| m.count_ones() >= 5);
    if let Some(t) = flush.and_then(straight_top) {
        return make(Category::StraightFlush, &[t]);
    }
    let (mut quads, mut trips, mut pairs) = (None, Vec::new(), Vec::new());
    for r in (0..13u8).rev() {
        match counts[r as usize] {
            4 => quads = Some(r),
            3 => trips.push(r),
            2 => pairs.push(r),
            _ => {}
        }
    }
    if let Some(q) = quads {
        let k = top_ranks(any & !(1 << q), 1);
        return make(Category::Quads, &[q, k.first().copied().unwrap_or(0)]);
    }
    if let Some(&t) = trips.first() {
        // The pair of a full house: the best other trips or pair.
        let second = trips.get(1).copied().into_iter().chain(pairs.first().copied()).max();
        if let Some(p) = second {
            return make(Category::FullHouse, &[t, p]);
        }
    }
    if let Some(m) = flush {
        return make(Category::Flush, &top_ranks(m, 5));
    }
    if let Some(t) = straight_top(any) {
        return make(Category::Straight, &[t]);
    }
    if let Some(&t) = trips.first() {
        let mut r = vec![t];
        r.extend(top_ranks(any & !(1 << t), 2));
        return make(Category::Trips, &r);
    }
    if pairs.len() >= 2 {
        let (a, b) = (pairs[0], pairs[1]);
        let mut r = vec![a, b];
        r.extend(top_ranks(any & !(1 << a) & !(1 << b), 1));
        return make(Category::TwoPair, &r);
    }
    if let Some(&p) = pairs.first() {
        let mut r = vec![p];
        r.extend(top_ranks(any & !(1 << p), 3));
        return make(Category::Pair, &r);
    }
    make(Category::HighCard, &top_ranks(any, 5))
}

impl Value {
    pub fn category(self) -> Category {
        Category::from_u32(self.0 >> 20)
    }

    fn rank_at(self, i: usize) -> u8 {
        ((self.0 >> (16 - 4 * i)) & 0xf) as u8
    }

    /// For people: "Full House, Kings over Sevens", "Pair of Aces", "Royal Flush".
    pub fn describe(self) -> String {
        let r = |i| Card::rank_name(self.rank_at(i));
        let rs = |i| Card::rank_plural(self.rank_at(i));
        match self.category() {
            Category::StraightFlush if self.rank_at(0) == 12 => "Royal Flush".into(),
            Category::StraightFlush => format!("Straight Flush, {} high", r(0)),
            Category::Quads => format!("Four of a Kind, {}", rs(0)),
            Category::FullHouse => format!("Full House, {} over {}", rs(0), rs(1)),
            Category::Flush => format!("Flush, {} high", r(0)),
            Category::Straight => format!("Straight, {} high", r(0)),
            Category::Trips => format!("Three of a Kind, {}", rs(0)),
            Category::TwoPair => format!("Two Pair, {} and {}", rs(0), rs(1)),
            Category::Pair => format!("Pair of {}", rs(0)),
            Category::HighCard => format!("{} High", r(0)),
        }
    }
}

/// Every way to pick `k` of `n` (as index lists).
fn combos(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut out = Vec::new();
    let mut idx: Vec<usize> = (0..k).collect();
    if k > n {
        return out;
    }
    loop {
        out.push(idx.clone());
        let mut i = k;
        loop {
            if i == 0 {
                return out;
            }
            i -= 1;
            if idx[i] != i + n - k {
                break;
            }
            if i == 0 && idx[0] == n - k {
                return out;
            }
        }
        idx[i] += 1;
        for j in i + 1..k {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

/// The five cards that make the hand (for showing at showdown), Hold'em's way: any five of
/// the hole cards and the board.
pub fn best_five(cards: &[Card]) -> (Value, Vec<Card>) {
    if cards.len() <= 5 {
        return (evaluate(cards), cards.to_vec());
    }
    let mut best = (Value(0), Vec::new());
    for c in combos(cards.len(), 5) {
        let five: Vec<Card> = c.iter().map(|&i| cards[i]).collect();
        let v = evaluate(&five);
        if v > best.0 || best.1.is_empty() {
            best = (v, five);
        }
    }
    best
}

/// Omaha's way: exactly two of the hole cards and three of the board.
pub fn best_omaha(hole: &[Card], board: &[Card]) -> (Value, Vec<Card>) {
    let mut best = (Value(0), Vec::new());
    if board.len() < 3 || hole.len() < 2 {
        return best;
    }
    for h in combos(hole.len(), 2) {
        for b in combos(board.len(), 3) {
            let five: Vec<Card> = h.iter().map(|&i| hole[i]).chain(b.iter().map(|&i| board[i])).collect();
            let v = evaluate(&five);
            if v > best.0 || best.1.is_empty() {
                best = (v, five);
            }
        }
    }
    best
}

/// Omaha's value only (quicker: no card lists).
pub fn omaha_value(hole: &[Card], board: &[Card]) -> Value {
    let mut best = Value(0);
    for (i, &a) in hole.iter().enumerate() {
        for &b in &hole[i + 1..] {
            for x in 0..board.len() {
                for y in x + 1..board.len() {
                    for z in y + 1..board.len() {
                        let v = evaluate_set(CardSet::of(&[a, b, board[x], board[y], board[z]]));
                        if v > best {
                            best = v;
                        }
                    }
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::parse_cards;

    fn v(s: &str) -> Value {
        evaluate(&parse_cards(s).unwrap())
    }

    #[test]
    fn categories() {
        assert_eq!(v("AhKhQhJhTh 2c 3d").describe(), "Royal Flush");
        assert_eq!(v("9h8h7h6h5h Ac Ad").describe(), "Straight Flush, Nine high");
        assert_eq!(v("Ah2h3h4h5h Kc Kd").describe(), "Straight Flush, Five high");
        assert_eq!(v("7c7d7h7s Ah 2c 3d").describe(), "Four of a Kind, Sevens");
        assert_eq!(v("KcKdKh7s7h 2c 3d").describe(), "Full House, Kings over Sevens");
        assert_eq!(v("KcKdKh7s7h7c 2d").describe(), "Full House, Kings over Sevens", "two trips");
        assert_eq!(v("KcKdKh7s7h Qc Qd").describe(), "Full House, Kings over Queens", "trips and two pairs");
        assert_eq!(v("Ah9h7h4h2h Kc Kd").describe(), "Flush, Ace high", "a flush beats a pair");
        assert_eq!(v("Ah9h7h4h2h Ac Ad").describe(), "Flush, Ace high", "and trips");
        assert_eq!(v("Ah9h7h4h2h 9c 9d 9s").describe(), "Four of a Kind, Nines");
        assert_eq!(v("Ah9h7h4h2h Ac Ad 9c").describe(), "Full House, Aces over Nines", "a full house beats a flush");
        assert_eq!(v("Tc9d8h7s6c 2d 2h").describe(), "Straight, Ten high");
        assert_eq!(v("Ac2d3h4s5c Kd Qh").describe(), "Straight, Five high", "the wheel");
        assert_eq!(v("AcKdQhJsTc 2d 3h").describe(), "Straight, Ace high");
        assert_eq!(v("QcKdAh2s3c 7d 8h").category(), Category::HighCard, "no wrap-around straight");
        assert_eq!(v("8c8d8h Ks 2c 4d 6h").describe(), "Three of a Kind, Eights");
        assert_eq!(v("8c8d Kh Ks 2c 2d 6h").describe(), "Two Pair, Kings and Eights");
        assert_eq!(v("AcAd Kh 9s 7c 4d 2h").describe(), "Pair of Aces");
        assert_eq!(v("AcJd 9h 7s 5c 3d 2h").describe(), "Ace High");
    }

    #[test]
    fn ties_break_the_right_way() {
        assert!(v("AcAd Kh 9s 7c") > v("AhAs Qh Js Tc"), "kicker");
        assert!(v("8c8d Kh Ks Ac") > v("8h8s Kd Kc Qc"), "two pair kicker");
        assert!(v("8c8d Kh Ks 2c 2d Ac") == v("8h8s Kd Kc 3c 3d Ad"), "third pair doesn't count");
        assert!(v("Tc9d8h7s6c") > v("9c8d7h6s5c"));
        assert!(v("6c5d4h3s2c") > v("Ac2d3h4s5c"), "six-high beats the wheel");
        assert!(v("Ah9h7h4h3h") > v("Ah9h7h4h2h"), "flush fifth card");
        assert!(v("2c2d2h2s") > v("AcAdAhKsKd"), "quads beat a full house");
        assert!(v("KcKdKh2s2h") > v("QcQdQhAsAh"));
        assert_eq!(v("AhKhQhJhTh 2c 3d"), v("AsKsQsJsTs 4c 5d"), "royal flushes split");
    }

    #[test]
    fn the_five_cards_shown() {
        let (val, five) = best_five(&parse_cards("AhKh 2c 3d QhJhTh").unwrap());
        assert_eq!(val.describe(), "Royal Flush");
        let mut s: Vec<String> = five.iter().map(|c| c.to_string()).collect();
        s.sort();
        assert_eq!(s, ["Ah", "Jh", "Kh", "Qh", "Th"]);
    }

    #[test]
    fn omaha_takes_two_and_three() {
        // Four hearts in hand, one on the board: no flush in Omaha (only two from the hand).
        let hole = parse_cards("AhKhQhJh").unwrap();
        let board = parse_cards("2h 7c 9d 3s 4c").unwrap();
        let (val, _) = best_omaha(&hole, &board);
        assert_eq!(val.category(), Category::HighCard);
        assert_eq!(omaha_value(&hole, &board), val);
        // Board has four to a straight; one card from hand isn't enough.
        let hole = parse_cards("9c 2d 2h 3c").unwrap();
        let board = parse_cards("Th Jd Qs Kc 4h").unwrap();
        let (val, five) = best_omaha(&hole, &board);
        assert_eq!(val.category(), Category::Pair, "{}", val.describe());
        assert_eq!(five.len(), 5);
    }

    #[test]
    fn combos_count() {
        assert_eq!(combos(7, 5).len(), 21);
        assert_eq!(combos(4, 2).len(), 6);
        assert_eq!(combos(5, 3).len(), 10);
        assert_eq!(combos(3, 5).len(), 0);
    }

    #[test]
    fn every_seven_card_hand_agrees_with_the_best_of_its_fives() {
        // The quick valuation against the slow one, over many deals.
        let mut s = crate::fair::Stream::new(crate::fair::seed_from(5));
        for _ in 0..3000 {
            let mut deck: Vec<Card> = Card::all().collect();
            for i in (1..52).rev() {
                deck.swap(i, s.below(i as u32 + 1) as usize);
            }
            let seven = &deck[..7];
            let quick = evaluate(seven);
            let slow = combos(7, 5).iter().map(|c| evaluate(&c.iter().map(|&i| seven[i]).collect::<Vec<_>>())).max().unwrap();
            assert_eq!(quick, slow, "{seven:?}: {} vs {}", quick.describe(), slow.describe());
        }
    }

    #[test]
    fn category_counts_over_all_five_card_hands() {
        // The known counts over all 2,598,960 five-card hands.
        let deck: Vec<Card> = Card::all().collect();
        let mut n = [0u32; 9];
        for a in 0..52 {
            for b in a + 1..52 {
                for c in b + 1..52 {
                    for d in c + 1..52 {
                        for e in d + 1..52 {
                            let v = evaluate_set(CardSet::of(&[deck[a], deck[b], deck[c], deck[d], deck[e]]));
                            n[v.category() as usize] += 1;
                        }
                    }
                }
            }
        }
        assert_eq!(n, [1_302_540, 1_098_240, 123_552, 54_912, 10_200, 5_108, 3_744, 624, 40]);
    }
}
