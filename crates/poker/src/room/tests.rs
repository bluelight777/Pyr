use super::*;
use crate::fair::seed_from;
use crate::game::Rules;

fn room() -> Room {
    Room::new(1, "Test", "alice", Rules::holdem(5, 10, 6), BuyIn::default()).unwrap()
}

/// Runs the clock in steps, with `people` acting (by `choose`) when it's their turn.
fn run(r: &mut Room, rng: &mut Stream, from: u64, until: u64, choose: &mut dyn FnMut(&Room, &str) -> Option<Action>, log: &mut Vec<(Rcpt, ToApp)>, effects: &mut Vec<Effect>) -> u64 {
    let mut now = from;
    while now < until {
        let out = r.tick(now, rng);
        log.extend(out.msgs);
        effects.extend(out.effects);
        // People whose turn it is.
        if let Some(seat) = r.table.hand.as_ref().and_then(|h| h.to_act)
            && let Some(uid) = r.uid_at(seat).map(|s| s.to_string())
            && !uid.starts_with("bot:")
            && let Some(a) = choose(r, &uid)
        {
            let hand = r.table.hand.as_ref().unwrap().number;
            let out = r.act(&uid, hand, a, now).unwrap();
            log.extend(out.msgs);
            effects.extend(out.effects);
        }
        now += 250;
    }
    now
}

#[test]
fn people_and_bots_play_and_every_chip_is_kept() {
    let mut r = room();
    let mut rng = Stream::new(seed_from(1));
    let look = Look::default();
    r.sit("alice", "Alice", 0, 1000, look.clone(), 0).unwrap();
    r.sit("bob", "Bob", 2, 1500, look.clone(), 0).unwrap();
    r.add_bot(3, Level::Shark, 0, &mut rng).unwrap();
    r.add_bot(5, Level::Rookie, 0, &mut rng).unwrap();
    let mut log = Vec::new();
    let mut effects = Vec::new();
    let mut choose = |r: &Room, uid: &str| {
        let seat = r.seat_of(uid)?;
        let l = r.table.legal(seat)?;
        Some(if l.can_check { Action::Check } else if l.call <= 40 { Action::Call } else { Action::Fold })
    };
    run(&mut r, &mut rng, 0, 600_000, &mut choose, &mut log, &mut effects);
    let hands = effects.iter().filter(|e| matches!(e, Effect::Hand { uid, .. } if uid == "alice")).count();
    assert!(hands > 20, "{hands} hands in ten minutes");
    // People's chips: at the table and nothing else (no credits: nobody left).
    assert!(effects.iter().all(|e| !matches!(e, Effect::Credit { .. })));
    // Every reveal checks out against the promise made before the hand.
    let mut promised: HashMap<u64, [u8; 32]> = HashMap::new();
    let mut seeds: HashMap<u64, Vec<[u8; 32]>> = HashMap::new();
    let mut checked = 0;
    for (_, m) in &log {
        match m {
            ToApp::Shuffle { hand, commit, .. } => {
                promised.insert(*hand, *commit);
            }
            ToApp::Seeds { hand, seeds: s, .. } => {
                seeds.insert(*hand, s.clone());
            }
            ToApp::Reveal { hand, dealer, dealt, .. } => {
                fair::verify(&promised[hand], dealer, &seeds[hand], *hand, dealt).unwrap();
                checked += 1;
            }
            _ => {}
        }
    }
    assert!(checked > 20);
    // Hole cards only ever go to their owner.
    for (to, m) in &log {
        if let ToApp::Event { event: Event::Dealt { seat, .. }, .. } = m {
            let owner = if *seat == 0 { "alice" } else { "bob" };
            assert_eq!(to, &Rcpt::User(owner.into()));
        }
    }
}

