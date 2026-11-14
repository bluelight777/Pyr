//! Dominó Cubano (0.13.11): Cuban double-nine dominoes for casino chips, at tables of four. In
//! Pairs (partners facing, two against two) or each for themselves (Solo, two to four), ten
//! tiles each; in Solo of three or fewer whoever can't play draws from what's left (the pozo). Whoever holds the highest
//! double leads, with any tile; then each in turn plays a tile that fits an end of the line, or
//! (can't) draws or passes. The hand's over when someone plays their last tile (¡Dominó!: a
//! last tile that fits both ends is a Capicúa, and winning while an opponent never laid a tile a
//! Chiva) or nobody can play (a tranque: the side with the fewest pips wins). Between two sides,
//! the stake can be doubled (¡Doblo!) on one's turn: the other side takes the cube or folds.
//!
//! As 0.13.28 plays it, to the move: its hands play again here exactly (worked out from its
//! hands, `rebuild/measured/domino.md`), so a hand dealt by either checks in the other. The
//! deal is the blackjack tables' kind of fair: the house's seed (its fingerprint shown first),
//! every person's own, and the hand's number (`round_seed`); the tiles shuffled from them.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::fair::Stream;

/// A tile: its two numbers, the smaller first (`[3, 7]`).
pub type Tile = [u8; 2];

/// How many tiles in a double-nine set, in each hand, and how high the stake goes.
pub const TILES: usize = 55;
pub const IN_HAND: usize = 10;
pub const MAX_MULT: u32 = 8;
/// The house's part of a pot, in hundredths.
pub const RAKE_PCT: u64 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Two against two, partners facing; no drawing.
    Pairs,
    /// Each for themselves; whoever can't play draws.
    Solo,
}

/// Which end of the line a tile goes on (the line's first tile is its left).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    Left,
    Right,
}

/// A move, as 0.13.28 writes it (`{"do":"play","tile":[4,7],"end":"left"}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum Move {
    Play { tile: Tile, end: End },
    /// (Solo) a tile from what's left, when nothing fits: the same player moves again.
    Draw,
    Pass,
    /// ¡Doblo!: the stake doubled, if the other side takes it.
    Double,
    Accept,
    Fold,
}

/// How a hand ended. Sides: in Pairs 0 (the first and third seats) and 1; in Solo each player,
/// by their place in the hand (`Hand::seats`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "by", rename_all = "snake_case")]
pub enum Over {
    /// Someone (`player`: their place in the hand) played their last tile.
    Domino { player: u8, capicua: bool, chiva: bool },
    /// Nobody could play: the side(s) with the fewest pips win; each side's pips.
    Blocked { winners: Vec<u8>, pips: Vec<u32> },
    /// A side folded a double: the side that doubled (`winner`) wins the stake as it stood.
    Fold { winner: u8 },
}

/// The seed a hand's tiles are shuffled with: the house's, every person's (by seat) and the
/// hand's number (0.13.28's, `pyr domino table v1`).
pub fn round_seed(house: &[u8; 32], seeds: &[(u8, [u8; 32])], round: u64) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"pyr domino table v1");
    h.update(house);
    let mut seeds = seeds.to_vec();
    seeds.sort_by_key(|s| s.0);
    for (seat, s) in &seeds {
        h.update([*seat]);
        h.update(s);
    }
    h.update(round.to_le_bytes());
    h.finalize().into()
}

/// The set in its order before the shuffle: 0-0, 0-1 ... 0-9, 1-1 ... 9-9.
pub fn set() -> Vec<Tile> {
    (0..10u8).flat_map(|a| (a..10u8).map(move |b| [a, b])).collect()
}

/// The set shuffled (Fisher–Yates, as the cards are).
pub fn shuffled(seed: [u8; 32]) -> Vec<Tile> {
    let mut d = set();
    let mut s = Stream::new(seed);
    for i in (1..d.len()).rev() {
        let j = s.below(i as u32 + 1) as usize;
        d.swap(i, j);
    }
    d
}

pub fn pips(t: Tile) -> u32 {
    (t[0] + t[1]) as u32
}

pub fn fits(t: Tile, n: u8) -> bool {
    t[0] == n || t[1] == n
}

/// A hand being played (or over).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hand {
    pub kind: Kind,
    /// The seats in the hand, in turn order.
    pub seats: Vec<u8>,
    /// Each one's tiles (by place in `seats`), smallest first.
    hands: Vec<Vec<Tile>>,
    /// What's left (Solo draws from its end).
    pool: Vec<Tile>,
    /// The line, each tile turned to meet its neighbours (`[[6,4],[4,7],[7,7]]`).
    pub chain: Vec<Tile>,
    /// Where the first tile laid is in the line.
    pub lead: usize,
    /// Whose turn (place in `seats`); while a double's offered, who answers.
    to_act: Option<usize>,
    pub mult: u32,
    /// Who holds the cube (a side), once a double's been taken; who offered one now.
    pub cube: Option<u8>,
    pub offer: Option<u8>,
    /// Whose turn it was when the double was offered (it carries on once taken).
    doubler: Option<usize>,
    /// Tiles each has laid (by place).
    pub laid: Vec<u8>,
    /// Every move, by seat.
    pub moves: Vec<(u8, Move)>,
    /// Passes in a row.
    passes: usize,
    pub over: Option<Over>,
}

