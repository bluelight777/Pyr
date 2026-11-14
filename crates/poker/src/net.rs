//! What Pyr and the server say to each other about poker (inside Pyr's own protocol: a
//! `Poker` request carries a [`ToServer`], a `Poker` event a [`ToApp`]).

use serde::{Deserialize, Serialize};

use crate::bots::Level;
use std::collections::BTreeMap;

use crate::cards::Card;
use crate::casino::{Bet, Game, Outcome, blackjack, craps};
use crate::progress::CasinoStats;
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

    pub fn to_hex(b: &[u8; 32]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
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
    /// A new table. Bots sit in the seats given, the ones in `who` by name (0.13.4: picked by
    /// who, the others whichever isn't sitting yet).
    Create {
        name: String,
        rules: Rules,
        bots: Vec<(usize, Level)>,
        buy_in: BuyIn,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        who: Vec<(usize, String)>,
    },
    /// Follow a table (its snapshot, then everything that happens there).
    Watch { table: TableId },
    Unwatch { table: TableId },
    Sit { table: TableId, seat: usize, chips: u64 },
    /// Leave my seat (folding first if I'm in a hand); my chips go back to my record.
    Stand { table: TableId },
    SitOut { table: TableId, out: bool },
    /// At a showdown, my hands that can't win go in face down (0.13.0 servers).
    Muck { table: TableId, on: bool },
    /// My cards from the hand just over, for everyone to see (0.13.0 servers).
    ShowCards { table: TableId, hand: u64 },
    /// More chips from my record (up to the table's most): between hands on my stack at once;
    /// in a hand (`poker_top_up_anytime`, 0.13.22) kept on my seat till it's over.
    TopUp { table: TableId, chips: u64 },
    /// Straddle (twice the big blind, as the first after it) from the next hand on, or stop
    /// (`poker_straddle`, 0.13.4).
    Straddle { table: TableId, on: bool },
    /// The board the hand just over would have had, turned over for everyone (`poker_rabbit`,
    /// 0.13.4).
    Rabbit { table: TableId, hand: u64 },
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
    /// A bot in an empty seat, whichever isn't sitting yet or the one named (0.13.4).
    AddBot {
        table: TableId,
        seat: usize,
        level: Level,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        who: Option<String>,
    },
    RemoveBot { table: TableId, seat: usize },
    Emote { table: TableId, emote: String },
    Daily,
    Equip { id: String },
    /// Keep my level's title off my name at tables, or show it again (0.13.6).
    HideTitle { hide: bool },
    /// Ember Back: the chips it's built up, claimed (0.13.5).
    ClaimEmber,
    /// The weekly VIP bonus, claimed (0.13.5).
    ClaimWeekly,
    Leaderboard,
    /// A finished hand at a table, everything about it (to replay it).
    History { table: TableId, hand: u64 },
    /// Someone at a table (a bot too): how they've played, and their last hands.
    Player { table: TableId, seat: usize },
    /// The Game Masters and Ambassadors, my allowance today, and (for the owner) what
    /// they've given.
    Staff,
    /// Make someone a Game Master or an Ambassador, or neither (the server's owner only).
    SetRole { uid: String, role: Option<Role> },
    /// A Game Master gives someone chips and/or a look.
    Gift { uid: String, chips: u64, item: Option<String>, note: String },
    /// A Game Master or the Owner of the Game takes chips away (0.13.4): as many as they have
    /// at most, in the log with their balance before and after.
    TakeChips { uid: String, chips: u64, note: String },
    /// The whole staff log (0.13.4), for the Owner of the Game (and the control room).
    StaffLogAll,
    /// Invite someone to a table (0.13.6): they get `TableInvite` on every Pyr they're on, with
    /// a Join. Once every 20 seconds for the same person.
    InviteToTable { uid: String, table: TableRef },
    /// The control room (0.13.5): the Owner of the Game's, and whoever they let in.
    Cms { req: CmsReq },
    /// A Game Master or Ambassador puts a prize box on a table: whoever wins the next hand
    /// (a person, not a bot) opens it.
    DropPrize { table: TableId, chips: u64, item: Option<String> },
    /// Pyr Casino's games against the house.
    Casino { req: CasinoReq },
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
    /// Play a Bomb Pot for this buy-in: a seat in one that's filling up, or a new one.
    TimeBomb { buy_in: u64 },
    /// Play Bounty Poker for this buy-in: a seat at one that's filling up, or a new one
    /// (0.13.6: 0.13.28's `bounty`).
    Bounty { buy_in: u64 },
    /// Poker's MMR ladder: everyone rated here, people and bots (0.12.0 servers).
    Ratings,
    /// Blackjack tables (0.12.0 servers): the list; follow one (or stop); sit in a seat or
    /// leave it; bet for the next round (0 sits it out; `seed` is my Pyr's part of the shuffle);
    /// a move; a bot in or out of an empty seat; a new table.
    BjTables,
    BjWatch { table: u64 },
    BjUnwatch { table: u64 },
    BjSit { table: u64, seat: u8 },
    BjLeave { table: u64 },
    BjBet {
        table: u64,
        chips: u64,
        #[serde(with = "hex32")]
        seed: [u8; 32],
    },
    BjAct { table: u64, action: crate::casino::blackjack::Action },
    BjBot { table: u64, seat: u8, add: bool },
    BjNew,
    /// Craps tables (0.12.0 servers): the list; follow one (or stop); sit in a seat or leave
    /// it; my bets on the layout for the coming roll (`seed`: my Pyr's part of the dice; `ready`:
    /// roll when you like); throw the dice (the shooter); a bot in or out of an empty seat.
    CrapsTables,
    CrapsWatch { table: u64 },
    CrapsUnwatch { table: u64 },
    CrapsSit { table: u64, seat: u8 },
    CrapsLeave { table: u64 },
    CrapsBets {
        table: u64,
        /// The roll they're for (refused if the dice have rolled since).
        roll: u64,
        bets: crate::casino::craps::Bets,
        #[serde(with = "hex32")]
        seed: [u8; 32],
        ready: bool,
    },
    CrapsThrow { table: u64 },
    CrapsBot { table: u64, seat: u8, add: bool },
    /// Baccarat tables (0.12.0 servers): the list; follow one (or stop); sit in a seat or leave
    /// it; my bets on the coming coup (`seed`: my Pyr's part of the shuffle; `ready`: deal when
    /// you like); a bot in or out of an empty seat.
    BacTables,
    BacWatch { table: u64 },
    BacUnwatch { table: u64 },
    BacSit { table: u64, seat: u8 },
    BacLeave { table: u64 },
    BacBets {
        table: u64,
        /// The coup they're for (refused if it's been dealt since).
        coup: u64,
        bets: crate::casino::baccarat::Bets,
        #[serde(with = "hex32")]
        seed: [u8; 32],
        ready: bool,
    },
    BacBot { table: u64, seat: u8, add: bool },
    /// Roulette tables (0.13.0 servers): as baccarat's, the chips anywhere on the layout.
    RlTables,
    RlWatch { table: u64 },
    RlUnwatch { table: u64 },
    RlSit { table: u64, seat: u8 },
    RlLeave { table: u64 },
    RlBets {
        table: u64,
        /// The spin they're for (refused if it's gone since).
        spin: u64,
        chips: Vec<crate::casino::roulette::Chip>,
        #[serde(with = "hex32")]
        seed: [u8; 32],
        ready: bool,
    },
    RlBot { table: u64, seat: u8, add: bool },
    /// Dominó tables (0.13.11; apps send `CASINO_01311_CAP`): the list; follow one (or stop); sit
    /// in a seat or leave it (in a hand: after it); in for the next hand, putting up `chips` (the
    /// ante to eight times it: what covers doubles) with my Pyr's seed; a move; a bot in or out of
    /// an empty seat; a new table (Pairs or Solo, at one of the antes). 0.13.28's words.
    DomTables,
    DomWatch { table: u64 },
    DomUnwatch { table: u64 },
    DomSit { table: u64, seat: u8 },
    DomLeave { table: u64 },
    DomReady {
        table: u64,
        chips: u64,
        #[serde(with = "hex32")]
        seed: [u8; 32],
    },
    DomMove { table: u64, mv: crate::casino::domino::Move },
    DomBot { table: u64, seat: u8, add: bool },
    DomNew { kind: crate::casino::domino::Kind, ante: u64 },
    /// The Bonfire (0.12.0 servers): how it stands; light it now (the server's owner).
    Bonfire,
    BonfireLight,
    /// An emote to everyone at a blackjack, craps or baccarat table I'm sitting at (0.12.0
    /// servers; at poker it's `Emote`).
    GameEmote { game: TableGame, table: u64, emote: String },
    /// The casino's chat (0.13.0 servers): say something to everyone in the casino; the last
    /// lines said; take a line down (mine, or anyone's for staff and the server's managers).
    CasinoChat { text: String },
    CasinoChatLog,
    CasinoChatDelete { id: u64 },
    /// Game Masters (0.13.0): on the home server, my certificate (if I'm a Game Master or its
    /// owner); anywhere else, here's mine.
    GmCertAsk,
    GmProof { signed: SignedGm },
    /// Prop bets in chat (servers with `PROPS_FEATURE`; see `props`): open one in a channel
    /// (`depth`: one of `props::DEPTHS`; `closes_in`: seconds, 0 for when it's settled), bet on
    /// an outcome (`min_shares`: what the app said it pays, less a little: the odds may move),
    /// sell an outcome's shares back, settle it or call it off (its author or a moderator),
    /// and look props up.
    PropOpen { channel_id: i64, question: String, outcomes: Vec<String>, depth: u64, closes_in: u64 },
    PropBet { id: u64, outcome: usize, chips: u64, min_shares: f64 },
    PropCashOut { id: u64, outcome: usize },
    PropSettle { id: u64, outcome: usize },
    PropVoid { id: u64 },
    PropGet { ids: Vec<u64> },
    /// The props open in a channel (newest first).
    PropList { channel_id: i64 },
}

