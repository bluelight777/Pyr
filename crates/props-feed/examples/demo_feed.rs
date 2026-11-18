//! A made-up feed, for trying a Pyr server's own props with no network and without the real
//! feed: props in every category, as the sources word them, at times counted from its first
//! run. Nothing in it is real: the games, the prices and the results are invented.
//!
//!   cargo run -p pyr-props --example demo_feed -- --out DIR --key FILE --cert FILE [--results] [--quick MINUTES] [--many N]
//!
//! The first run notes its time in DIR/demo.json; later runs keep the props' times. A few
//! props close `--quick` minutes after that first run (6 unless said): with `--results` those
//! have come out (one of them called off), as the feed says it when a game is final.
//! `--many N`: more invented games until the feed has N props (the real one runs to several
//! hundred), to see what a full feed asks of a server and of Prop Bets' window.
//!
//! A server follows it with `PYR_PROPS_FEED=<its address>` and `PYR_PROPS_FEED_KEY=<the public
//! half of the key that certified --key>` (see `crates/server/src/props_feed.rs`).

use std::path::PathBuf;

use pyr_proto::crypto::Identity;
use pyr_proto::props_feed::{Feed, FeedCert, FeedProp, FeedResult, MAX_PROPS, VERSION};
use pyr_props::engine;
use pyr_props::when::{self, DAY, HOUR, MIN};

const ESPN: &str = "Settled on the final score as ESPN reports it, overtime included. Called off if the game is postponed or canceled, or ends in a tie.";

struct Demo {
    t0: i64,
    quick: i64,
    results: bool,
    props: Vec<FeedProp>,
}

impl Demo {
    #[allow(clippy::too_many_arguments)]
    fn add(&mut self, id: &str, cat: &str, event: &str, event_name: &str, q: &str, outcomes: &[&str], prior: &[f64], closes: i64, starts: i64, detail: &str, source: &str, rules: &str, featured: u8) -> &mut FeedProp {
        self.props.push(FeedProp {
            id: id.into(),
            cat: cat.into(),
            event: event.into(),
            event_name: event_name.into(),
            q: q.into(),
            outcomes: outcomes.iter().map(|o| o.to_string()).collect(),
            prior: prior.to_vec(),
            closes,
            starts,
            detail: detail.into(),
            source: source.into(),
            rules: rules.into(),
            featured,
            result: None,
        });
        self.props.last_mut().unwrap()
    }

    /// How one of the quick ones came out, once `--results` says so.
    fn came_out(&mut self, id: &str, winner: Option<usize>, note: &str) {
        if !self.results {
            return;
        }
        let at = self.t0 + self.quick + 5 * MIN;
        if let Some(p) = self.props.iter_mut().find(|p| p.id == id) {
            p.result = Some(FeedResult { winner, note: note.into(), at });
        }
    }

