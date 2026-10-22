//! Emotes at the tables: from someone sitting at one, to everyone there (at poker through its
//! room; at blackjack, craps and baccarat here), and not too often: one that comes too soon
//! after the last is dropped, so nobody floods a table.

use std::collections::HashMap;

use pyr_poker::net::{TableGame, ToApp};

use super::{Poker, State};
use crate::hub::Hub;

/// The least time between someone's emotes.
const GAP_MS: u64 = 900;
/// The longest emote (a sticker's name, or an emoji or a word from an older Pyr).
const LONGEST: usize = 16;

#[derive(Default)]
pub struct EmoteState {
    /// When each person last sent one.
    last: HashMap<String, u64>,
}

impl EmoteState {
    /// Whether someone may send one now (counted if so).
    pub fn allow(&mut self, uid: &str, now: u64) -> bool {
        if self.last.len() > 4_096 {
            self.last.retain(|_, t| now.saturating_sub(*t) < GAP_MS);
        }
        if self.last.get(uid).is_some_and(|t| now.saturating_sub(*t) < GAP_MS) {
            return false;
        }
        self.last.insert(uid.to_string(), now);
        true
    }
}

/// What's said, and to whom: everyone at the table (watching or sitting), from my seat. None:
/// one too soon after my last, dropped.
pub(super) fn game_emote_to(st: &mut State, uid: &str, game: TableGame, table: u64, emote: &str, now: u64) -> Result<Option<(Vec<String>, ToApp)>, String> {
    if emote.is_empty() || emote.chars().count() > LONGEST {
        return Err("that isn't an emote".into());
    }
    let (seat, audience) = match game {
        TableGame::Blackjack => st.bj.tables.get(&table).map(|t| (t.seat_of(uid), t.audience())),
        TableGame::Craps => st.craps.tables.get(&table).map(|t| (t.seat_of(uid), t.audience())),
        TableGame::Baccarat => st.bac.tables.get(&table).map(|t| (t.seat_of(uid), t.audience())),
        TableGame::Roulette => st.rl.tables.get(&table).map(|t| (t.seat_of(uid), t.audience())),
    }
    .ok_or("that table has closed")?;
    let seat = seat.ok_or("sit down to send those")?;
    if !st.emotes.allow(uid, now) {
        return Ok(None);
    }
    let to = audience.into_iter().filter(|u| !u.starts_with("bot:")).collect();
    Ok(Some((to, ToApp::GameEmote { game, table, seat, emote: emote.to_string() })))
}

impl Poker {
    /// An emote at a blackjack, craps or baccarat table, sent on to everyone there.
    pub(super) fn game_emote(hub: &Hub, st: &mut State, uid: &str, game: TableGame, table: u64, emote: &str, now: u64) -> Result<Option<ToApp>, String> {
        if let Some((to, msg)) = game_emote_to(st, uid, game, table, emote, now)? {
            for u in to {
                hub.send_poker(&u, msg.clone());
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use pyr_poker::net::ToServer;

    use super::*;
    use crate::db::Db;

    fn hub() -> std::sync::Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-emote-test-{}", std::process::id()));
        Hub::new(Db::open_in_memory().unwrap(), [0; 32], "127.0.0.1".into(), 1, dir)
    }

    /// Someone sitting at a table sends one to everyone there (watchers too, not bots); someone
    /// only watching can't; one too soon after the last is dropped; nonsense is refused.
    #[test]
    fn an_emote_goes_to_everyone_at_the_table() {
        let hub = hub();
        let ask = |uid: &str, msg: ToServer| hub.poker.handle(&hub, uid, false, msg);
        let id = match ask("ann", ToServer::CrapsTables).unwrap() {
            Some(ToApp::CrapsTables { tables }) => tables[0].id,
            other => panic!("{other:?}"),
        };
        ask("ann", ToServer::CrapsSit { table: id, seat: 3 }).unwrap();
        ask("ann", ToServer::CrapsBot { table: id, seat: 0, add: true }).unwrap();
        ask("bo", ToServer::CrapsWatch { table: id }).unwrap();
        let mut st = State::default();
        std::mem::swap(&mut st.craps, &mut hub.poker.state.lock().craps);
        let (mut to, msg) = game_emote_to(&mut st, "ann", TableGame::Craps, id, "e:gg", 10_000).unwrap().expect("sent");
        to.sort();
        assert_eq!(to, ["ann", "bo"]);
        assert_eq!(msg, ToApp::GameEmote { game: TableGame::Craps, table: id, seat: 3, emote: "e:gg".into() });
        assert_eq!(game_emote_to(&mut st, "ann", TableGame::Craps, id, "e:wow", 10_400).unwrap(), None, "too soon");
        assert!(game_emote_to(&mut st, "ann", TableGame::Craps, id, "e:wow", 11_000).unwrap().is_some());
        assert!(game_emote_to(&mut st, "bo", TableGame::Craps, id, "e:gg", 20_000).is_err(), "watching only");
        assert!(game_emote_to(&mut st, "ann", TableGame::Craps, id, "", 30_000).is_err());
        assert!(game_emote_to(&mut st, "ann", TableGame::Craps, id, "a much too long emote", 30_000).is_err());
        assert!(game_emote_to(&mut st, "ann", TableGame::Baccarat, id, "e:gg", 30_000).is_err(), "no such table");
        // Through the server, and only from a Pyr that knows it.
        std::mem::swap(&mut st.craps, &mut hub.poker.state.lock().craps);
        assert_eq!(ask("ann", ToServer::GameEmote { game: TableGame::Craps, table: id, emote: "e:nice".into() }), Ok(None));
        assert!(hub.poker.handle_from(&hub, "ann", false, pyr_poker::net::CASINO_011, ToServer::GameEmote { game: TableGame::Craps, table: id, emote: "e:nice".into() }).is_err());
    }
}