/// The home of Pyr Casino's Game Masters: Brethren of Blood, by its server's fingerprint
/// (base64url, as invite links carry it). Its owner is the game's owner, and the Game Masters
/// it names are Game Masters on every server: it signs each a certificate ([`GmCert`]) with its
/// TLS key, which their Pyr shows to the other servers. No other server names Game Masters.
/// (Its invite links pin the first 12 characters: pyr.gg/brethren.) Blue confirmed it.
pub const HOME_SERVER: &str = "zbcw0f5-a_4t1PJkuKJiIS2bnKHObr_5GGGdvPV_5dQ";
/// What a Game Master certificate's signature covers starts with this.
pub const GM_CONTEXT: &[u8] = b"pyr game master v1\n";
/// How long a certificate counts for (renewed while they use the home server).
pub const GM_DAYS: u64 = 14;

/// What the home server vouches for: who, whether they're its owner (the game's owner) or a
/// Game Master, until when.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GmCert {
    pub uid: String,
    pub home_owner: bool,
    /// Their role on every server (a Game Master, or an Ambassador).
    #[serde(default = "game_master")]
    pub role: Role,
    pub issued_ms: u64,
    pub expires_ms: u64,
}

fn game_master() -> Role {
    Role::GameMaster
}

/// A certificate as the home server signed it: its TLS certificate (whose SHA-256 is
/// [`HOME_SERVER`]), the scheme, the [`GmCert`] JSON exactly as signed, and the signature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SignedGm {
    pub cert: String,
    pub scheme: String,
    pub payload: String,
    pub sig: String,
}

/// A line in the casino's chat.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChatLine {
    pub id: u64,
    pub uid: String,
    pub name: String,
    pub text: String,
    pub at_ms: u64,
    /// What this Pyr doesn't know (a newer one's, or 0.13.28's not rebuilt yet): kept and
    /// written back as it was.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// A table of any of the casino's games, as an invite names it (0.13.6), written as 0.13.28
/// writes it: `{"game":"roulette","id":3}`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "game", content = "id", rename_all = "snake_case")]
pub enum TableRef {
    Poker(TableId),
    Blackjack(u64),
    Craps(u64),
    Baccarat(u64),
    Roulette(u64),
    /// Dominó Cubano's (0.13.11).
    Domino(u64),
}

impl TableRef {
    /// The game, as an invite says it ("invites you to roulette: Grand Salle").
    pub fn game_name(&self) -> &'static str {
        match self {
            TableRef::Poker(_) => "poker",
            TableRef::Blackjack(_) => "blackjack",
            TableRef::Craps(_) => "craps",
            TableRef::Baccarat(_) => "baccarat",
            TableRef::Roulette(_) => "roulette",
            TableRef::Domino(_) => "dominó",
        }
    }
}

/// The casino's tables with seats besides poker's (for what's sent to everyone at one).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TableGame {
    Blackjack,
    Craps,
    Baccarat,
    /// (0.13.0)
    Roulette,
    /// Dominó Cubano's (0.13.11).
    Domino,
}

/// The emoji an older Pyr shows for one of 0.12.0's emote stickers (`e:<name>`): it'd show
/// the words otherwise.
pub fn emote_for_older_app(emote: &str) -> String {
    let Some(name) = emote.strip_prefix("e:") else { return emote.to_string() };
    match name {
        "laugh" => "😂",
        "cry" => "😢",
        "cool" | "boss" => "😎",
        "wow" => "😮",
        "angry" => "😤",
        "think" => "🤔",
        "nice" => "👍",
        "love" => "😍",
        "rich" => "🤑",
        "gg" => "GG",
        "allin" => "🔥",
        "aces" => "AA",
        "sob" => "😭",
        "lol" => "🎲",
        "badbeat" => "💔",
        "bag" => "💰",
        "king" => "👑",
        _ => "🔥",
    }
    .to_string()
}

impl ToServer {
    /// The table it's about, if it's about one.
    pub fn table(&self) -> Option<TableId> {
        use ToServer::*;
        match self {
            Watch { table } | Unwatch { table } | Sit { table, .. } | Stand { table } | SitOut { table, .. } | Muck { table, .. } | ShowCards { table, .. } | TopUp { table, .. } | Act { table, .. }
            | Seed { table, .. } | TimeBank { table } | AddBot { table, .. } | RemoveBot { table, .. } | Emote { table, .. } | History { table, .. }
            | Player { table, .. } | DropPrize { table, .. } | Close { table } | Straddle { table, .. } | Rabbit { table, .. } => Some(*table),
            _ => None,
        }
    }
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
    /// Prize money the house puts in on top of the buy-ins (Pyr Friday Night: the week's rake).
    #[serde(default)]
    pub added: u64,
    /// A Bomb Pot: a fast sit-and-go for a prize the wheel picks, where after the fuse burns
    /// down every hand is all in.
    #[serde(default)]
    pub bomb: Option<Bomb>,
    /// Bounty Poker (0.13.6): a price on every head.
    #[serde(default)]
    pub bounty: Option<Bounty>,
}

impl TourneyRules {
    /// A sit & go the house runs (a Bomb Pot or Bounty Poker): it starts once it's full, bots
    /// filling the seats nobody took.
    pub fn house_game(&self) -> bool {
        self.bomb.is_some() || self.bounty.is_some()
    }
}

/// Bounty Poker's bounties: this share of each buy-in (in percent) is the price on that
/// player's head; the rest makes the prize pool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bounty {
    pub pct: u32,
}

/// A Bomb Pot's fuse.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bomb {
    /// From the first hand until it goes off.
    pub fuse_ms: u64,
}

