//! What each game pays back over time (worked out exactly where it can be, played out where it
//! can't), and that rounds play the same again from their seeds.

use super::*;
use crate::cards::Card;
use crate::fair::seed_from;

fn s(n: u64) -> Stream {
    Stream::new(seed_from(n))
}

fn card(x: &str) -> Card {
    Card::parse(x).unwrap()
}

// ---- Fairness ----

#[test]
fn a_round_plays_the_same_from_the_same_seeds() {
    let server = seed_from(1);
    let client = seed_from(2);
    let bet = Bet::Slots { line_bet: 5 };
    let a = play(&bet, &mut stream(&server, &client, Game::Slots, 7), 50_000).unwrap();
    let b = play(&bet, &mut stream(&server, &client, Game::Slots, 7), 50_000).unwrap();
    assert_eq!(a, b);
    // Another round number, another client seed: another round.
    let c = play(&bet, &mut stream(&server, &client, Game::Slots, 8), 50_000).unwrap();
    let d = play(&bet, &mut stream(&server, &seed_from(3), Game::Slots, 7), 50_000).unwrap();
    assert!(a != c || a != d);
    assert_ne!(commit(&server), commit(&client));
    assert_ne!(round_seed(&server, &client, Game::Dice, 1), round_seed(&server, &client, Game::Keno, 1));
}

#[test]
fn bets_outside_the_limits_are_refused() {
    assert!(Bet::Slots { line_bet: 0 }.check().is_err());
    assert!(Bet::Slots { line_bet: slots::MAX_LINE_BET + 1 }.check().is_err());
    assert!(Bet::Dice { chips: 10, chance: 100, over: false }.check().is_err());
    assert!(Bet::Keno { chips: 10, picks: vec![1, 1] }.check().is_err());
    assert!(Bet::Keno { chips: 10, picks: (1..=11).collect() }.check().is_err());
    assert!(Bet::Roulette { chips: vec![roulette::Chip { spot: roulette::Spot::Split { a: 3, b: 4 }, chips: 5 }] }.check().is_err(), "3 and 4 aren't side by side");
    assert!(Bet::Roulette { chips: vec![roulette::Chip { spot: roulette::Spot::Straight { n: 17 }, chips: 2_001 }] }.check().is_err());
    assert!(Bet::Roulette { chips: vec![roulette::Chip { spot: roulette::Spot::Corner { a: 2 }, chips: 5 }] }.check().is_ok());
    assert!(Bet::Baccarat(baccarat::Bets::default()).check().is_err());
}

// ---- Slots ----

/// Exactly: each reel's symbols are equally likely to land on any row, so each line's chances
/// are the product of each reel's; the stars' too (one per reel at most).
fn slots_exact() -> (f64, f64, f64) {
    let lens: Vec<f64> = slots::STRIPS.iter().map(|s| s.len() as f64).collect();
    let p = |r: usize, sym: u8| slots::STRIPS[r].iter().filter(|&&x| x == sym).count() as f64 / lens[r];
    let mut line = 0.0;
    let mut idx = [0u8; 5];
    loop {
        let syms = idx;
        let pr: f64 = (0..5).map(|r| p(r, syms[r])).product();
        if pr > 0.0
            && let Some((_, _, pay)) = slots::line_win(syms)
        {
            line += pr * pay as f64;
        }
        let mut r = 0;
        loop {
            idx[r] += 1;
            if (idx[r] as usize) < slots::SYMBOLS {
                break;
            }
            idx[r] = 0;
            r += 1;
            if r == 5 {
                break;
            }
        }
        if r == 5 {
            break;
        }
    }
    // Per line bet; a spin bets 20 lines, so per spin bet the lines return `line` too.
    let ps: Vec<f64> = (0..5).map(|r| (3.0 * p(r, slots::STAR)).min(1.0)).collect();
    let mut dist = vec![1.0];
    for q in &ps {
        let mut nd = vec![0.0; dist.len() + 1];
        for (k, d) in dist.iter().enumerate() {
            nd[k] += d * (1.0 - q);
            nd[k + 1] += d * q;
        }
        dist = nd;
    }
    let star_ev: f64 = (3..=5).map(|k| dist[k] * slots::STAR_PAYS[k - 3] as f64).sum();
    let base = line + star_ev;
    let added: f64 = (3..=5).map(|k| dist[k] * slots::STAR_SPINS[k - 3] as f64).sum();
    let free: f64 = (3..=5).map(|k| dist[k] * slots::STAR_SPINS[k - 3] as f64 / (1.0 - added)).sum::<f64>() * slots::FREE_SPIN_MULTIPLIER as f64 * base;
    (base + free, base, (3..=5).map(|k| dist[k]).sum())
}

