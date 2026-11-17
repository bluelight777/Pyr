//! Art. With public data only there's no feed of what happens in the art world, so this
//! category has two props a day of its own kind:
//!
//! **The Gallery Draw.** Six masterpieces are hung each day (from a catalog of works long in
//! the public domain) and one is drawn at 20:00 UTC, by drand: the League of Entropy's public
//! randomness beacon, a number nobody can know beforehand and anybody can check afterwards.
//! A game of chance, said plainly to be one.
//!
//! **The Astronomy Picture of the Day.** What NASA's picture for a day to come shows (a
//! galaxy, a nebula, the Sun, Moon or a planet, or something else), going by the words in its
//! title. Its answer's shape wasn't read from the workspace this was written in: it's the
//! documented one (`date`, `title`).

use serde_json::{Value, json};

use crate::engine::{Kept, Out, Source};
use crate::http::Fetch;
use crate::when::{self, DAY, HOUR, MIN};
use pyr_proto::props_feed::FeedProp;

/// The catalog: (what an outcome is called, who made it, about when).
pub const WORKS: [(&str, &str, &str); 72] = [
    ("Mona Lisa", "Leonardo da Vinci", "1503"),
    ("The Last Supper", "Leonardo da Vinci", "1498"),
    ("Lady with an Ermine", "Leonardo da Vinci", "1490"),
    ("The Birth of Venus", "Botticelli", "1485"),
    ("Primavera", "Botticelli", "1482"),
    ("The School of Athens", "Raphael", "1511"),
    ("Sistine Madonna", "Raphael", "1512"),
    ("The Creation of Adam", "Michelangelo", "1512"),
    ("The Arnolfini Portrait", "Jan van Eyck", "1434"),
    ("Garden of Earthly Delights", "Hieronymus Bosch", "1500"),
    ("The Tower of Babel", "Pieter Bruegel the Elder", "1563"),
    ("Hunters in the Snow", "Pieter Bruegel the Elder", "1565"),
    ("Venus of Urbino", "Titian", "1534"),
    ("Bacchus and Ariadne", "Titian", "1523"),
    ("The Ambassadors", "Hans Holbein the Younger", "1533"),
    ("Las Meninas", "Velázquez", "1656"),
    ("The Night Watch", "Rembrandt", "1642"),
    ("The Return of the Prodigal Son", "Rembrandt", "1669"),
    ("Girl with a Pearl Earring", "Vermeer", "1665"),
    ("The Milkmaid", "Vermeer", "1658"),
    ("View of Delft", "Vermeer", "1661"),
    ("The Calling of Saint Matthew", "Caravaggio", "1600"),
    ("Judith Slaying Holofernes", "Artemisia Gentileschi", "1620"),
    ("The Swing", "Fragonard", "1767"),
    ("The Third of May 1808", "Goya", "1814"),
    ("Liberty Leading the People", "Delacroix", "1830"),
    ("The Raft of the Medusa", "Géricault", "1819"),
    ("Wanderer above the Sea of Fog", "Caspar David Friedrich", "1818"),
    ("The Hay Wain", "John Constable", "1821"),
    ("The Fighting Temeraire", "J. M. W. Turner", "1839"),
    ("Rain, Steam and Speed", "J. M. W. Turner", "1844"),
    ("The Great Wave off Kanagawa", "Hokusai", "1831"),
    ("Fine Wind, Clear Morning", "Hokusai", "1831"),
    ("Sudden Shower over Ohashi", "Hiroshige", "1857"),
    ("Ophelia", "John Everett Millais", "1852"),
    ("The Gleaners", "Millet", "1857"),
    ("Olympia", "Manet", "1863"),
    ("A Bar at the Folies-Bergère", "Manet", "1882"),
    ("Impression, Sunrise", "Monet", "1872"),
    ("Water Lilies", "Monet", "1906"),
    ("Woman with a Parasol", "Monet", "1875"),
    ("Bal du moulin de la Galette", "Renoir", "1876"),
    ("Luncheon of the Boating Party", "Renoir", "1881"),
    ("The Dance Class", "Degas", "1874"),
    ("Paris Street; Rainy Day", "Caillebotte", "1877"),
    ("A Sunday on La Grande Jatte", "Seurat", "1886"),
    ("The Starry Night", "Vincent van Gogh", "1889"),
    ("Sunflowers", "Vincent van Gogh", "1888"),
    ("Café Terrace at Night", "Vincent van Gogh", "1888"),
    ("Wheatfield with Crows", "Vincent van Gogh", "1890"),
    ("The Card Players", "Cézanne", "1895"),
    ("Mont Sainte-Victoire", "Cézanne", "1904"),
    ("Where Do We Come From?", "Gauguin", "1898"),
    ("At the Moulin Rouge", "Toulouse-Lautrec", "1895"),
    ("The Sleeping Gypsy", "Henri Rousseau", "1897"),
    ("Whistler's Mother", "James McNeill Whistler", "1871"),
    ("The Gross Clinic", "Thomas Eakins", "1875"),
    ("Madame X", "John Singer Sargent", "1884"),
    ("The Gulf Stream", "Winslow Homer", "1899"),
    ("The Scream", "Edvard Munch", "1893"),
    ("The Kiss", "Gustav Klimt", "1908"),
    ("Adele Bloch-Bauer I", "Gustav Klimt", "1907"),
    ("The Thinker", "Rodin", "1904"),
    ("David", "Michelangelo", "1504"),
    ("Pietà", "Michelangelo", "1499"),
    ("Venus de Milo", "Alexandros of Antioch", "130 BC"),
    ("Winged Victory of Samothrace", "unknown", "190 BC"),
    ("Bust of Nefertiti", "Thutmose", "1345 BC"),
    ("Composition VII", "Kandinsky", "1913"),
    ("Black Square", "Malevich", "1915"),
    ("Flaming June", "Frederic Leighton", "1895"),
    ("The Dream", "Henri Rousseau", "1910"),
];