/// A Bomb Pot as it stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BombView {
    pub fuse_ms: u64,
    /// The prize wheel's pick (times the buy-in), and when it was spun (Unix ms; the first hand
    /// is dealt once the wheel has stopped).
    pub multiplier: Option<u32>,
    pub spun_ms: Option<u64>,
    /// When it goes off (Unix ms), once the first hand's dealt; and whether it has.
    pub goes_off_ms: Option<u64>,
    pub exploded: bool,
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
    /// A Bomb Pot's prize multiplier, once the wheel has spun.
    #[serde(default)]
    pub multiplier: Option<u32>,
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
    /// Bounty Poker: the price on their head now, and what they've won from others' so far
    /// (0.13.28's fields; 0 elsewhere).
    #[serde(default)]
    pub bounty: u64,
    #[serde(default)]
    pub bounties_won: u64,
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
    /// A Bomb Pot's wheel and fuse.
    #[serde(default)]
    pub bomb: Option<BombView>,
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
    /// A bot's word at the table (0.12.0: older apps aren't sent it).
    Say { table: TableId, seat: usize, text: String },
    /// The MMR ladder (0.12.0).
    Ratings { rows: Vec<RatingRow> },
    /// Blackjack tables (0.12.0): the list, and one I follow as it is now.
    BjTables { tables: Vec<BjSummary> },
    Bj { view: BjView },
    /// Dominó tables (0.13.11): the list, and one I follow as it is now.
    DomTables { tables: Vec<DomSummary> },
    Dom { view: DomView },
    /// Craps tables (0.12.0): the list, and one I follow as it is now.
    CrapsTables { tables: Vec<CrapsSummary> },
    Craps { view: CrapsView },
    /// Baccarat tables (0.12.0): the list, and one I follow as it is now.
    BacTables { tables: Vec<BacSummary> },
    Bac { view: BacView },
    /// Roulette tables (0.13.0): the list, and one I follow as it is now.
    RlTables { tables: Vec<RlSummary> },
    Rl { view: RlView },
    /// The Bonfire (0.12.0): how it stands for me; and, to everyone, the moment it's lit.
    Bonfire { view: BonfireView },
    BonfireLit { draw: BonfireDraw },
    /// The casino's chat (0.13.0): new lines (`all`: the whole of what's kept, in place of what
    /// I had), and a line taken down.
    CasinoChat { lines: Vec<ChatLine>, all: bool },
    CasinoChatGone { id: u64 },
    /// The home server's answer to `GmCertAsk`: my certificate, or none (I'm not, or no longer,
    /// a Game Master: forget any I have).
    GmCert { signed: Option<SignedGm> },
    /// Someone at a blackjack, craps or baccarat table sent an emote (0.12.0).
    GameEmote { game: TableGame, table: u64, seat: u8, emote: String },
    /// My record changed (after a hand, the daily chips, a look chosen).
    Record { me: RecordView, gains: Option<GainsView> },
    Leaderboard { rows: Vec<LeaderRow> },
    History { table: TableId, hand: HandHistory },
    Player { table: TableId, player: PlayerView },
    Staff { view: StaffView },
    /// The whole staff log, newest first (0.13.4).
    StaffLogAll { log: Vec<StaffLog> },
    /// `from` invites me to a table (0.13.6), called `name`.
    TableInvite { from: String, table: TableRef, name: String },
    /// The control room's answers (0.13.5).
    Cms { res: CmsRes },
    /// Said by the Owner of the Game (or the control room) to everyone in Pyr Casino (0.13.5).
    Announcement { from: String, text: String },
    /// A Game Master gave me something.
    Gifted { from: String, chips: u64, item: Option<String>, note: String },
    /// Someone won a table's prize box.
    PrizeWon { table: TableId, seat: usize, name: String, prize: PrizeView },
    Casino { res: CasinoRes },
    Closed { table: TableId },
    /// A rabbit hunted (0.13.4): the whole board as it would have come (`had` of them really
    /// dealt), by whom; the next hand waits for it.
    Rabbit { table: TableId, hand: u64, by: String, board: Vec<Card>, had: usize },
    /// The daily chips: how many (none: already taken today), and when the next can be taken.
    Daily { chips: Option<u64>, streak: u32 },
    Tourneys { list: Vec<TourneySummary> },
    Tourney { view: TourneyView },
    /// Props, as I see them (looked up, or changed: everyone in their channel hears).
    Props { props: Vec<crate::props::PropView> },
    /// The prop I just opened (for its message in chat).
    PropOpened { prop: crate::props::PropView },
    /// What happened to me on a prop: a bet taken, a payout, chips back.
    PropNote { id: u64, text: String, chips: i64 },
}

impl ToApp {
    /// A prop bets message: only for apps that say they read them (`PROPS_CAP`).
    pub fn is_props(&self) -> bool {
        matches!(self, ToApp::Props { .. } | ToApp::PropOpened { .. } | ToApp::PropNote { .. })
    }
}

/// How much of Pyr Casino an app knows: what it says when it logs in (`CASINO_012_CAP`,
/// `CASINO_013_CAP`).
pub const CASINO_011: u8 = 11;
pub const CASINO_012: u8 = 12;
pub const CASINO_013: u8 = 13;
/// An app that knows 0.13.7's casino (`CASINO_0137_CAP`): Hard bots.
pub const CASINO_0137: u8 = 20;
/// An app that knows 0.13.11's casino (`CASINO_01311_CAP`): Dominó Cubano.
pub const CASINO_01311: u8 = 30;
/// An app that also reads prop bets (`PROPS_CAP`, the rebuild's): above every 0.13 level (an
/// app that says it knows everything that came before).
pub const CASINO_PROPS: u8 = 100;

impl ToApp {
    /// This message as an app that knows Pyr Casino up to `casino` can read it (`None`: one it
    /// can't, left unsent).
    pub fn for_app(self, casino: u8) -> Option<ToApp> {
        if self.is_props() && casino < CASINO_PROPS {
            return None;
        }
        // (Dominó's tables only to apps that know them.)
        if casino < CASINO_01311 && self.is_domino() {
            return None;
        }
        // (Before 0.13.7 there were no big wins' moments: nothing of them.)
        if casino < CASINO_0137 && matches!(&self, ToApp::Casino { res: CasinoRes::Cheered { .. } | CasinoRes::CheerSent { .. } }) {
            return None;
        }
        // (Before 0.13.7 there were no Hard bots: a Medium one, as 0.13.28's server says.)
        let m = if casino < CASINO_0137 { self.without_hard_bots() } else { self };
        match casino {
            c if c >= CASINO_013 => Some(m),
            CASINO_012 => m.for_012_app(),
            _ => m.for_older_app(),
        }
    }

    /// About Dominó's tables (0.13.11): its lists and tables, an emote or an invite at one.
    pub fn is_domino(&self) -> bool {
        matches!(self, ToApp::DomTables { .. } | ToApp::Dom { .. } | ToApp::GameEmote { game: TableGame::Domino, .. } | ToApp::TableInvite { table: TableRef::Domino(_), .. })
    }

    /// Hard bots told as Medium ones (their looks' level as it is), for an app from before
    /// 0.13.7.
    fn without_hard_bots(self) -> ToApp {
        let medium = |b: &mut Option<Level>| {
            if *b == Some(Level::Hard) {
                *b = Some(Level::Regular);
            }
        };
        match self {
            ToApp::Snapshot { mut table } => {
                table.seats.iter_mut().flatten().for_each(|s| medium(&mut s.bot));
                ToApp::Snapshot { table }
            }
            ToApp::Player { table, mut player } => {
                medium(&mut player.bot);
                ToApp::Player { table, player }
            }
            ToApp::Tourney { mut view } => {
                view.entrants.iter_mut().for_each(|e| medium(&mut e.bot));
                ToApp::Tourney { view }
            }
            ToApp::Ratings { mut rows } => {
                rows.iter_mut().for_each(|r| medium(&mut r.bot));
                ToApp::Ratings { rows }
            }
            m => m,
        }
    }

    /// This message as a 0.12 app can read it: roulette tables, and what's new about the slots,
    /// are 0.13.0's.
    pub fn for_012_app(self) -> Option<ToApp> {
        match self {
            ToApp::RlTables { .. } | ToApp::Rl { .. } | ToApp::GameEmote { game: TableGame::Roulette, .. } | ToApp::CasinoChat { .. } | ToApp::CasinoChatGone { .. } | ToApp::GmCert { .. } | ToApp::Rabbit { .. } => None,
            ToApp::StaffLogAll { .. } | ToApp::Cms { .. } | ToApp::Announcement { .. } | ToApp::TableInvite { .. } => None,
            m if m.is_props() => None,
            ToApp::Casino { res } => res.for_012_app().map(|res| ToApp::Casino { res }),
            m => Some(m),
        }
    }

