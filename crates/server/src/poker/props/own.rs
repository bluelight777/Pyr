//! The casino's props about itself (Pyr 0.15): whether its house comes out ahead tomorrow,
//! which game takes the most in bets, whether a jackpot falls. The feed can't know a server's
//! own casino, so each server makes these from its own count of rounds (`super::super::casino`),
//! a day (UTC) at a time: tomorrow's open today, close as the day starts, say how it's going
//! while it's on, and settle by themselves once it's over. Nobody settles anything.
//!
//! Whoever plays moves these numbers. So the casino stakes less on them than on the feed's
//! (`SHALLOWER`), and nothing is asked that one bet decides (the day's biggest win, how many
//! rounds are played): what's asked takes chips at the tables to push about.

use std::collections::BTreeMap;

use pyr_poker::casino::Game;
use pyr_poker::net::PeriodStats;
use pyr_poker::props::{self as p, PropState};
use pyr_proto::props_feed::FeedProp;

use super::super::casino::{DAYS, JACKPOT_DAYS};
use super::super::{Poker, State, progress_chips};
use super::{AutoKept, Prop, WORTH_OPENING_MS, tell_house};
use crate::hub::Hub;

const DAY_MS: i64 = 86_400_000;
/// A day's numbers are final this long after it ends (a round is counted as it finishes).
const FINAL_AFTER_MS: i64 = 2 * 60_000;
/// How much less deep a prop about the casino is than one of the feed's.
const SHALLOWER: u64 = 4;
/// Nothing's offered about a casino nobody has played in for this many days.
const QUIET_DAYS: u64 = 7;
/// Whose numbers these are.
const SOURCE: &str = "This casino";
const ANOTHER: &str = "Another game";

/// A day of the casino, over all its games.
#[derive(Clone, Debug, Default)]
struct Day {
    rounds: u64,
    wagered: u64,
    paid: u64,
    /// What was bet, by game (Reel Row's machines together).
    games: BTreeMap<String, u64>,
    jackpots: u32,
}

impl Day {
    /// The game the most was bet on, and whether another had as much.
    fn busiest(&self) -> Option<(&str, u64, bool)> {
        let (name, most) = self.games.iter().filter(|g| *g.1 > 0).max_by_key(|g| *g.1)?;
        Some((name.as_str(), *most, self.games.iter().filter(|g| g.1 == most).count() > 1))
    }
}

/// The game a statistics key is of, by its name on the floor (`reel_row:3` is Reel Row).
fn game_name(key: &str) -> String {
    let id = key.split(':').next().unwrap_or(key);
    Game::from_id(id).map(|g| g.name().to_string()).unwrap_or_else(|| id.to_string())
}

/// Every day the casino has a count for (the last four weeks), by its number since 1970.
fn days(hub: &Hub) -> BTreeMap<u64, Day> {
    let mut out: BTreeMap<u64, Day> = BTreeMap::new();
    for (key, value) in hub.db.meta_with_prefix(DAYS) {
        let Ok(by_day) = serde_json::from_str::<BTreeMap<u64, PeriodStats>>(&value) else { continue };
        let game = game_name(key.trim_start_matches(DAYS));
        for (day, s) in by_day {
            let d = out.entry(day).or_default();
            d.rounds += s.rounds;
            d.wagered += s.wagered;
            d.paid += s.paid;
            *d.games.entry(game.clone()).or_default() += s.wagered;
        }
    }
    let jackpots: BTreeMap<u64, u32> = hub.db.meta_json(JACKPOT_DAYS).unwrap_or_default();
    for (day, n) in jackpots {
        out.entry(day).or_default().jackpots += n;
    }
    out
}

/// "Wednesday, November 18", of a day since 1970 (UTC).
fn day_and_date(day: u64) -> String {
    const NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    // (Howard Hinnant's `civil_from_days`.)
    let z = day as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{}, {} {d}", NAMES[((day + 3) % 7) as usize], MONTHS[(m - 1) as usize])
}