#[test]
fn a_seed_from_a_player_goes_into_the_shuffle() {
    let mut r = room();
    let mut rng = Stream::new(seed_from(2));
    r.sit("alice", "Alice", 0, 1000, Look::default(), 0).unwrap();
    r.sit("bob", "Bob", 1, 1000, Look::default(), 0).unwrap();
    let mut now = 0;
    let mut hand = None;
    while hand.is_none() {
        for (_, m) in r.tick(now, &mut rng).msgs {
            if let ToApp::Shuffle { hand: h, .. } = m {
                hand = Some(h);
            }
        }
        now += 250;
    }
    let h = hand.unwrap();
    r.seed("alice", h, [1; 32]);
    r.seed("bob", h, [2; 32]);
    // Both in: dealt at once, without waiting out the shuffle time.
    let out = r.tick(now, &mut rng);
    let s = out.msgs.iter().find_map(|(_, m)| if let ToApp::Seeds { seeds, .. } = m { Some(seeds.clone()) } else { None }).unwrap();
    assert_eq!(s.len(), 2);
    assert!(s.contains(&[1; 32]) && s.contains(&[2; 32]));
    assert!(r.table.hand.is_some());
}

#[test]
fn time_runs_out_into_the_bank_then_checks_or_folds_and_sits_out() {
    let mut r = room();
    let mut rng = Stream::new(seed_from(3));
    r.sit("alice", "Alice", 0, 1000, Look::default(), 0).unwrap();
    r.sit("bob", "Bob", 1, 1000, Look::default(), 0).unwrap();
    let mut log = Vec::new();
    let mut effects = Vec::new();
    // Bob never acts; Alice calls everything.
    let mut choose = |r: &Room, uid: &str| (uid == "alice").then(|| if r.table.legal(0).unwrap().can_check { Action::Check } else { Action::Call });
    run(&mut r, &mut rng, 0, 400_000, &mut choose, &mut log, &mut effects);
    let clocks: Vec<u64> = log.iter().filter_map(|(_, m)| if let ToApp::Clock { seat: 1, ms, .. } = m { Some(*ms) } else { None }).collect();
    assert!(clocks.contains(&TURN_MS) && clocks.contains(&BANK_START_MS), "his turn, then his bank: {clocks:?}");
    assert!(r.table.seats[1].as_ref().unwrap().sitting_out, "two missed turns: sat out");
    // With Bob sat out, no more hands.
    assert!(r.table.hand.is_none());
}

#[test]
fn standing_up_in_a_hand_folds_and_gives_the_chips_back_after() {
    let mut r = room();
    let mut rng = Stream::new(seed_from(4));
    r.sit("alice", "Alice", 0, 1000, Look::default(), 0).unwrap();
    r.sit("bob", "Bob", 1, 1000, Look::default(), 0).unwrap();
    r.add_bot(2, Level::Regular, 0, &mut rng).unwrap();
    let mut now = 0;
    while r.table.hand.is_none() {
        r.tick(now, &mut rng);
        now += 250;
    }
    let out = r.stand("bob", now).unwrap();
    let mut effects = out.effects;
    let mut log = Vec::new();
    let mut choose = |_: &Room, _: &str| Some(Action::Fold);
    run(&mut r, &mut rng, now, now + 60_000, &mut choose, &mut log, &mut effects);
    let credit = effects.iter().find_map(|e| if let Effect::Credit { uid, chips } = e { (uid == "bob").then_some(*chips) } else { None });
    assert!(credit.is_some(), "Bob's chips back: {effects:?}");
    assert!(r.seat_of("bob").is_none());
}

#[test]
fn closing_gives_everyone_what_they_had() {
    let mut r = room();
    let mut rng = Stream::new(seed_from(5));
    r.sit("alice", "Alice", 0, 1000, Look::default(), 0).unwrap();
    r.sit("bob", "Bob", 1, 800, Look::default(), 0).unwrap();
    let mut now = 0;
    while r.table.hand.is_none() {
        r.tick(now, &mut rng);
        now += 250;
    }
    assert_eq!(r.chips_of("bob"), 800, "in a hand: what they started it with");
    let out = r.close();
    let mut credits: Vec<(String, u64)> = out.effects.iter().filter_map(|e| if let Effect::Credit { uid, chips } = e { Some((uid.clone(), *chips)) } else { None }).collect();
    credits.sort();
    assert_eq!(credits, vec![("alice".into(), 1000), ("bob".into(), 800)]);
}

