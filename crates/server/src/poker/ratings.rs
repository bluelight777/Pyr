//! Poker's MMR on this server (see `pyr_poker::rating`): people's ratings are kept in their
//! poker records, bots' in a note of their own (each bot by its name and level, the same bot
//! wherever it sits). Cash hands and finished tournaments move them; tables show them.

use std::collections::BTreeMap;

use pyr_poker::bots::Level;
use pyr_poker::net::{RatingRow, TableView, ToApp};
use pyr_poker::rating::{self, Rating};
use serde::{Deserialize, Serialize};

use super::{Poker, State};
use crate::hub::Hub;

/// The database's note of the bots' ratings.
const BOTS: &str = "poker_bot_ratings";
/// The ladder's length.
const LADDER: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BotRating {
    name: String,
    level: Level,
    rating: Rating,
}

/// Who's being rated: their rating id, name, and a bot's level.
pub(super) type Rated = (String, String, Option<Level>);

impl Poker {
    fn bot_ratings(hub: &Hub) -> BTreeMap<String, BotRating> {
        hub.db.get_meta(BOTS).and_then(|v| serde_json::from_str(&v).ok()).unwrap_or_default()
    }

    fn set_bot_ratings(hub: &Hub, m: &BTreeMap<String, BotRating>) {
        if let Ok(j) = serde_json::to_string(m) {
            let _ = hub.db.set_meta(BOTS, &j);
        }
    }

    /// Rates something played (`f` moves the ratings, in `players`' order) and keeps the new
    /// ratings; `tell`: each person's app is sent their record (with its new MMR).
    pub(super) fn rate(hub: &Hub, st: &State, players: &[Rated], f: impl FnOnce(&mut [Rating]), tell: bool) {
        let mut bots = Self::bot_ratings(hub);
        let mut ratings: Vec<Rating> = players
            .iter()
            .map(|(id, _, bot)| match bot {
                Some(level) => bots.get(id).map(|b| b.rating).unwrap_or_else(|| Rating::for_bot(*level)),
                None => hub.db.poker_record(id).0.rating,
            })
            .collect();
        f(&mut ratings);
        let mut bots_changed = false;
        for ((id, name, bot), r) in players.iter().zip(ratings) {
            match bot {
                Some(level) => {
                    bots.insert(id.clone(), BotRating { name: name.split_whitespace().next().unwrap_or(name).to_string(), level: *level, rating: r });
                    bots_changed = true;
                }
                None => {
                    let (mut rec, _) = hub.db.poker_record(id);
                    rec.rating = r;
                    let held = st.held(id);
                    let _ = hub.db.set_poker_record(id, &rec, held);
                    if tell {
                        hub.send_poker(id, ToApp::Record { me: Self::view(hub, id, &rec, held), gains: None });
                    }
                }
            }
        }
        if bots_changed {
            Self::set_bot_ratings(hub, &bots);
        }
    }

    /// The MMR ladder: everyone rated here, people and bots, best first.
    pub(super) fn ladder(hub: &Hub) -> Vec<RatingRow> {
        let row = |id: String, name: String, bot: Option<Level>, r: &Rating| RatingRow {
            id,
            name,
            bot,
            mmr: r.mmr.round() as u32,
            peak: r.peak.round() as u32,
            tourneys: r.tourneys,
            hands: r.hands,
            placing: r.placing(),
        };
        let mut rows: Vec<RatingRow> = hub
            .db
            .poker_records()
            .into_iter()
            .filter(|(_, rec, _)| rec.rating.hands + rec.rating.tourneys > 0)
            .map(|(uid, rec, _)| row(uid.clone(), Self::name_of(hub, &uid), None, &rec.rating))
            .collect();
        rows.extend(Self::bot_ratings(hub).into_iter().map(|(id, b)| row(id, b.name, Some(b.level), &b.rating)));
        rows.sort_by(|a, b| b.mmr.cmp(&a.mmr).then(a.name.cmp(&b.name)));
        rows.truncate(LADDER);
        rows
    }