/// A prop's day and kind, from the name the casino gave it (`casino:20781:house`).
fn day_and_kind(feed_id: &str) -> Option<(u64, &str)> {
    let mut parts = feed_id.split(':');
    (parts.next()? == p::CASINO).then_some(())?;
    Some((parts.next()?.parse().ok()?, parts.next()?))
}

/// The up or down of the house over a day, in words: "up 3,200 chips over 412 rounds".
fn house_words(d: &Day) -> String {
    let rounds = if d.rounds == 1 { "1 round".to_string() } else { format!("{} rounds", progress_chips(d.rounds)) };
    match d.wagered.cmp(&d.paid) {
        std::cmp::Ordering::Greater => format!("up {} chips over {rounds}", progress_chips(d.wagered - d.paid)),
        std::cmp::Ordering::Less => format!("down {} chips over {rounds}", progress_chips(d.paid - d.wagered)),
        std::cmp::Ordering::Equal => format!("even over {rounds}"),
    }
}

/// The props for a day that hasn't started: what they ask, and the odds they open at, from
/// the days the casino was played in before it (`before`, oldest first; `pool`: the Phoenix
/// Jackpot as it stands).
fn for_day(day: u64, before: &[&Day], pool: u64) -> Vec<FeedProp> {
    let on = day_and_date(day);
    let at = day as i64 * DAY_MS;
    let played = before.len() as f64;
    let base = |kind: &str, featured: u8| FeedProp {
        id: format!("{}:{day}:{kind}", p::CASINO),
        cat: p::CASINO.into(),
        event: format!("{}:{day}", p::CASINO),
        event_name: format!("The casino on {on}"),
        closes: at,
        starts: at,
        source: SOURCE.into(),
        featured,
        ..Default::default()
    };
    let yes_no = || vec!["Yes".to_string(), "No".to_string()];
    let mut out = Vec::new();

    // Whether the house comes out ahead. (It has the edge, but a day is short: its record
    // here, drawn toward six days in ten.)
    let ahead = before.iter().filter(|d| d.wagered > d.paid).count() as f64;
    let chance = ((ahead + 0.6 * 6.0) / (played + 6.0)).clamp(0.25, 0.85);
    let mut house = base("house", 2);
    house.q = format!("Does the house come out ahead on {on}?");
    house.outcomes = yes_no();
    house.prior = vec![chance, 1.0 - chance];
    house.detail = format!("It did on {} of the last {} days it was played", ahead as u64, played as u64);
    house.rules = "Yes if, over every round finished in this server's casino that day (UTC), more chips were bet than were paid back. Called off if nobody plays, or if it comes out exactly even.".into();
    out.push(house);

    // Which game takes the most in bets: the week's four biggest, or another.
    let mut bet: BTreeMap<&str, u64> = BTreeMap::new();
    for d in before.iter().rev().take(7) {
        for (g, n) in &d.games {
            *bet.entry(g.as_str()).or_default() += n;
        }
    }
    let mut games: Vec<(&str, u64)> = bet.into_iter().filter(|g| g.1 > 0).collect();
    games.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    if games.len() >= 2 {
        let all: u64 = games.iter().map(|g| g.1).sum();
        let named: Vec<(&str, u64)> = games.iter().take(4).copied().collect();
        // (Softened: a week's favorite isn't that sure of a day.)
        let soft = |share: f64| share.max(0.03).powf(0.6);
        let rest = 1.0 - named.iter().map(|g| g.1 as f64 / all as f64).sum::<f64>();
        let mut prior: Vec<f64> = named.iter().map(|g| soft(g.1 as f64 / all as f64)).collect();
        prior.push(soft(rest));
        let sum: f64 = prior.iter().sum();
        let mut busiest = base("busiest", 1);
        busiest.q = format!("Which game takes the most in bets on {on}?");
        busiest.outcomes = named.iter().map(|g| g.0.to_string()).chain([ANOTHER.to_string()]).collect();
        busiest.prior = prior.into_iter().map(|x| x / sum).collect();
        busiest.detail = format!("The last week's: {}", named[0].0);
        busiest.rules = "The game the most chips are bet on, over every round finished in this server's casino that day (UTC); Reel Row's machines count together. Called off if nobody plays, or if two games tie.".into();
        out.push(busiest);
    }

    // Whether a jackpot is won. (Rare: its record here, drawn toward one day in ten.)
    let won = before.iter().filter(|d| d.jackpots > 0).count() as f64;
    let chance = ((won + 0.1 * 10.0) / (played + 10.0)).clamp(0.03, 0.9);
    let mut jackpot = base("jackpot", 1);
    jackpot.q = format!("Is a jackpot won on {on}?");
    jackpot.outcomes = yes_no();
    jackpot.prior = vec![chance, 1.0 - chance];
    jackpot.detail = format!("The Phoenix Jackpot stands at {} chips", progress_chips(pool));
    jackpot.rules = "Yes if the Phoenix Jackpot, a Reel Row progressive or an Ember Dragon jackpot is won in this server's casino that day (UTC). Called off if nobody plays.".into();
    out.push(jackpot);
    out
}

