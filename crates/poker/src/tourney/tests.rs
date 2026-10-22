use super::*;
use crate::fair::{Stream, seed_from};
use crate::game::{Action, Limit, Variant};

fn rules(seats: usize, buy_in: u64, start_ms: u64) -> TourneyRules {
    TourneyRules {
        name: "Friday Night".into(),
        variant: Variant::Holdem,
        limit: Limit::NoLimit,
        seats,
        stack: 1500,
        buy_in,
        min_players: 2,
        max_players: 100,
        start_ms,
        level_ms: 60_000,
        levels: standard_levels(1500),
        late_levels: 2,
        added: 0,
        bomb: None,
    }
}

#[test]
fn payouts_add_up_and_fall_away() {
    for n in 2..=180 {
        for pool in [0u64, 1, 7, 999, 50_000, 1_234_567] {
            let p = payouts(pool, n);
            assert_eq!(p.iter().sum::<u64>(), pool, "{n} {pool}");
            assert_eq!(p.len(), paid_places(n));
            assert!(p.windows(2).all(|w| w[0] >= w[1]), "{p:?}");
            assert!(p.len() < n.max(2), "not everyone is paid");
        }
    }
    assert_eq!(paid_places(9), 3);
    assert_eq!(paid_places(100), 15);
    let p = payouts(10_000, 9);
    assert!(p[0] > 4_500 && p[0] < 5_500, "{p:?}");
}

#[test]
fn the_standard_structure_climbs_and_checks_out() {
    let l = standard_levels(1500);
    assert_eq!((l[0].small, l[0].big), (10, 20));
    assert!(l.windows(2).all(|w| w[1].big > w[0].big), "{l:?}");
    assert!(l.iter().all(|x| x.small * 2 <= x.big + 1 && x.ante <= x.big));
    assert!(check(&rules(9, 100, 0)).is_ok());
    let mut bad = rules(9, 100, 0);
    bad.seats = 10;
    assert!(check(&bad).is_err());
    bad = rules(9, 100, 0);
    bad.stack = 50;
    assert!(check(&bad).is_err());
}

#[test]
fn unregistering_and_calling_off_give_buy_ins_back() {
    let mut t = Tourney::new(1, rules(6, 100, 10_000), "host", "Host").unwrap();
    let mut out = Output::default();
    t.register("alice", "Alice", Look::default(), 100, 0, &mut out).unwrap();
    assert!(t.register("alice", "Alice", Look::default(), 100, 0, &mut out).is_err());
    t.register("bob", "Bob", Look::default(), 100, 0, &mut out).unwrap();
    t.unregister("bob", &mut out).unwrap();
    assert!(out.effects.contains(&Effect::Credit { uid: "bob".into(), chips: 100 }));
    assert_eq!(t.prize_pool(), 100);
    // Start time comes with one player: called off, Alice paid back.
    let mut rng = Stream::new(seed_from(3));
    let mut ids = 100;
    let out = t.tick(10_000, &mut ids, &mut rng);
    assert_eq!(t.phase, TourneyPhase::Cancelled);
    assert!(out.effects.contains(&Effect::Credit { uid: "alice".into(), chips: 100 }));
    assert!(t.tables.is_empty());
}

/// Runs a whole tournament: `people` act by a simple rule, bots by theirs.
fn play_out(t: &mut Tourney, seed: u64, until: u64) -> (Vec<Effect>, usize) {
    let mut rng = Stream::new(seed_from(seed));
    let mut ids: TableId = 1000;
    let mut effects = Vec::new();
    let mut most_tables = 0;
    let n = t.entrants_count() as u64;
    let mut now = t.rules.start_ms;
    while now < until && t.phase != TourneyPhase::Finished {
        let out = t.tick(now, &mut ids, &mut rng);
        effects.extend(out.effects);
        most_tables = most_tables.max(t.tables.len());
        if t.phase == TourneyPhase::Running {
            // No table over-full; every chip still there (none made, none lost) between hands.
            for r in t.tables.values() {
                assert!(r.seated().len() <= t.rules.seats);
            }
            if t.tables.values().all(|r| r.between_hands()) {
                let chips: u64 = t.tables.values().map(|r| r.table.seats.iter().flatten().map(|s| s.stack).sum::<u64>()).sum();
                assert_eq!(chips, n * t.rules.stack, "at {now}");
            }
            // People: call small bets, else fold; shove with nothing to lose sometimes.
            let tables: Vec<TableId> = t.tables.keys().copied().collect();
            for id in tables {
                let r = t.room_mut(id).unwrap();
                if let Some(seat) = r.table.hand.as_ref().and_then(|h| h.to_act)
                    && let Some(uid) = r.uid_at(seat).map(|s| s.to_string())
                    && !uid.starts_with("bot:")
                    && let Some(l) = r.table.legal(seat)
                {
                    let hand = r.table.hand.as_ref().unwrap().number;
                    let a = if l.can_check { Action::Check } else if l.call * 5 <= r.table.behind(seat) { Action::Call } else if (hand + seat as u64) % 3 == 0 { Action::AllIn } else { Action::Fold };
                    let out = r.act(&uid, hand, a, now).unwrap();
                    effects.extend(out.effects);
                }
            }
        }
        now += 250;
    }
    (effects, most_tables)
}