/// The six hung on a day: picked by the day's number alone, so anyone can see they weren't
/// chosen to suit anybody (a small shuffle seeded by the day).
pub fn hung(day: i64) -> Vec<usize> {
    let mut seed = (day.div_euclid(DAY) as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let mut picked: Vec<usize> = Vec::new();
    while picked.len() < 6 {
        let i = (next() % WORKS.len() as u64) as usize;
        if !picked.contains(&i) {
            picked.push(i);
        }
    }
    picked
}

/// drand's default beacon: a round every thirty seconds since July 2020.
const DRAND_GENESIS: i64 = 1_595_431_050;
const DRAND_PERIOD: i64 = 30;

/// The round that's made at a moment (Unix ms), or the first after it.
pub fn round_at(ms: i64) -> i64 {
    let secs = ms.div_euclid(1000);
    (secs - DRAND_GENESIS + DRAND_PERIOD - 1).div_euclid(DRAND_PERIOD) + 1
}

/// Which of `n` a round's randomness picks: its first sixteen hex digits as a number, modulo n.
pub fn drawn(randomness: &str, n: usize) -> Option<usize> {
    let head = randomness.trim().get(..16)?;
    u64::from_str_radix(head, 16).ok().map(|x| (x % n.max(1) as u64) as usize)
}

/// The Gallery Draw.
pub struct Gallery;

/// The hour of the day (UTC) it's drawn.
const DRAW_HOUR: i64 = 20;

impl Source for Gallery {
    fn name(&self) -> &'static str {
        "gallery"
    }

    fn every(&self, now: i64, mine: &[&Kept]) -> i64 {
        let next = mine.iter().filter_map(|k| k.memo["at"].as_i64()).filter(|t| *t + MIN > now).min();
        next.map_or(HOUR, |t| (t + MIN - now).clamp(MIN, HOUR))
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], _: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        // Tomorrow's and the day after's (needing nothing from anywhere to open).
        for d in 0..=2 {
            let at = when::day_start(now) + d * DAY + DRAW_HOUR * HOUR;
            let id = format!("art:draw:{}", when::yyyymmdd(at));
            if at - now < 2 * HOUR || mine.iter().any(|k| k.prop.id == id) {
                continue;
            }
            let works: Vec<&(&str, &str, &str)> = hung(at).into_iter().map(|i| &WORKS[i]).collect();
            let round = round_at(at);
            let mut artists: Vec<&str> = works.iter().map(|w| w.1).collect();
            artists.dedup();
            let p = FeedProp {
                id,
                cat: "art".into(),
                event: "art:draw".into(),
                event_name: "The Gallery Draw".into(),
                q: format!("The Gallery Draw: which masterpiece is drawn on {}?", when::day_and_date(at)),
                outcomes: works.iter().map(|w| w.0.to_string()).collect(),
                prior: vec![1.0 / 6.0; 6],
                closes: at - HOUR,
                starts: at,
                detail: format!("Drawn at {}:00 UTC · {}", DRAW_HOUR, works.iter().map(|w| format!("{} ({})", w.1, w.2)).collect::<Vec<_>>().join(", ")).chars().take(200).collect(),
                source: "drand".into(),
                rules: format!("A game of chance. One of the six is drawn by drand, the League of Entropy's public randomness beacon: round {round}, due at {}:00 UTC on {}. The first sixteen hex digits of its randomness, as a number, divided by six: the remainder picks the winner (0 is the first outcome). Anyone can check it at api.drand.sh/public/{round}. Called off if the round can't be read within two days.", DRAW_HOUR, when::day_and_date(at)),
                featured: 1,
                ..Default::default()
            };
            out.open(p, json!({"at": at, "round": round}));
        }
        for k in mine {
            let (Some(at), Some(round)) = (k.memo["at"].as_i64(), k.memo["round"].as_i64()) else { continue };
            if now < at + 20 * 1000 {
                continue;
            }
            let got = web.get(&format!("https://api.drand.sh/public/{round}")).and_then(|t| super::json(&t, "drand")).and_then(|v| {
                if v["round"].as_i64() != Some(round) {
                    return Err("drand answered with another round".to_string());
                }
                v["randomness"].as_str().and_then(|r| Some((drawn(r, k.prop.outcomes.len())?, r.to_string()))).ok_or_else(|| "drand's round has no randomness".to_string())
            });
            match got {
                Ok((winner, randomness)) => out.settle(&k.prop.id, winner, format!("round {round}: {}…", &randomness[..16])),
                Err(_) if now > at + 2 * DAY => out.call_off(&k.prop.id, "the round couldn't be read"),
                Err(e) => out.trouble.push(e),
            }
        }
        Ok(out)
    }
}