#[test]
fn slots_pay_back_about_98_percent_and_the_jackpot_1_more() {
    let (rtp, base, trigger) = slots_exact();
    println!("slots: {:.3}% (base game {:.3}%, free spins 1 in {:.0} spins)", rtp * 100.0, base * 100.0, 1.0 / trigger);
    assert!((0.975..0.985).contains(&rtp), "{rtp}");
    // The jackpot's 1% comes back on top.
    assert_eq!(slots::jackpot_share(5), 1);
    assert_eq!(slots::jackpot_share(500), 100);
}

#[test]
fn slots_played_out_agree_with_the_sums() {
    let (rtp, _, _) = slots_exact();
    let mut s = s(11);
    let (mut bet, mut paid, mut hits) = (0u64, 0u64, 0u64);
    let n = 400_000;
    for _ in 0..n {
        // (No jackpot here: its pool is paid back separately.)
        let o = slots::play(1, &mut s, 0);
        bet += slots::LINES as u64;
        paid += o.paid;
        hits += (o.paid > 0) as u64;
    }
    let played = paid as f64 / bet as f64;
    println!("slots played: {:.2}% ({} spins, a win 1 in {:.2})", played * 100.0, n, n as f64 / hits as f64);
    assert!((played - rtp).abs() < 0.03, "{played} vs {rtp}");
}

#[test]
fn a_line_pays_the_better_of_its_wilds_and_its_symbol() {
    use slots::*;
    assert_eq!(line_win([SEVEN, SEVEN, SEVEN, CHERRY, CHERRY]), Some((SEVEN, 3, 40)));
    assert_eq!(line_win([WILD, SEVEN, WILD, SEVEN, BELL]), Some((SEVEN, 4, 120)));
    assert_eq!(line_win([WILD, WILD, WILD, CHERRY, LEMON]), Some((WILD, 3, 60)), "three flames beat four cherries");
    assert_eq!(line_win([WILD, WILD, WILD, WILD, WILD]), Some((WILD, 5, 1500)));
    assert_eq!(line_win([STAR, STAR, STAR, BELL, BELL]), None, "stars pay anywhere, not on lines");
    assert_eq!(line_win([CHERRY, CHERRY, LEMON, CHERRY, CHERRY]), None);
}

#[test]
fn five_flames_win_the_jackpot_once() {
    // Stops putting a flame on the middle row of every reel.
    let mut stops = [0u8; 5];
    for (r, strip) in slots::STRIPS.iter().enumerate() {
        let i = strip.iter().position(|&x| x == slots::WILD).unwrap();
        stops[r] = ((i + strip.len() - 1) % strip.len()) as u8;
    }
    let g = slots::grid(&stops);
    assert!(g.iter().all(|reel| reel[1] == slots::WILD));
    let (mut pool, mut won) = (1_000_000, 0);
    let spin = super::slots::tests_evaluate(stops, 250, &mut pool, &mut won);
    assert_eq!(won, 500_000, "half the biggest line bet: half the pool");
    assert_eq!(pool, 0);
    assert!(spin.pay >= 500_000);
}

// ---- Roulette ----

#[test]
fn every_roulette_bet_pays_back_36_in_37() {
    use roulette::Spot::*;
    let spots = [Straight { n: 0 }, Straight { n: 17 }, Split { a: 0, b: 2 }, Split { a: 14, b: 17 }, Split { a: 20, b: 21 }, Street { row: 5 }, Corner { a: 25 }, Line { row: 10 }, Column { c: 1 }, Dozen { d: 2 }, Red, Black, Odd, Even, Low, High];
    for spot in spots {
        spot.check().unwrap();
        let back: u64 = (0..=36).map(|n| if spot.covers(n) { spot.pays() + 1 } else { 0 }).sum();
        assert_eq!(back, 36, "{spot:?}");
    }
    assert_eq!(roulette::WHEEL.len(), 37);
    let mut seen = roulette::WHEEL.to_vec();
    seen.sort();
    assert_eq!(seen, (0..=36).collect::<Vec<u8>>());
    assert_eq!(roulette::RED.len(), 18);
}