#[test]
fn a_full_field_plays_down_to_one_winner() {
    let mut t = Tourney::new(7, rules(9, 200, 5_000), "host", "Host").unwrap();
    let mut out = Output::default();
    t.register("alice", "Alice", Look::default(), 200, 0, &mut out).unwrap();
    t.register("bob", "Bob", Look::default(), 200, 0, &mut out).unwrap();
    for i in 0..25 {
        t.add_bot([Level::Rookie, Level::Regular, Level::Shark][i % 3], &mut out).unwrap();
    }
    assert_eq!(t.entrants_count(), 27);
    assert_eq!(t.prize_pool(), 400, "bots enter free");
    let (effects, most_tables) = play_out(&mut t, 11, 6 * 3_600_000);
    assert_eq!(t.phase, TourneyPhase::Finished, "finished within six hours of play");
    assert_eq!(most_tables, 3, "27 players at 9-handed tables");
    // The final table stays a moment (its last hand's cards up), then closes.
    assert_eq!(t.tables.len(), 1);
    let end = t.ended.unwrap();
    let mut ids = 9_000;
    let mut rng = Stream::new(seed_from(1));
    t.tick(end + FINAL_MS - 1, &mut ids, &mut rng);
    assert_eq!(t.tables.len(), 1);
    // (Someone's watching it, whoever's still there: they're told it's closed.)
    t.tables.values_mut().next().unwrap().watch("alice");
    let out = t.tick(end + FINAL_MS, &mut ids, &mut rng);
    assert!(t.tables.is_empty());
    assert!(out.msgs.iter().any(|(_, m)| matches!(m, ToApp::Closed { .. })));
    // Every place from 1 to 27, once each.
    let v = t.view("alice");
    let mut places: Vec<usize> = v.entrants.iter().map(|e| e.place.unwrap()).collect();
    places.sort();
    assert_eq!(places, (1..=27).collect::<Vec<_>>());
    // Prizes: paid to whoever placed in the money, adding up to what the people's places won.
    let pay = payouts(400, 27);
    for e in &v.entrants {
        let place = e.place.unwrap();
        assert_eq!(e.prize, pay.get(place - 1).copied().unwrap_or(0), "{e:?}");
    }
    let credited: u64 = effects.iter().filter_map(|e| if let Effect::Credit { chips, .. } = e { Some(*chips) } else { None }).sum();
    let people_won: u64 = v.entrants.iter().filter(|e| e.bot.is_none()).map(|e| e.prize).sum();
    assert_eq!(credited, people_won);
    // Each person is told where they finished, once.
    for who in ["alice", "bob"] {
        assert_eq!(effects.iter().filter(|e| matches!(e, Effect::Placed { uid, .. } if uid == who)).count(), 1);
    }
}

#[test]
fn late_registration_sits_you_down_at_once() {
    let mut t = Tourney::new(2, rules(6, 0, 1_000), "host", "Host").unwrap();
    let mut out = Output::default();
    for _ in 0..7 {
        t.add_bot(Level::Regular, &mut out).unwrap();
    }
    let mut rng = Stream::new(seed_from(5));
    let mut ids = 1;
    t.tick(1_000, &mut ids, &mut rng);
    assert_eq!(t.phase, TourneyPhase::Running);
    assert_eq!(t.tables.len(), 2);
    let mut out = Output::default();
    t.register("carol", "Carol", Look::default(), 0, 2_000, &mut out).unwrap();
    let at = t.view("carol").my_table.expect("seated");
    assert!(out.msgs.iter().any(|(to, m)| *to == crate::room::Rcpt::User("carol".into()) && matches!(m, ToApp::Snapshot { table } if table.id == at && table.tourney == Some(2))));
    // Past the late levels, it's closed.
    t.tick(1_000 + 2 * 60_000 + 10, &mut ids, &mut rng);
    assert!(t.register("dave", "Dave", Look::default(), 0, 200_000, &mut Output::default()).is_err());
}

