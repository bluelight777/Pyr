//! Poker as this Pyr sees it on a server: the lobby, the tables it's watching or sitting at
//! (with what just happened, for the table to animate), the clock, my record, and the check of
//! every shuffle (this Pyr adds a seed to each hand it's dealt into, and checks the dealer's
//! seed against its promise when the hand is over).

use std::collections::{BTreeMap, VecDeque};
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
    /// What bots said at the table lately: (seat, words, when).
    pub says: Vec<(usize, String, Instant)>,
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
    /// The MMR ladder, as last asked for.
    pub ratings: Option<Vec<pyr_poker::net::RatingRow>>,
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
    /// Blackjack tables (0.12.0 servers): the list, and the one I'm at or watching.
    pub bj_tables: Option<Vec<pyr_poker::net::BjSummary>>,
    pub bj: Option<BjLive>,
    /// Craps and baccarat tables (0.12.0 servers): the lists, and the one I'm at or watching.
    pub craps_tables: Option<Vec<pyr_poker::net::CrapsSummary>>,
    pub craps: Option<SharedLive<pyr_poker::net::CrapsView>>,
    pub bac_tables: Option<Vec<pyr_poker::net::BacSummary>>,
    pub bac: Option<SharedLive<pyr_poker::net::BacView>>,
    /// The Bonfire (0.12.0 servers): how it stands and when that came; each fire's fingerprint
    /// as first seen here; and a fire just lit (to show), as it checked out here.
    pub bonfire: Option<(pyr_poker::net::BonfireView, Instant)>,
    pub bonfire_commits: BTreeMap<u64, [u8; 32]>,
    pub bonfire_lit: Option<(pyr_poker::net::BonfireDraw, Instant, Result<(), String>)>,
    /// Emotes at blackjack, craps and baccarat tables lately: (game, table, seat, emote, when).
    pub game_emotes: Vec<(pyr_poker::net::TableGame, u64, u8, String, Instant)>,
    /// What's come for a table but is held back to be shown in its turn, with when: an all-in's
    /// board a street at a time (as on television), and everything after it (the pot, the next
    /// hand, a tournament's news) behind it.
    pub held: BTreeMap<TableId, VecDeque<(Instant, ToApp)>>,
}

/// The blackjack table I'm at or watching.
#[derive(Clone, Debug)]
pub struct BjLive {
    pub view: pyr_poker::net::BjView,
    /// When it came (its clock counts down from then).
    pub at: Instant,
    /// The house's fingerprint for each round, as shown before the round was dealt; my Pyr's
    /// seed for each round I bet in. (Checked against what's shown when the round's over.)
    pub commits: BTreeMap<u64, [u8; 32]>,
    pub my_seeds: Vec<(u64, [u8; 32])>,
    /// The last round checked, and how many have checked out here.
    pub checked_round: u64,
    pub fair: u64,
}

impl BjLive {
    fn new(view: pyr_poker::net::BjView, now: Instant) -> BjLive {
        BjLive { view, at: now, commits: BTreeMap::new(), my_seeds: Vec::new(), checked_round: 0, fair: 0 }
    }
}

/// A craps or baccarat table I'm at or watching: how it is, the house's fingerprint for each
/// roll (or coup) as shown before it, my Pyr's seed for each I bet on, and how the checks went.
#[derive(Clone, Debug)]
pub struct SharedLive<V> {
    pub view: V,
    /// When it came (its clock counts down from then).
    pub at: Instant,
    pub commits: BTreeMap<u64, [u8; 32]>,
    pub my_seeds: Vec<(u64, [u8; 32])>,
    /// The last roll (or coup) checked, how many have checked out, and the dice of the last few
    /// rolls (craps) seen here, newest first.
    pub checked: u64,
    pub fair: u64,
    pub rolls: Vec<(u8, u8)>,
}