/// Why a move can't be made.
pub const NOT_OPEN: &str = "it isn't your turn, or that move isn't open";

impl Hand {
    /// Deals a hand to `seats` (in turn order) from `seed`: each in turn a tile, ten each.
    pub fn deal(kind: Kind, seats: &[u8], seed: [u8; 32]) -> Hand {
        let d = shuffled(seed);
        let n = seats.len();
        let mut hands: Vec<Vec<Tile>> = (0..n).map(|i| (0..IN_HAND).map(|k| d[i + k * n]).collect()).collect();
        for h in &mut hands {
            h.sort();
        }
        let pool = d[IN_HAND * n..].to_vec();
        // Whoever holds the highest double leads (any tile). (No doubles dealt at all: the
        // heaviest tile's holder.)
        let first = hands
            .iter()
            .enumerate()
            .flat_map(|(i, h)| h.iter().filter(|t| t[0] == t[1]).map(move |t| (t[0], i)))
            .max()
            .map(|(_, i)| i)
            .or_else(|| hands.iter().enumerate().flat_map(|(i, h)| h.iter().map(move |t| ((pips(*t), t[1]), i))).max().map(|(_, i)| i))
            .unwrap_or(0);
        Hand {
            kind,
            seats: seats.to_vec(),
            hands,
            pool,
            chain: Vec::new(),
            lead: 0,
            to_act: Some(first),
            mult: 1,
            cube: None,
            offer: None,
            doubler: None,
            laid: vec![0; n],
            moves: Vec::new(),
            passes: 0,
            over: None,
        }
    }

    pub fn n(&self) -> usize {
        self.seats.len()
    }

    /// Whether whoever can't play draws: in Solo of three or fewer (0.13.28: "three or fewer
    /// draw from the pozo"; four in Solo pass, as Pairs do, seen at El Malecón).
    pub fn draws(&self) -> bool {
        self.kind == Kind::Solo && self.n() <= 3
    }

    /// Two sides (Pairs, or Solo between two): only then can the stake be doubled.
    pub fn two_sides(&self) -> bool {
        self.kind == Kind::Pairs || self.n() == 2
    }

    /// A place's side.
    pub fn side(&self, place: usize) -> u8 {
        match self.kind {
            Kind::Pairs => (place % 2) as u8,
            Kind::Solo => place as u8,
        }
    }

    pub fn sides(&self) -> usize {
        match self.kind {
            Kind::Pairs => 2,
            Kind::Solo => self.n(),
        }
    }

    pub fn place(&self, seat: u8) -> Option<usize> {
        self.seats.iter().position(|s| *s == seat)
    }

    pub fn to_act(&self) -> Option<u8> {
        self.to_act.map(|p| self.seats[p])
    }

    pub fn tiles(&self, place: usize) -> &[Tile] {
        &self.hands[place]
    }

    pub fn pool_left(&self) -> usize {
        self.pool.len()
    }

    pub fn ends(&self) -> Option<(u8, u8)> {
        Some((self.chain.first()?[0], self.chain.last()?[1]))
    }

    /// The plays open to a place: each of its tiles (smallest first) that fits the left end,
    /// then the right (only the left when both ends show the same number); anything, to lead.
    pub fn plays(&self, place: usize) -> Vec<Move> {
        let mut out = Vec::new();
        for &t in &self.hands[place] {
            match self.ends() {
                None => out.push(Move::Play { tile: t, end: End::Left }),
                Some((l, r)) => {
                    if fits(t, l) {
                        out.push(Move::Play { tile: t, end: End::Left });
                    }
                    if l != r && fits(t, r) {
                        out.push(Move::Play { tile: t, end: End::Right });
                    }
                }
            }
        }
        out
    }

    /// What a seat may do now (0.13.28's `allowed`): on its turn, its plays, or (none) a draw
    /// (Solo, while any are left) or a pass; and ¡Doblo! between two sides, once a tile's down,
    /// if the cube's in the middle or its side's, and the stake can go higher. While a double's
    /// offered, the other side (either partner) takes it or folds.
    pub fn allowed(&self, seat: u8) -> Vec<Move> {
        let Some(place) = self.place(seat) else { return Vec::new() };
        if self.over.is_some() {
            return Vec::new();
        }
        if let Some(by) = self.offer {
            return if self.side(place) != by && self.to_act.is_some_and(|p| self.side(p) == self.side(place)) { vec![Move::Accept, Move::Fold] } else { Vec::new() };
        }
        if self.to_act != Some(place) {
            return Vec::new();
        }
        let mut out = self.plays(place);
        if out.is_empty() {
            out.push(if self.draws() && !self.pool.is_empty() { Move::Draw } else { Move::Pass });
        }
        if self.may_double(place) {
            out.push(Move::Double);
        }
        out
    }