    /// This message as a 0.11 app can read it (`None`: one it can't, left unsent). A 0.11 app
    /// drops its connection at anything it doesn't know, so what's new since is left out or
    /// put the old way: a Genius bot is a Shark to it.
    pub fn for_older_app(self) -> Option<ToApp> {
        let older = |b: &mut Option<Level>| *b = b.map(Level::for_older_app);
        if self.is_props() {
            return None;
        }
        Some(match self {
            ToApp::Say { .. } | ToApp::Ratings { .. } | ToApp::BjTables { .. } | ToApp::Bj { .. } | ToApp::CrapsTables { .. } | ToApp::Craps { .. } | ToApp::BacTables { .. } | ToApp::Bac { .. } | ToApp::RlTables { .. } | ToApp::Rl { .. } | ToApp::Bonfire { .. } | ToApp::BonfireLit { .. } | ToApp::GameEmote { .. } | ToApp::CasinoChat { .. } | ToApp::CasinoChatGone { .. } | ToApp::GmCert { .. } | ToApp::StaffLogAll { .. } | ToApp::Cms { .. } | ToApp::Announcement { .. } | ToApp::TableInvite { .. } | ToApp::Rabbit { .. } => return None,
            ToApp::Emote { table, seat, emote } => ToApp::Emote { table, seat, emote: emote_for_older_app(&emote) },
            ToApp::Snapshot { mut table } => {
                table.seats.iter_mut().flatten().for_each(|s| older(&mut s.bot));
                ToApp::Snapshot { table }
            }
            ToApp::Player { table, mut player } => {
                older(&mut player.bot);
                ToApp::Player { table, player }
            }
            ToApp::Tourney { mut view } => {
                view.entrants.iter_mut().for_each(|e| older(&mut e.bot));
                ToApp::Tourney { view }
            }
            m => m,
        })
    }
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
    /// Who made it (0.13.24: they can close it from the lobby).
    #[serde(default)]
    pub owner: String,
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
    /// Their MMR (filled in by the server as it's sent; 0.12.0).
    #[serde(default)]
    pub mmr: Option<u32>,
    /// Chips they've added in this hand, on their stack when it's over (0.13.22).
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub pending: u64,
    /// Their staff title, shown on their seat unless they've hidden it (0.13.4/0.13.6; filled
    /// in by the server as it's sent, as 0.13.28's server does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<Badge>,
}

/// A staff title on a seat, as 0.13.28 sends it: the Owner of the Game, a Game Master, an
/// Ambassador.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Badge {
    Owner,
    GameMaster,
    Ambassador,
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
    /// A prize box on the table, for whoever wins the next hand.
    #[serde(default)]
    pub prize: Option<PrizeView>,
    /// A Bomb Pot gone off: everyone's all in, every hand (nobody chooses anything).
    #[serde(default)]
    pub bomb: bool,
}

/// What I ask Pyr Casino.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "do", rename_all = "snake_case")]
pub enum CasinoReq {
    /// The casino as it is for me (and tell me of big wins and the jackpot while I'm in).
    Open,
    /// A game's statistics (0.12.0): by its statistics key (`Outcome::stats_key`).
    Stats { key: String },
    Close,
    /// A round: the fingerprint of the house's seed I was shown, a seed of my own, my bet.
    Play {
        #[serde(with = "hex32")]
        commit: [u8; 32],
        #[serde(with = "hex32")]
        seed: [u8; 32],
        bet: Bet,
    },
    /// A move in my blackjack hand.
    Blackjack { action: blackjack::Action },
    /// My craps bets as they should be before the next roll (chips go between me and the table).
    CrapsBets { bets: craps::Bets },
    /// Roll the dice (my craps bets stay as they are).
    CrapsRoll {
        #[serde(with = "hex32")]
        commit: [u8; 32],
        #[serde(with = "hex32")]
        seed: [u8; 32],
    },
    /// Someone's big win, while its moment lasts (0.13.7, `BigWin::cheer`): Congrats! to the
    /// winner, or a drink (`CHEER_DRINK_CHIPS` of mine, to them).
    Cheer { cheer: u64, drink: bool },
}

/// A drink bought for someone's big win: this many of the buyer's chips, to the winner
/// (0.13.7).
pub const CHEER_DRINK_CHIPS: u64 = 5_000;

/// What Pyr Casino answers (or tells everyone in it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "is", rename_all = "snake_case")]
pub enum CasinoRes {
    State { view: CasinoView },
    /// A round that's over, with the house's seed shown (and the next round's fingerprint).
    Round { round: Box<RoundView> },
    /// My blackjack hand, still being played (and the fingerprint for any other game meanwhile).
    Blackjack {
        view: blackjack::View,
        chips: u64,
        #[serde(with = "hex32")]
        next_commit: [u8; 32],
        next_nonce: u64,
    },
    /// My craps bets, changed.
    Craps { table: craps::Table, chips: u64 },
    BigWin { win: BigWin },
    Jackpot { pool: u64 },
    /// A game's statistics (0.12.0, asked for with `Stats`).
    Stats { view: GameStats },
    /// Someone said Congrats! for my big win, or bought me a drink (`chips` with it): 0.13.7.
    Cheered { from: String, drink: bool, chips: u64, cheer: u64 },
    /// My Congrats! (or drink) went to the winner.
    CheerSent { cheer: u64, drink: bool },
}

impl CasinoRes {
    /// As a 0.12 app can read it.
    pub fn for_012_app(self) -> Option<CasinoRes> {
        Some(self)
    }
}

/// What a game (a slot, a Reel Row machine) paid back over time, for everyone on this server,
/// its biggest wins and its luckiest (by how many times the bet), and how many are playing it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameStats {
    pub key: String,
    /// The last 24 hours, 7 days, 28 days and all time, in that order.
    pub periods: Vec<PeriodStats>,
    /// People who played it in the last few minutes.
    pub playing: u32,
    pub big: Vec<BigWin>,
    pub lucky: Vec<BigWin>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PeriodStats {
    pub rounds: u64,
    pub wagered: u64,
    pub paid: u64,
    /// What this Pyr doesn't know (a newer one's, or 0.13.28's not rebuilt yet): kept and
    /// written back as it was.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// The casino as it is for me.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CasinoView {
    pub chips: u64,
    /// The fingerprint of the house's seed for my next round, and its number.
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    pub nonce: u64,
    pub jackpot: u64,
    /// Pyr Friday Night: when it starts, and its prize money so far.
    pub friday_ms: u64,
    pub friday_prize: u64,
    pub big_wins: Vec<BigWin>,
    pub blackjack: Option<blackjack::View>,
    pub craps: craps::Table,
    pub stats: BTreeMap<String, CasinoStats>,
    /// Ember Dragon (0.12.0 apps): my dragon's growth at each bet, and the jackpots' pools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dragon: Option<DragonView>,
    /// Reel Row's progressive jackpots (0.13.0): each machine's pool (all of it at the biggest
    /// bet).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub row_pools: Vec<(u8, u64)>,
    /// How many are playing each game (0.13.4): who's played a round of it in the last five
    /// minutes, by its statistics key (games nobody is playing left out), as 0.13.28's server
    /// says it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub playing: Vec<(String, u32)>,
}

/// Ember Dragon as it stands for me: how grown my dragon is at each bet, and the jackpots'
/// pools (Mini to Grand).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DragonView {
    /// (As pairs on the wire, [bet, growth]: a map's number keys are text in JSON, and inside a
    /// tagged message serde can't read them back as numbers. The whole casino's state was
    /// passed over, and nothing could be played, once anyone's dragon had grown.)
    #[serde(with = "pairs")]
    pub growth: BTreeMap<u64, u32>,
    pub pools: [u64; 4],
}

/// A map with number keys, as a list of pairs (see `DragonView::growth`).
mod pairs {
    use std::collections::BTreeMap;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(m: &BTreeMap<u64, u32>, s: S) -> Result<S::Ok, S::Error> {
        s.collect_seq(m.iter())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<BTreeMap<u64, u32>, D::Error> {
        Ok(Vec::<(u64, u32)>::deserialize(d)?.into_iter().collect())
    }
}

/// A round that's over, and everything to check it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoundView {
    pub nonce: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    #[serde(with = "hex32")]
    pub server_seed: [u8; 32],
    #[serde(with = "hex32")]
    pub client_seed: [u8; 32],
    pub outcome: Outcome,
    #[serde(with = "hex32")]
    pub next_commit: [u8; 32],
    pub next_nonce: u64,
    /// My chips after it, and the jackpot's pool.
    pub chips: u64,
    pub jackpot: u64,
    /// Craps: the table after the roll.
    #[serde(default)]
    pub craps: Option<craps::Table>,
}