impl<V> SharedLive<V> {
    fn new(view: V, now: Instant) -> SharedLive<V> {
        SharedLive { view, at: now, commits: BTreeMap::new(), my_seeds: Vec::new(), checked: 0, fair: 0, rolls: Vec::new() }
    }
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

    /// Takes in a message from the server (shown now, or held to be shown in its turn).
    pub fn apply(&mut self, msg: ToApp, now: Instant) -> Option<SendSeed> {
        self.pump(now);
        let msg = self.hold(msg, now)?;
        self.apply_now(msg, now)
    }

    /// When the next held message is due (to show it then).
    pub fn next_due(&self) -> Option<Instant> {
        self.held.values().filter_map(|q| q.front().map(|(at, _)| *at)).min()
    }

    /// Shows what's held and now due.
    pub fn pump(&mut self, now: Instant) {
        let mut due = Vec::new();
        for q in self.held.values_mut() {
            while q.front().is_some_and(|(at, _)| *at <= now) {
                due.extend(q.pop_front().map(|(_, m)| m));
            }
        }
        self.held.retain(|_, q| !q.is_empty());
        for m in due {
            let _ = self.apply_now(m, now);
        }
    }

    /// A table gone at once (whatever was still to be shown at it with it).
    pub fn forget_table(&mut self, table: TableId) {
        self.held.remove(&table);
        let _ = self.apply_now(ToApp::Closed { table }, Instant::now());
    }

    /// Holds a message back when it has to wait its turn: the board of a hand everyone's all in
    /// comes a street at a time (each card turned over a moment after the last), with the
    /// hands turned over first, and the pot pushed a moment after the river. Whatever comes
    /// for the table meanwhile waits behind it (so nothing gives away how it ends: the stacks,
    /// a bot's word, the tournament's news). The rest comes back to be shown now: emotes too
    /// (people watching the board come are seeing what everyone sees, and that's the moment
    /// for them).
    fn hold(&mut self, msg: ToApp, now: Instant) -> Option<ToApp> {
        use pyr_poker::room::{RUNOUT_FIRST_MS, RUNOUT_SHOW_MS, RUNOUT_STREET_MS, RUNOUT_WIN_MS};
        let ms = Duration::from_millis;
        let table = match &msg {
            ToApp::Event { table, .. }
            | ToApp::Clock { table, .. }
            | ToApp::Say { table, .. }
            | ToApp::PrizeWon { table, .. }
            | ToApp::Closed { table } => Some(*table),
            ToApp::Snapshot { table } => Some(table.id),
            ToApp::Tourney { view } => self.held.iter().find(|(id, q)| !q.is_empty() && self.tables.get(id).is_some_and(|t| t.view.tourney == Some(view.id))).map(|(id, _)| *id),
            _ => None,
        };
        let Some(id) = table else { return Some(msg) };
        let street = |m: &ToApp| matches!(m, ToApp::Event { event: Event::Street { .. }, .. });
        let shown = |m: &ToApp| matches!(m, ToApp::Event { event: Event::Shown { .. }, .. });
        if self.held.get(&id).is_none_or(|q| q.is_empty()) {
            // Only an all-in's street starts it: two or more still in, fewer than two able to bet.
            if !street(&msg) || !self.tables.get(&id).is_some_and(|t| runout(&t.view)) {
                return Some(msg);
            }
            self.held.entry(id).or_default().push_back((now + ms(RUNOUT_FIRST_MS), msg));
            return None;
        }
        let q = self.held.entry(id).or_default();
        let last = q.back().map(|(at, _)| *at).unwrap_or(now).max(now);
        let due = if street(&msg) {
            last + ms(RUNOUT_STREET_MS)
        } else if shown(&msg) && q.iter().all(|(_, m)| street(m) || shown(m)) {
            // Everyone's cards turn over before the board comes.
            let at = q.iter().position(|(_, m)| street(m)).unwrap_or(q.len());
            q.insert(at, (now + ms(RUNOUT_SHOW_MS), msg));
            return None;
        } else if matches!(msg, ToApp::Event { event: Event::Won { .. }, .. }) && q.back().is_some_and(|(_, m)| street(m)) {
            last + ms(RUNOUT_WIN_MS)
        } else {
            last
        };
        q.push_back((due, msg));
        // (Never more than a few hands' worth: with nobody looking, it's shown all at once.)
        if q.len() > 400 {
            let all: Vec<ToApp> = q.drain(..).map(|(_, m)| m).collect();
            self.held.remove(&id);
            for m in all {
                let _ = self.apply_now(m, now);
            }
        }
        None
    }