    /// A game: who wins, the spread, the total, and its league's extra.
    #[allow(clippy::too_many_arguments)]
    fn game(&mut self, cat: &str, n: u32, away: (&str, &str), home: (&str, &str), start: i64, home_chance: f64, spread: f64, total: f64, where_: &str, featured: u8) {
        let event = format!("{cat}:4017720{n:02}");
        let name = format!("{} at {}", away.0, home.0);
        let (a, h) = (away.1, home.1);
        let signed = |x: f64| if x > 0.0 { format!("+{x}") } else { format!("{x}") };
        let unit = if cat == "nhl" { "goals" } else { "points" };
        let add = |me: &mut Demo, kind: &str, q: String, outcomes: Vec<String>, prior: Vec<f64>, f: u8| {
            let o: Vec<&str> = outcomes.iter().map(|s| s.as_str()).collect();
            me.add(&format!("{event}:{kind}"), cat, &event, &name, &q, &o, &prior, start, start, where_, "ESPN", ESPN, f);
        };
        add(self, "win", format!("Who wins: {name}?"), vec![a.into(), h.into()], vec![1.0 - home_chance, home_chance], featured + 2);
        add(self, "spread", format!("Against the spread: {a} at {h}?"), vec![format!("{a} {}", signed(-spread)), format!("{h} {}", signed(spread))], vec![0.5, 0.5], featured + 1);
        add(self, "total", format!("Total {unit}: {a} at {h}?"), vec![format!("Over {total}"), format!("Under {total}")], vec![0.49, 0.51], featured + 1);
        match cat {
            "nfl" => {
                add(self, "margin", format!("Margin of victory: {a} at {h}?"), ["1 to 3 points", "4 to 7 points", "8 to 14 points", "15 or more"].map(String::from).to_vec(), vec![0.22, 0.26, 0.25, 0.27], featured);
                add(self, "half", format!("Who leads at halftime: {a} at {h}?"), vec![a.into(), h.into(), "Tied".into()], vec![0.41, 0.52, 0.07], featured);
            }
            "nba" => add(self, "q1", format!("Who leads after the first quarter: {a} at {h}?"), vec![a.into(), h.into(), "Tied".into()], vec![0.44, 0.51, 0.05], featured),
            "nhl" => add(self, "ot", format!("Overtime or a shootout: {a} at {h}?"), vec!["Yes".into(), "No".into()], vec![0.22, 0.78], featured),
            _ => {}
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let (Some(out), Some(key), Some(cert)) = (arg("--out"), arg("--key"), arg("--cert")) else {
        eprintln!("demo_feed --out DIR --key FILE --cert FILE [--results] [--quick MINUTES] [--many N]");
        std::process::exit(2);
    };
    let out = PathBuf::from(out);
    std::fs::create_dir_all(&out).expect("the folder to write in");
    let now = pyr_proto::now_ms();
    // The time everything is counted from: this run's, unless one before it left its own.
    let noted = out.join("demo.json");
    let t0 = std::fs::read_to_string(&noted).ok().and_then(|t| t.trim().parse::<i64>().ok()).unwrap_or(now);
    let _ = std::fs::write(&noted, t0.to_string());
    let quick = arg("--quick").and_then(|m| m.parse::<i64>().ok()).unwrap_or(6) * MIN;
    let mut d = Demo { t0, quick, results: args.iter().any(|a| a == "--results"), props: Vec::new() };
    let soon = t0 + quick;
    let tomorrow = when::day_start(t0) + DAY;
    let day = when::day_and_date(tomorrow);
    let name = when::day_name(tomorrow);
    let week = when::week_start(t0) + 7 * DAY;
    let week_of = format!("the week of {}", when::month_day(week));

    // The leagues. The first NFL game is one of the quick ones.
    d.game("nfl", 1, ("Los Angeles Chargers", "Chargers"), ("Baltimore Ravens", "Ravens"), soon, 0.669, -4.5, 39.5, "M&T Bank Stadium · ESPN", 5);
    d.game("nfl", 2, ("Dallas Cowboys", "Cowboys"), ("Philadelphia Eagles", "Eagles"), t0 + 26 * HOUR, 0.61, -3.0, 46.5, "Lincoln Financial Field · FOX", 5);
    d.game("nfl", 3, ("Green Bay Packers", "Packers"), ("Chicago Bears", "Bears"), t0 + 29 * HOUR, 0.44, 1.5, 42.5, "Soldier Field · CBS", 5);
    d.game("nba", 1, ("Orlando Magic", "Magic"), ("Boston Celtics", "Celtics"), t0 + 5 * HOUR, 0.71, -6.5, 221.5, "TD Garden · NBCSN", 3);
    d.game("nba", 2, ("Los Angeles Lakers", "Lakers"), ("Golden State Warriors", "Warriors"), t0 + 8 * HOUR, 0.55, -1.5, 232.5, "Chase Center · TNT", 3);
    d.game("nhl", 1, ("Montreal Canadiens", "Canadiens"), ("New York Rangers", "Rangers"), t0 + 4 * HOUR + 30 * MIN, 0.489, 1.5, 6.5, "Madison Square Garden · TNT", 3);
    d.game("nhl", 2, ("Vancouver Canucks", "Canucks"), ("Utah Mammoth", "Mammoth"), t0 + 6 * HOUR + 30 * MIN, 0.748, -1.5, 6.5, "Delta Center · ESPN+", 3);
    d.game("cfb", 1, ("#12 Michigan Wolverines", "Michigan"), ("#2 Ohio State Buckeyes", "Ohio State"), t0 + 3 * DAY, 0.74, -9.5, 44.5, "Ohio Stadium · FOX", 5);
    d.game("cbb", 1, ("#5 Duke Blue Devils", "Duke"), ("#12 Kansas Jayhawks", "Kansas"), t0 + 7 * HOUR, 0.47, 1.5, 151.5, "Allen Fieldhouse · ESPN", 4);
    d.came_out("nfl:401772001:win", Some(1), "Chargers 20, Ravens 27");
    d.came_out("nfl:401772001:spread", Some(1), "Chargers 20, Ravens 27");
    d.came_out("nfl:401772001:total", Some(0), "47 points: Chargers 20, Ravens 27");
    d.came_out("nfl:401772001:margin", Some(1), "By 7: Chargers 20, Ravens 27");
    d.came_out("nfl:401772001:half", Some(1), "Chargers 10, Ravens 13 at the half");

    // Crypto: Bitcoin's day is a quick one.
    let coinbase = "Each price is the first trade of that minute on Coinbase. Called off if the two are the same or can't be had.";
    for (coin, n, featured) in [("btc", "Bitcoin", 3u8), ("eth", "Ether", 2), ("sol", "Solana", 1), ("xrp", "XRP", 1), ("doge", "Dogecoin", 1)] {
        let closes = if coin == "btc" { soon } else { tomorrow };
        d.add(&format!("crypto:{coin}:{}:up", when::yyyymmdd(tomorrow)), "crypto", &format!("crypto:{coin}"), n, &format!("{n}: up or down on {day}?"), &["Up", "Down"], &[0.5, 0.5], closes, closes, &format!("From 00:00 UTC on {name} to 00:00 UTC the day after"), "Coinbase", coinbase, featured);
    }
    d.add(
        &format!("crypto:btc:{}:range", when::yyyymmdd(tomorrow + DAY)),
        "crypto",
        "crypto:btc",
        "Bitcoin",
        &format!("Bitcoin's price at 00:00 UTC on {}?", when::day_and_date(tomorrow + DAY)),
        &["Under $61,750", "$61,750 to $64,000", "$64,000 to $66,500", "$66,500 to $68,750", "$68,750 or more"],
        &[0.08, 0.17, 0.42, 0.24, 0.09],
        tomorrow,
        tomorrow + DAY,
        "$65,231 when this opened",
        "Coinbase",
        "Bitcoin's price on Coinbase (BTC-USD) at that minute: its first trade. A price exactly on an edge counts for the range above it.",
        3,
    );
    d.came_out(&format!("crypto:btc:{}:up", when::yyyymmdd(tomorrow)), Some(0), "$65,231.10 to $66,012.44 (+1.20%)");

    // The weather.
    let nws = "as the National Weather Service's daily climate report for the city says, the morning after. Called off if no report comes within two days.";
    for (id, city, high, wet) in [("nyc", "New York", 55, 0.76), ("chi", "Chicago", 41, 0.30), ("mia", "Miami", 81, 0.30), ("sea", "Seattle", 49, 0.62)] {
        let (event, event_name) = (format!("weather:{id}:{}", when::yyyymmdd(tomorrow)), format!("{city}, {day}"));
        let ranges = [format!("{}° or lower", high - 4), format!("{}° to {}°", high - 3, high - 2), format!("{}° to {}°", high - 1, high + 1), format!("{}° to {}°", high + 2, high + 3), format!("{}° or higher", high + 4)];
        let ranges: Vec<&str> = ranges.iter().map(|s| s.as_str()).collect();
        d.add(&format!("{event}:high"), "weather", &event, &event_name, &format!("{city}: {name}'s high?"), &ranges, &[0.09, 0.19, 0.44, 0.19, 0.09], tomorrow, tomorrow, &format!("The forecast: {high}°F"), "National Weather Service", &format!("The highest temperature on {day}, in degrees Fahrenheit, {nws}"), if id == "nyc" { 2 } else { 1 });
        d.add(&format!("{event}:wet"), "weather", &event, &event_name, &format!("{city}: rain or snow on {name}?"), &["Yes", "No"], &[wet, 1.0 - wet], tomorrow, tomorrow, &format!("The forecast: {:.0}% chance", wet * 100.0), "National Weather Service", &format!("Yes if at least 0.01 inches of precipitation is measured on {day}, {nws}"), 1);
    }

    // Science: the launch is a quick one, and gets called off.
    let quakes = (format!("science:quake:{}", when::yyyymmdd(week)), format!("Earthquakes, {week_of}"));
    d.add(&format!("{}:big", quakes.0), "science", &quakes.0, &quakes.1, &format!("A magnitude 6.5 or stronger earthquake in {week_of}?"), &["Yes", "No"], &[0.545, 0.455], week, week, "41 in the last year", "USGS", "Yes if there's at least one earthquake of magnitude 6.5 or more, anywhere in the world, as the USGS earthquake catalog lists them a day after that week ends.", 1);
    d.add(&format!("{}:count", quakes.0), "science", &quakes.0, &quakes.1, &format!("How many magnitude 6+ earthquakes in {week_of}?"), &["1 or fewer", "2 or 3", "4 or 5", "6 or more"], &[0.27, 0.47, 0.21, 0.05], week, week, "135 in the last year: about 2.6 a week", "USGS", "Earthquakes of magnitude 6.0 or more, anywhere in the world, as the USGS earthquake catalog lists them a day after that week ends.", 1);
    let sun = (format!("science:sun:{}", when::yyyymmdd(week)), format!("The Sun, {week_of}"));
    d.add(&format!("{}:flare", sun.0), "science", &sun.0, &sun.1, &format!("The strongest solar flare in {week_of}?"), &["C class or weaker", "M1 to M4.9", "M5 to M9.9", "X class"], &[0.06, 0.69, 0.12, 0.13], week, week, "The last week's strongest: M1.1", "NOAA Space Weather Prediction Center", "The highest class among the flares that peaked that week (UTC), as NOAA's Space Weather Prediction Center lists them.", 1);
    d.add("science:launch:766170334d", "science", "science:launch:766170334d", "Falcon 9 Block 5 (USSF-317)", &format!("Falcon 9 Block 5 (USSF-317): liftoff by {} on {}?", when::hhmm_utc(soon + 6 * HOUR), when::day_and_date(soon + 6 * HOUR)), &["Yes", "No"], &[0.75, 0.25], soon, soon + 6 * HOUR, "Due an hour before that", "Launch Library 2", "Yes if Launch Library 2 shows it lifted off by then. Called off if the launch is taken off the schedule.", 2);
    d.add(&format!("science:asteroids:{}:count", when::yyyymmdd(week)), "science", &format!("science:asteroids:{}", when::yyyymmdd(week)), &format!("Asteroids, {week_of}"), &format!("How many asteroids pass closer than the Moon in {week_of}?"), &["1 or fewer", "2 or 3", "4 or 5", "6 or more"], &[0.38, 0.45, 0.14, 0.03], week, week, "17 in the last eight weeks", "NASA JPL", "As NASA JPL's close-approach table lists them two days after that week ends.", 1);
    d.came_out("science:launch:766170334d", None, "the launch was taken off the schedule");

    // Topical.
    let friday = week + 4 * DAY;
    let on_friday = when::day_and_date(friday);
    d.add(&format!("topical:ust10:{}:above", when::yyyymmdd(friday)), "topical", "topical:ust10", "The 10-year Treasury yield", &format!("The 10-year Treasury yield above 5.10% on {on_friday}?"), &["Yes", "No"], &[0.552, 0.448], week, friday, "5.12% when this opened", "US Treasury", "Yes if the 10-year par yield the US Treasury publishes for that day is above 5.10%.", 1);
    for (pair, q, at) in [("eurusd", "The euro above $1.100", "$1.1014"), ("usdjpy", "The dollar above ¥159.5", "¥159.60"), ("gbpusd", "The pound above $1.320", "$1.3198")] {
        d.add(&format!("topical:{pair}:{}:above", when::yyyymmdd(friday)), "topical", &format!("topical:money:{}", when::yyyymmdd(friday)), "Currencies", &format!("{q} on {on_friday}?"), &["Yes", "No"], &[0.52, 0.48], week, friday, &format!("{at} when this opened"), "European Central Bank", "By the European Central Bank's euro reference rates for that day.", 1);
    }
    d.add(&format!("topical:orders:{}:count", when::yyyymmdd(week)), "topical", "topical:orders", "Executive orders", &format!("How many executive orders are published in {week_of}?"), &["None", "One", "Two or three", "Four or more"], &[0.42, 0.36, 0.19, 0.03], week, week, "7 in the last eight weeks", "Federal Register", "Executive orders with a publication date in that week, as the Federal Register lists them.", 1);

    // Entertainment.
    let at = when::day_start(t0) + 2 * DAY + 18 * HOUR;
    d.add(&format!("ent:steam:730:{}:over", when::yyyymmdd(at)), "entertainment", "entertainment:steam", "Steam's biggest games", &format!("Counter-Strike 2: more than 1,240,000 players on Steam at 18:00 UTC on {}?", when::day_and_date(at)), &["Yes", "No"], &[0.52, 0.48], at - DAY, at, "1,243,817 at that hour when last seen", "Steam", "Yes if the number of players Steam reports in the game just after that hour is more than 1,240,000.", 2);
    d.add(&format!("ent:steam:570v578080:{}", when::yyyymmdd(at + DAY)), "entertainment", "entertainment:steam", "Steam's biggest games", &format!("More players on Steam at 18:00 UTC on {}: Dota 2 or PUBG: Battlegrounds?", when::day_and_date(at + DAY)), &["Dota 2", "PUBG: Battlegrounds"], &[0.85, 0.15], at, at + DAY, "612,044 and 540,210 at that hour when last seen", "Steam", "Whichever has the higher of the number of players Steam reports in each game just after that hour.", 1);

    // Art: today's Gallery Draw is a quick one.
    d.add(
        &format!("art:draw:{}", when::yyyymmdd(t0)),
        "art",
        "art:draw",
        "The Gallery Draw",
        &format!("The Gallery Draw: which masterpiece is drawn on {}?", when::day_and_date(t0)),
        &["The Starry Night", "Girl with a Pearl Earring", "The Great Wave off Kanagawa", "Las Meninas", "The Kiss", "Water Lilies"],
        &[1.0 / 6.0; 6],
        soon,
        soon + HOUR,
        "Vincent van Gogh (1889), Vermeer (1665), Hokusai (1831), Velázquez (1656), Gustav Klimt (1908), Monet (1906)",
        "drand",
        "A game of chance. One of the six is drawn by drand, the League of Entropy's public randomness beacon, from the round due at that hour.",
        1,
    );
    d.add(&format!("art:apod:{}", when::yyyymmdd(tomorrow)), "art", "art:apod", "Astronomy Picture of the Day", &format!("What does NASA's Astronomy Picture of the Day show on {day}?"), &["A galaxy", "A nebula", "The Sun, Moon or a planet", "Something else"], &[0.12, 0.14, 0.24, 0.50], tomorrow, tomorrow, "Going by the words in its title", "NASA", "By the words in the picture's title, in this order: galaxy, nebula, the Sun, Moon or a planet; else something else.", 1);
    d.came_out(&format!("art:draw:{}", when::yyyymmdd(t0)), Some(2), "Round 6650112 drew The Great Wave off Kanagawa");

    // --many: more games, a few days out, until there are that many props.
    if let Some(many) = arg("--many").and_then(|m| m.parse::<usize>().ok()) {
        const TEAMS: [&str; 24] = [
            "Hawks", "Nets", "Hornets", "Mavericks", "Nuggets", "Pistons", "Rockets", "Pacers", "Clippers", "Grizzlies", "Heat", "Bucks", "Timberwolves", "Pelicans", "Knicks", "Thunder", "Sixers", "Suns", "Blazers", "Kings", "Spurs", "Raptors",
            "Jazz", "Wizards",
        ];
        let leagues = [("nba", 221.5, 3u8), ("nhl", 6.5, 3), ("cfb", 48.5, 3), ("cbb", 148.5, 2), ("nfl", 44.5, 5)];
        let mut k = 0usize;
        while d.props.len() < many.min(MAX_PROPS) {
            let (cat, total, featured) = leagues[k % leagues.len()];
            // (Never the same team twice: 4k + 5 is odd.)
            let (a, h) = (TEAMS[(k * 7 + 3) % TEAMS.len()], TEAMS[(k * 11 + 8) % TEAMS.len()]);
            let start = t0 + (3 + (k as i64 * 37) % 150) * HOUR + (k as i64 % 4) * 15 * MIN;
            d.game(cat, 100 + k as u32, (&format!("The {a}"), a), (&format!("The {h}"), h), start, 0.35 + ((k * 17) % 40) as f64 / 100.0, -(1.5 + (k % 9) as f64), total + (k % 7) as f64, "The Arena · ESPN", if k % 9 == 0 { featured } else { 1 });
            k += 1;
        }
    }

    let bad: Vec<String> = d.props.iter().filter_map(|p| p.check().err().map(|e| format!("{}: {e}", p.id))).collect();
    assert!(bad.is_empty(), "props the feed wouldn't take: {bad:?}");
    let key = Identity::from_seed(pyr_proto::b64::decode_array::<32>(std::fs::read_to_string(&key).expect("the feed key").trim()).expect("a key"));
    let cert: FeedCert = serde_json::from_str(&std::fs::read_to_string(&cert).expect("the certificate")).expect("a certificate");
    assert_eq!(cert.key, key.user_id(), "the certificate is for another key");
    let n = d.props.len();
    let over = d.props.iter().filter(|p| p.result.is_some()).count();
    let feed = Feed { v: VERSION, made: now, props: d.props };
    engine::write_whole(&out.join("feed.json"), &engine::signed(&key, &cert, &feed)).expect("writing feed.json");
    println!("{} props ({over} of them over) in {}; the quick ones close at {}", n, out.join("feed.json").display(), when::iso(soon));
}
