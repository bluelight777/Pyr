//! Poker as this Pyr sees it on a server: the lobby, the tables it's watching or sitting at
//! (with what just happened, for the table to animate), the clock, my record, and the check of
//! every shuffle (this Pyr adds a seed to each hand it's dealt into, and checks the dealer's
//! seed against its promise when the hand is over).

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use pyr_poker::cards::Card;
use pyr_poker::fair;
use pyr_poker::game::Event;
use pyr_poker::net::{GainsView, HandHistory, LeaderRow, PlayerView, PrizeView, StaffView, RecordView, TableId, TableSummary, TableView, ToApp, TourneyId, TourneySummary, TourneyView};

/// How a hand's shuffle checked out.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    /// Waiting for the hand to end.
    Pending,
    /// The dealer kept its promise, my seed was used, and every card matched.
    Fair,
    Problem(String),
}

#[derive(Clone, Debug)]
pub struct HandCheck {
    pub commit: [u8; 32],
    pub my_seed: Option<[u8; 32]>,
    pub seeds: Option<Vec<[u8; 32]>>,
    /// The order seats were dealt in, cards each, and my seat (to find my cards in the deck).
    pub dealt_order: Vec<usize>,
    pub per_seat: usize,
    pub my_cards: Vec<Card>,
    pub my_seat: Option<usize>,
    pub check: Check,
}

/// A table this Pyr follows.
#[derive(Clone, Debug)]
pub struct TableLive {
    pub view: TableView,
    /// This hand's events as they came (for the table to animate and show the log).
    pub events: Vec<(Instant, Event)>,
    /// Whose turn and when their time runs out; their time bank.
    pub clock: Option<(usize, Instant, Duration, u64)>,
    pub emotes: Vec<(usize, String, Instant)>,
    /// Shuffle checks by hand.
    pub checks: BTreeMap<u64, HandCheck>,
    /// Every shuffle checked and found fair at this table since I opened it (the count shown:
    /// `checks` keeps only the last few, so counting those would go 29, 30, 29, 30).
    pub fair_total: u64,
    /// The player to act ran out of time and is on their time bank now.
    pub in_bank: bool,
    /// How many of the hand's events there were when the clock last started.
    pub clock_events: usize,
    /// When the last hand ended (to leave the result on the table for a moment).
    pub ended_at: Option<Instant>,
}

#[derive(Clone, Debug, Default)]
pub struct PokerState {
    pub lobby: Option<Vec<TableSummary>>,
    pub me: Option<RecordView>,
    pub tables: BTreeMap<TableId, TableLive>,
    pub leaderboard: Option<Vec<LeaderRow>>,
    pub history: Option<(TableId, HandHistory)>,
    /// Someone right-clicked at a table: how they've played, and their last hands.
    pub player: Option<(TableId, PlayerView)>,
    /// Experience, achievements and level-ups to celebrate (the app takes them).
    pub gains: Vec<GainsView>,
    /// News to show big: a Game Master's gift to me, a prize box won.
    pub notices: Vec<String>,
    /// Pyr Poker's staff, when asked for.
    pub staff: Option<StaffView>,
    /// The daily chips just taken (how many; none: already taken today).
    pub daily: Option<Option<u64>>,
    /// The last refusal, in the server's words.
    pub error: Option<String>,
    /// Tournaments here, and the ones followed in full.
    pub tourneys: Option<Vec<TourneySummary>>,
    pub tourney: BTreeMap<TourneyId, TourneyView>,
    /// Pyr Casino.
    pub casino: crate::casino::CasinoLive,
}

/// What the engine does after taking a message in: send this Pyr's seed for a shuffle.
pub struct SendSeed {
    pub table: TableId,
    pub hand: u64,
    pub seed: [u8; 32],
}

impl PokerState {
    pub fn table(&self, id: TableId) -> Option<&TableLive> {
        self.tables.get(&id)
    }