/// Someone's big win, for the floor's ticker.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BigWin {
    pub name: String,
    pub game: Game,
    pub wagered: u64,
    pub paid: u64,
    pub what: String,
    pub at_ms: u64,
    /// Its game's statistics key (a Reel Row machine's own; 0.12.0).
    #[serde(default)]
    pub key: String,
    /// Its moment (0.13.7): while it lasts, everyone else in the casino can say Congrats! or
    /// buy the winner a drink (`CasinoReq::Cheer` with this). 0 for a big win without one.
    #[serde(default)]
    pub cheer: u64,
    /// What this Pyr doesn't know (a newer one's, or 0.13.28's not rebuilt yet): kept and
    /// written back as it was.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// What the control room asks (0.13.5), as 0.13.28 does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CmsReq {
    /// The casino at a glance.
    Overview,
    /// The players with the most chips, or those whose names have `query` in them.
    Players { query: String },
    /// Chips given (`delta` above 0) or taken away (below: as many as they have at most), with
    /// a note for the log.
    Adjust { uid: String, delta: i64, note: String },
    /// Something said to everyone in the casino.
    Announce { text: String },
    /// Who can open the control room.
    Access,
    /// Let someone into the control room, or take them out (the Owner of the Game only).
    SetAccess { uid: String, on: bool },
}

/// The control room's answers (0.13.5), as 0.13.28 sends them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum CmsRes {
    Overview(CmsOverview),
    Players { rows: Vec<CmsPlayer> },
    /// Whether I'm the Owner of the Game (who lets people in), and who's in: (name, id).
    Access { owner: bool, people: Vec<(String, String)> },
    /// Done: what happened, as the log says it.
    Done { what: String },
}

/// The casino at a glance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CmsOverview {
    /// Players with a record, and their chips (and those sitting at poker tables).
    pub players: u64,
    pub chips: u64,
    pub at_tables: u64,
    /// The casino's games, all time: bet, paid back, rounds.
    pub wagered: u64,
    pub paid: u64,
    pub rounds: u64,
    pub rake_week: u64,
    pub friday_pool: u64,
    /// The Phoenix Jackpot.
    pub jackpot: u64,
    /// Poker tables open, tournaments listed, staff, entries in the log.
    pub tables: u64,
    pub tourneys: u64,
    pub staff: u64,
    pub log: u64,
    /// Chips, and players, at each VIP tier (Spark to Eternal Flame).
    pub by_tier: Vec<u64>,
    pub tier_players: Vec<u64>,
    pub big_wins: Vec<BigWin>,
    /// The newest in the log.
    pub recent: Vec<StaffLog>,
}

/// A player, for the control room.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CmsPlayer {
    pub uid: String,
    pub name: String,
    pub chips: u64,
    pub at_tables: u64,
    pub level: u32,
    pub hands: u64,
    /// Rounds in the casino, and what they bet and were paid there.
    pub rounds: u64,
    pub wagered: u64,
    pub paid: u64,
    pub ember_claimed: u64,
    pub role: Option<Role>,
}

/// Pyr Poker's staff: Game Masters (the owner's admins for the game: they gift chips and looks
/// and drop prize boxes) and Ambassadors (they drop prize boxes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    GameMaster,
    Ambassador,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::GameMaster => "Game Master",
            Role::Ambassador => "Ambassador",
        }
    }
}

/// A prize box: chips and/or a look (a cosmetic's id), and who put it there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PrizeView {
    pub chips: u64,
    pub item: Option<String>,
    pub from: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaffMember {
    pub uid: String,
    pub name: String,
    pub role: Role,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaffLog {
    pub at_ms: u64,
    pub by: String,
    pub what: String,
    /// What this Pyr doesn't know (a newer one's, or 0.13.28's not rebuilt yet): kept and
    /// written back as it was.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StaffView {
    /// I can open the control room (0.13.5): the Owner of the Game, or let in.
    #[serde(default)]
    pub cms: bool,
    pub me: Option<Role>,
    /// The server's owner: may name staff and read the log.
    pub owner: bool,
    /// Chips I may give away a day (gifts and prize boxes together), and what's left today.
    pub allowance: u64,
    pub left: u64,
    /// Chips raked at the cash tables over the last 7 days (the allowances follow it).
    pub rake_week: u64,
    pub staff: Vec<StaffMember>,
    /// Newest first (the owner only).
    pub log: Vec<StaffLog>,
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
    /// Poker's MMR, its best yet, and whether it's still placing (0.12.0).
    #[serde(default)]
    pub mmr: u32,
    #[serde(default)]
    pub mmr_peak: u32,
    #[serde(default)]
    pub mmr_placing: bool,
    pub daily_ready: bool,
    pub daily_streak: u32,
    pub card_back: String,
    pub felt: String,
    pub chip_set: String,
    pub frame: String,
    pub unlocked: Vec<String>,
    /// Game Master or Ambassador.
    #[serde(default)]
    pub role: Option<Role>,
    // Levels and the VIP Club (0.13.5), as 0.13.28 sends them.
    /// My staff title (Game Master, Ambassador) kept off my seat at the tables (0.13.6).
    #[serde(default)]
    pub hide_title: bool,
    /// My tier (0, Spark, to 9, Eternal Flame).
    #[serde(default)]
    pub tier: u32,
    /// Ember Stars (past level 100).
    #[serde(default)]
    pub stars: u64,
    /// Ember Back to claim (chips), and my tier's share of the house's edge (hundredths of a
    /// percent: 500 is 5%).
    #[serde(default)]
    pub ember_back: u64,
    #[serde(default)]
    pub ember_bp: u64,
    /// What the next level pays (or, past 100, the next star).
    #[serde(default)]
    pub next_reward: u64,
    /// Today's missions.
    #[serde(default)]
    pub missions: Vec<MissionView>,
    /// The weekly VIP bonus waiting, and what this week's Ember Back has built of next week's.
    #[serde(default)]
    pub weekly_ready: u64,
    #[serde(default)]
    pub weekly_building: u64,
}

/// One of today's missions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MissionView {
    pub text: String,
    pub progress: u32,
    pub target: u32,
    pub done: bool,
    pub xp: u64,
    pub chips: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GainsView {
    pub xp: u64,
    pub achievements: Vec<String>,
    pub level_up: Option<u32>,
    /// Chips won or lost in the hand.
    pub net: i64,
    // What came with it (0.13.5), as 0.13.28 sends it.
    /// Chips paid besides the round's own: levels reached, a new tier's chest, missions, the
    /// day's chest, an Ember Star.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub chips: u64,
    /// A new tier reached: the level it starts at (10, Ember, to 100, Eternal Flame).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_up: Option<u32>,
    /// Missions done, as they read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missions: Vec<String>,
    /// All three of today's missions done: the day's chest.
    #[serde(default, skip_serializing_if = "is_false")]
    pub all_missions: bool,
    /// A new Ember Star: how many now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub star: Option<u64>,
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A blackjack table, in the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BjSummary {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    pub people: usize,
    pub bots: usize,
}

/// Where a blackjack table's round has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BjStage {
    /// Nobody (no person) sitting.
    Waiting,
    Betting,
    Playing,
    /// The round's over: its results on the table for a moment.
    Settled,
}

/// Someone at a blackjack table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BjSeatView {
    pub player: String,
    pub name: String,
    pub bot: bool,
    pub level: u32,
    pub card_back: String,
    pub frame: String,
    /// Their bet for the round (while betting), and whether they've bet or sat it out.
    pub bet: u64,
    pub decided: bool,
}

/// A finished round's seeds, bets and moves, for everyone's Pyr to play it again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BjReveal {
    pub round: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    #[serde(with = "hex32")]
    pub house: [u8; 32],
    /// The players' seeds by seat (hex), the bets and the moves.
    pub seeds: Vec<(u8, String)>,
    pub bets: Vec<(u8, u64)>,
    pub actions: Vec<(u8, crate::casino::blackjack::Action)>,
    /// The round as the table showed it (to compare with the replay).
    pub dealer: Vec<Card>,
    pub seats: Vec<crate::casino::bjtable::SeatPlay>,
}

/// A blackjack table as someone there sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BjView {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    pub seats: Vec<Option<BjSeatView>>,
    pub stage: BjStage,
    /// Time left to bet, or for the player to act.
    pub ms_left: u64,
    pub round_no: u64,
    /// The house's fingerprint for the next round (or this one).
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    pub round: Option<crate::casino::bjtable::RoundView>,
    pub last: Option<BjReveal>,
    pub my_seat: Option<u8>,
}

/// A dominó table, in the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomSummary {
    pub id: u64,
    pub name: String,
    pub kind: crate::casino::domino::Kind,
    pub ante: u64,
    pub people: usize,
    pub bots: usize,
    /// A hand being played.
    pub playing: bool,
}

/// Where a dominó table's hand has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DomStage {
    /// Nobody (no person) ready.
    Waiting,
    /// The next hand's coming: a moment once everyone's in, longer for those not in yet.
    Gathering,
    Playing,
    /// The hand's over: its result on the table for a moment.
    Settled,
}

