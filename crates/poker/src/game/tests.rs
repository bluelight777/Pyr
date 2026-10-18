use super::*;
use crate::cards::parse_cards;
use crate::fair::{Stream, seed_from, shuffle};

/// A deck that deals these hole cards (in dealing order: the first player dealt is the small
/// blind, or the button heads-up) and this board; the rest in any order.
fn deck(holes: &[&str], board: &str) -> Vec<Card> {
    let holes: Vec<Vec<Card>> = holes.iter().map(|h| parse_cards(h).unwrap()).collect();
    let board = parse_cards(board).unwrap();
    let k = holes[0].len();
    let mut top = Vec::new();
    for r in 0..k {
        for h in &holes {
            top.push(h[r]);
        }
    }
    let mut rest: Vec<Card> = Card::all().filter(|c| !top.contains(c) && !board.contains(c)).collect();
    let mut burn = || rest.remove(0);
    let mut d = top.clone();
    d.push(burn());
    d.extend(&board[..3]);
    d.push(burn());
    d.push(board[3]);
    d.push(burn());
    d.push(board[4]);
    d.extend(rest);
    assert_eq!(d.len(), 52);
    d
}

fn table(n: usize, stacks: &[u64]) -> Table {
    let mut t = Table::new(Rules::holdem(5, 10, n));
    for (i, &s) in stacks.iter().enumerate() {
        t.sit(i, &format!("p{i}"), &format!("P{i}"), s).unwrap();
    }
    t
}

fn turn(out: &Out) -> Option<usize> {
    out.iter().rev().find_map(|(_, e)| match e {
        Event::Turn { seat, .. } => Some(*seat),
        Event::HandEnded { .. } => Some(usize::MAX),
        _ => None,
    })
}

fn stacks(t: &Table) -> Vec<u64> {
    t.seats.iter().flatten().map(|s| s.stack).collect()
}

#[test]
fn heads_up_the_button_posts_small_and_acts_first() {
    let mut t = table(2, &[1000, 1000]);
    let out = t.start_hand(1, shuffle(seed_from(1))).unwrap();
    assert_eq!(t.button, Some(0));
    assert!(out.iter().any(|(_, e)| *e == Event::Posted { seat: 0, what: Posting::SmallBlind, amount: 5 }));
    assert!(out.iter().any(|(_, e)| *e == Event::Posted { seat: 1, what: Posting::BigBlind, amount: 10 }));
    assert_eq!(turn(&out), Some(0));
    // Hole cards go only to their owner.
    for (to, e) in &out {
        if let Event::Dealt { seat, .. } = e {
            assert_eq!(*to, To::Seat(*seat));
        }
    }
    let out = t.act(0, Action::Fold).unwrap();
    assert!(out.iter().any(|(_, e)| matches!(e, Event::Returned { seat: 1, amount: 5 })), "the uncalled half of the big blind");
    assert!(out.iter().any(|(_, e)| matches!(e, Event::Won { seats, amounts, .. } if seats == &vec![1] && amounts == &vec![10])));
    assert_eq!(stacks(&t), vec![995, 1005]);
    assert!(t.hand.is_none());
    // The button moves.
    t.start_hand(2, shuffle(seed_from(2))).unwrap();
    assert_eq!(t.button, Some(1));
}

#[test]
fn a_hand_to_the_showdown_pays_the_best() {
    let mut t = table(3, &[1000, 1000, 1000]);
    // Button 0; small blind 1, big blind 2: dealt 1, 2, 0.
    let d = deck(&["AhAd", "KcKd", "2c7d"], "As 9h 4c Tc 3s");
    let out = t.start_hand(1, d).unwrap();
    assert_eq!(turn(&out), Some(0), "first to act is after the big blind");
    t.act(0, Action::Call).unwrap();
    t.act(1, Action::Call).unwrap();
    let out = t.act(2, Action::Check).unwrap();
    assert!(out.iter().any(|(_, e)| matches!(e, Event::Street { street: Street::Flop, .. })));
    assert_eq!(turn(&out), Some(1), "after the flop: the first live player left of the button");
    for _ in 0..3 {
        for s in [1, 2, 0] {
            t.act(s, Action::Check).unwrap();
        }
    }
    let won: Vec<_> = t.hand.is_none().then_some(()).into_iter().collect();
    assert_eq!(won.len(), 1);
    assert_eq!(stacks(&t), vec![990, 1020, 990]);
}

