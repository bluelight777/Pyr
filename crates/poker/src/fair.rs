//! A shuffle anyone can check.
//!
//! Before a hand the dealer (the Pyr server hosting the table) picks a secret seed and shows
//! only its fingerprint (`commit`). Each player's Pyr then adds a seed of its own. The deck is
//! shuffled from all of them together, so the dealer can't pick the cards (its seed was fixed
//! before it saw the players'), and no player can either (they never see the dealer's seed
//! until the hand is over). After the hand the dealer shows its seed: every player's Pyr checks
//! it matches the fingerprint, that its own seed was used, and shuffles the deck again to see
//! that every card dealt was the one the shuffle says.
//!
//! The random numbers are SHA-256 in counter mode, and each card is picked with rejection
//! sampling, so every order of the deck is equally likely.

use sha2::{Digest, Sha256};

use crate::cards::Card;

/// What the dealer shows before a hand: the fingerprint of its secret seed.
pub fn commit(dealer_seed: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr-poker commit v1");
    h.update(dealer_seed);
    h.finalize().into()
}

/// The seed the deck is shuffled with: the dealer's, every player's (in a fixed order, so it
/// doesn't matter who sent theirs first) and the hand's number.
pub fn deck_seed(dealer_seed: &[u8; 32], player_seeds: &[[u8; 32]], hand: u64) -> [u8; 32] {
    let mut ps = player_seeds.to_vec();
    ps.sort();
    ps.dedup();
    let mut h = Sha256::new();
    h.update(b"pyr-poker deck v1");
    h.update(dealer_seed);
    h.update((ps.len() as u32).to_le_bytes());
    for p in &ps {
        h.update(p);
    }
    h.update(hand.to_le_bytes());
    h.finalize().into()
}

/// The fingerprint of the players' seeds used (sent with the deal, so each player can see theirs
/// was counted before any card is turned over).
pub fn seeds_fingerprint(player_seeds: &[[u8; 32]]) -> [u8; 32] {
    let mut ps = player_seeds.to_vec();
    ps.sort();
    ps.dedup();
    let mut h = Sha256::new();
    h.update(b"pyr-poker seeds v1");
    for p in &ps {
        h.update(p);
    }
    h.finalize().into()
}

/// Random numbers from a seed (SHA-256 in counter mode).
pub struct Stream {
    seed: [u8; 32],
    counter: u64,
    block: [u8; 32],
    used: usize,
}

impl Stream {
    pub fn new(seed: [u8; 32]) -> Stream {
        Stream { seed, counter: 0, block: [0; 32], used: 32 }
    }

    pub fn next_u32(&mut self) -> u32 {
        if self.used + 4 > 32 {
            let mut h = Sha256::new();
            h.update(self.seed);
            h.update(self.counter.to_le_bytes());
            self.block = h.finalize().into();
            self.counter += 1;
            self.used = 0;
        }
        let v = u32::from_le_bytes(self.block[self.used..self.used + 4].try_into().unwrap());
        self.used += 4;
        v
    }

    /// A number from 0 to `n - 1`, each as likely as the others (rejection sampling).
    pub fn below(&mut self, n: u32) -> u32 {
        assert!(n > 0);
        let zone = u32::MAX - (u32::MAX % n);
        loop {
            let v = self.next_u32();
            if v < zone {
                return v % n;
            }
        }
    }
}

/// The deck in the order it's dealt from (Fisher–Yates).
pub fn shuffle(seed: [u8; 32]) -> Vec<Card> {
    let mut deck: Vec<Card> = Card::all().collect();
    let mut s = Stream::new(seed);
    for i in (1..deck.len()).rev() {
        let j = s.below(i as u32 + 1) as usize;
        deck.swap(i, j);
    }
    deck
}