#[test]
fn buy_ins_are_within_the_table_limits() {
    let mut r = room();
    assert!(r.sit("alice", "Alice", 0, 100, Look::default(), 0).is_err(), "below 40 big blinds");
    assert!(r.sit("alice", "Alice", 0, 5000, Look::default(), 0).is_err(), "above 200");
    r.sit("alice", "Alice", 0, 400, Look::default(), 0).unwrap();
    assert!(r.top_up("alice", 1700).is_err());
    r.top_up("alice", 1600).unwrap();
    assert_eq!(r.table.seats[0].as_ref().unwrap().stack, 2000);
}

#[test]
fn the_history_shows_only_what_each_person_saw() {
    let mut r = room();
    let mut rng = Stream::new(seed_from(6));
    r.sit("alice", "Alice", 0, 1000, Look::default(), 0).unwrap();
    r.sit("bob", "Bob", 1, 1000, Look::default(), 0).unwrap();
    let mut log = Vec::new();
    let mut effects = Vec::new();
    // Bob folds at once: Alice's cards are never shown.
    let mut choose = |r: &Room, uid: &str| Some(if uid == "bob" { Action::Fold } else if r.table.legal(0).unwrap().can_check { Action::Check } else { Action::Call });
    run(&mut r, &mut rng, 0, 20_000, &mut choose, &mut log, &mut effects);
    let h = r.history.front().unwrap().0.number;
    let for_bob = r.history("bob", h).unwrap();
    let for_alice = r.history("alice", h).unwrap();
    let dealt = |hh: &HandHistory| hh.events.iter().filter(|e| matches!(e, Event::Dealt { .. })).map(|e| if let Event::Dealt { seat, .. } = e { *seat } else { 9 }).collect::<Vec<_>>();
    assert_eq!(dealt(&for_bob), vec![1]);
    assert_eq!(dealt(&for_alice), vec![0]);
}

/// A Flash Pot's bomb going off in the middle of a hand: that hand plays out as it was (people
/// still choose), and from the very next hand everyone's all in: nobody can choose, every move
/// is an all-in (or a call of one), each push a moment after the last.
#[test]
fn the_bomb_makes_the_very_next_hand_all_in() {
    let mut r = room();
    let mut rng = Stream::new(seed_from(5));
    let look = Look::default();
    r.sit("alice", "Alice", 0, 1000, look.clone(), 0).unwrap();
    r.sit("bob", "Bob", 1, 1000, look.clone(), 0).unwrap();
    r.add_bot(2, Level::Rookie, 0, &mut rng).unwrap();
    r.add_bot(3, Level::Regular, 0, &mut rng).unwrap();
    // On till a hand's being played and it's a person's turn.
    let mut now = 0;
    loop {
        let _ = r.tick(now, &mut rng);
        if let Some(seat) = r.table.hand.as_ref().and_then(|h| h.to_act)
            && !r.is_bot(seat)
        {
            break;
        }
        now += 100;
        assert!(now < 120_000, "a hand was dealt");
    }
    let hand = r.table.hand.as_ref().unwrap().number;
    let mut out = Output::default();
    r.bomb_goes_off(&mut out);
    assert!(!r.bomb, "the hand being played isn't all in");
    // That hand plays on as it was: people still choose (here, check or fold).
    let mut acted: Vec<(u64, u64, Action)> = Vec::new();
    let mut next = None;
    while now < 300_000 {
        let o = r.tick(now, &mut rng);
        for (_, m) in o.msgs {
            if let ToApp::Event { hand: h, event: Event::Acted { action, .. }, .. } = m {
                acted.push((h, now, action));
            }
        }
        if let Some(h) = r.table.hand.as_ref() {
            if h.number == hand
                && let Some(seat) = h.to_act
                && let Some(uid) = r.uid_at(seat).map(|u| u.to_string())
                && !uid.starts_with("bot:")
            {
                let l = r.table.legal(seat).unwrap();
                let a = if l.can_check { Action::Check } else { Action::Fold };
                let o = r.act(&uid, hand, a, now).expect("people choose in the hand the bomb went off in");
                for (_, m) in o.msgs {
                    if let ToApp::Event { hand: h, event: Event::Acted { action, .. }, .. } = m {
                        acted.push((h, now, action));
                    }
                }
            } else if h.number > hand {
                next.get_or_insert(h.number);
                // The very next hand: all in, and nobody can choose.
                assert!(r.bomb, "the very next hand is all in");
                if let Some(seat) = h.to_act
                    && let Some(uid) = r.uid_at(seat).map(|u| u.to_string())
                    && !uid.starts_with("bot:")
                {
                    assert!(r.act(&uid, h.number, Action::Fold, now).is_err());
                }
            }
        }
        if next.is_some_and(|n| r.table.hand.as_ref().is_none_or(|h| h.number != n)) {
            break;
        }
        now += 100;
    }
    let next = next.expect("another hand was dealt");
    assert_eq!(next, hand + 1, "the very next hand");
    let pushes: Vec<&(u64, u64, Action)> = acted.iter().filter(|(h, ..)| *h == next).collect();
    assert!(pushes.len() >= 2, "{acted:?}");
    // (An all-in shows as a raise to everything, or a call of one.)
    assert!(pushes.iter().all(|(_, _, a)| matches!(a, Action::AllIn | Action::Raise { .. } | Action::Call)), "only all-ins: {pushes:?}");
    // A moment between pushes, and not always the same moment.
    let gaps: Vec<u64> = pushes.windows(2).map(|w| w[1].1 - w[0].1).collect();
    assert!(gaps.iter().all(|g| *g >= BOMB_PUSH_MS - 100), "{gaps:?}");
    assert!(gaps.windows(2).any(|w| w[0] != w[1]) || gaps.len() < 2, "{gaps:?}");
}