    fn may_double(&self, place: usize) -> bool {
        self.two_sides() && !self.chain.is_empty() && self.offer.is_none() && self.cube.is_none_or(|c| c == self.side(place)) && self.mult < MAX_MULT
    }

    /// Makes a move for `seat` (checked: `NOT_OPEN` if it can't).
    pub fn act(&mut self, seat: u8, m: Move) -> Result<(), String> {
        if !self.allowed(seat).contains(&m) {
            return Err(NOT_OPEN.into());
        }
        let place = self.place(seat).ok_or(NOT_OPEN)?;
        self.moves.push((seat, m));
        let n = self.n();
        match m {
            Move::Play { tile, end } => {
                let i = self.hands[place].iter().position(|t| *t == tile).ok_or(NOT_OPEN)?;
                self.hands[place].remove(i);
                let before = self.ends();
                self.lay(tile, end);
                self.laid[place] += 1;
                self.passes = 0;
                if self.hands[place].is_empty() {
                    // (Its two numbers the two ends': 3-7 on 3 and 7. On 7 and 7 a 3-7 isn't, and
                    // nor is the 7-7: a double never is, as 0.13.28 has it.)
                    let capicua = before.is_some_and(|(l, r)| l != r && ((tile[0] == l && tile[1] == r) || (tile[0] == r && tile[1] == l)));
                    let me = self.side(place);
                    let chiva = (0..n).any(|p| self.side(p) != me && self.laid[p] == 0);
                    self.over = Some(Over::Domino { player: place as u8, capicua, chiva });
                    self.to_act = None;
                } else {
                    self.to_act = Some((place + 1) % n);
                }
            }
            Move::Draw => {
                let t = self.pool.pop().ok_or(NOT_OPEN)?;
                let at = self.hands[place].partition_point(|x| *x < t);
                self.hands[place].insert(at, t);
            }
            Move::Pass => {
                self.passes += 1;
                if self.passes >= n {
                    self.block();
                } else {
                    self.to_act = Some((place + 1) % n);
                }
            }
            Move::Double => {
                let me = self.side(place);
                self.offer = Some(me);
                self.doubler = Some(place);
                // (The next in turn on the other side answers; either partner may.)
                self.to_act = (1..n).map(|k| (place + k) % n).find(|p| self.side(*p) != me);
            }
            Move::Accept => {
                self.mult = (self.mult * 2).min(MAX_MULT);
                self.cube = Some(self.side(place));
                self.offer = None;
                self.to_act = self.doubler.take();
            }
            Move::Fold => {
                let winner = self.offer.take().unwrap_or(0);
                self.doubler = None;
                self.over = Some(Over::Fold { winner });
                self.to_act = None;
            }
        }
        Ok(())
    }

    fn lay(&mut self, [a, b]: Tile, end: End) {
        match self.ends() {
            None => {
                self.chain.push([a, b]);
                self.lead = 0;
            }
            Some((l, r)) => match end {
                End::Left => {
                    self.chain.insert(0, if b == l { [a, b] } else { [b, a] });
                    self.lead += 1;
                }
                End::Right => self.chain.push(if a == r { [a, b] } else { [b, a] }),
            },
        }
    }

    /// Nobody can play: each side's pips; the fewest win.
    fn block(&mut self) {
        let mut each = vec![0u32; self.sides()];
        for (p, h) in self.hands.iter().enumerate() {
            each[self.side(p) as usize] += h.iter().map(|t| pips(*t)).sum::<u32>();
        }
        let least = each.iter().copied().min().unwrap_or(0);
        let winners = (0..each.len() as u8).filter(|s| each[*s as usize] == least).collect();
        self.over = Some(Over::Blocked { winners, pips: each });
        self.to_act = None;
    }

    /// The seats that won (empty while it's being played).
    pub fn winners(&self) -> Vec<u8> {
        let sides: Vec<u8> = match &self.over {
            None => return Vec::new(),
            Some(Over::Domino { player, .. }) => vec![self.side(*player as usize)],
            Some(Over::Blocked { winners, .. }) => winners.clone(),
            Some(Over::Fold { winner }) => vec![*winner],
        };
        (0..self.n()).filter(|p| sides.contains(&self.side(*p))).map(|p| self.seats[p]).collect()
    }