/// Checks a hand that's over: the dealer's seed matches what it showed first, and the deck it
/// makes deals `dealt` (cards in the order they came off the deck, `None` for ones nobody saw).
pub fn verify(commitment: &[u8; 32], dealer_seed: &[u8; 32], player_seeds: &[[u8; 32]], hand: u64, dealt: &[Option<Card>]) -> Result<(), String> {
    if &commit(dealer_seed) != commitment {
        return Err("the dealer's seed isn't the one it promised before the hand".into());
    }
    let deck = shuffle(deck_seed(dealer_seed, player_seeds, hand));
    for (i, d) in dealt.iter().enumerate() {
        if let Some(c) = d
            && deck.get(i) != Some(c)
        {
            return Err(format!("card {} dealt was {c}, but the shuffle says {}", i + 1, deck.get(i).map(|c| c.to_string()).unwrap_or_default()));
        }
    }
    Ok(())
}

/// A seed from the system's random number generator would come from the caller (this crate
/// has no randomness of its own, so everything in it can be replayed); this is a convenience
/// for tests and bots: a seed made from a number.
pub fn seed_from(n: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr-poker test seed");
    h.update(n.to_le_bytes());
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shuffle_is_a_deck() {
        let d = shuffle(seed_from(1));
        assert_eq!(d.len(), 52);
        let mut s = d.clone();
        s.sort();
        s.dedup();
        assert_eq!(s.len(), 52);
        assert_eq!(d, shuffle(seed_from(1)), "the same seed, the same deck");
        assert_ne!(d, shuffle(seed_from(2)));
    }

    #[test]
    fn every_seed_counts_and_order_doesnt() {
        let dealer = seed_from(7);
        let (a, b) = (seed_from(8), seed_from(9));
        assert_eq!(deck_seed(&dealer, &[a, b], 1), deck_seed(&dealer, &[b, a], 1));
        assert_ne!(deck_seed(&dealer, &[a, b], 1), deck_seed(&dealer, &[a], 1));
        assert_ne!(deck_seed(&dealer, &[a, b], 1), deck_seed(&dealer, &[a, b], 2));
        assert_eq!(seeds_fingerprint(&[a, b]), seeds_fingerprint(&[b, a]));
    }

    #[test]
    fn a_hand_checks_out_or_doesnt() {
        let dealer = seed_from(3);
        let players = [seed_from(4), seed_from(5)];
        let c = commit(&dealer);
        let deck = shuffle(deck_seed(&dealer, &players, 42));
        let dealt: Vec<Option<Card>> = deck.iter().take(9).enumerate().map(|(i, c)| (i % 2 == 0).then_some(*c)).collect();
        assert!(verify(&c, &dealer, &players, 42, &dealt).is_ok());
        assert!(verify(&c, &seed_from(99), &players, 42, &dealt).unwrap_err().contains("promised"));
        assert!(verify(&c, &dealer, &players[..1], 42, &dealt).is_err(), "a player's seed left out");
        let mut swapped = dealt.clone();
        swapped[0] = Some(deck[20]);
        assert!(verify(&c, &dealer, &players, 42, &swapped).unwrap_err().contains("card 1"));
    }

    #[test]
    fn every_card_is_as_likely_in_every_place() {
        // 52 places x 52 cards; 20,000 shuffles: each count should be near 20000/52 = 385.
        let mut counts = vec![[0u32; 52]; 52];
        for n in 0..20_000 {
            for (pos, c) in shuffle(seed_from(n)).iter().enumerate() {
                counts[pos][c.index() as usize] += 1;
            }
        }
        let expected = 20_000.0 / 52.0;
        // Chi-squared over all 2704 cells, 2601 degrees of freedom: the mean is 2601 and the
        // standard deviation about 72; 3000 is over five deviations out.
        let chi: f64 = counts.iter().flatten().map(|&o| (o as f64 - expected).powi(2) / expected).sum();
        assert!(chi < 3000.0, "chi-squared {chi}");
    }

    #[test]
    fn below_is_even() {
        let mut s = Stream::new(seed_from(11));
        let mut c = [0u32; 3];
        for _ in 0..30_000 {
            c[s.below(3) as usize] += 1;
        }
        assert!(c.iter().all(|&n| (9_500..10_500).contains(&n)), "{c:?}");
    }
}