/// Everyone all in before the flop: the whole board comes at once from the table, and the next
/// hand waits while apps show it a street at a time, then as long as after any showdown.
#[test]
fn an_all_in_board_is_given_time_to_be_shown() {
    let mut r = room();
    let mut rng = Stream::new(seed_from(9));
    let look = Look::default();
    r.sit("alice", "Alice", 0, 1000, look.clone(), 0).unwrap();
    r.sit("bob", "Bob", 1, 1000, look.clone(), 0).unwrap();
    r.sit("carol", "Carol", 2, 2000, look, 0).unwrap();
    let mut now = 0;
    while r.table.hand.as_ref().and_then(|h| h.to_act).is_none() {
        let _ = r.tick(now, &mut rng);
        now += 100;
        assert!(now < 60_000, "a hand was dealt");
    }
    let hand = r.table.hand.as_ref().unwrap().number;
    // The first to act shoves, the next calls, the last folds.
    let mut moves = [Action::AllIn, Action::Call, Action::Fold].into_iter();
    let last = loop {
        let seat = r.table.hand.as_ref().and_then(|h| h.to_act).expect("someone to act");
        let uid = r.uid_at(seat).unwrap().to_string();
        let o = r.act(&uid, hand, moves.next().unwrap(), now).unwrap();
        if o.msgs.iter().any(|(_, m)| matches!(m, ToApp::Event { event: Event::HandEnded { .. }, .. })) {
            break o;
        }
    };
    let streets = last.msgs.iter().filter(|(_, m)| matches!(m, ToApp::Event { event: Event::Street { .. }, .. })).count();
    assert_eq!(streets, 3, "the flop, turn and river, all at once");
    let ended = now;
    let mut shuffled = None;
    while now < ended + 30_000 {
        let o = r.tick(now, &mut rng);
        if o.msgs.iter().any(|(_, m)| matches!(m, ToApp::Shuffle { .. })) {
            shuffled = Some(now);
            break;
        }
        now += 100;
    }
    let wait = shuffled.expect("the next hand came") - ended;
    let want = AFTER_SHOWDOWN_MS + runout_ms(3);
    assert!((want..want + 200).contains(&wait), "{wait} ms (want {want})");
    // A board that came with betting along the way waits only as long as any showdown.
    assert_eq!(runout_ms(0), 0);
    assert!(runout_ms(1) < runout_ms(2) && runout_ms(2) < runout_ms(3));
}
