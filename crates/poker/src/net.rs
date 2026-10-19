//! What Pyr and the server say to each other about poker (inside Pyr's own protocol: a
//! `Poker` request carries a [`ToServer`], a `Poker` event a [`ToApp`]).

use serde::{Deserialize, Serialize};

use crate::bots::Level;
use crate::cards::Card;
use crate::game::{Action, Event, Legal, Limit, Rules, Street, Variant};

pub type TableId = u64;
pub type TourneyId = u64;

/// Seeds and fingerprints travel as hex.
pub mod hex32 {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(b: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&b.iter().map(|x| format!("{x:02x}")).collect::<String>())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let s = String::deserialize(d)?;
        parse(&s).ok_or_else(|| serde::de::Error::custom("not 32 bytes of hex"))
    }

    pub fn parse(s: &str) -> Option<[u8; 32]> {
        if s.len() != 64 {
            return None;
        }
        let mut out = [0u8; 32];
        for (i, o) in out.iter_mut().enumerate() {
            *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
        }
        Some(out)
    }

    pub mod vec {
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        pub fn serialize<S: Serializer>(v: &[[u8; 32]], s: S) -> Result<S::Ok, S::Error> {
            let strs: Vec<String> = v.iter().map(|b| b.iter().map(|x| format!("{x:02x}")).collect()).collect();
            strs.serialize(s)
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<[u8; 32]>, D::Error> {
            let v = Vec::<String>::deserialize(d)?;
            v.iter().map(|s| super::parse(s).ok_or_else(|| serde::de::Error::custom("not 32 bytes of hex"))).collect()
        }
    }
}

/// From Pyr to the server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ToServer {
    /// The tables here, and my record.
    Lobby,
    /// A new table. Bots sit in the seats given.
    Create { name: String, rules: Rules, bots: Vec<(usize, Level)>, buy_in: BuyIn },
    /// Follow a table (its snapshot, then everything that happens there).
    Watch { table: TableId },
    Unwatch { table: TableId },
    Sit { table: TableId, seat: usize, chips: u64 },
    /// Leave my seat (folding first if I'm in a hand); my chips go back to my record.
    Stand { table: TableId },
    SitOut { table: TableId, out: bool },
    /// More chips from my record, between hands (up to the table's most).
    TopUp { table: TableId, chips: u64 },
    Act { table: TableId, hand: u64, action: Action },
    /// My Pyr's part of the shuffle for the next hand.
    Seed {
        table: TableId,
        hand: u64,
        #[serde(with = "hex32")]
        seed: [u8; 32],
    },
    /// Use my time bank now (it's also used by itself when my time runs out).
    TimeBank { table: TableId },
    AddBot { table: TableId, seat: usize, level: Level },
    RemoveBot { table: TableId, seat: usize },
    Emote { table: TableId, emote: String },
    Daily,
    Equip { id: String },
    Leaderboard,
    /// A finished hand at a table, everything about it (to replay it).
    History { table: TableId, hand: u64 },
    /// Close a table I made (or any, with Manage server): everyone's chips go back.
    Close { table: TableId },
    /// Tournaments here (multi-table, with play-chip buy-ins).
    Tourneys,
    /// A new tournament (Manage server, or anyone where the server allows it); bots join it
    /// at once.
    TourneyCreate {
        rules: TourneyRules,
        #[serde(default)]
        bots: Vec<Level>,
    },
    /// Register: the buy-in comes off my chips.
    TourneyRegister { id: TourneyId },
    /// Before it starts: my buy-in back.
    TourneyUnregister { id: TourneyId },
    /// Follow a tournament (its standings and levels; my table when I'm in it).
    TourneyWatch { id: TourneyId },
    TourneyUnwatch { id: TourneyId },
    /// Call one off before it starts (whoever made it, or Manage server): buy-ins go back.
    TourneyCancel { id: TourneyId },
}

/// One level of a tournament's blinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlindLevel {
    pub small: u64,
    pub big: u64,
    #[serde(default)]
    pub ante: u64,
}