    /// Takes in a message from the server.
    pub fn apply(&mut self, msg: ToApp, now: Instant) -> Option<SendSeed> {
        match msg {
            ToApp::Lobby { tables, me } => {
                self.lobby = Some(tables);
                self.me = Some(me);
            }
            ToApp::Snapshot { table } => {
                let id = table.id;
                match self.tables.get_mut(&id) {
                    Some(t) => {
                        // A new hand: the last one's events go.
                        if t.view.hand != table.hand && table.hand.is_some() {
                            t.events.clear();
                            t.ended_at = None;
                        }
                        let mut table = table;
                        if t.ended_at.is_some() && table.hand.is_none_or(|h| t.view.hand == Some(h)) {
                            // A hand just ended: the board and the cards shown stay on the
                            // table until the next one's dealt.
                            table.board = std::mem::take(&mut t.view.board);
                            table.hand = t.view.hand;
                            for (new, old) in table.seats.iter_mut().zip(t.view.seats.iter()) {
                                if let (Some(n), Some(o)) = (new.as_mut(), old.as_ref())
                                    && n.player == o.player
                                {
                                    n.cards = o.cards.clone();
                                    n.in_hand = o.in_hand;
                                    n.folded = o.folded;
                                }
                            }
                        }
                        t.view = table;
                    }
                    None => {
                        self.tables.insert(id, TableLive { view: table, events: Vec::new(), clock: None, emotes: Vec::new(), checks: BTreeMap::new(), fair_total: 0, in_bank: false, clock_events: 0, ended_at: None });
                    }
                }
            }
            ToApp::Event { table, hand, event } => {
                let Some(t) = self.tables.get_mut(&table) else { return None };
                if t.view.hand != Some(hand) {
                    t.events.clear();
                    t.view.hand = Some(hand);
                }
                apply_event(t, hand, &event);
                t.events.push((now, event));
            }
            ToApp::Shuffle { table, hand, commit, .. } => {
                let Some(t) = self.tables.get_mut(&table) else { return None };
                let seated = t.view.my_seat.is_some();
                let mut seed = [0u8; 32];
                let send = seated && getrandom::getrandom(&mut seed).is_ok();
                t.checks.insert(
                    hand,
                    HandCheck { commit, my_seed: send.then_some(seed), seeds: None, dealt_order: Vec::new(), per_seat: 0, my_cards: Vec::new(), my_seat: t.view.my_seat, check: Check::Pending },
                );
                // (Only the last few hands' checks are kept.)
                while t.checks.len() > 30 {
                    let first = *t.checks.keys().next().unwrap();
                    t.checks.remove(&first);
                }
                if send {
                    return Some(SendSeed { table, hand, seed });
                }
            }
            ToApp::Seeds { table, hand, seeds, fingerprint } => {
                let Some(c) = self.tables.get_mut(&table).and_then(|t| t.checks.get_mut(&hand)) else { return None };
                if fair::seeds_fingerprint(&seeds) != fingerprint {
                    c.check = Check::Problem("the list of seeds doesn't match its fingerprint".into());
                } else if let Some(mine) = c.my_seed
                    && !seeds.contains(&mine)
                {
                    c.check = Check::Problem("this Pyr's seed was left out of the shuffle".into());
                }
                c.seeds = Some(seeds);
            }
            ToApp::Reveal { table, hand, dealer, dealt } => {
                let Some(t) = self.tables.get_mut(&table) else { return None };
                let Some(c) = t.checks.get_mut(&hand) else { return None };
                if c.check != Check::Pending {
                    return None;
                }
                let Some(seeds) = c.seeds.clone() else {
                    c.check = Check::Problem("the hand was dealt without saying which seeds it used".into());
                    return None;
                };
                // What everyone saw, plus my own cards where they were dealt from.
                let mut seen = dealt.clone();
                if let Some(me) = c.my_seat
                    && let Some(i) = c.dealt_order.iter().position(|s| *s == me)
                {
                    let n = c.dealt_order.len();
                    for (round, card) in c.my_cards.iter().enumerate() {
                        let pos = round * n + i;
                        if seen.len() <= pos {
                            seen.resize(pos + 1, None);
                        }
                        seen[pos] = Some(*card);
                    }
                }
                c.check = match fair::verify(&c.commit, &dealer, &seeds, hand, &seen) {
                    Ok(()) => Check::Fair,
                    Err(e) => Check::Problem(e),
                };
                if c.check == Check::Fair {
                    t.fair_total += 1;
                }
            }
            ToApp::Clock { table, seat, ms, bank_ms } => {
                if let Some(t) = self.tables.get_mut(&table) {
                    // A second clock for the same turn (nothing happened since): the time bank.
                    t.in_bank = t.clock.is_some_and(|(s, ..)| s == seat) && t.view.to_act == Some(seat) && t.clock_events == t.events.len();
                    t.clock_events = t.events.len();
                    t.clock = Some((seat, now + Duration::from_millis(ms), Duration::from_millis(ms), bank_ms));
                }
            }
            ToApp::Emote { table, seat, emote } => {
                if let Some(t) = self.tables.get_mut(&table) {
                    t.emotes.retain(|e| now.duration_since(e.2) < Duration::from_secs(4));
                    t.emotes.push((seat, emote, now));
                }
            }
            ToApp::Record { me, gains } => {
                self.me = Some(me);
                if let Some(g) = gains {
                    self.gains.push(g);
                }
            }
            ToApp::Leaderboard { rows } => self.leaderboard = Some(rows),
            ToApp::History { table, hand } => self.history = Some((table, hand)),
            ToApp::Player { table, player } => self.player = Some((table, player)),
            ToApp::Staff { view } => self.staff = Some(view),
            ToApp::Casino { res } => {
                self.casino.apply(res);
                if let Some(me) = self.me.as_mut()
                    && self.casino.open
                {
                    me.chips = self.casino.chips;
                }
            }
            ToApp::Gifted { from, chips, item, note } => {
                let what = prize_words(&PrizeView { chips, item, from: from.clone() });
                let note = if note.is_empty() { String::new() } else { format!(": \u{201c}{note}\u{201d}") };
                self.notices.push(format!("{from} gave you {what}{note}"));
            }
            ToApp::PrizeWon { table, name, prize, .. } => {
                if let Some(t) = self.tables.get_mut(&table) {
                    t.view.prize = None;
                    self.notices.push(format!("{name} won the prize box: {}", prize_words(&prize)));
                }
            }
            ToApp::Closed { table } => {
                self.tables.remove(&table);
                if let Some(l) = &mut self.lobby {
                    l.retain(|t| t.id != table);
                }
            }
            ToApp::Daily { chips, .. } => self.daily = Some(chips),
            ToApp::Tourneys { list } => self.tourneys = Some(list),
            ToApp::Tourney { view } => {
                // Its line in the list follows along.
                if let Some(l) = &mut self.tourneys
                    && let Some(row) = l.iter_mut().find(|t| t.id == view.id)
                {
                    row.phase = view.phase;
                    row.entrants = view.entrants.len();
                    row.players_left = view.entrants.iter().filter(|e| e.place.is_none()).count();
                    row.prize_pool = view.prize_pool;
                    row.level = view.level;
                }
                self.tourney.insert(view.id, view);
            }
        }
        None
    }