// ---- Dice ----

#[test]
fn dice_pays_back_99_percent_at_any_chance() {
    for chance in [200u16, 1_000, 3_333, 5_000, 7_777, 9_800] {
        for over in [false, true] {
            let wins = (0..10_000u16).filter(|r| dice::wins(*r, chance, over)).count() as u64;
            assert_eq!(wins, chance as u64);
            let back = wins as f64 * dice::payout(10_000, chance) as f64 / 10_000.0 / 10_000.0;
            assert!((0.989..=0.990).contains(&back), "{chance} {over}: {back}");
        }
    }
    assert_eq!(dice::payout(100, 5_000), 198);
    assert_eq!(dice::multiplier_x100(200), 4_950);
}

// ---- Keno ----

fn choose(n: u64, k: u64) -> f64 {
    if k > n {
        return 0.0;
    }
    (0..k).fold(1.0, |a, i| a * (n - i) as f64 / (i + 1) as f64)
}

#[test]
fn keno_pays_back_about_95_percent_for_any_number_of_picks() {
    for picks in 1..=10u64 {
        let mut back = 0.0;
        for caught in 0..=picks {
            let p = choose(20, caught) * choose(60, picks - caught) / choose(80, picks);
            back += p * keno::PAYS[picks as usize - 1][caught as usize] as f64 / 10.0;
        }
        println!("keno {picks} picks: {:.2}%", back * 100.0);
        assert!((0.935..0.965).contains(&back), "{picks}: {back}");
    }
    let o = keno::play(10, &[1, 2, 3], &mut s(5));
    assert_eq!(o.drawn.len(), 20);
    let mut d = o.drawn.clone();
    d.sort();
    d.dedup();
    assert_eq!(d.len(), 20);
    assert!(d.iter().all(|n| (1..=80).contains(n)));
}

// ---- Baccarat ----

#[test]
fn baccarat_draws_by_the_tableau() {
    use baccarat::*;
    // A natural stands.
    assert_eq!(total(&[card("9c"), card("Kd")]), 9);
    assert!(banker_draws(5, None));
    assert!(!banker_draws(6, None));
    assert!(banker_draws(3, Some(7)) && !banker_draws(3, Some(8)));
    assert!(banker_draws(6, Some(6)) && !banker_draws(6, Some(5)));
    assert!(!banker_draws(7, Some(1)));
    let (w, pp, _, paid) = settle(&Bets { banker: 100, tie: 10, player_pair: 10, ..Default::default() }, &[card("4c"), card("4d"), card("Kd")], &[card("5c"), card("4h")]);
    assert_eq!(w, Side::Banker);
    assert!(pp);
    assert_eq!(paid, 195 + 120);
}

#[test]
fn baccarat_played_out_keeps_about_one_percent() {
    let mut s = s(21);
    let n = 400_000u64;
    let (mut player, mut banker, mut tie) = (0u64, 0u64, 0u64);
    for _ in 0..n {
        let (p, b) = baccarat::deal(&mut s);
        let one = |bets: baccarat::Bets| baccarat::settle(&bets, &p, &b).3;
        player += one(baccarat::Bets { player: 100, ..Default::default() });
        banker += one(baccarat::Bets { banker: 100, ..Default::default() });
        tie += one(baccarat::Bets { tie: 100, ..Default::default() });
    }
    let r = |x: u64| x as f64 / (n * 100) as f64;
    println!("baccarat: player {:.2}%, banker {:.2}%, tie {:.2}%", r(player) * 100.0, r(banker) * 100.0, r(tie) * 100.0);
    assert!((0.980..0.995).contains(&r(player)));
    assert!((0.981..0.997).contains(&r(banker)));
    assert!((0.80..0.90).contains(&r(tie)));
}

// ---- Blackjack ----