/// What a tournament is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TourneyRules {
    pub name: String,
    pub variant: Variant,
    pub limit: Limit,
    /// Players at each table (2 to 9).
    pub seats: usize,
    /// Everyone starts with these chips (tournament chips, not play chips).
    pub stack: u64,
    /// Play chips to enter (0: free).
    pub buy_in: u64,
    pub min_players: usize,
    pub max_players: usize,
    /// When it starts (Unix ms). It's called off if there aren't `min_players` then.
    pub start_ms: u64,
    /// How long each level of blinds lasts.
    pub level_ms: u64,
    pub levels: Vec<BlindLevel>,
    /// Registration stays open for this many levels after the start.
    #[serde(default)]
    pub late_levels: u32,
}

/// Where a tournament is in its life.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TourneyPhase {
    Registering,
    Running,
    Finished,
    Cancelled,
}

/// A tournament in a list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TourneySummary {
    pub id: TourneyId,
    pub rules: TourneyRules,
    pub phase: TourneyPhase,
    pub entrants: usize,
    pub players_left: usize,
    pub prize_pool: u64,
    /// The level being played (0 before it starts).
    pub level: usize,
    /// Am I in it (still, or was)?
    pub registered: bool,
    /// Who made it.
    pub host: String,
}

/// Someone in a tournament.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EntrantView {
    pub uid: String,
    pub name: String,
    pub bot: Option<Level>,
    /// Chips now (0 once out).
    pub chips: u64,
    /// Where they finished (once out, or the winner).
    pub place: Option<usize>,
    pub prize: u64,
    pub table: Option<TableId>,
}

/// A tournament in full.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TourneyView {
    pub id: TourneyId,
    pub rules: TourneyRules,
    pub phase: TourneyPhase,
    pub host: String,
    pub entrants: Vec<EntrantView>,
    /// The level now (index into the rules' levels) and when the next starts (Unix ms).
    pub level: usize,
    pub next_level_ms: Option<u64>,
    pub prize_pool: u64,
    /// What each paid place wins, first place first.
    pub payouts: Vec<u64>,
    /// The table I'm at, when I'm still in it.
    pub my_table: Option<TableId>,
    pub tables: Vec<TableId>,
}

/// How many chips a player sits down with, in big blinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuyIn {
    pub min_bb: u64,
    pub max_bb: u64,
}

impl Default for BuyIn {
    fn default() -> BuyIn {
        BuyIn { min_bb: 40, max_bb: 200 }
    }
}