    /// Someone's MMR by their rating id (a bot not yet rated: its level's start).
    fn mmr_of(hub: &Hub, bots: &BTreeMap<String, BotRating>, id: &str, bot: Option<Level>) -> u32 {
        let r = match bot {
            Some(level) => bots.get(id).map(|b| b.rating).unwrap_or_else(|| Rating::for_bot(level)),
            None => hub.db.poker_record(id).0.rating,
        };
        r.mmr.round() as u32
    }

    /// A table as it's sent: each seat with its MMR.
    pub(super) fn with_mmr(hub: &Hub, table: &mut TableView) {
        let bots = Self::bot_ratings(hub);
        for s in table.seats.iter_mut().flatten() {
            let id = rating::id_of(&s.player, &s.name, s.bot);
            s.mmr = Some(Self::mmr_of(hub, &bots, &id, s.bot));
        }
    }

    /// Someone's MMR, for the card about them at a table.
    pub(super) fn mmr_for(hub: &Hub, uid: &str, name: &str, bot: Option<Level>) -> u32 {
        Self::mmr_of(hub, &Self::bot_ratings(hub), &rating::id_of(uid, name, bot), bot)
    }

    /// A message as it goes out: tables dressed with everyone's MMR.
    pub(super) fn dress(hub: &Hub, msg: ToApp) -> ToApp {
        match msg {
            ToApp::Snapshot { mut table } => {
                Self::with_mmr(hub, &mut table);
                ToApp::Snapshot { table }
            }
            m => m,
        }
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::net::ToServer;

    use super::*;
    use crate::db::Db;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-ratings-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    fn ladder(hub: &Hub) -> Vec<RatingRow> {
        match hub.poker.handle(hub, "ann", false, ToServer::Ratings).unwrap() {
            Some(ToApp::Ratings { rows }) => rows,
            other => panic!("{other:?}"),
        }
    }

    /// A tournament finished moves everyone's MMR, people's and bots' alike (a bot's kept by
    /// its name and level, the same bot wherever it sits), and the ladder shows everyone rated,
    /// best first; nobody's on it before they've played.
    #[test]
    fn places_move_everyones_mmr_and_the_ladder_shows_them() {
        let hub = hub();
        hub.db.insert_user("ann", "Ann", pyr_proto::control::Role::Member).unwrap();
        hub.db.insert_user("bo", "Bo", pyr_proto::control::Role::Member).unwrap();
        assert!(ladder(&hub).is_empty());
        let st = State::default();
        let blaze = rating::id_of("bot:7", "Blaze 7", Some(Level::Shark));
        let players: Vec<Rated> = vec![("ann".into(), "Ann".into(), None), (blaze.clone(), "Blaze 7".into(), Some(Level::Shark)), ("bo".into(), "Bo".into(), None)];
        Poker::rate(&hub, &st, &players, |r| rating::rate_places(r, &[1, 2, 3]), false);
        let rows = ladder(&hub);
        assert_eq!(rows.len(), 3);
        let of = |id: &str| rows.iter().find(|r| r.id == id).unwrap().clone();
        let (ann, bot, bo) = (of("ann"), of(&blaze), of("bo"));
        assert!(ann.mmr > 1_000 && bo.mmr < 1_000, "{ann:?} {bo:?}");
        assert_eq!((bot.name.as_str(), bot.bot), ("Blaze", Some(Level::Shark)));
        assert!(bot.mmr < 1_150, "beaten by a lower-rated player: down from a Shark's start");
        assert!(rows.windows(2).all(|w| w[0].mmr >= w[1].mmr), "best first");
        assert!(rows.iter().all(|r| r.placing && r.tourneys == 1));
        // The same bot again (another seat, another table): its rating carries on.
        Poker::rate(&hub, &st, &[(blaze.clone(), "Blaze 2".into(), Some(Level::Shark)), ("bo".into(), "Bo".into(), None)], |r| rating::rate_places(r, &[1, 2]), false);
        let again = ladder(&hub);
        let bot2 = again.iter().find(|r| r.id == blaze).unwrap();
        assert_eq!(bot2.tourneys, 2);
        assert!(bot2.mmr > bot.mmr);
        assert_eq!(Poker::mmr_for(&hub, "bot:9", "Blaze", Some(Level::Shark)), bot2.mmr, "a table shows the same MMR");
    }
}