    /// Forgets the tables (the connection dropped: the server stood us up).
    pub fn disconnected(&mut self) {
        self.tables.clear();
    }
}

/// Keeps the table's view up to date from an event (the server's snapshots come at the start
/// and end of a hand and when people sit or stand; in between, events move it along).
fn apply_event(t: &mut TableLive, hand: u64, e: &Event) {
    let v = &mut t.view;
    match e {
        Event::HandStarted { button, seats, .. } => {
            v.button = Some(*button);
            v.board.clear();
            v.pot = 0;
            for (i, s) in v.seats.iter_mut().enumerate() {
                if let Some(s) = s {
                    s.in_hand = seats.contains(&i);
                    s.folded = false;
                    s.all_in = false;
                    s.bet = 0;
                    s.cards.clear();
                }
            }
            t.ended_at = None;
        }
        Event::Posted { seat, amount, .. } => {
            if let Some(Some(s)) = v.seats.get_mut(*seat) {
                s.bet += amount;
                s.stack = s.stack.saturating_sub(*amount);
                if s.stack == 0 {
                    s.all_in = true;
                }
            }
            v.pot += amount;
        }
        Event::Dealt { seat, cards } => {
            if let Some(Some(s)) = v.seats.get_mut(*seat) {
                s.cards = cards.clone();
            }
            if let Some(c) = t.checks.get_mut(&hand) {
                c.my_cards = cards.clone();
                c.per_seat = cards.len();
            }
        }
        Event::DealtFaceDown { seats, count } => {
            if let Some(c) = t.checks.get_mut(&hand) {
                c.dealt_order = seats.clone();
                c.per_seat = *count;
            }
        }
        Event::Acted { seat, action, amount, stack } => {
            if let Some(Some(s)) = v.seats.get_mut(*seat) {
                s.bet += amount;
                s.stack = *stack;
                if matches!(action, pyr_poker::game::Action::Fold) {
                    s.folded = true;
                }
                if *stack == 0 && *amount > 0 {
                    s.all_in = true;
                }
            }
            v.pot += amount;
            if v.to_act == Some(*seat) {
                v.to_act = None;
                v.legal = None;
            }
            t.clock = None;
        }
        Event::Returned { seat, amount } => {
            if let Some(Some(s)) = v.seats.get_mut(*seat) {
                s.stack += amount;
                s.bet = s.bet.saturating_sub(*amount);
            }
            v.pot = v.pot.saturating_sub(*amount);
        }
        Event::Street { street, board, .. } => {
            v.street = Some(*street);
            v.board = board.clone();
            for s in v.seats.iter_mut().flatten() {
                s.bet = 0;
            }
        }
        Event::Turn { seat, legal } => {
            v.to_act = Some(*seat);
            // (Nothing to choose once a Time Bomb's gone off: everyone's all in.)
            v.legal = (v.my_seat == Some(*seat) && !v.bomb).then_some(*legal);
        }
        Event::Shown { seat, cards, .. } => {
            if let Some(Some(s)) = v.seats.get_mut(*seat) {
                s.cards = cards.clone();
            }
        }
        Event::Mucked { .. } => {}
        Event::Won { seats, amounts, .. } => {
            for (s, a) in seats.iter().zip(amounts) {
                if let Some(Some(x)) = v.seats.get_mut(*s) {
                    x.stack += a;
                }
            }
        }
        Event::HandEnded { stacks, .. } => {
            for (seat, stack) in stacks {
                if let Some(Some(s)) = v.seats.get_mut(*seat) {
                    s.stack = *stack;
                    s.bet = 0;
                }
            }
            v.to_act = None;
            v.legal = None;
            v.pot = 0;
            t.clock = None;
            t.ended_at = Some(Instant::now());
        }
    }
}