#[test]
fn side_pots_go_to_who_could_win_them() {
    let mut t = table(3, &[100, 1000, 1000]);
    // Dealt 1, 2, 0 (button 0). Seat 0 short, with the best hand; seat 1 second best.
    let d = deck(&["KcKd", "QcQd", "AhAd"], "2s 7h 9c 3d 4s");
    t.start_hand(1, d).unwrap();
    t.act(0, Action::AllIn).unwrap();
    t.act(1, Action::Raise { to: 300 }).unwrap();
    let out = t.act(2, Action::Call).unwrap();
    assert_eq!(turn(&out), Some(1));
    // Seat 1 bets, seat 2 folds: seat 1 takes the side pot, seat 0 the main.
    t.act(1, Action::Raise { to: 200 }).unwrap();
    let out = t.act(2, Action::Fold).unwrap();
    let pots: Vec<_> = out.iter().filter_map(|(_, e)| if let Event::Won { seats, amounts, .. } = e { Some((seats.clone(), amounts.clone())) } else { None }).collect();
    // Main: 100 x 3 = 300 to seat 0 (aces); side: 200 + 200 = 400 to seat 1 (not shown down).
    assert!(pots.contains(&(vec![0], vec![300])), "{pots:?}");
    assert!(pots.iter().any(|(s, a)| s == &vec![1] && a == &vec![400]), "{pots:?}");
    assert_eq!(stacks(&t), vec![300, 1000 - 300 + 400 - 0, 1000 - 300]);
    assert_eq!(stacks(&t).iter().sum::<u64>(), 2100);
}

#[test]
fn ties_split_and_the_odd_chip_goes_left_of_the_button() {
    let mut t = Table::new(Rules::holdem(5, 10, 3));
    for i in 0..3 {
        t.sit(i, &format!("p{i}"), "x", 1000).unwrap();
    }
    // Seats 1 and 2 tie with the board's straight; seat 0 folds after putting in 5... make the
    // pot odd: small blind 5 folds, so the pot is 10 + 10 + 5 = 25 between two.
    let d = deck(&["2c3d", "2h3s", "4c4d"], "Th Jd Qs Kc Ah");
    t.start_hand(1, d).unwrap();
    // Dealt 1 (SB), 2 (BB), 0 (button).
    t.act(0, Action::Call).unwrap();
    t.act(1, Action::Fold).unwrap();
    t.act(2, Action::Check).unwrap();
    for _ in 0..3 {
        t.act(2, Action::Check).unwrap();
        t.act(0, Action::Check).unwrap();
    }
    assert!(t.hand.is_none());
    // 25 between seats 2 and 0: the odd chip to seat 2 (first left of the button, seat 0... after 1 who folded).
    let s = stacks(&t);
    assert_eq!(s[1], 995);
    assert_eq!(s[0] + s[2], 2005);
    assert_eq!(s[2], 1003, "{s:?}");
    assert_eq!(s[0], 1002);
}

#[test]
fn no_limit_raises() {
    let mut t = table(3, &[1000, 1000, 1000]);
    t.start_hand(1, shuffle(seed_from(3))).unwrap();
    let l = t.legal(0).unwrap();
    assert_eq!((l.call, l.min_to, l.max_to), (10, 20, 1000));
    assert!(matches!(t.act(0, Action::Raise { to: 15 }), Err(Error::Illegal(_))));
    assert!(matches!(t.act(1, Action::Call), Err(Error::NotYourTurn)));
    t.act(0, Action::Raise { to: 40 }).unwrap();
    let l = t.legal(1).unwrap();
    assert_eq!((l.call, l.min_to), (35, 70), "a re-raise is at least the last raise (30) more");
    assert!(matches!(t.act(1, Action::Raise { to: 1001 }), Err(Error::Illegal(_))));
}

#[test]
fn an_all_in_for_less_doesnt_reopen_the_betting() {
    let mut t = table(3, &[1000, 1000, 55]);
    t.start_hand(1, shuffle(seed_from(4))).unwrap();
    // Dealt 1 (SB), 2 (BB, 55), 0 first.
    t.act(0, Action::Raise { to: 40 }).unwrap();
    t.act(1, Action::Call).unwrap();
    // Big blind all-in to 55: a raise of 15, less than the 30 full raise.
    t.act(2, Action::AllIn).unwrap();
    let l = t.legal(0).unwrap();
    assert_eq!(l.call, 15);
    assert!(!l.can_raise, "they already acted, and the raise wasn't a full one");
    t.act(0, Action::Call).unwrap();
    let l = t.legal(1).unwrap();
    assert!(!l.can_raise);
}

#[test]
fn pot_limit_tops_out_at_the_pot() {
    let mut t = Table::new(Rules { limit: Limit::PotLimit, ..Rules::holdem(5, 10, 3) });
    for i in 0..3 {
        t.sit(i, &format!("p{i}"), "x", 1000).unwrap();
    }
    t.start_hand(1, shuffle(seed_from(5))).unwrap();
    // First to act faces 10: call 10 makes the pot 25; raise by 25 to 35.
    let l = t.legal(0).unwrap();
    assert_eq!(l.max_to, 35);
    t.act(0, Action::Raise { to: 35 }).unwrap();
    // Small blind: pot 50, to call 30 makes 80; raise by 80 to 115.
    assert_eq!(t.legal(1).unwrap().max_to, 115);
}