#[test]
fn blackjack_pays_and_settles_the_usual_way() {
    use blackjack::*;
    assert_eq!(value(&[card("Ac"), card("6d")]), (17, true));
    assert_eq!(value(&[card("Ac"), card("6d"), card("Tc")]), (17, false));
    assert_eq!(value(&[card("Ac"), card("Ad"), card("9c")]), (21, true));
    assert!(is_blackjack(&[card("Ah"), card("Kd")]));
    // Hands to the end with basic strategy: every one settles, and replays the same.
    for n in 0..400 {
        let mut r = Round::deal(100, s(1000 + n));
        while r.phase != Phase::Over {
            let a = if r.phase == Phase::Insurance {
                Action::NoInsurance
            } else {
                let h = &r.hands[r.active];
                let allowed = r.allowed();
                let m = basic(&h.cards, r.dealer[0], allowed.contains(&Action::Double), allowed.contains(&Action::Split));
                if allowed.contains(&m) { m } else { Action::Hit }
            };
            r.act(a).unwrap();
        }
        let o = r.outcome();
        assert!(o.hands.iter().all(|h| h.result.is_some()));
        let again = replay(100, s(1000 + n), &o.actions).unwrap();
        assert_eq!(again, o);
    }
}

#[test]
fn blackjack_played_well_keeps_about_half_a_percent() {
    use blackjack::*;
    let n = 300_000u64;
    let (mut bet, mut paid) = (0u64, 0u64);
    for i in 0..n {
        let mut r = Round::deal(100, s(5_000_000 + i));
        while r.phase != Phase::Over {
            let a = if r.phase == Phase::Insurance {
                Action::NoInsurance
            } else {
                let h = &r.hands[r.active];
                let allowed = r.allowed();
                let m = basic(&h.cards, r.dealer[0], allowed.contains(&Action::Double), allowed.contains(&Action::Split));
                if allowed.contains(&m) { m } else { Action::Hit }
            };
            r.act(a).unwrap();
        }
        bet += r.wagered();
        paid += r.paid();
    }
    let back = paid as f64 / bet as f64;
    println!("blackjack with basic strategy: {:.2}% back", back * 100.0);
    assert!((0.985..1.003).contains(&back), "{back}");
}

#[test]
fn blackjack_moves_out_of_turn_are_refused() {
    use blackjack::*;
    let mut r = Round::deal(100, s(77));
    if r.phase == Phase::Playing {
        assert!(r.act(Action::Insure).is_err());
    }
    while r.phase != Phase::Over {
        let a = if r.phase == Phase::Insurance { Action::Insure } else { Action::Stand };
        r.act(a).unwrap();
    }
    assert!(r.act(Action::Hit).is_err());
}

// ---- Craps ----

#[test]
fn craps_settles_the_usual_way() {
    use craps::*;
    let bets = Bets { pass: 100, field: 10, place: [0, 0, 60, 0, 0, 0], hard: [0, 10, 0, 0], ..Default::default() };
    let t = Table { point: None, bets: bets.clone(), streak: 0 };
    // Come-out 7: pass wins; place and hardways are off; the field loses.
    let (t1, o) = settle(&t, (3, 4));
    assert_eq!(o.paid, 200);
    assert_eq!(t1.point, None);
    assert_eq!(t1.bets.place[2], 60);
    // Come-out 6 sets the point; then the odds go up and 6 comes again.
    let (mut t2, _) = settle(&t, (2, 4));
    assert_eq!(t2.point, Some(6));
    let mut more = t2.bets.clone();
    more.pass_odds = 500;
    check_change(&t2, &more).unwrap();
    more.pass_odds = 501;
    assert!(check_change(&t2, &more).is_err(), "5 times the pass line at most on 6");
    more.pass_odds = 500;
    more.pass = 50;
    assert!(check_change(&t2, &more).is_err(), "the pass line stays once there's a point");
    more.pass = 100;
    t2.bets = more;
    let (t3, o) = settle(&t2, (3, 3));
    // Pass 100 -> 200, odds 500 at 6:5 -> 1100, place 6 wins 70 (stays up), hard 6 wins 90.
    assert_eq!(o.paid, 200 + 1_100 + 70 + 90);
    assert_eq!(t3.point, None);
    assert_eq!(t3.bets.place[2], 60);
    assert_eq!(t3.streak, 1);
    // A seven-out takes the place bets and hardways.
    let t4 = Table { point: Some(8), bets: Bets { pass: 10, place: [5, 5, 6, 6, 5, 5], hard: [1, 1, 1, 1], ..Default::default() }, streak: 2 };
    let (t5, o) = settle(&t4, (5, 2));
    assert_eq!(o.paid, 0);
    assert!(t5.bets.is_empty());
    assert_eq!(t5.streak, 0);
    // The field pays double on 2, triple on 12.
    let f = Table { point: None, bets: Bets { field: 10, ..Default::default() }, streak: 0 };
    assert_eq!(settle(&f, (1, 1)).1.paid, 30);
    assert_eq!(settle(&f, (6, 6)).1.paid, 40);
    assert_eq!(settle(&f, (4, 1)).1.paid, 0);
}

