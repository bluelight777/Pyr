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