/// From the server to Pyr.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ToApp {
    Lobby { tables: Vec<TableSummary>, me: RecordView },
    Snapshot { table: TableView },
    /// Something at a table (hole cards only to their owner).
    Event { table: TableId, hand: u64, event: Event },
    /// A hand is about to be dealt: the dealer's promise, and how long Pyrs have to send
    /// their seeds.
    Shuffle {
        table: TableId,
        hand: u64,
        #[serde(with = "hex32")]
        commit: [u8; 32],
        ms: u64,
    },
    /// The deal: which seeds went into the shuffle (each Pyr checks its own is there).
    Seeds {
        table: TableId,
        hand: u64,
        #[serde(with = "hex32")]
        fingerprint: [u8; 32],
        #[serde(with = "hex32::vec")]
        seeds: Vec<[u8; 32]>,
    },
    /// After the hand: the dealer's seed, so anyone can check the shuffle.
    Reveal {
        table: TableId,
        hand: u64,
        #[serde(with = "hex32")]
        dealer: [u8; 32],
        /// The cards off the deck in order, where everyone saw them (others' unshown cards are
        /// left out).
        dealt: Vec<Option<Card>>,
    },
    /// Whose turn, and when their time runs out (ms from now), and their time bank.
    Clock { table: TableId, seat: usize, ms: u64, bank_ms: u64 },
    Emote { table: TableId, seat: usize, emote: String },
    /// My record changed (after a hand, the daily chips, a look chosen).
    Record { me: RecordView, gains: Option<GainsView> },
    Leaderboard { rows: Vec<LeaderRow> },
    History { table: TableId, hand: HandHistory },
    Closed { table: TableId },
    /// The daily chips: how many (none: already taken today), and when the next can be taken.
    Daily { chips: Option<u64>, streak: u32 },
    Tourneys { list: Vec<TourneySummary> },
    Tourney { view: TourneyView },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableSummary {
    pub id: TableId,
    pub name: String,
    pub rules: Rules,
    pub buy_in: BuyIn,
    pub players: usize,
    pub bots: usize,
    pub seats: usize,
    pub average_pot: u64,
    pub hands_per_hour: u32,
    /// Names of who's sitting (to find friends).
    pub names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SeatView {
    pub player: String,
    pub name: String,
    pub stack: u64,
    pub bot: Option<Level>,
    pub sitting_out: bool,
    pub level: u32,
    /// Their chosen looks (card back and frame).
    pub card_back: String,
    pub frame: String,
    /// In the current hand: chips in front of them this round, folded, all-in, and cards
    /// (shown ones, or mine).
    pub bet: u64,
    pub in_hand: bool,
    pub folded: bool,
    pub all_in: bool,
    pub cards: Vec<Card>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TableView {
    pub id: TableId,
    pub name: String,
    pub rules: Rules,
    pub buy_in: BuyIn,
    pub owner: String,
    pub seats: Vec<Option<SeatView>>,
    pub button: Option<usize>,
    pub hand: Option<u64>,
    pub street: Option<Street>,
    pub board: Vec<Card>,
    pub pot: u64,
    pub to_act: Option<usize>,
    /// What I may do, when it's my turn.
    pub legal: Option<Legal>,
    pub my_seat: Option<usize>,
    /// The last few hands' numbers (for the history).
    pub recent: Vec<u64>,
    /// A tournament's table (nobody buys in or leaves here; the tournament moves players).
    #[serde(default)]
    pub tourney: Option<TourneyId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordView {
    pub uid: String,
    pub name: String,
    pub chips: u64,
    /// Chips sitting at tables now.
    pub at_tables: u64,
    pub xp: u64,
    pub level: u32,
    /// Experience at this level's start and the next's.
    pub level_from: u64,
    pub level_to: u64,
    pub hands: u64,
    pub hands_won: u64,
    pub showdowns: u64,
    pub showdowns_won: u64,
    pub vpip: u64,
    pub pfr: u64,
    pub biggest_pot: u64,
    pub best_hand: String,
    pub achievements: Vec<String>,
    pub daily_ready: bool,
    pub daily_streak: u32,
    pub card_back: String,
    pub felt: String,
    pub chip_set: String,
    pub frame: String,
    pub unlocked: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GainsView {
    pub xp: u64,
    pub achievements: Vec<String>,
    pub level_up: Option<u32>,
    /// Chips won or lost in the hand.
    pub net: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LeaderRow {
    pub uid: String,
    pub name: String,
    pub chips: u64,
    pub level: u32,
    pub hands: u64,
    pub biggest_pot: u64,
    pub best_hand: String,
}

/// Everything about a finished hand.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HandHistory {
    pub number: u64,
    pub rules: Rules,
    /// Who sat where, with what, when it started.
    pub seats: Vec<Option<(String, u64)>>,
    pub button: usize,
    /// What everyone saw happen (hole cards: only shown ones and mine).
    pub events: Vec<Event>,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    #[serde(with = "hex32")]
    pub dealer: [u8; 32],
    #[serde(with = "hex32::vec")]
    pub seeds: Vec<[u8; 32]>,
    pub ended_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_read_back() {
        let m = ToServer::Seed { table: 3, hand: 9, seed: [7; 32] };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"op\":\"seed\""));
        assert_eq!(serde_json::from_str::<ToServer>(&j).unwrap(), m);
        let m = ToApp::Seeds { table: 1, hand: 2, fingerprint: [1; 32], seeds: vec![[2; 32], [3; 32]] };
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<ToApp>(&j).unwrap(), m);
        let m = ToServer::Act { table: 1, hand: 4, action: Action::Raise { to: 40 } };
        assert_eq!(serde_json::from_str::<ToServer>(&serde_json::to_string(&m).unwrap()).unwrap(), m);
    }
}