#[test]
fn craps_pass_line_keeps_about_one_and_a_half_percent() {
    use craps::*;
    let mut s = s(31);
    let (mut bet, mut back) = (0u64, 0u64);
    for _ in 0..400_000 {
        let mut t = Table { point: None, bets: Bets { pass: 100, ..Default::default() }, streak: 0 };
        bet += 100;
        loop {
            let (nt, o) = play(&t, &mut s);
            back += o.paid;
            t = nt;
            if t.point.is_none() {
                break;
            }
        }
    }
    let r = back as f64 / bet as f64;
    println!("craps pass line: {:.2}% back", r * 100.0);
    assert!((0.978..0.994).contains(&r), "{r}");
}

#[test]
fn rounds_check_out_and_a_changed_one_doesnt() {
    let server = seed_from(40);
    let client = seed_from(41);
    let shown = commit(&server);
    for (i, bet) in [
        Bet::Slots { line_bet: 3 },
        Bet::Roulette { chips: vec![roulette::Chip { spot: roulette::Spot::Red, chips: 10 }] },
        Bet::Baccarat(baccarat::Bets { banker: 50, ..Default::default() }),
        Bet::Dice { chips: 10, chance: 4_950, over: true },
        Bet::Keno { chips: 5, picks: vec![3, 9, 27, 41, 80] },
    ]
    .into_iter()
    .enumerate()
    {
        let nonce = 100 + i as u64;
        let o = play(&bet, &mut stream(&server, &client, bet.game(), nonce), 60_000).unwrap();
        verify(&shown, &server, &client, nonce, &o).unwrap();
        assert!(verify(&shown, &server, &client, nonce + 1, &o).is_err() || o == play(&bet, &mut stream(&server, &client, bet.game(), nonce + 1), 60_000).unwrap());
        assert!(verify(&commit(&client), &server, &client, nonce, &o).is_err(), "another fingerprint");
    }
    // A blackjack hand, played to the end.
    let mut r = blackjack::Round::deal(100, stream(&server, &client, Game::Blackjack, 7));
    while r.phase != blackjack::Phase::Over {
        let a = if r.phase == blackjack::Phase::Insurance { blackjack::Action::NoInsurance } else { blackjack::Action::Stand };
        r.act(a).unwrap();
    }
    let mut o = Outcome::Blackjack(r.outcome());
    verify(&shown, &server, &client, 7, &o).unwrap();
    if let Outcome::Blackjack(b) = &mut o {
        b.paid += 100;
    }
    assert!(verify(&shown, &server, &client, 7, &o).is_err(), "paid more than the hand says");
    // A craps roll.
    let t = craps::Table { point: Some(6), bets: craps::Bets { pass: 10, place: [5, 0, 6, 6, 0, 5], ..Default::default() }, streak: 1 };
    let (_, c) = craps::play(&t, &mut stream(&server, &client, Game::Craps, 9));
    verify(&shown, &server, &client, 9, &Outcome::Craps(c)).unwrap();
}

/// Pyr Sevens' Play Feature, worked out exactly (in spin bets): `cargo test --release -p
/// pyr-poker sevens_feature_math -- --ignored --nocapture`.
#[test]
#[ignore]
fn sevens_feature_math() {
    let e = slots::exact();
    let lines = slots::LINES as f64;
    let spin: f64 = (0..6).map(|k| e.stars[k] * (e.lines_by_stars[k] + if k >= 3 { slots::STAR_PAYS[k - 3] as f64 * lines } else { 0.0 })).sum::<f64>() / lines;
    let p_t: f64 = e.stars[3..].iter().sum();
    let added: f64 = (3..6).map(|k| e.stars[k] * slots::STAR_SPINS[k - 3] as f64).sum();
    let fs = |n: f64, x: f64| n * x * spin / (1.0 - added);
    let given_t = |x: f64| (3..6).map(|k| e.stars[k] / p_t * (e.lines_by_stars[k] / lines + slots::STAR_PAYS[k - 3] as f64 + fs(slots::STAR_SPINS[k - 3] as f64, x))).sum::<f64>();
    let given_n: f64 = (0..3).map(|k| e.stars[k] * e.lines_by_stars[k]).sum::<f64>() / (1.0 - p_t) / lines;
    let wild_t: f64 = e.wild_stars[3..].iter().sum();
    let wild_fs: f64 = (3..6).map(|k| e.wild_stars[k] * fs(slots::STAR_SPINS[k - 3] as f64, 3.0)).sum();
    println!("spin {spin:.5} (1 in {:.1} free spins), base round {:.5}", 1.0 / p_t, p_t * given_t(3.0) + (1.0 - p_t) * given_n);
    println!("given free spins: x3 {:.4}, x10 {:.4}; without {given_n:.5}", given_t(3.0), given_t(10.0));
    println!("wild: {:.4} (stars {wild_t:.5})", e.wild_pay / lines + wild_fs);
}