/// Someone at a dominó table: in for the next hand (`ready`), and what they've put up.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomSeatView {
    pub player: String,
    pub name: String,
    pub bot: bool,
    pub level: u32,
    pub frame: String,
    pub ready: bool,
    pub escrow: u64,
}

/// A hand as someone at the table sees it: the line, how many each holds (by place in
/// `seats`), my own tiles (`mine`) and what I may do; everyone's at the end (`shown`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomHandView {
    pub kind: crate::casino::domino::Kind,
    pub seats: Vec<u8>,
    pub counts: Vec<u8>,
    /// What's left to draw from, and whether anyone draws (Solo).
    pub pool: usize,
    pub draws: bool,
    pub chain: Vec<crate::casino::domino::Tile>,
    pub to_act: Option<u8>,
    pub mult: u32,
    pub cube: Option<u8>,
    pub offer: Option<u8>,
    pub laid: Vec<u8>,
    pub last: Option<(u8, crate::casino::domino::Move)>,
    pub moves: usize,
    pub mine: Vec<crate::casino::domino::Tile>,
    pub shown: Vec<Vec<crate::casino::domino::Tile>>,
    pub over: Option<crate::casino::domino::Over>,
    pub allowed: Vec<crate::casino::domino::Move>,
    /// Where the first tile laid is in the line.
    pub lead: usize,
}

/// A finished hand's seeds and moves, for everyone's Pyr to play it again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomRecord {
    pub round: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    #[serde(with = "hex32")]
    pub house: [u8; 32],
    /// The people's seeds by seat (hex).
    pub seeds: Vec<(u8, String)>,
    pub kind: crate::casino::domino::Kind,
    pub seats: Vec<u8>,
    pub moves: Vec<(u8, crate::casino::domino::Move)>,
    pub over: crate::casino::domino::Over,
}

/// What a finished hand paid: how it ended (0.13.28's words), who won (seats), the pot, the
/// house's part, the bonus (a Capicúa, a Chiva), the stake, each seat's share.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomResult {
    pub how: String,
    pub winners: Vec<u8>,
    pub pot: u64,
    pub rake: u64,
    pub bonus_pct: u32,
    pub mult: u32,
    pub paid: Vec<(u8, u64)>,
}

/// A dominó table as someone there sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DomView {
    pub id: u64,
    pub name: String,
    pub kind: crate::casino::domino::Kind,
    pub ante: u64,
    pub seats: Vec<Option<DomSeatView>>,
    pub stage: DomStage,
    pub ms_left: u64,
    pub round_no: u64,
    /// The house's fingerprint for the next hand (or this one).
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    pub hand: Option<DomHandView>,
    pub last: Option<DomRecord>,
    pub result: Option<DomResult>,
    pub my_seat: Option<u8>,
    /// How far the stake can go this hand (the least anyone in it covered); 8 between hands.
    pub cap: u32,
}

/// A craps table, in the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrapsSummary {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    pub people: usize,
    pub bots: usize,
    /// The point, if there is one.
    pub point: Option<u8>,
}

/// Where a craps table's roll has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrapsStage {
    /// Nobody (no person) sitting.
    Waiting,
    /// Bets going down for the coming roll.
    Betting,
    /// The dice have landed: the roll's results on the table for a moment.
    Shown,
}

/// Someone at a craps table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrapsSeatView {
    pub player: String,
    pub name: String,
    pub bot: bool,
    pub level: u32,
    pub frame: String,
    /// What's on the layout for them, and whether they're ready for the dice.
    pub bets: crate::casino::craps::Bets,
    pub ready: bool,
    /// What the last roll did for them: the bets it decided, and the chips back.
    pub decided: Vec<crate::casino::craps::Decided>,
    pub paid: u64,
    /// A bot's way of betting, in a few words.
    pub style: String,
}

/// A roll's seeds and dice, for everyone's Pyr to roll it again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrapsReveal {
    pub roll: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    #[serde(with = "hex32")]
    pub house: [u8; 32],
    /// The players' seeds by seat (hex).
    pub seeds: Vec<(u8, String)>,
    pub dice: (u8, u8),
    pub point_before: Option<u8>,
    pub point_after: Option<u8>,
    /// Who threw them (a seat).
    pub shooter: Option<u8>,
}

/// A craps table as someone there sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrapsView {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    pub seats: Vec<Option<CrapsSeatView>>,
    pub stage: CrapsStage,
    /// Time left to bet before the dice fly, or till the next roll's betting.
    pub ms_left: u64,
    /// The coming roll's number and the house's fingerprint for it.
    pub roll_no: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    pub point: Option<u8>,
    /// The shooter's points made in a row.
    pub streak: u32,
    /// Whose dice they are (a seat).
    pub shooter: Option<u8>,
    pub last: Option<CrapsReveal>,
    pub my_seat: Option<u8>,
    /// My pass line (and its odds) still working from when I stood up.
    pub left: crate::casino::craps::Bets,
}

/// A baccarat table, in the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BacSummary {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    pub people: usize,
    pub bots: usize,
}

/// Where a baccarat table's coup has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BacStage {
    /// Nobody (no person) sitting.
    Waiting,
    /// Bets going down on the coming coup.
    Betting,
    /// The coup's dealt: turned over card by card, its result on the table for a moment.
    Shown,
}

/// Someone at a baccarat table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BacSeatView {
    pub player: String,
    pub name: String,
    pub bot: bool,
    pub level: u32,
    pub frame: String,
    /// Their bets on the coming coup, and whether they're ready for the cards.
    pub bets: crate::casino::baccarat::Bets,
    pub ready: bool,
    /// The last coup: what they had on it, and what came back.
    pub last_bets: crate::casino::baccarat::Bets,
    pub paid: u64,
    /// A bot's way of betting, in a few words.
    pub style: String,
}

/// A coup on the scoreboard: who won, the pairs, a natural, and the winning total.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacMark {
    pub winner: crate::casino::baccarat::Side,
    pub player_pair: bool,
    pub banker_pair: bool,
    pub natural: bool,
    pub total: u8,
}

/// A coup's seeds and cards, for everyone's Pyr to deal it again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BacReveal {
    pub coup: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    #[serde(with = "hex32")]
    pub house: [u8; 32],
    /// The players' seeds by seat (hex).
    pub seeds: Vec<(u8, String)>,
    /// The cards as dealt to each side.
    pub player: Vec<Card>,
    pub banker: Vec<Card>,
    pub winner: crate::casino::baccarat::Side,
    pub player_pair: bool,
    pub banker_pair: bool,
}

/// A baccarat table as someone there sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BacView {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    pub seats: Vec<Option<BacSeatView>>,
    pub stage: BacStage,
    /// Time left to bet before the cards come, or till the next coup's betting.
    pub ms_left: u64,
    /// The coming coup's number and the house's fingerprint for it.
    pub coup_no: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    pub last: Option<BacReveal>,
    /// The shoe so far, for the scoreboard (oldest first).
    pub history: Vec<BacMark>,
    pub my_seat: Option<u8>,
}

/// A roulette table, in the list.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RlSummary {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    pub people: usize,
    pub bots: usize,
}

/// Where a roulette table's spin has got to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RlStage {
    /// Nobody (no person) sitting.
    Waiting,
    /// Chips going down on the coming spin.
    Betting,
    /// The ball's going round (where it lands is decided), then its number's on the table.
    Shown,
}

/// Someone at a roulette table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RlSeatView {
    pub player: String,
    pub name: String,
    pub bot: bool,
    pub level: u32,
    pub frame: String,
    /// Their chips on the coming spin, and whether they're ready for the ball.
    pub chips: Vec<crate::casino::roulette::Chip>,
    pub ready: bool,
    /// The last spin: what they had on it, and what came back.
    pub last_chips: Vec<crate::casino::roulette::Chip>,
    pub paid: u64,
    /// A bot's way of betting, in a few words.
    pub style: String,
}

/// A spin's seeds and where the ball landed, for everyone's Pyr to spin it again.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RlReveal {
    pub spin: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    #[serde(with = "hex32")]
    pub house: [u8; 32],
    /// The players' seeds by seat (hex).
    pub seeds: Vec<(u8, String)>,
    pub number: u8,
    /// Its place round the wheel.
    pub pocket: u8,
}

