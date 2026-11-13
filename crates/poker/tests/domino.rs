//! 0.13.28's dominó hands (filmed off its server, `tests/domino/hands-0.13.28.json`) play again
//! here exactly: the deal from their seeds, every move allowed, the line, what each had left,
//! how it ended and what it paid.

use pyr_poker::casino::domino::{self, Hand, Kind, Move, Over, Tile};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    table: String,
    ante: u64,
    record: Record,
    end: End,
    result: Result_,
}

#[derive(Deserialize)]
struct Record {
    round: u64,
    commit: String,
    house: String,
    seeds: Vec<(u8, String)>,
    kind: Kind,
    seats: Vec<u8>,
    moves: Vec<(u8, Move)>,
    over: Over,
}

#[derive(Deserialize)]
struct End {
    seats: Vec<u8>,
    counts: Vec<u8>,
    pool: usize,
    chain: Vec<Tile>,
    lead: usize,
    laid: Vec<u8>,
    shown: Vec<Vec<Tile>>,
    over: Over,
    mult: u32,
    cube: Option<u8>,
    moves: usize,
}

#[derive(Deserialize)]
struct Result_ {
    winners: Vec<u8>,
    pot: u64,
    rake: u64,
    bonus_pct: u32,
    mult: u32,
    paid: Vec<(u8, u64)>,
}

fn hex32(s: &str) -> [u8; 32] {
    let mut b = [0u8; 32];
    for i in 0..32 {
        b[i] = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap();
    }
    b
}

#[test]
fn hands_dealt_by_0_13_28_play_again_exactly() {
    let all: Vec<Fixture> = serde_json::from_str(include_str!("domino/hands-0.13.28.json")).unwrap();
    assert!(all.len() >= 20);
    for f in &all {
        let r = &f.record;
        let what = format!("{} round {}", f.table, r.round);
        assert_eq!(pyr_poker::fair::commit(&hex32(&r.house)), hex32(&r.commit), "{what}: the house's seed");
        let seeds: Vec<(u8, [u8; 32])> = r.seeds.iter().map(|(s, h)| (*s, hex32(h))).collect();
        let h = Hand::replay(r.kind, &r.seats, domino::round_seed(&hex32(&r.house), &seeds, r.round), &r.moves).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert_eq!(h.over.as_ref(), Some(&r.over), "{what}: how it ended");
        assert_eq!(h.over.as_ref(), Some(&f.end.over), "{what}");
        assert_eq!(h.seats, f.end.seats, "{what}");
        assert_eq!(h.chain, f.end.chain, "{what}: the line");
        assert_eq!(h.lead, f.end.lead, "{what}: the lead tile's place");
        assert_eq!(h.laid, f.end.laid, "{what}: tiles laid");
        assert_eq!(h.mult, f.end.mult, "{what}");
        assert_eq!(h.cube, f.end.cube, "{what}");
        assert_eq!(h.moves.len(), f.end.moves, "{what}");
        assert_eq!(h.pool_left(), f.end.pool, "{what}: what's left");
        for p in 0..h.n() {
            assert_eq!(h.tiles(p).len(), f.end.counts[p] as usize, "{what}: place {p}'s count");
            let mut shown = f.end.shown[p].clone();
            shown.sort();
            assert_eq!(h.tiles(p), &shown[..], "{what}: place {p}'s tiles");
        }
        assert_eq!(h.winners(), f.result.winners, "{what}: who won");
        let pay = domino::payout(&h, f.ante);
        assert_eq!((pay.pot, pay.rake, pay.bonus_pct, pay.mult), (f.result.pot, f.result.rake, f.result.bonus_pct, f.result.mult), "{what}: the pot");
        assert_eq!(pay.paid, f.result.paid, "{what}: paid");
    }
}