#[test]
fn fixed_limit_stops_at_four_bets() {
    let mut t = Table::new(Rules { limit: Limit::FixedLimit, ..Rules::holdem(5, 10, 2) });
    t.sit(0, "a", "A", 1000).unwrap();
    t.sit(1, "b", "B", 1000).unwrap();
    t.start_hand(1, shuffle(seed_from(6))).unwrap();
    let l = t.legal(0).unwrap();
    assert_eq!((l.min_to, l.max_to), (20, 20));
    t.act(0, Action::Raise { to: 20 }).unwrap();
    t.act(1, Action::Raise { to: 30 }).unwrap();
    t.act(0, Action::Raise { to: 40 }).unwrap();
    let l = t.legal(1).unwrap();
    assert!(!l.can_raise, "the big blind, a raise and two more: capped");
    t.act(1, Action::Call).unwrap();
    // The turn's bets are twice the size.
    t.act(1, Action::Check).unwrap();
    t.act(0, Action::Check).unwrap();
    assert_eq!(t.legal(1).unwrap().min_to, 20 - 10 + 10);
}

#[test]
fn everyone_all_in_runs_the_board_out_with_the_cards_up() {
    let mut t = table(2, &[100, 100]);
    let out = t.start_hand(1, deck(&["AhAd", "KcKd"], "2s 7h 9c 3d 4s")).unwrap();
    assert_eq!(turn(&out), Some(0));
    t.act(0, Action::AllIn).unwrap();
    let out = t.act(1, Action::Call).unwrap();
    let shown = out.iter().filter(|(_, e)| matches!(e, Event::Shown { .. })).count();
    let first_shown = out.iter().position(|(_, e)| matches!(e, Event::Shown { .. })).unwrap();
    let river = out.iter().position(|(_, e)| matches!(e, Event::Street { street: Street::River, .. })).unwrap();
    assert_eq!(shown, 2, "shown once");
    assert!(first_shown < river, "shown before the board runs out");
    assert_eq!(stacks(&t), vec![200, 0]);
}

#[test]
fn omaha_deals_four_and_plays_two() {
    let mut t = Table::new(Rules { variant: Variant::Omaha, limit: Limit::PotLimit, ..Rules::holdem(5, 10, 2) });
    t.sit(0, "a", "A", 1000).unwrap();
    t.sit(1, "b", "B", 1000).unwrap();
    // Seat 0: four hearts (one heart on board: no flush). Seat 1: a pair of twos and a nine.
    let out = t.start_hand(1, deck(&["AhKhQhJh", "2c2d9s3c"], "2h 7c 9d 3s 4c")).unwrap();
    let mine = out.iter().find_map(|(_, e)| if let Event::Dealt { seat: 0, cards } = e { Some(cards.len()) } else { None });
    assert_eq!(mine, Some(4));
    t.act(0, Action::Call).unwrap();
    t.act(1, Action::Check).unwrap();
    for _ in 0..3 {
        t.act(1, Action::Check).unwrap();
        t.act(0, Action::Check).unwrap();
    }
    assert_eq!(stacks(&t), vec![990, 1010], "seat 1's set of twos beats seat 0's ace high");
}

/// Thousands of hands with random (legal) play: chips never appear or vanish, every hand ends,
/// and nobody's asked to act who can't.
#[test]
fn random_play_keeps_every_chip() {
    let mut rng = Stream::new(seed_from(77));
    for game in 0..300u64 {
        let n = 2 + rng.below(8) as usize;
        let limit = [Limit::NoLimit, Limit::PotLimit, Limit::FixedLimit][rng.below(3) as usize];
        let variant = if rng.below(4) == 0 { Variant::Omaha } else { Variant::Holdem };
        let mut t = Table::new(Rules { variant, limit, small_blind: 5, big_blind: 10, ante: if rng.below(3) == 0 { 2 } else { 0 }, seats: n });
        for i in 0..n {
            t.sit(i, &format!("p{i}"), "x", 50 + rng.below(1000) as u64).unwrap();
        }
        let total: u64 = stacks(&t).iter().sum();
        for hand in 0..40u64 {
            if t.ready().len() < 2 {
                break;
            }
            let out = t.start_hand(hand, shuffle(seed_from(game * 1000 + hand))).unwrap();
            let mut next = turn(&out);
            let mut steps = 0;
            while let Some(seat) = next.filter(|&s| s != usize::MAX) {
                steps += 1;
                assert!(steps < 200, "a hand that doesn't end");
                let l = t.legal(seat).expect("asked to act but can't");
                let a = match rng.below(10) {
                    0 => Action::Fold,
                    1..=4 => Action::Call,
                    5 if l.can_check => Action::Check,
                    6 | 7 if l.can_raise => Action::Raise { to: l.min_to + rng.below((l.max_to - l.min_to + 1) as u32) as u64 },
                    8 if l.can_raise => Action::AllIn,
                    _ => Action::Call,
                };
                let out = t.act(seat, a).unwrap_or_else(|e| panic!("{a:?} with {l:?}: {e}"));
                next = turn(&out);
            }
            assert!(t.hand.is_none(), "the hand ended");
            assert_eq!(stacks(&t).iter().sum::<u64>(), total, "game {game} hand {hand}");
        }
    }
}