/// A roulette table as someone there sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RlView {
    pub id: u64,
    pub name: String,
    pub min_bet: u64,
    pub max_bet: u64,
    pub seats: Vec<Option<RlSeatView>>,
    pub stage: RlStage,
    /// Time left to bet before the ball goes, or till the next spin's betting.
    pub ms_left: u64,
    /// The coming spin's number and the house's fingerprint for it.
    pub spin_no: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    pub last: Option<RlReveal>,
    /// The numbers so far (oldest first).
    pub history: Vec<u8>,
    pub my_seat: Option<u8>,
}

/// The Bonfire as someone sees it: the pool, when it's lit, my embers and everyone's, the most
/// embers this time, and the last time it was lit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BonfireView {
    /// This fire's number, its pool, and how long till it's lit.
    pub round: u64,
    pub pool: u64,
    pub ms_left: u64,
    /// The house's fingerprint for this fire's draw (its seed's shown when it's lit).
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    pub my_embers: u64,
    pub total_embers: u64,
    pub entrants: usize,
    /// The most embers on this fire: (name, embers).
    pub top: Vec<(String, u64)>,
    pub last: Option<BonfireDraw>,
    /// Whether I can light it now (the server's owner).
    pub can_light: bool,
}

/// Someone's embers on a fire, as it's lit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BonfireEntry {
    pub uid: String,
    pub name: String,
    pub embers: u64,
}

/// A prize won.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BonfireWinner {
    pub tier: crate::casino::bonfire::Tier,
    pub uid: String,
    pub name: String,
    pub chips: u64,
    pub embers: u64,
}

/// A fire lit: its seed (against the fingerprint shown all along), everyone's embers (by id),
/// the winners in the order drawn, and what goes on the next fire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BonfireDraw {
    pub round: u64,
    pub at_ms: u64,
    pub pool: u64,
    #[serde(with = "hex32")]
    pub commit: [u8; 32],
    #[serde(with = "hex32")]
    pub seed: [u8; 32],
    pub entries: Vec<BonfireEntry>,
    pub winners: Vec<BonfireWinner>,
    pub rollover: u64,
    /// What this Pyr doesn't know (a newer one's, or 0.13.28's not rebuilt yet): kept and
    /// written back as it was.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Someone on the MMR ladder.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RatingRow {
    /// Their rating id (a person's Pyr id; `bot:<level>:<name>` for a bot).
    pub id: String,
    pub name: String,
    pub bot: Option<Level>,
    pub mmr: u32,
    pub peak: u32,
    pub tourneys: u32,
    pub hands: u32,
    /// Still finding their level (their first few games).
    pub placing: bool,
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

/// How someone has played: hands dealt in, won, seen to a showdown and won there, chips up or
/// down, and the like.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tally {
    pub hands: u64,
    pub won: u64,
    pub showdowns: u64,
    pub showdowns_won: u64,
    /// Hands they put chips in before the flop by choice, and raised before the flop.
    pub vpip: u64,
    pub pfr: u64,
    /// Chips up (or down) over these hands.
    pub net: i64,
    pub biggest_pot: u64,
    pub best_hand: String,
    #[serde(default, skip_serializing)]
    pub best_value: u32,
}

/// One of someone's hands, as whoever asks may see it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerHand {
    pub number: u64,
    pub ended_ms: u64,
    /// Their cards, when they were shown (or they're mine); empty otherwise.
    pub cards: Vec<Card>,
    pub board: Vec<Card>,
    /// Chips up or down in it.
    pub net: i64,
    /// What happened: "Won with Two Pair, Kings and Sevens", "Folded before the flop".
    pub how: String,
    /// Whether the whole hand is still kept (to open it).
    pub kept: bool,
}