/// What a picture's title says it shows: 0 a galaxy, 1 a nebula, 2 the Sun, Moon or a
/// planet, 3 something else. By its words alone, in that order.
pub fn shows(title: &str) -> usize {
    let words: Vec<String> = title.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.to_string()).collect();
    let has = |list: &[&str]| words.iter().any(|w| list.contains(&w.as_str()));
    if has(&["galaxy", "galaxies"]) {
        0
    } else if has(&["nebula", "nebulae", "nebulas"]) {
        1
    } else if has(&["sun", "solar", "moon", "lunar", "eclipse", "mercury", "venus", "mars", "jupiter", "saturn", "uranus", "neptune", "pluto", "planet", "planets"]) {
        2
    } else {
        3
    }
}

/// NASA's Astronomy Picture of the Day.
pub struct Picture;

impl Source for Picture {
    fn name(&self) -> &'static str {
        "picture"
    }

    fn every(&self, _: i64, _: &[&Kept]) -> i64 {
        HOUR
    }

    fn run(&self, now: i64, web: &dyn Fetch, mine: &[&Kept], _: &mut Value) -> Result<Out, String> {
        let mut out = Out::default();
        // The next two days' pictures (a day's comes out at about 05:00 UTC; betting closes
        // as the day starts).
        for d in 1..=2 {
            let day = when::day_start(now) + d * DAY;
            let id = format!("art:apod:{}", when::yyyymmdd(day));
            if mine.iter().any(|k| k.prop.id == id) {
                continue;
            }
            let p = FeedProp {
                id,
                cat: "art".into(),
                event: "art:apod".into(),
                event_name: "Astronomy Picture of the Day".into(),
                q: format!("What does NASA's Astronomy Picture of the Day show on {}?", when::day_and_date(day)),
                outcomes: vec!["A galaxy".into(), "A nebula".into(), "The Sun, Moon or a planet".into(), "Something else".into()],
                prior: vec![0.12, 0.14, 0.24, 0.50],
                closes: day,
                starts: day,
                detail: "Going by the words in its title".into(),
                source: "NASA".into(),
                rules: "By the words in the picture's title (apod.nasa.gov), in this order: \"galaxy\" or \"galaxies\": a galaxy. Else \"nebula\" or \"nebulae\": a nebula. Else any of Sun, solar, Moon, lunar, eclipse, a planet's name (Pluto too), \"planet\" or \"planets\": the Sun, Moon or a planet. Else something else. Called off if no picture is published for that day within two days.".into(),
                featured: 1,
                ..Default::default()
            };
            out.open(p, json!({"day": day}));
        }
        for k in mine {
            let Some(day) = k.memo["day"].as_i64().filter(|d| now > d + 6 * HOUR) else { continue };
            let got = web.get(&format!("https://api.nasa.gov/planetary/apod?api_key=DEMO_KEY&date={}", when::iso_date(day))).and_then(|t| super::json(&t, "NASA")).and_then(|v| match (v["date"].as_str(), v["title"].as_str()) {
                (Some(d), Some(title)) if when::parse_date(d) == Some(day) && !title.trim().is_empty() => Ok(title.trim().to_string()),
                _ => Err("NASA has no picture for that day yet".to_string()),
            });
            match got {
                Ok(title) => out.settle(&k.prop.id, shows(&title), super::short(&title, 120)),
                Err(_) if now > day + 2 * DAY => out.call_off(&k.prop.id, "no picture was published for that day"),
                Err(e) => out.trouble.push(e),
            }
        }
        out.trouble.dedup();
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::{Maker, State, fit};
    use crate::http::Canned;

    #[test]
    fn the_catalog_fits_and_a_day_s_six_are_always_the_same_six() {
        for (i, w) in WORKS.iter().enumerate() {
            assert!(w.0.chars().count() <= 30, "{}", w.0);
            assert!(!WORKS[..i].iter().any(|o| o.0.eq_ignore_ascii_case(w.0)), "twice: {}", w.0);
        }
        let day = when::at(2026, 11, 19, 20, 0);
        let six = hung(day);
        assert_eq!(six.len(), 6);
        assert_eq!(six, hung(day - 5 * HOUR), "by the day, not the hour");
        assert_ne!(six, hung(day + DAY));
        let mut all = six.clone();
        all.sort();
        all.dedup();
        assert_eq!(all.len(), 6, "six different works");
        // Over a year every work is hung, and none hogs the wall.
        let mut seen = vec![0; WORKS.len()];
        for d in 0..365 {
            for i in hung(day + d * DAY) {
                seen[i] += 1;
            }
        }
        assert!(seen.iter().all(|n| *n > 8 && *n < 60), "{seen:?}");
    }

    #[test]
    fn drand_s_rounds_come_every_thirty_seconds_and_pick_one_of_the_six() {
        // Round 6647791 was made at 21:32:30 UTC on Nov 16 2026.
        let t = (DRAND_GENESIS + 6_647_790 * DRAND_PERIOD) * 1000;
        assert_eq!(when::iso(t), "2026-11-16T21:32:30Z");
        assert_eq!((round_at(t), round_at(t - 1000), round_at(t + 1000)), (6_647_791, 6_647_791, 6_647_792));
        assert_eq!(drawn("d2c83f99fca6ab79aac56f5a9862f174adc94386e6fcd4e8e01c31f1b0793c39", 6), Some((0xd2c8_3f99_fca6_ab79u64 % 6) as usize));
        assert_eq!(drawn("zz", 6), None);
    }

    #[test]
    fn the_gallery_draw_opens_by_itself_and_is_settled_by_the_beacon() {
        let now = when::at(2026, 11, 17, 12, 0);
        let mut m = Maker::new(State::default(), vec![Box::new(Gallery)]);
        assert!(m.step(now, &Canned::default(), false));
        let props = m.feed(now).props;
        assert_eq!(props.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), vec!["art:draw:20261117", "art:draw:20261118", "art:draw:20261119"]);
        let today = &props[0];
        fit(today, now).unwrap();
        assert_eq!((today.q.as_str(), today.closes, today.outcomes.len()), ("The Gallery Draw: which masterpiece is drawn on Tuesday, November 17?", when::at(2026, 11, 17, 19, 0), 6));
        let round = round_at(when::at(2026, 11, 17, 20, 0));
        assert!(today.rules.contains(&format!("round {round}, due at 20:00 UTC on Tuesday, November 17")), "{}", today.rules);
        assert!(today.detail.starts_with("Drawn at 20:00 UTC · "));
        // At 20:00 and a bit: the round is read and one of the six has won.
        let then = when::at(2026, 11, 17, 20, 1);
        let answer = format!(r#"{{"round":{round},"randomness":"d2c83f99fca6ab79aac56f5a9862f174adc94386e6fcd4e8e01c31f1b0793c39","signature":"8ecc"}}"#);
        let which = format!("api.drand.sh/public/{round}");
        let web = Canned::new(&[(which.as_str(), answer.as_str())]);
        assert!(m.step(then, &web, false));
        let r = m.feed(then).props.into_iter().find(|p| p.id == "art:draw:20261117").unwrap().result.unwrap();
        assert_eq!((r.winner, r.note), (Some((0xd2c8_3f99_fca6_ab79u64 % 6) as usize), format!("round {round}: d2c83f99fca6ab79…")));
        // Another round's answer isn't taken for this one.
        let wrong = answer.replace(&round.to_string(), "5");
        let mut m2 = Maker::new(State::default(), vec![Box::new(Gallery)]);
        m2.step(now, &Canned::default(), false);
        m2.step(then, &Canned::new(&[("api.drand.sh", wrong.as_str())]), false);
        assert!(m2.feed(then).props.iter().all(|p| p.result.is_none()));
    }

    #[test]
    fn the_picture_of_the_day_goes_by_the_words_in_its_title() {
        assert_eq!(shows("The Andromeda Galaxy in Ultraviolet"), 0);
        assert_eq!(shows("Galaxies in the River"), 0);
        assert_eq!(shows("NGC 7000: The North America Nebula"), 1);
        assert_eq!(shows("A Galaxy Behind the Nebula"), 0, "in order: a galaxy first");
        assert_eq!(shows("Total Lunar Eclipse over Tajikistan"), 2);
        assert_eq!(shows("Saturn's Rings from the Other Side"), 2);
        assert_eq!(shows("Sunset at the Portara"), 3, "the word, not a part of one");
        assert_eq!(shows("Comet Lemmon and the Milky Way"), 3);
        let now = when::at(2026, 11, 17, 12, 0);
        let mut m = Maker::new(State::default(), vec![Box::new(Picture)]);
        assert!(m.step(now, &Canned::default(), false));
        let props = m.feed(now).props;
        assert_eq!(props.iter().map(|p| (p.id.as_str(), p.closes)).collect::<Vec<_>>(), vec![("art:apod:20261118", when::at(2026, 11, 18, 0, 0)), ("art:apod:20261119", when::at(2026, 11, 19, 0, 0))]);
        fit(&props[0], now).unwrap();
        // Wednesday 07:00: its picture is out.
        let then = when::at(2026, 11, 18, 7, 0);
        let web = Canned::new(&[("date=2026-11-18", r#"{"date":"2026-11-18","explanation":"What's happening to this spiral galaxy?","hdurl":"https://apod.nasa.gov/apod/image/2611/x.jpg","media_type":"image","service_version":"v1","title":"NGC 7000: The North America Nebula","url":"https://apod.nasa.gov/apod/image/2611/x_1024.jpg"}"#)]);
        assert!(m.step(then, &web, false));
        let r = m.feed(then).props.into_iter().find(|p| p.id == "art:apod:20261118").unwrap().result.unwrap();
        assert_eq!((r.winner, r.note.as_str()), (Some(1), "NGC 7000: The North America Nebula"));
        // Yesterday's picture served for today isn't taken for today's.
        let late = when::at(2026, 11, 19, 7, 0);
        let stale = Canned::new(&[("apod", r#"{"date":"2026-11-18","title":"NGC 7000: The North America Nebula"}"#)]);
        m.step(late, &stale, false);
        assert!(m.feed(late).props.iter().find(|p| p.id == "art:apod:20261119").unwrap().result.is_none());
    }
}
