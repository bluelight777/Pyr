//! The Bonfire: Pyr Casino's giant prize pool, given away by raffle.
//!
//! It starts big ([`SEED_POOL`]) and grows by a cut of every chip bet in the casino (out of the
//! house's take, never anyone's winnings). Every round anyone plays, in any game, throws embers
//! on the fire: one for the round, and one more for every [`EMBER_CHIPS`] bet. Nobody has to sign
//! up. Every so often the Bonfire's lit: fourteen embers are drawn, each the chance of its owner's
//! share of all the embers, and nobody wins twice; the Grand Ember takes half the pool, three
//! Blazes a tenth each, ten Sparks a fiftieth each. What nobody won goes on the next fire.
//!
//! It's drawn fairly: the house fixes its seed before the fire's first ember and shows its
//! fingerprint all the while; when it's lit, it shows the seed and everyone's embers, and anyone's
//! Pyr can draw it again (`check`).

use serde::{Deserialize, Serialize};

use crate::fair::{self, Stream};

/// What a new fire starts at, before anyone's played.
pub const SEED_POOL: u64 = 1_000_000;
/// The house's cut of every chip bet that goes on the fire (basis points: 1%).
pub const CUT_BP: u64 = 100;
/// Chips bet for each ember past the round's own.
pub const EMBER_CHIPS: u64 = 100;
/// How long a fire burns before it's lit.
pub const BURN_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Half the pool.
    Grand,
    /// A tenth each, three of them.
    Blaze,
    /// A fiftieth each, ten of them.
    Spark,
}

impl Tier {
    pub fn name(self) -> &'static str {
        match self {
            Tier::Grand => "Grand Ember",
            Tier::Blaze => "Blaze",
            Tier::Spark => "Spark",
        }
    }
}

/// The prizes, in the order they're drawn, each its share of the pool (basis points).
pub const PRIZES: [(Tier, u64); 14] = [
    (Tier::Grand, 5_000),
    (Tier::Blaze, 1_000),
    (Tier::Blaze, 1_000),
    (Tier::Blaze, 1_000),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
    (Tier::Spark, 200),
];

/// The embers a round throws on the fire: one for playing, one more for every 100 chips bet.
pub fn embers_for(wagered: u64) -> u64 {
    1 + wagered / EMBER_CHIPS
}

/// The pool's cut of a bet.
pub fn cut(wagered: u64) -> u64 {
    wagered.saturating_mul(CUT_BP) / 10_000
}

/// A prize drawn: which, whose (an index into the entries), and the chips it pays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick {
    pub tier: Tier,
    pub entry: usize,
    pub chips: u64,
}

/// A number below `n` (above 0), evenly.
fn below(s: &mut Stream, n: u64) -> u64 {
    let zone = u64::MAX - u64::MAX % n;
    loop {
        let r = ((s.next_u32() as u64) << 32) | s.next_u32() as u64;
        if r < zone {
            return r % n;
        }
    }
}

/// The draw, from the house's seed and everyone's embers (`entries`: each one's embers, in
/// the order of their ids): each prize goes to an ember drawn from all those still in, its owner
/// out of the rest of the draw. With fewer people than prizes, the last prizes go unwon.
pub fn draw(seed: &[u8; 32], embers: &[u64], pool: u64) -> Vec<Pick> {
    let mut s = Stream::new(fair::derive(seed, "pyr bonfire v1"));
    let mut left: Vec<usize> = (0..embers.len()).filter(|i| embers[*i] > 0).collect();
    let mut out = Vec::new();
    for (tier, bp) in PRIZES {
        let total: u64 = left.iter().map(|i| embers[*i]).sum();
        if total == 0 {
            break;
        }
        let mut x = below(&mut s, total);
        let mut k = 0;
        for (n, i) in left.iter().enumerate() {
            if x < embers[*i] {
                k = n;
                break;
            }
            x -= embers[*i];
        }
        let entry = left.remove(k);
        out.push(Pick { tier, entry, chips: pool.saturating_mul(bp) / 10_000 });
    }
    out
}

/// What a draw leaves unwon (prizes with nobody left for them, and the rounding).
pub fn rollover(pool: u64, picks: &[Pick]) -> u64 {
    pool.saturating_sub(picks.iter().map(|p| p.chips).sum())
}

/// A fire as it's lit: everyone's entries (in the order of their ids) and the winners.
pub fn lit(round: u64, at_ms: u64, seed: [u8; 32], pool: u64, entries: Vec<crate::net::BonfireEntry>) -> crate::net::BonfireDraw {
    let mut entries = entries;
    entries.sort_by(|a, b| a.uid.cmp(&b.uid));
    let embers: Vec<u64> = entries.iter().map(|e| e.embers).collect();
    let picks = draw(&seed, &embers, pool);
    let winners = picks
        .iter()
        .map(|p| crate::net::BonfireWinner { tier: p.tier, uid: entries[p.entry].uid.clone(), name: entries[p.entry].name.clone(), chips: p.chips, embers: entries[p.entry].embers })
        .collect();
    crate::net::BonfireDraw { round, at_ms, pool, commit: fair::commit(&seed), seed, rollover: rollover(pool, &picks), entries, winners }
}