/// Pyr Sevens' Play Feature pays back what the reels do (98%, the jackpot's 1% on top), worked
/// out exactly; played out, features cost what they say and start what they say.
#[test]
fn sevens_features_pay_back_about_98_percent() {
    use crate::casino::feature::Feature;
    let e = slots::exact();
    let lines = slots::LINES as f64;
    let spin: f64 = (0..6).map(|k| e.stars[k] * (e.lines_by_stars[k] + if k >= 3 { slots::STAR_PAYS[k - 3] as f64 * lines } else { 0.0 })).sum::<f64>() / lines;
    let p_t: f64 = e.stars[3..].iter().sum();
    let added: f64 = (3..6).map(|k| e.stars[k] * slots::STAR_SPINS[k - 3] as f64).sum();
    let fs = |n: f64, x: f64| n * x * spin / (1.0 - added);
    let given_t = |x: f64| (3..6).map(|k| e.stars[k] / p_t * (e.lines_by_stars[k] / lines + slots::STAR_PAYS[k - 3] as f64 + fs(slots::STAR_SPINS[k - 3] as f64, x))).sum::<f64>();
    let given_n: f64 = (0..3).map(|k| e.stars[k] * e.lines_by_stars[k]).sum::<f64>() / (1.0 - p_t) / lines;
    let wild = e.wild_pay / lines + (3..6).map(|k| e.wild_stars[k] * fs(slots::STAR_SPINS[k - 3] as f64, 3.0)).sum::<f64>();
    let q = slots::CHANCE_PPM as f64 / 1e6;
    let cost = |f: Feature| slots::price(f) as f64 / 100.0;
    let (t3, t10) = (given_t(3.0), given_t(slots::SUPER_FREE_X as f64));
    for (what, rtp) in [
        ("Chance", (q * t3 + (1.0 - q) * given_n) / cost(Feature::Chance)),
        ("Wild", wild / cost(Feature::Wild)),
        ("Bonus", t3 / cost(Feature::Bonus)),
        ("Super", t10 / cost(Feature::Super)),
        ("Mystery", (0.7 * t3 + 0.3 * t10) / cost(Feature::Mystery)),
        ("All or nothing", slots::ALL_IN_X as f64 * slots::ALL_IN_PER_10K as f64 / 10_000.0 / cost(Feature::AllIn)),
    ] {
        assert!((0.975..0.985).contains(&rtp), "{what}: {rtp}");
    }
    let mut st = s(5);
    for f in [Feature::Bonus, Feature::Super, Feature::Mystery] {
        let o = slots::play_feature(5, f, &mut st, 0);
        assert!(o.spins[0].stars >= 3 && o.spins.len() > 1, "{f:?}");
        assert_eq!(o.free_x, if f == Feature::Super || o.lucky { 10 } else { 3 });
        assert_eq!(o.wagered(), 100 * slots::price(f) / 100);
        assert_eq!(o.paid, o.spins.iter().map(|x| x.pay).sum::<u64>());
    }
    let o = slots::play_feature(5, Feature::Wild, &mut st, 1_000_000);
    assert!(o.spins[0].wild_reel && o.spins[0].grid()[slots::WILD_REEL] == [slots::WILD; slots::ROWS]);
    assert_eq!(o.jackpot_won, 0, "no jackpot on a Wild spin");
    // A round with a feature checks out like any other.
    let seed = [4u8; 32];
    let out = slots::play_feature(2, Feature::Chance, &mut stream(&seed, &seed, Game::Slots, 9), 0);
    verify(&commit(&seed), &seed, &seed, 9, &Outcome::Slots(out)).unwrap();
}