    fn apply_now(&mut self, msg: ToApp, now: Instant) -> Option<SendSeed> {
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
                        self.tables.insert(id, TableLive { view: table, events: Vec::new(), clock: None, emotes: Vec::new(), says: Vec::new(), checks: BTreeMap::new(), fair_total: 0, in_bank: false, clock_events: 0, ended_at: None });
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
            ToApp::Say { table, seat, text } => {
                if let Some(t) = self.tables.get_mut(&table) {
                    t.says.retain(|e| now.duration_since(e.2) < Duration::from_secs(10));
                    t.says.push((seat, text, now));
                }
            }
            ToApp::Record { me, gains } => {
                if self.casino.open {
                    self.casino.chips = me.chips;
                }
                self.me = Some(me);
                if let Some(g) = gains {
                    self.gains.push(g);
                }
            }
            ToApp::Leaderboard { rows } => self.leaderboard = Some(rows),
            ToApp::Ratings { rows } => self.ratings = Some(rows),
            ToApp::BjTables { tables } => self.bj_tables = Some(tables),
            ToApp::Bj { view } => self.bj_view(view, now),
            ToApp::CrapsTables { tables } => self.craps_tables = Some(tables),
            ToApp::Craps { view } => self.craps_view(view, now),
            ToApp::BacTables { tables } => self.bac_tables = Some(tables),
            ToApp::Bac { view } => self.bac_view(view, now),
            ToApp::Bonfire { view } => {
                self.bonfire_commits.entry(view.round).or_insert(view.commit);
                while self.bonfire_commits.len() > 20 {
                    let first = *self.bonfire_commits.keys().next().unwrap();
                    self.bonfire_commits.remove(&first);
                }
                self.bonfire = Some((view, now));
            }
            ToApp::GameEmote { game, table, seat, emote } => {
                self.game_emotes.retain(|e| now.duration_since(e.4) < Duration::from_secs(4));
                self.game_emotes.push((game, table, seat, emote, now));
            }
            ToApp::BonfireLit { draw } => {
                // Drawn again here: its seed against the fingerprint seen before it was lit.
                let checked = pyr_poker::casino::bonfire::check(&draw, self.bonfire_commits.get(&draw.round).copied());
                match &checked {
                    Ok(()) => self.casino.checked += 1,
                    Err(e) => self.casino.problems.push(e.clone()),
                }
                self.bonfire_lit = Some((draw, now, checked));
            }
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

    /// A blackjack table as it is now. A round just over is played again here from its seeds
    /// (the house's, shown only now, against the fingerprint it showed before the deal; and
    /// mine, which has to be among them), and counted in my statistics.
    fn bj_view(&mut self, view: pyr_poker::net::BjView, now: Instant) {
        use pyr_poker::net::BjStage;
        let b = match self.bj.as_mut() {
            Some(b) if b.view.id == view.id => {
                b.view = view;
                b.at = now;
                b
            }
            _ => self.bj.insert(BjLive::new(view, now)),
        };
        // (Before the deal, the fingerprint shown is the coming round's.)
        if matches!(b.view.stage, BjStage::Waiting | BjStage::Betting) {
            b.commits.entry(b.view.round_no).or_insert(b.view.commit);
        }
        let Some(last) = b.view.last.clone().filter(|l| l.round != b.checked_round) else { return };
        b.checked_round = last.round;
        let mine = b.my_seeds.iter().find(|(r, _)| *r == last.round).map(|(_, s)| *s);
        let result = pyr_poker::bjroom::checked(&last).and_then(|o| {
            if b.commits.get(&last.round).is_some_and(|c| *c != last.commit) {
                return Err(format!("round {}: the house showed a different fingerprint before the deal", last.round));
            }
            if let Some(seed) = mine
                && !last.seeds.iter().any(|(_, h)| *h == pyr_poker::bjroom::seed_hex(&seed))
            {
                return Err(format!("round {}: my Pyr's seed wasn't used", last.round));
            }
            Ok(o)
        });
        match result {
            Ok(o) => {
                b.fair += 1;
                self.casino.checked += 1;
                // My hands, in my statistics as the server counts them.
                if mine.is_some()
                    && let Some(seat) = b.view.my_seat
                    && let Some(me) = o.for_seat(seat)
                {
                    let o = pyr_poker::casino::Outcome::Blackjack(me);
                    pyr_poker::progress::count_casino(self.casino.stats.entry(o.stats_key()).or_default(), &o);
                }
            }
            Err(e) => self.casino.problems.push(format!("Blackjack table: {e}")),
        }
        b.commits.retain(|r, _| *r + 50 > last.round);
        b.my_seeds.retain(|(r, _)| *r + 50 > last.round);
    }

    /// A craps table as it is now. A roll just made is rolled again here from its seeds (the
    /// house's, shown only now, against the fingerprint it showed before; and mine, which has to
    /// be among them).
    fn craps_view(&mut self, view: pyr_poker::net::CrapsView, now: Instant) {
        let c = match self.craps.as_mut() {
            Some(c) if c.view.id == view.id => {
                c.view = view;
                c.at = now;
                c
            }
            _ => self.craps.insert(SharedLive::new(view, now)),
        };
        c.commits.entry(c.view.roll_no).or_insert(c.view.commit);
        let Some(last) = c.view.last.clone().filter(|l| l.roll != c.checked) else { return };
        c.checked = last.roll;
        c.rolls.insert(0, last.dice);
        c.rolls.truncate(24);
        let mine = c.my_seeds.iter().find(|(r, _)| *r == last.roll).map(|(_, s)| *s);
        let result = pyr_poker::crapsroom::check(&last).and_then(|()| {
            if c.commits.get(&last.roll).is_some_and(|x| *x != last.commit) {
                return Err(format!("roll {}: the house showed a different fingerprint before the dice flew", last.roll));
            }
            if let Some(seed) = mine
                && !last.seeds.iter().any(|(_, h)| *h == pyr_poker::crapsroom::seed_hex(&seed))
            {
                return Err(format!("roll {}: my Pyr's seed wasn't used", last.roll));
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                c.fair += 1;
                self.casino.checked += 1;
            }
            Err(e) => self.casino.problems.push(format!("Craps table: {e}")),
        }
        c.commits.retain(|r, _| *r + 50 > last.roll);
        c.my_seeds.retain(|(r, _)| *r + 50 > last.roll);
    }

    /// A baccarat table as it is now; a coup just dealt is dealt again here from its seeds.
    fn bac_view(&mut self, view: pyr_poker::net::BacView, now: Instant) {
        let b = match self.bac.as_mut() {
            Some(b) if b.view.id == view.id => {
                b.view = view;
                b.at = now;
                b
            }
            _ => self.bac.insert(SharedLive::new(view, now)),
        };
        b.commits.entry(b.view.coup_no).or_insert(b.view.commit);
        let Some(last) = b.view.last.clone().filter(|l| l.coup != b.checked) else { return };
        b.checked = last.coup;
        let mine = b.my_seeds.iter().find(|(r, _)| *r == last.coup).map(|(_, s)| *s);
        let result = pyr_poker::bacroom::check(&last).and_then(|()| {
            if b.commits.get(&last.coup).is_some_and(|x| *x != last.commit) {
                return Err(format!("coup {}: the house showed a different fingerprint before the deal", last.coup));
            }
            if let Some(seed) = mine
                && !last.seeds.iter().any(|(_, h)| *h == pyr_poker::bacroom::seed_hex(&seed))
            {
                return Err(format!("coup {}: my Pyr's seed wasn't used", last.coup));
            }
            Ok(())
        });
        match result {
            Ok(()) => {
                b.fair += 1;
                self.casino.checked += 1;
            }
            Err(e) => self.casino.problems.push(format!("Baccarat table: {e}")),
        }
        b.commits.retain(|r, _| *r + 50 > last.coup);
        b.my_seeds.retain(|(r, _)| *r + 50 > last.coup);
    }

    /// Forgets the tables (the connection dropped: the server stood us up).
    pub fn disconnected(&mut self) {
        self.tables.clear();
        self.held.clear();
        self.bj = None;
        self.craps = None;
        self.bac = None;
    }
}

/// Whether the betting's over with a hand still to finish: two or more still in it, and fewer
/// than two of them able to bet (the rest all in). The board then just comes.
fn runout(v: &TableView) -> bool {
    let live = v.seats.iter().flatten().filter(|s| s.in_hand && !s.folded);
    let (mut n, mut can) = (0, 0);
    for s in live {
        n += 1;
        if !s.all_in {
            can += 1;
        }
    }
    n >= 2 && can < 2
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
            // (Nothing to choose once a Flash Pot's gone off: everyone's all in.)
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
        let at = move |now: u64| t0 + Duration::from_millis(now);
        let snap = room.watch("alice");
        ps.apply(snap, t0);
        let deliver = |out: pyr_poker::room::Output, ps: &mut PokerState, room: &mut Room, now: u64| {
            for (to, m) in out.msgs {
                if to == Rcpt::All || to == Rcpt::User("alice".into()) {
                    if let Some(s) = ps.apply(m, at(now)) {
                        room.seed("alice", s.hand, s.seed);
                    }
                }
            }
        };
        let out = room.sit("alice", "Alice", 0, 1000, Look::default(), 0).unwrap();
        deliver(out, &mut ps, &mut room, 0);
        let out = room.add_bot(1, pyr_poker::bots::Level::Regular, 0, &mut rng).unwrap();
        deliver(out, &mut ps, &mut room, 0);
        let mut now = 0;
        let mut runouts = 0;
        while now < 300_000 {
            let out = room.tick(now, &mut rng);
            deliver(out, &mut ps, &mut room, now);
            ps.pump(at(now));
            // What she sees matches the table, all the way through (stacks, bets, the pot),
            // once an all-in's board has been shown.
            if !ps.held.is_empty() {
                runouts += 1;
                now += 250;
                continue;
            }
            let mine = &ps.table(1).unwrap().view;
            let real = room.view(Some("alice"));
            let stacks = |v: &pyr_poker::net::TableView| v.seats.iter().map(|s| s.as_ref().map(|s| (s.stack, s.bet))).collect::<Vec<_>>();
            assert_eq!(stacks(mine), stacks(&real), "hand {:?} at {now}", real.hand);
            if let Some(l) = ps.table(1).and_then(|t| t.view.legal) {
                let hand = ps.table(1).unwrap().view.hand.unwrap();
                let a = if l.can_check { pyr_poker::game::Action::Check } else { pyr_poker::game::Action::Call };
                if let Ok(out) = room.act("alice", hand, a, now) {
                    deliver(out, &mut ps, &mut room, now);
                }
            }
            now += 250;
        }
        eprintln!("{runouts} steps with an all-in's board still being shown");
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

    /// Everyone all in before the flop: their cards turn over, then the flop, the turn and the
    /// river come a moment apart, and only then the pot goes to whoever won it (with whatever
    /// came meanwhile, a bot's word say, after it).
    #[test]
    fn an_all_in_board_comes_a_street_at_a_time() {
        use pyr_poker::room::{RUNOUT_FIRST_MS, RUNOUT_SHOW_MS, RUNOUT_STREET_MS, RUNOUT_WIN_MS};
        let mut room = Room::new(1, "T", "alice", Rules::holdem(5, 10, 3), BuyIn::default()).unwrap();
        let mut rng = Stream::new(fair::seed_from(3));
        let mut ps = PokerState::default();
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        ps.apply(room.watch("alice"), t0);
        let feed = |out: pyr_poker::room::Output, ps: &mut PokerState| {
            for (to, m) in out.msgs {
                if to == Rcpt::All || to == Rcpt::User("alice".into()) {
                    ps.apply(m, t0);
                }
            }
        };
        feed(room.sit("alice", "Alice", 0, 1000, Look::default(), 0).unwrap(), &mut ps);
        feed(room.sit("bob", "Bob", 1, 1000, Look::default(), 0).unwrap(), &mut ps);
        let mut now = 0;
        while room.table.hand.as_ref().and_then(|h| h.to_act).is_none() {
            feed(room.tick(now, &mut rng), &mut ps);
            now += 100;
        }
        let hand = room.table.hand.as_ref().unwrap().number;
        let first = room.table.hand.as_ref().unwrap().to_act.unwrap();
        let uid = |s: usize| if s == 0 { "alice" } else { "bob" };
        feed(room.act(uid(first), hand, pyr_poker::game::Action::AllIn, now).unwrap(), &mut ps);
        let mut out = room.act(uid(1 - first), hand, pyr_poker::game::Action::Call, now).unwrap();
        out.msgs.push((Rcpt::All, ToApp::Say { table: 1, seat: 1, text: "Nice hand.".into() }));
        feed(out, &mut ps);
        let view = |ps: &PokerState| ps.table(1).unwrap().view.clone();
        let bob_shown = |ps: &PokerState| view(ps).seats[1].as_ref().is_some_and(|s| !s.cards.is_empty());
        // The call's in; nothing else yet.
        assert!(view(&ps).board.is_empty() && !bob_shown(&ps));
        assert!(ps.next_due().is_some());
        ps.pump(ms(RUNOUT_SHOW_MS));
        assert!(bob_shown(&ps), "the cards turn over first");
        assert!(view(&ps).board.is_empty());
        ps.pump(ms(RUNOUT_FIRST_MS));
        assert_eq!(view(&ps).board.len(), 3, "the flop");
        ps.pump(ms(RUNOUT_FIRST_MS + RUNOUT_STREET_MS - 1));
        assert_eq!(view(&ps).board.len(), 3);
        ps.pump(ms(RUNOUT_FIRST_MS + RUNOUT_STREET_MS));
        assert_eq!(view(&ps).board.len(), 4, "the turn");
        ps.pump(ms(RUNOUT_FIRST_MS + 2 * RUNOUT_STREET_MS));
        assert_eq!(view(&ps).board.len(), 5, "the river");
        assert!(view(&ps).pot > 0, "the pot's still in the middle");
        assert!(ps.table(1).unwrap().says.is_empty(), "nothing said before it's over");
        ps.pump(ms(RUNOUT_FIRST_MS + 2 * RUNOUT_STREET_MS + RUNOUT_WIN_MS));
        let t = ps.table(1).unwrap();
        assert!(t.ended_at.is_some() && t.view.pot == 0, "and then the pot's pushed");
        assert_eq!(t.says.len(), 1);
        assert!(ps.held.is_empty() && ps.next_due().is_none());
        // It all adds up as at the table (less the rake).
        let stacks = |v: &TableView| v.seats.iter().flatten().map(|s| s.stack).collect::<Vec<_>>();
        assert_eq!(stacks(&t.view), stacks(&room.view(Some("alice"))));
        assert_eq!(pyr_poker::room::runout_ms(3), RUNOUT_FIRST_MS + 2 * RUNOUT_STREET_MS + RUNOUT_WIN_MS);
    }
}