/// How a day that's on is going, for a prop of a kind.
fn so_far(kind: &str, d: &Day) -> String {
    if d.rounds == 0 {
        return "So far: nobody has played".into();
    }
    match kind {
        "house" => format!("So far: the house is {}", house_words(d)),
        "busiest" => match d.busiest() {
            Some((name, most, false)) => format!("So far: {name}, with {} chips bet", progress_chips(most)),
            _ => "So far: it's a tie".into(),
        },
        "jackpot" => match d.jackpots {
            0 => "So far: none".into(),
            n => format!("So far: {n} won"),
        },
        _ => String::new(),
    }
}

/// What came of a day that's over, for a prop of a kind with these outcomes: the one that won
/// (none: it's called off), and a word about it.
fn came_of(kind: &str, d: &Day, outcomes: &[String]) -> (Option<usize>, String) {
    let named = |name: &str| outcomes.iter().position(|o| o == name);
    if d.rounds == 0 {
        return (None, "nobody played that day".into());
    }
    match kind {
        "house" => match d.wagered.cmp(&d.paid) {
            std::cmp::Ordering::Equal => (None, "the house came out exactly even".into()),
            o => (named(if o == std::cmp::Ordering::Greater { "Yes" } else { "No" }), format!("The house was {}", house_words(d))),
        },
        "busiest" => match d.busiest() {
            Some((_, _, true)) => (None, "two games tied".into()),
            Some((name, most, false)) => (named(name).or_else(|| named(ANOTHER)), format!("{name}, with {} chips bet", progress_chips(most))),
            None => (None, "nothing was bet that day".into()),
        },
        "jackpot" => match d.jackpots {
            0 => (named("No"), "None was won".into()),
            1 => (named("Yes"), "One was won".into()),
            n => (named("Yes"), format!("{n} were won")),
        },
        _ => (None, "it isn't one this Pyr knows".into()),
    }
}