/// Someone at a table, when right-clicked.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlayerView {
    pub seat: usize,
    pub name: String,
    pub bot: Option<Level>,
    /// Their MMR (0.12.0).
    #[serde(default)]
    pub mmr: Option<u32>,
    /// At this table.
    pub here: Tally,
    /// Everywhere on this server (people; a bot's whole life is at its table).
    pub overall: Option<Tally>,
    /// Their last hands here, newest first.
    pub hands: Vec<PlayerHand>,
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
        let m = ToServer::GameEmote { game: TableGame::Baccarat, table: 2, emote: "e:gg".into() };
        let j = serde_json::to_string(&m).unwrap();
        assert!(j.contains("\"game\":\"baccarat\""), "{j}");
        assert_eq!(serde_json::from_str::<ToServer>(&j).unwrap(), m);
    }

    /// A new table's bots and a bot at an empty seat picked by who and how good, as 0.13.28's
    /// app sends them (taken from it, on the rebuilt server): wire names are the old ones
    /// (genius is Shark, shark Expert), `who` only for the ones picked by name.
    #[test]
    fn bots_picked_as_0_13_28_asks() {
        let create = r#"{"op":"create","name":"Friend's table","rules":{"variant":"holdem","limit":"no_limit","small_blind":10,"big_blind":20,"ante":0,"seats":6},"bots":[[2,"genius"],[3,"shark"],[4,"regular"]],"buy_in":{"min_bb":40,"max_bb":200},"who":[[2,"Sage"],[3,"Moss"]]}"#;
        let m: ToServer = serde_json::from_str(create).unwrap();
        let ToServer::Create { bots, who, .. } = &m else { panic!() };
        assert_eq!(bots, &vec![(2, Level::Genius), (3, Level::Shark), (4, Level::Regular)]);
        assert_eq!(who, &vec![(2, "Sage".to_string()), (3, "Moss".to_string())]);
        assert_eq!(serde_json::to_string(&m).unwrap(), create);
        let named = r#"{"op":"add_bot","table":1,"seat":1,"level":"hard","who":"Nova"}"#;
        let m: ToServer = serde_json::from_str(named).unwrap();
        assert_eq!(m, ToServer::AddBot { table: 1, seat: 1, level: Level::Hard, who: Some("Nova".into()) });
        assert_eq!(serde_json::to_string(&m).unwrap(), named);
        let anyone = r#"{"op":"add_bot","table":1,"seat":5,"level":"regular"}"#;
        let m: ToServer = serde_json::from_str(anyone).unwrap();
        assert_eq!(m, ToServer::AddBot { table: 1, seat: 5, level: Level::Regular, who: None });
        assert_eq!(serde_json::to_string(&m).unwrap(), anyone);
        // A new table from an older app: no `who`.
        let old = create.replace(r#","who":[[2,"Sage"],[3,"Moss"]]"#, "");
        let ToServer::Create { who, .. } = serde_json::from_str(&old).unwrap() else { panic!() };
        assert!(who.is_empty());
    }

    /// An app from before 0.13.7 (without `casino_0_13_7`) is told a Hard bot is a Medium one,
    /// as 0.13.28's server tells it (the rebuilt app watching its table, a Hard Quill: MEDIUM
    /// BOT); one that knows them, as it is.
    #[test]
    fn hard_bots_are_medium_to_apps_from_before_0_13_7() {
        let row = |bot| RatingRow { id: "bot:hard:Quill".into(), name: "Quill".into(), bot, mmr: 1075, peak: 1075, tourneys: 0, hands: 0, placing: true };
        let m = ToApp::Ratings { rows: vec![row(Some(Level::Hard)), row(Some(Level::Genius))] };
        assert_eq!(m.clone().for_app(CASINO_0137), Some(m.clone()));
        assert_eq!(m.clone().for_app(CASINO_PROPS), Some(m.clone()));
        assert_eq!(m.clone().for_app(CASINO_013), Some(ToApp::Ratings { rows: vec![row(Some(Level::Regular)), row(Some(Level::Genius))] }));
        assert_eq!(m.for_app(CASINO_012), Some(ToApp::Ratings { rows: vec![row(Some(Level::Regular)), row(Some(Level::Genius))] }));
    }

    /// A table invite, as 0.13.28's app asks for it and its server sends it (both taken from
    /// them: its app on the rebuilt server, the rebuilt app on its server).
    #[test]
    fn table_invites_as_0_13_28_writes_them() {
        let ask = r#"{"op":"invite_to_table","uid":"n2w7iGY1qtleS1OKEuMvgKJk0bY5jsa1ATbtp9V9-uQ","table":{"game":"roulette","id":1}}"#;
        let m: ToServer = serde_json::from_str(ask).unwrap();
        assert_eq!(m, ToServer::InviteToTable { uid: "n2w7iGY1qtleS1OKEuMvgKJk0bY5jsa1ATbtp9V9-uQ".into(), table: TableRef::Roulette(1) });
        assert_eq!(serde_json::to_string(&m).unwrap(), ask);
        let poker = r#"{"op":"invite_to_table","uid":"x","table":{"game":"poker","id":1}}"#;
        assert_eq!(serde_json::from_str::<ToServer>(poker).unwrap(), ToServer::InviteToTable { uid: "x".into(), table: TableRef::Poker(1) });
        let told = r#"{"op":"table_invite","from":"v0KoUewTmf80VoYBVvUjUij1A_getcz8Jx9B1YR_KQs","table":{"game":"roulette","id":3},"name":"Grand Salle"}"#;
        let m: ToApp = serde_json::from_str(told).unwrap();
        assert_eq!(m, ToApp::TableInvite { from: "v0KoUewTmf80VoYBVvUjUij1A_getcz8Jx9B1YR_KQs".into(), table: TableRef::Roulette(3), name: "Grand Salle".into() });
        assert_eq!(serde_json::to_string(&m).unwrap(), told);
        // (Dominó's too, though there's no Dominó here yet.)
        assert_eq!(serde_json::from_str::<TableRef>(r#"{"game":"domino","id":2}"#).unwrap(), TableRef::Domino(2));
        // Never to an app from before 0.13.0.
        assert_eq!(m.clone().for_app(CASINO_012), None);
        assert_eq!(m.clone().for_app(CASINO_011), None);
        assert!(m.for_app(CASINO_013).is_some());
    }

    /// Every kind of message the casino sends, with everything in it filled in, reads back as
    /// it was: the casino's state with a grown dragon in it once couldn't be (its bets were a
    /// map's keys), and the app passed it over.
    #[test]
    fn the_casinos_state_reads_back_with_everything_in_it() {
        let view = CasinoView {
            chips: 56_309,
            commit: [5; 32],
            nonce: 12,
            jackpot: 900,
            friday_ms: 1_700_000_000_000,
            friday_prize: 4_000,
            big_wins: Vec::new(),
            blackjack: None,
            craps: craps::Table::default(),
            stats: [("slots".to_string(), CasinoStats::default())].into_iter().collect(),
            dragon: Some(DragonView { growth: [(100, 3), (2_000, 41)].into_iter().collect(), pools: [1, 2, 3, 4] }),
            row_pools: vec![(4, 123_456_789), (9, 100_000_000)],
            playing: vec![("plinko".to_string(), 1), ("reels:3".to_string(), 2)],
        };
        let m = ToApp::Casino { res: CasinoRes::State { view } };
        let j = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<ToApp>(&j).unwrap(), m, "{j}");
    }

    /// The casino's state as 0.13.28's server sends it (taken off the wire, trimmed): its
    /// "playing" list, by game, reads.
    #[test]
    fn the_casinos_state_from_0_13_28_reads_with_who_is_playing() {
        let j = r#"{"op":"casino","res":{"is":"state","view":{"chips":100000,"commit":"967ff0c0494d02dce19d73e1db88f068e45b0ccbe95a87e4fdead0fc1c36fa11","nonce":1,"jackpot":10009000,"friday_ms":1794625200000,"friday_prize":100938,"big_wins":[],"blackjack":null,"craps":{"point":null,"bets":{"pass":0,"pass_odds":0,"dont_pass":0,"dont_odds":0,"field":0,"any_seven":0,"any_craps":0,"place":[0,0,0,0,0,0],"hard":[0,0,0,0]},"streak":0},"stats":{},"dragon":{"growth":[],"pools":[2000,10000,50000,250000]},"row_pools":[[4,100000000],[9,100000000]],"playing":[["plinko",1]]}}}"#;
        let m: ToApp = serde_json::from_str(j).unwrap();
        let ToApp::Casino { res: CasinoRes::State { view } } = m else { panic!("not the casino's state") };
        assert_eq!(view.playing, vec![("plinko".to_string(), 1)]);
    }

    /// A big win's moment (0.13.7), as 0.13.28's apps and server say it (taken off the wire):
    /// read and written back the same; and an app from before 0.13.7 is told none of it.
    #[test]
    fn a_big_wins_moment_reads_as_0_13_28s() {
        let same = |j: &str, to_app: bool| {
            let v: serde_json::Value = serde_json::from_str(j).unwrap();
            let back = if to_app { serde_json::to_value(serde_json::from_value::<ToApp>(v.clone()).unwrap()) } else { serde_json::to_value(serde_json::from_value::<ToServer>(v.clone()).unwrap()) };
            assert_eq!(back.unwrap(), v, "{j}");
        };
        same(r#"{"op":"casino","req":{"do":"cheer","cheer":1794622405386,"drink":true}}"#, false);
        same(r#"{"op":"casino","res":{"is":"cheered","from":"Bea","drink":true,"chips":5000,"cheer":1794622405386}}"#, true);
        same(r#"{"op":"casino","res":{"is":"cheered","from":"Bea","drink":false,"chips":0,"cheer":1794622405386}}"#, true);
        same(r#"{"op":"casino","res":{"is":"cheer_sent","cheer":1794622405386,"drink":true}}"#, true);
        let w: BigWin = serde_json::from_str(r#"{"name":"Friend","game":"plinko","wagered":100,"paid":11400,"what":"114\u00d7 on Expert with 9 rows","at_ms":1794622405385,"key":"plinko","cheer":1794622405386}"#).unwrap();
        assert_eq!(w.cheer, 1794622405386);
        let cheered = ToApp::Casino { res: CasinoRes::Cheered { from: "Bea".into(), drink: true, chips: CHEER_DRINK_CHIPS, cheer: 1 } };
        assert_eq!(cheered.clone().for_app(CASINO_013), None);
        assert_eq!(ToApp::Casino { res: CasinoRes::CheerSent { cheer: 1, drink: false } }.for_app(CASINO_013), None);
        assert_eq!(cheered.clone().for_app(CASINO_0137), Some(cheered));
    }

    /// What a server keeps (in its database's meta table) and a later Pyr added to: read by this
    /// one and written back, nothing of it is lost (0.13.28's server writes these, and the
    /// rebuilt one, after the swap, writes them back).
    #[test]
    fn what_a_newer_pyr_kept_comes_back_as_it_was() {
        fn same<T: serde::Serialize + serde::de::DeserializeOwned>(json: &str) {
            let v: serde_json::Value = serde_json::from_str(json).unwrap();
            let t: T = serde_json::from_value(v.clone()).unwrap_or_else(|e| panic!("{json}: {e}"));
            assert_eq!(serde_json::to_value(&t).unwrap(), v, "{json}");
        }
        let newer = r#""from_later":{"a":[1,2],"b":"x"},"also":7"#;
        same::<ChatLine>(&format!(r#"{{"id":4,"uid":"u","name":"Ann","text":"hi","at_ms":9,{newer}}}"#));
        same::<PeriodStats>(&format!(r#"{{"rounds":3,"wagered":300,"paid":297,{newer}}}"#));
        same::<BigWin>(&format!(r#"{{"name":"Mara","game":"dice","wagered":10,"paid":495,"what":"49.5x","at_ms":1,"key":"dice","cheer":2,{newer}}}"#));
        same::<StaffLog>(&format!(r#"{{"at_ms":1,"by":"Olive","what":"gave Bo 5,000 chips","no":12,{newer}}}"#));
        let draw = crate::casino::bonfire::lit(3, 77, [9; 32], 1_000_000, Vec::new());
        let mut v = serde_json::to_value(&draw).unwrap();
        v["from_later"] = serde_json::json!({"a": 1});
        same::<BonfireDraw>(&v.to_string());
    }

    /// An older Pyr isn't sent an emote from a casino table, and sees a sticker at poker as the
    /// emoji nearest it (an emoji sent by one is left as it is).
    #[test]
    fn older_apps_see_stickers_as_emoji() {
        assert_eq!(ToApp::GameEmote { game: TableGame::Craps, table: 1, seat: 2, emote: "e:gg".into() }.for_older_app(), None);
        let sticker = ToApp::Emote { table: 4, seat: 1, emote: "e:laugh".into() };
        assert_eq!(sticker.for_older_app(), Some(ToApp::Emote { table: 4, seat: 1, emote: "😂".into() }));
        let emoji = ToApp::Emote { table: 4, seat: 1, emote: "🔥".into() };
        assert_eq!(emoji.clone().for_older_app(), Some(emoji));
        assert_eq!(emote_for_older_app("e:gg"), "GG");
        assert_eq!(emote_for_older_app("e:king"), "👑");
    }
}