    /// The bonus on a win, in hundredths: a Capicúa half again, a Chiva double.
    pub fn bonus_pct(&self) -> u32 {
        match &self.over {
            Some(Over::Domino { capicua, chiva, .. }) => (if *capicua { 50 } else { 0 }) + if *chiva { 100 } else { 0 },
            _ => 0,
        }
    }

    /// Plays a hand again from its seed and moves (as the record says); the hand, or why not.
    pub fn replay(kind: Kind, seats: &[u8], seed: [u8; 32], moves: &[(u8, Move)]) -> Result<Hand, String> {
        let mut h = Hand::deal(kind, seats, seed);
        for (i, (seat, m)) in moves.iter().enumerate() {
            h.act(*seat, *m).map_err(|_| format!("move {} ({seat}: {m:?}) isn't one the hand allowed", i + 1))?;
        }
        Ok(h)
    }
}

/// A hand played out for a person, for their records (experience, achievements, the game's
/// statistics): their stake (the ante times the stake's multiple) and their share.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outcome {
    pub kind: Kind,
    pub ante: u64,
    pub mult: u32,
    /// How it ended (0.13.28's words: "¡Dominó!", "Tranque", "Folded to a double").
    pub how: String,
    pub won: bool,
    pub wagered: u64,
    pub paid: u64,
    pub capicua: bool,
    pub chiva: bool,
    pub bonus_pct: u32,
}

impl Outcome {
    /// The hand in a few words, for the big-win feed (0.13.28's: "¡Dominó! at ×2").
    pub fn headline(&self) -> String {
        if self.mult > 1 { format!("{} at ×{}", self.how, self.mult) } else { self.how.clone() }
    }
}

/// What a hand pays: the pot (every player's ante times the stake), the house's part, and each
/// winner's share (bots' shares stay with the house).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payout {
    pub pot: u64,
    pub rake: u64,
    pub bonus_pct: u32,
    pub mult: u32,
    /// Each seat in the hand: what it's paid.
    pub paid: Vec<(u8, u64)>,
}

pub fn payout(h: &Hand, ante: u64) -> Payout {
    let pot = ante * h.mult as u64 * h.n() as u64;
    let bonus_pct = h.bonus_pct();
    let rake = pot * RAKE_PCT / 100;
    let winners = h.winners();
    let share = if winners.is_empty() { 0 } else { (pot - rake) * (100 + bonus_pct as u64) / 100 / winners.len() as u64 };
    Payout { pot, rake, bonus_pct, mult: h.mult, paid: h.seats.iter().map(|s| (*s, if winners.contains(s) { share } else { 0 })).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_set_is_double_nine() {
        let s = set();
        assert_eq!(s.len(), TILES);
        assert_eq!(s[0], [0, 0]);
        assert_eq!(s[1], [0, 1]);
        assert_eq!(s[10], [1, 1]);
        assert_eq!(s[54], [9, 9]);
    }

    #[test]
    fn plays_fit_the_ends() {
        let mut h = Hand::deal(Kind::Pairs, &[0, 1, 2, 3], crate::fair::seed_from(1));
        let first = h.to_act().unwrap();
        // (Leading: every tile, to the left.)
        assert_eq!(h.allowed(first).len(), IN_HAND);
        let m = h.allowed(first)[0];
        h.act(first, m).unwrap();
        assert_eq!(h.chain.len(), 1);
        let next = h.to_act().unwrap();
        assert_eq!(next, (first + 1) % 4);
        let (l, r) = h.ends().unwrap();
        for m in h.allowed(next) {
            match m {
                Move::Play { tile, end: End::Left } => assert!(fits(tile, l)),
                Move::Play { tile, end: End::Right } => assert!(fits(tile, r) && l != r),
                Move::Pass | Move::Double => {}
                other => panic!("{other:?}"),
            }
        }
    }

    /// Solo: three or fewer draw from the pozo when nothing fits; four pass (0.13.28's, at El
    /// Malecón with four: `draws` false, fifteen left, only plays and passes).
    #[test]
    fn four_in_solo_pass_and_three_draw() {
        for (seats, draws) in [(vec![0u8, 1, 2, 3], false), (vec![0, 1, 2], true), (vec![0, 2], true)] {
            for k in 0..40u64 {
                let mut h = Hand::deal(Kind::Solo, &seats, crate::fair::seed_from(k));
                assert_eq!(h.draws(), draws);
                assert_eq!(h.pool.len(), TILES - IN_HAND * seats.len());
                // Played out by the first move each time: never a draw with four.
                for _ in 0..400 {
                    let Some(p) = h.to_act() else { break };
                    let m = h.allowed(p).into_iter().find(|m| *m != Move::Double).unwrap();
                    if !draws {
                        assert_ne!(m, Move::Draw);
                    }
                    h.act(p, m).unwrap();
                }
                assert!(h.over.is_some());
            }
        }
    }
}