/// Checks a fire that was lit: its seed is the one fingerprinted all along, and drawn again from
/// it and everyone's embers, the same people won the same prizes.
pub fn check(d: &crate::net::BonfireDraw, commit_seen: Option<[u8; 32]>) -> Result<(), String> {
    if fair::commit(&d.seed) != d.commit || commit_seen.is_some_and(|c| c != d.commit) {
        return Err(format!("Bonfire {}: the seed doesn't match the fingerprint shown before it was lit", d.round));
    }
    if d.entries.windows(2).any(|w| w[0].uid >= w[1].uid) {
        return Err(format!("Bonfire {}: the entries are out of order", d.round));
    }
    let again = lit(d.round, d.at_ms, d.seed, d.pool, d.entries.clone());
    if again.winners != d.winners || again.rollover != d.rollover {
        return Err(format!("Bonfire {}: drawn again, different embers won", d.round));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fair::seed_from;

    #[test]
    fn the_prizes_share_the_whole_pool() {
        let total: u64 = PRIZES.iter().map(|p| p.1).sum();
        assert_eq!(total, 10_000);
        assert_eq!(embers_for(0), 1);
        assert_eq!(embers_for(99), 1);
        assert_eq!(embers_for(250), 3);
        assert_eq!(cut(10_000), 100);
    }

    /// Fourteen winners, nobody twice; the same seed draws the same; each prize goes about as
    /// often as its owner's share of the embers.
    #[test]
    fn embers_are_chances_and_nobody_wins_twice() {
        let embers: Vec<u64> = (0..40).map(|i| 1 + i * 3).collect();
        let picks = draw(&seed_from(1), &embers, 2_000_000);
        assert_eq!(picks.len(), 14);
        let mut who: Vec<usize> = picks.iter().map(|p| p.entry).collect();
        who.sort();
        who.dedup();
        assert_eq!(who.len(), 14, "nobody twice");
        assert_eq!(picks[0].tier, Tier::Grand);
        assert_eq!(picks[0].chips, 1_000_000);
        assert_eq!(picks, draw(&seed_from(1), &embers, 2_000_000));
        assert_eq!(rollover(2_000_000, &picks), 0);
        // The Grand Ember, over many fires: someone with a third of the embers wins it about a
        // third of the time.
        let embers = [10u64, 20, 30, 0, 30];
        let mut grand = [0u32; 5];
        for n in 0..30_000 {
            grand[draw(&seed_from(n), &embers, 100)[0].entry] += 1;
        }
        assert_eq!(grand[3], 0, "no embers, no chance");
        for (i, share) in [(0, 10.0 / 90.0), (1, 20.0 / 90.0), (2, 30.0 / 90.0), (4, 30.0 / 90.0)] {
            let got = grand[i] as f64 / 30_000.0;
            assert!((got - share).abs() < 0.012, "{i}: {got} vs {share}");
        }
    }

    /// A fire lit checks out; one with a winner changed doesn't.
    #[test]
    fn a_draw_checks_out_and_a_changed_one_doesnt() {
        let entries: Vec<crate::net::BonfireEntry> = ["zed", "amy", "bo", "cy"].iter().enumerate().map(|(i, u)| crate::net::BonfireEntry { uid: u.to_string(), name: u.to_uppercase(), embers: 10 * (i as u64 + 1) }).collect();
        let d = lit(4, 0, seed_from(8), 1_500_000, entries);
        assert_eq!(d.entries[0].uid, "amy", "in id order");
        assert_eq!(d.winners.len(), 4);
        assert_eq!(d.rollover, 1_500_000 - 1_500_000 / 2 - 3 * 150_000);
        check(&d, Some(d.commit)).unwrap();
        let mut bad = d.clone();
        bad.winners.swap(0, 1);
        assert!(check(&bad, None).is_err());
        let mut bad = d.clone();
        bad.seed[0] ^= 1;
        assert!(check(&bad, None).is_err());
        assert!(check(&d, Some([0; 32])).is_err());
    }

    /// With fewer people than prizes, the rest goes on the next fire.
    #[test]
    fn what_nobody_wins_rolls_over() {
        let picks = draw(&seed_from(3), &[5, 0, 7], 1_000_000);
        assert_eq!(picks.len(), 2);
        assert_eq!(picks[0].tier, Tier::Grand);
        assert_eq!(picks[1].tier, Tier::Blaze);
        assert_eq!(rollover(1_000_000, &picks), 400_000);
        assert!(draw(&seed_from(3), &[], 1_000_000).is_empty());
    }
}