#[test]
fn nobody_buys_in_or_leaves_a_tournament_table() {
    let mut t = Tourney::new(3, rules(6, 0, 0), "host", "Host").unwrap();
    let mut out = Output::default();
    t.register("alice", "Alice", Look::default(), 0, 0, &mut out).unwrap();
    t.add_bot(Level::Rookie, &mut out).unwrap();
    let mut rng = Stream::new(seed_from(9));
    let mut ids = 1;
    t.tick(0, &mut ids, &mut rng);
    let id = t.view("alice").my_table.unwrap();
    let r = t.room_mut(id).unwrap();
    assert!(r.stand("alice", 0).is_err());
    assert!(r.top_up("alice", 100).is_err());
    assert!(r.sit("bob", "Bob", 3, 1000, Look::default(), 0).is_err());
}

#[test]
fn friday_night_is_10pm_in_new_york() {
    // Tue 2026-10-20 03:00 UTC (Monday 11 pm in New York, summer time): Friday the 23rd,
    // 22:00 EDT = Saturday 02:00 UTC.
    let now = 1_792_465_200_000;
    assert_eq!(next_friday_night(now), 1_792_807_200_000);
    // In winter (Mon 2026-12-07 12:00 UTC): Friday the 11th, 22:00 EST = Saturday 03:00 UTC.
    assert_eq!(next_friday_night(1_796_644_800_000), 1_797_044_400_000);
    // Just after one starts, the next is a week on.
    assert_eq!(next_friday_night(1_792_807_200_000), 1_792_807_200_000 + 7 * 86_400_000);
    // Summer time starts (March 8) and ends (November 1).
    assert_eq!(next_friday_night(1773144000000), 1773453600000);
    assert_eq!(next_friday_night(1793620800000), 1794020400000);
    check(&friday_rules(1_792_807_200_000, FRIDAY_FLOOR)).unwrap();
}

#[test]
fn the_prize_wheel_keeps_about_three_percent() {
    let total: u32 = MULTIPLIERS.iter().map(|m| m.1).sum();
    assert_eq!(total, 10_000);
    let ev = MULTIPLIERS.iter().map(|(m, w)| *m as f64 * *w as f64).sum::<f64>() / total as f64;
    assert_eq!(BOMB_SEATS, 5);
    assert!((4.8..4.9).contains(&ev), "{ev}: about 4.85 buy-ins back for the 5 paid in");
    // The least the winner takes is more than their own buy-in and a bit: 3 of the 5.
    assert_eq!(MULTIPLIERS[0].0, 3);
    // Spun many times, it picks each multiplier about as often as its weight says.
    let mut rng = Stream::new(seed_from(77));
    let mut seen = std::collections::BTreeMap::new();
    for _ in 0..200_000 {
        *seen.entry(spin(&mut rng)).or_insert(0u32) += 1;
    }
    for (m, w) in MULTIPLIERS {
        let got = *seen.get(&m).unwrap_or(&0) as f64 / 200_000.0;
        let want = w as f64 / 10_000.0;
        assert!((got - want).abs() < 0.006 + want * 0.25, "{m}x: {got} vs {want}");
    }
}

#[test]
fn a_time_bomb_nobody_waits_for_is_called_off() {
    let mut t = Tourney::new(4, bomb_rules(100, 0), BOMB_HOST, "Pyr Casino").unwrap();
    let mut out = Output::default();
    t.register("alice", "Alice", Look::default(), 100, 0, &mut out).unwrap();
    t.unregister("alice", &mut out).unwrap();
    let mut rng = Stream::new(seed_from(3));
    let mut ids = 1;
    let out = t.tick(BOMB_WAIT_MS, &mut ids, &mut rng);
    assert_eq!(t.phase, TourneyPhase::Cancelled, "only bots would play it");
    assert!(out.effects.is_empty(), "nothing to give back: Alice had her buy-in back already");
}