/// How the shuffles of the last hands at a table checked out: (fair, problems).
/// What's in a prize box (or a gift): "25,000 chips and the Inferno card back".
pub fn prize_words(p: &PrizeView) -> String {
    let chips = if p.chips > 0 {
        let s = p.chips.to_string();
        let mut out = String::new();
        for (i, c) in s.chars().enumerate() {
            if i > 0 && (s.len() - i) % 3 == 0 {
                out.push(',');
            }
            out.push(c);
        }
        Some(format!("{out} chips"))
    } else {
        None
    };
    let look = p.item.as_deref().and_then(pyr_poker::progress::cosmetic).map(|c| {
        let kind = match c.kind {
            pyr_poker::progress::Kind::CardBack => "card back",
            pyr_poker::progress::Kind::Felt => "table",
            pyr_poker::progress::Kind::Chips => "chips",
            pyr_poker::progress::Kind::Frame => "picture frame",
        };
        format!("the {} {kind}", c.name)
    });
    match (chips, look) {
        (Some(c), Some(l)) => format!("{c} and {l}"),
        (Some(c), None) => c,
        (None, Some(l)) => l,
        (None, None) => "nothing".into(),
    }
}

pub fn checks_summary(t: &TableLive) -> (usize, Vec<(u64, String)>) {
    let fair = t.fair_total as usize;
    let problems = t.checks.iter().filter_map(|(h, c)| if let Check::Problem(p) = &c.check { Some((*h, p.clone())) } else { None }).collect();
    (fair, problems)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_poker::fair::Stream;
    use pyr_poker::game::Rules;
    use pyr_poker::net::BuyIn;
    use pyr_poker::room::{Look, Rcpt, Room};

    /// A room as the server runs it, and this Pyr following along as "alice": every hand's
    /// shuffle checks out, her seed included.
    #[test]
    fn every_shuffle_checks_out_from_the_players_side() {
        let mut room = Room::new(1, "T", "alice", Rules::holdem(5, 10, 3), BuyIn::default()).unwrap();
        let mut rng = Stream::new(fair::seed_from(1));
        let mut ps = PokerState::default();
        let t0 = Instant::now();
        let snap = room.watch("alice");
        ps.apply(snap, t0);
        let deliver = |out: pyr_poker::room::Output, ps: &mut PokerState, room: &mut Room| {
            for (to, m) in out.msgs {
                if to == Rcpt::All || to == Rcpt::User("alice".into()) {
                    if let Some(s) = ps.apply(m, t0) {
                        room.seed("alice", s.hand, s.seed);
                    }
                }
            }
        };
        let out = room.sit("alice", "Alice", 0, 1000, Look::default(), 0).unwrap();
        deliver(out, &mut ps, &mut room);
        let out = room.add_bot(1, pyr_poker::bots::Level::Regular, 0, &mut rng).unwrap();
        deliver(out, &mut ps, &mut room);
        let mut now = 0;
        while now < 300_000 {
            let out = room.tick(now, &mut rng);
            deliver(out, &mut ps, &mut room);
            // What she sees matches the table, all the way through (stacks, bets, the pot).
            let mine = &ps.table(1).unwrap().view;
            let real = room.view(Some("alice"));
            let stacks = |v: &pyr_poker::net::TableView| v.seats.iter().map(|s| s.as_ref().map(|s| (s.stack, s.bet))).collect::<Vec<_>>();
            assert_eq!(stacks(mine), stacks(&real), "hand {:?} at {now}", real.hand);
            if let Some(l) = ps.table(1).and_then(|t| t.view.legal) {
                let hand = ps.table(1).unwrap().view.hand.unwrap();
                let a = if l.can_check { pyr_poker::game::Action::Check } else { pyr_poker::game::Action::Call };
                if let Ok(out) = room.act("alice", hand, a, now) {
                    deliver(out, &mut ps, &mut room);
                }
            }
            now += 250;
        }
        let t = ps.table(1).unwrap();
        let (fair, problems) = checks_summary(t);
        assert!(problems.is_empty(), "{problems:?}");
        assert!(fair > 10, "{fair} hands checked");
        // Her seed went into each one.
        assert!(t.checks.values().filter(|c| c.check == Check::Fair).all(|c| c.my_seed.is_some()));
        // The table's view kept up: chips at the table add up.
        let total: u64 = t.view.seats.iter().flatten().map(|s| s.stack).sum::<u64>() + t.view.pot;
        assert_eq!(total, 1000 + 2000, "{:?}", t.view);
    }
}