impl Poker {
    /// The casino's props about itself, looked over (a minute, with the others): the ones
    /// whose day is on say how it's going, the ones whose day is over settle, and tomorrow's
    /// open (when the casino's own props are on here, and this category of them).
    pub(super) fn props_about_casino(&self, hub: &Hub, st: &State, now: i64) {
        let days = days(hub);
        let today = (now.max(0) / DAY_MS) as u64;
        let mut told: Vec<Prop> = Vec::new();
        for j in hub.db.props_open() {
            let Ok(mut prop) = serde_json::from_str::<Prop>(&j) else { continue };
            if !prop.house || prop.category != p::CASINO {
                continue;
            }
            let Some((day, kind)) = day_and_kind(&prop.feed_id).map(|(d, k)| (d, k.to_string())) else { continue };
            let that = days.get(&day).cloned().unwrap_or_default();
            if now >= (day as i64 + 1) * DAY_MS + FINAL_AFTER_MS {
                let (winner, note) = came_of(&kind, &that, &prop.outcomes);
                let done = match winner {
                    Some(w) => Self::settle(hub, st, &mut prop, w, "", &note, now),
                    None => Self::void(hub, st, &mut prop, "", &note, now),
                };
                if done.is_ok() {
                    told.push(prop);
                }
            } else if now >= day as i64 * DAY_MS {
                let detail = so_far(&kind, &that);
                if detail != prop.detail || prop.state == PropState::Open {
                    prop.detail = detail;
                    prop.taking_bets(now);
                    if prop.save(hub).is_ok() {
                        told.push(prop);
                    }
                }
            }
        }
        let kept = AutoKept::load(hub);
        let tomorrow = today + 1;
        // (Oldest first; only days it was played on count toward the odds.)
        let before: Vec<&Day> = days.range(tomorrow.saturating_sub(28)..tomorrow).map(|d| d.1).filter(|d| d.rounds > 0).collect();
        let lately = days.range(tomorrow.saturating_sub(QUIET_DAYS)..tomorrow).any(|d| d.1.rounds > 0);
        if kept.takes(p::CASINO) && lately {
            let known = hub.db.prop_feed_map();
            for fp in for_day(tomorrow, &before, Self::jackpot(hub)) {
                if known.contains_key(&fp.id) || fp.closes < now + WORTH_OPENING_MS {
                    continue;
                }
                match Self::house_open(hub, &fp, (kept.depth() / SHALLOWER).max(1), now) {
                    Ok(prop) => told.push(prop),
                    Err(e) => tracing::debug!(prop = %fp.id, "a prop about the casino wasn't opened: {e}"),
                }
            }
        }
        tell_house(hub, &told);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;
    use pyr_poker::net::{ToApp, ToServer};
    use pyr_poker::progress::Record;
    use pyr_poker::props::{PropAutoSet, PropView};

    use super::super::Asker;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-props-own-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ask(hub: &Hub, uid: &str, manages: bool, msg: ToServer) -> Result<Option<ToApp>, String> {
        let who = Asker { uid, sees: &|_| true, moderates: &|_| false, manages, house: true, conn: None };
        hub.poker.props(hub, &who, msg)
    }

    fn turn_on(hub: &Hub, off: &[&str]) {
        ask(hub, "owner", true, ToServer::PropAuto { set: Some(PropAutoSet { on: true, off: off.iter().map(|c| c.to_string()).collect(), depth: 100_000 }) }).unwrap();
    }

    /// A game's count for a day, as the casino keeps it.
    fn played(hub: &Hub, game: &str, day: u64, rounds: u64, wagered: u64, paid: u64) {
        let key = format!("{DAYS}{game}");
        let mut by: BTreeMap<u64, PeriodStats> = hub.db.meta_json(&key).unwrap_or_default();
        by.insert(day, PeriodStats { rounds, wagered, paid, ..Default::default() });
        hub.db.set_meta(&key, &serde_json::to_string(&by).unwrap()).unwrap();
    }

    fn look(hub: &Hub, now: i64) {
        let st = hub.poker.state.lock();
        hub.poker.props_about_casino(hub, &st, now);
    }

    fn all(hub: &Hub, uid: &str) -> Vec<PropView> {
        match ask(hub, uid, false, ToServer::PropList { channel_id: 0 }).unwrap() {
            Some(ToApp::Props { props }) => props,
            other => panic!("{other:?}"),
        }
    }

    fn chips(hub: &Hub, uid: &str) -> u64 {
        hub.db.poker_record(uid).0.chips
    }

    /// "Today", for these: a few days from now by the real clock, since bets are taken by it
    /// (a prop for the day after is still open, whenever this runs).
    fn today() -> u64 {
        (pyr_proto::now_ms() / DAY_MS) as u64 + 3
    }

    fn noon(day: u64) -> i64 {
        day as i64 * DAY_MS + DAY_MS / 2
    }

    #[test]
    fn days_are_said_by_their_names() {
        // (Day 0 was a Thursday; November 17, 2026 is a Tuesday.)
        assert_eq!(day_and_date(0), "Thursday, January 1");
        assert_eq!(day_and_date(20_774), "Tuesday, November 17");
        assert_eq!(day_and_date(20_775), "Wednesday, November 18");
        assert_eq!(day_and_date(19_782), "Thursday, February 29");
        assert_eq!(day_and_kind("casino:20775:house"), Some((20_775, "house")));
        assert_eq!(day_and_kind("nfl:20775:win"), None);
    }

    #[test]
    fn nothing_about_a_casino_nobody_plays_in_or_where_they_re_off() {
        let hub = hub();
        #[allow(non_snake_case)]
        let TODAY = today();
        look(&hub, noon(TODAY));
        assert!(all(&hub, "bob").is_empty(), "off until someone who manages the server turns them on");
        turn_on(&hub, &[]);
        look(&hub, noon(TODAY));
        assert!(all(&hub, "bob").is_empty(), "nobody has played here");
        // Played, but more than a week ago.
        played(&hub, "blackjack", TODAY - 9, 10, 1_000, 900);
        look(&hub, noon(TODAY));
        assert!(all(&hub, "bob").is_empty());
        // Played yesterday; but the category's off.
        played(&hub, "blackjack", TODAY - 1, 10, 1_000, 900);
        turn_on(&hub, &["casino"]);
        look(&hub, noon(TODAY));
        assert!(all(&hub, "bob").is_empty());
        turn_on(&hub, &[]);
        look(&hub, noon(TODAY));
        assert_eq!(all(&hub, "bob").len(), 2, "the house and the jackpot (one game only: no busiest)");
    }

    #[test]
    fn tomorrow_s_open_today_from_the_casino_s_own_record() {
        let hub = hub();
        #[allow(non_snake_case)]
        let TODAY = today();
        turn_on(&hub, &[]);
        // Three days of play: the house ahead on two of them; blackjack the most bet on.
        played(&hub, "blackjack", TODAY - 3, 50, 40_000, 36_000);
        played(&hub, "slots", TODAY - 3, 200, 10_000, 9_000);
        played(&hub, "blackjack", TODAY - 2, 50, 30_000, 34_000);
        played(&hub, "reel_row:2", TODAY - 1, 90, 6_000, 3_000);
        played(&hub, "reel_row:5", TODAY - 1, 90, 6_000, 5_000);
        hub.db.set_meta(JACKPOT_DAYS, &serde_json::to_string(&BTreeMap::from([(TODAY - 3, 1u32)])).unwrap()).unwrap();
        look(&hub, noon(TODAY));
        let props = all(&hub, "bob");
        assert_eq!(props.len(), 3);
        let of = |kind: &str| props.iter().find(|x| x.question.starts_with(kind)).unwrap_or_else(|| panic!("{kind}")).clone();
        let house = of("Does the house");
        let on = day_and_date(TODAY + 1);
        assert_eq!(house.question, format!("Does the house come out ahead on {on}?"));
        assert_eq!((house.category.as_str(), house.house, house.event_name.clone()), ("casino", true, format!("The casino on {on}")));
        assert_eq!((house.closes_at, house.starts_at), ((TODAY as i64 + 1) * DAY_MS, (TODAY as i64 + 1) * DAY_MS));
        assert_eq!(house.detail, "It did on 2 of the last 3 days it was played");
        assert_eq!(house.source, "This casino");
        // (Two of three, drawn toward six in ten: 5.6 of 9.)
        assert!((house.prices()[0] - 5.6 / 9.0).abs() < 1e-6, "{:?}", house.prices());
        // A quarter as deep as the feed's.
        assert_eq!(house.b, 25_000.0);
        let busiest = of("Which game");
        assert_eq!(busiest.outcomes, vec!["Blackjack", "Reel Row", "Pyr Sevens", "Another game"]);
        let odds = busiest.prices();
        assert!(odds[0] > odds[1] && odds[1] > odds[2] && odds[2] > odds[3] && odds[3] > 0.02, "{odds:?}");
        assert!((odds.iter().sum::<f64>() - 1.0).abs() < 1e-9);
        assert_eq!(busiest.detail, "The last week's: Blackjack");
        let jackpot = of("Is a jackpot");
        // (One day of three, drawn toward one in ten: 2 of 13.)
        assert!((jackpot.prices()[0] - 2.0 / 13.0).abs() < 1e-6, "{:?}", jackpot.prices());
        assert!(jackpot.detail.starts_with("The Phoenix Jackpot stands at "));
        // Looked over again, nothing's opened twice.
        look(&hub, noon(TODAY) + 60_000);
        assert_eq!(all(&hub, "bob").len(), 3);
    }

    #[test]
    fn they_close_as_the_day_starts_say_how_it_s_going_and_settle_when_it_s_over() {
        let hub = hub();
        #[allow(non_snake_case)]
        let TODAY = today();
        turn_on(&hub, &[]);
        for u in ["bob", "carol"] {
            let mut rec = Record::new();
            rec.chips = 10_000;
            hub.db.set_poker_record(u, &rec, 0).unwrap();
        }
        played(&hub, "blackjack", TODAY - 1, 50, 40_000, 36_000);
        played(&hub, "roulette", TODAY - 1, 50, 20_000, 21_000);
        look(&hub, noon(TODAY));
        let props = all(&hub, "bob");
        let id_of = |kind: &str| props.iter().find(|x| x.question.starts_with(kind)).unwrap().id;
        let (house, busiest, jackpot) = (id_of("Does the house"), id_of("Which game"), id_of("Is a jackpot"));
        // Bob says the house wins and roulette's the busiest; Carol says a jackpot falls.
        ask(&hub, "bob", false, ToServer::PropBet { id: house, outcome: 0, chips: 1_000, min_shares: 0.0 }).unwrap();
        ask(&hub, "bob", false, ToServer::PropBet { id: busiest, outcome: 1, chips: 1_000, min_shares: 0.0 }).unwrap();
        ask(&hub, "carol", false, ToServer::PropBet { id: jackpot, outcome: 0, chips: 500, min_shares: 0.0 }).unwrap();
        let (bob, carol) = (chips(&hub, "bob"), chips(&hub, "carol"));
        assert_eq!((bob, carol), (8_000, 9_500));
        // The day starts: no more bets; how it's going is said as it goes.
        let day = TODAY + 1;
        let start = day as i64 * DAY_MS;
        look(&hub, start + 5_000);
        let now = all(&hub, "bob");
        let get = |props: &[PropView], id: u64| props.iter().find(|x| x.id == id).unwrap().clone();
        assert_eq!(get(&now, house).state, pyr_poker::props::PropState::Closed);
        assert_eq!(get(&now, house).detail, "So far: nobody has played");
        played(&hub, "blackjack", day, 20, 9_000, 7_500);
        played(&hub, "roulette", day, 30, 12_000, 12_400);
        look(&hub, start + 3_600_000);
        let now = all(&hub, "bob");
        assert_eq!(get(&now, house).detail, "So far: the house is up 1,100 chips over 50 rounds");
        assert_eq!(get(&now, busiest).detail, "So far: Roulette, with 12,000 chips bet");
        assert_eq!(get(&now, jackpot).detail, "So far: none");
        // Not settled the moment the day ends (a round may still be counted), but soon after.
        look(&hub, start + DAY_MS + 30_000);
        assert_eq!(get(&all(&hub, "bob"), house).state, pyr_poker::props::PropState::Closed);
        look(&hub, start + DAY_MS + FINAL_AFTER_MS);
        let over = all(&hub, "bob");
        let h = get(&over, house);
        assert_eq!((h.state, h.winner, h.note.as_str()), (pyr_poker::props::PropState::Settled, Some(0), "The house was up 1,100 chips over 50 rounds"));
        let b = get(&over, busiest);
        assert_eq!((b.state, b.winner, b.note.as_str()), (pyr_poker::props::PropState::Settled, Some(1), "Roulette, with 12,000 chips bet"));
        let j = get(&all(&hub, "carol"), jackpot);
        assert_eq!((j.state, j.winner, j.note.as_str()), (pyr_poker::props::PropState::Settled, Some(1), "None was won"));
        // Bob won both (a chip a share); Carol's bet lost.
        assert!(chips(&hub, "bob") > 10_000, "{}", chips(&hub, "bob"));
        assert_eq!(chips(&hub, "carol"), 9_500);
    }

    #[test]
    fn a_day_nobody_played_is_called_off_and_a_game_not_named_is_another_game() {
        let hub = hub();
        #[allow(non_snake_case)]
        let TODAY = today();
        turn_on(&hub, &[]);
        let mut rec = Record::new();
        rec.chips = 10_000;
        hub.db.set_poker_record("bob", &rec, 0).unwrap();
        for (game, bet) in [("blackjack", 50_000), ("roulette", 40_000), ("craps", 30_000), ("slots", 20_000), ("keno", 1_000)] {
            played(&hub, game, TODAY - 1, 10, bet, bet - 100);
        }
        look(&hub, noon(TODAY));
        let props = all(&hub, "bob");
        let busiest = props.iter().find(|x| x.question.starts_with("Which game")).unwrap().clone();
        assert_eq!(busiest.outcomes, vec!["Blackjack", "Roulette", "Craps", "Pyr Sevens", "Another game"]);
        let house = props.iter().find(|x| x.question.starts_with("Does the house")).unwrap().id;
        ask(&hub, "bob", false, ToServer::PropBet { id: house, outcome: 0, chips: 1_000, min_shares: 0.0 }).unwrap();
        ask(&hub, "bob", false, ToServer::PropBet { id: busiest.id, outcome: 4, chips: 1_000, min_shares: 0.0 }).unwrap();
        // Tomorrow only Keno is played (it wasn't one of the four named).
        let day = TODAY + 1;
        played(&hub, "keno", day, 5, 500, 0);
        look(&hub, (day as i64 + 1) * DAY_MS + FINAL_AFTER_MS);
        let over = all(&hub, "bob");
        let b = over.iter().find(|x| x.id == busiest.id).unwrap();
        assert_eq!((b.winner, b.note.as_str()), (Some(4), "Keno, with 500 chips bet"));
        assert!(chips(&hub, "bob") > 9_000);
        // That look opened the props for the day after next. Nobody plays on it at all: what
        // was bet on them comes back.
        let later = day + 2;
        let open: Vec<PropView> = all(&hub, "bob").into_iter().filter(|x| x.closes_at == later as i64 * DAY_MS).collect();
        assert_eq!(open.len(), 3);
        let before = chips(&hub, "bob");
        ask(&hub, "bob", false, ToServer::PropBet { id: open[0].id, outcome: 0, chips: 700, min_shares: 0.0 }).unwrap();
        assert_eq!(chips(&hub, "bob"), before - 700);
        look(&hub, (later as i64 + 1) * DAY_MS + FINAL_AFTER_MS);
        let called_off = all(&hub, "bob").into_iter().find(|x| x.id == open[0].id).unwrap();
        assert_eq!((called_off.state, called_off.note.as_str()), (PropState::Void, "nobody played that day"));
        assert_eq!(chips(&hub, "bob"), before);
    }
}