#[test]
fn a_time_bomb_fills_with_bots_spins_and_goes_off() {
    let mut t = Tourney::new(5, bomb_rules(1_000, 0), BOMB_HOST, "Pyr Casino").unwrap();
    let mut out = Output::default();
    t.register("alice", "Alice", Look::default(), 1_000, 0, &mut out).unwrap();
    let mut rng = Stream::new(seed_from(21));
    let mut ids: TableId = 500;
    // Waiting: one person, and the wheel hasn't spun.
    t.tick(10_000, &mut ids, &mut rng);
    assert_eq!(t.phase, TourneyPhase::Registering);
    assert_eq!(t.view("alice").bomb.unwrap().multiplier, None);
    // The wait's over: bots take the four empty seats, and the wheel spins.
    let mut effects = Vec::new();
    effects.extend(t.tick(BOMB_WAIT_MS, &mut ids, &mut rng).effects);
    assert_eq!(t.phase, TourneyPhase::Running);
    assert_eq!(t.entrants_count(), 5);
    assert_eq!(t.tables.len(), 1, "one table of five");
    let v = t.view("alice");
    let b = v.bomb.unwrap();
    let m = b.multiplier.expect("spun") as u64;
    assert_eq!(v.prize_pool, 1_000 * m, "the house stakes the bots' seats");
    assert_eq!(v.payouts, vec![1_000 * m], "the winner takes it all");
    let first = BOMB_WAIT_MS + WHEEL_MS;
    assert_eq!(b.goes_off_ms, Some(first + BOMB_FUSE_MS));
    // No hand while the wheel spins.
    let id = v.my_table.unwrap();
    for now in (BOMB_WAIT_MS..first).step_by(250) {
        t.tick(now, &mut ids, &mut rng);
        assert!(t.tables[&id].table.hand.is_none(), "dealt at {now} before the wheel stopped");
    }
    // Alice checks or calls small bets, folds to big ones, until the bomb goes off; then she
    // can do nothing but watch, and every hand is all in.
    let mut now = first;
    let mut went_off = None;
    let mut bomb_hands = 0;
    let mut last_hand = 0;
    while t.phase == TourneyPhase::Running && now < first + 60 * 60_000 {
        effects.extend(t.tick(now, &mut ids, &mut rng).effects);
        if went_off.is_none() && t.view("alice").bomb.unwrap().exploded {
            assert!(now >= first + BOMB_FUSE_MS);
            went_off = Some(now);
        }
        if let Some(r) = t.tables.get_mut(&id) {
            if r.bomb
                && let Some(h) = r.table.hand.as_ref()
                && h.number != last_hand
            {
                last_hand = h.number;
                bomb_hands += 1;
            }
            if let Some(seat) = r.table.hand.as_ref().and_then(|h| h.to_act)
                && r.uid_at(seat) == Some("alice")
                && let Some(l) = r.table.legal(seat)
            {
                let hand = r.table.hand.as_ref().unwrap().number;
                let a = if l.can_check { Action::Check } else if l.call * 4 <= r.table.behind(seat) { Action::Call } else { Action::Fold };
                let res = r.act("alice", hand, a, now);
                if r.bomb {
                    assert!(res.is_err(), "no choices once it's gone off");
                } else if let Ok(o) = res {
                    effects.extend(o.effects);
                }
            }
        }
        now += 250;
    }
    assert_eq!(t.phase, TourneyPhase::Finished);
    let v = t.view("alice");
    let alice = v.entrants.iter().find(|e| e.uid == "alice").unwrap();
    let credited: u64 = effects.iter().filter_map(|e| if let Effect::Credit { chips, .. } = e { Some(*chips) } else { None }).sum();
    if alice.place == Some(1) {
        assert_eq!(credited, 1_000 * m);
    } else {
        assert_eq!(credited, 0);
    }
    assert_eq!(effects.iter().filter(|e| matches!(e, Effect::Placed { uid, .. } if uid == "alice")).count(), 1);
    // If it lasted till the bomb, all-in hands settled it soon after.
    if let Some(off) = went_off {
        assert!(bomb_hands >= 1 || t.ended.unwrap() <= off + 30_000, "{bomb_hands} all-in hands");
        assert!(t.ended.unwrap() <= off + 10 * 60_000, "all in every hand ends it quickly");
    }
}
