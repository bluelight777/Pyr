//! Bots that fill seats. They play fair: a bot decides from [`Seen`], what a player in its seat
//! sees (its own cards, the board, the chips, everyone's actions, and what those actions have
//! shown of each player over past hands), and nothing else: there's no room in it for anyone
//! else's cards or the deck. Each level plays its own way:
//!
//! - **Rookie**: loose and passive, calls too much, rarely bluffs. Good to learn against.
//! - **Regular**: tight and aggressive, plays its strong hands hard and folds the rest.
//! - **Shark**: works out its chances against the hands others are likely to hold, thinks about
//!   the price of a call and where it sits, bets for value and bluffs at the right times.
//! - **Genius** (see `genius`): ranges by position, its chances against the hands each
//!   opponent's actions say they hold, bets sized for the spot and bluffs in balance, reads on
//!   each player from what they've done, push or fold when short.
//!
//! Their chances come from dealing out the rest of the hand many times (Monte Carlo), so they
//! play Omaha as well as Hold'em.
//!
//! How long a bot takes over a turn ([`think_ms`]) comes, as a person's does (0.13.26), from how
//! hard its choice was and what's riding on it, at its own pace: a long think can end in a fold
//! as well as a bet, rags go at once. And it bluffs as people do: the preflop raiser bets the
//! flop, draws are bet and raised, a missed draw is fired on the river heads up, the stronger
//! bots check-raise a bluff now and then (`measured/bots.md` in the rebuild's notes).
//!
//! Each bot is someone, too ([`persona`]): by its name, a way of playing (a maniac, a rock, a
//! trapper, a calling station...), a pace and a voice of its own. The level says how well it
//! plays that way.

mod genius;
pub mod persona;
pub mod talk;

pub use persona::Persona;

use serde::{Deserialize, Serialize};

use crate::cards::{Card, CardSet};
use crate::eval::{self, Value};
use crate::fair::Stream;
use crate::game::{Action, Legal, Limit, Played, Street, Table, Variant};

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Easy (Rookie until 0.13.7).
    Rookie,
    /// Medium (Regular).
    Regular,
    /// Hard (0.13.7), between Medium and Expert.
    Hard,
    /// Expert (Shark).
    Shark,
    /// Shark (Genius).
    Genius,
}

impl Level {
    pub const ALL: [Level; 5] = [Level::Rookie, Level::Regular, Level::Hard, Level::Shark, Level::Genius];

    /// Its wire name (the old names, kept: `rookie` is Easy, and so on), also what its rating's
    /// called (`bot:shark:Blaze` is an Expert Blaze, as 0.13.28 keeps them).
    pub fn wire(self) -> &'static str {
        match self {
            Level::Rookie => "rookie",
            Level::Regular => "regular",
            Level::Hard => "hard",
            Level::Shark => "shark",
            Level::Genius => "genius",
        }
    }

    /// Its name since 0.13.7.
    pub fn name(self) -> &'static str {
        match self {
            Level::Rookie => "Easy",
            Level::Regular => "Medium",
            Level::Hard => "Hard",
            Level::Shark => "Expert",
            Level::Genius => "Shark",
        }
    }

    /// The level shown on a bot's seat (people show theirs).
    pub fn look_level(self) -> u32 {
        match self {
            Level::Rookie => 3,
            Level::Regular => 12,
            Level::Hard => 20,
            Level::Shark => 30,
            Level::Genius => 50,
        }
    }

    /// What a 0.11 app can be told (it doesn't know Genius: a Shark, to it; nor Hard: a
    /// Regular).
    pub fn for_older_app(self) -> Level {
        match self {
            Level::Genius => Level::Shark,
            Level::Hard => Level::Regular,
            l => l,
        }
    }

    fn style(self) -> Style {
        match self {
            // tight: the chances it wants before playing on; aggression: how often it bets or
            // raises with a good hand rather than calling; bluff: how often it bets with nothing;
            // noise: how far off its sense of its chances is.
            Level::Rookie => Style { tight: 0.75, aggression: 0.25, bluff: 0.03, noise: 0.15, sims: 150 },
            Level::Regular => Style { tight: 1.0, aggression: 0.6, bluff: 0.08, noise: 0.07, sims: 300 },
            Level::Hard => Style { tight: 1.03, aggression: 0.68, bluff: 0.12, noise: 0.05, sims: 450 },
            Level::Shark | Level::Genius => Style { tight: 1.05, aggression: 0.75, bluff: 0.16, noise: 0.03, sims: 600 },
        }
    }
}

struct Style {
    tight: f64,
    aggression: f64,
    bluff: f64,
    noise: f64,
    sims: usize,
}

/// Someone in the hand, as everyone at the table sees them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Public {
    pub seat: usize,
    /// Chips they have behind (not yet put in).
    pub behind: u64,
    /// Chips in front of them this round, and all they've put in this hand.
    pub street_bet: u64,
    pub total: u64,
    pub folded: bool,
    pub all_in: bool,
}

/// The table at someone's turn as everyone sees it: no cards but the board's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    /// Whose turn it is.
    pub seat: usize,
    pub variant: Variant,
    pub limit: Limit,
    pub big_blind: u64,
    /// Seats round the table (taken or not).
    pub table_seats: usize,
    pub street: Street,
    pub board: Vec<Card>,
    pub legal: Legal,
    pub button: usize,
    /// Everyone dealt into the hand, in the order they were dealt (the small blind first).
    pub players: Vec<Public>,
    /// Every action this hand, in order.
    pub actions: Vec<Played>,
}

/// What one seat sees at its turn: the spot, its own cards, and what it's learned of the
/// others from their actions in past hands. (Nothing here can hold anyone else's cards or the
/// deck: a bot deciding from this can't cheat.)
#[derive(Clone, Debug, PartialEq)]
pub struct Seen {
    pub spot: Spot,
    pub hole: Vec<Card>,
    /// By seat.
    pub reads: Vec<(usize, Reads)>,
}

/// What a player's actions have shown over the hands seen at a table: everyone there could
/// count the same (it's all public).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reads {
    pub hands: u32,
    /// Put chips in before the flop by choice (called or raised), and raised before the flop.
    pub vpip: u32,
    pub pfr: u32,
    /// After the flop: bets and raises, and calls.
    pub aggressive: u32,
    pub passive: u32,
}

impl Reads {
    /// Hands it takes before a read is trusted.
    const ENOUGH: u32 = 12;

    pub fn vpip(&self) -> Option<f64> {
        (self.hands >= Self::ENOUGH).then(|| self.vpip as f64 / self.hands as f64)
    }

    pub fn pfr(&self) -> Option<f64> {
        (self.hands >= Self::ENOUGH).then(|| self.pfr as f64 / self.hands as f64)
    }

    /// The share of their actions after the flop that bet or raise.
    pub fn aggression(&self) -> Option<f64> {
        let n = self.aggressive + self.passive;
        (n >= 8).then(|| self.aggressive as f64 / n as f64)
    }

    /// Counts one hand's actions for each of `seats` (everyone dealt in).
    pub fn count(&mut self, seat: usize, actions: &[Played]) {
        self.hands += 1;
        let mine = actions.iter().filter(|a| a.seat == seat);
        let (mut vpip, mut pfr) = (false, false);
        for a in mine {
            match (a.street, a.action) {
                (Street::Preflop, Action::Call) => vpip = true,
                (Street::Preflop, Action::Raise { .. }) => {
                    vpip = true;
                    pfr = true;
                }
                (Street::Preflop, _) => {}
                (_, Action::Raise { .. }) => self.aggressive += 1,
                (_, Action::Call) => self.passive += 1,
                _ => {}
            }
        }
        self.vpip += vpip as u32;
        self.pfr += pfr as u32;
    }
}

impl Spot {
    pub fn me(&self) -> Public {
        self.players.iter().find(|p| p.seat == self.seat).copied().unwrap_or(Public { seat: self.seat, behind: 0, street_bet: 0, total: 0, folded: true, all_in: false })
    }

    /// All the chips put in this hand.
    pub fn pot(&self) -> u64 {
        self.players.iter().map(|p| p.total).sum()
    }

    /// Others still in the hand.
    pub fn opponents(&self) -> impl Iterator<Item = &Public> {
        self.players.iter().filter(move |p| !p.folded && p.seat != self.seat)
    }

    /// Whether it acts last after the flop (the live seat closest before the button).
    pub fn in_position(&self) -> bool {
        let n = self.table_seats.max(1);
        let live: Vec<usize> = self.players.iter().filter(|p| !p.folded).map(|p| p.seat).collect();
        let last = (0..n).map(|d| (self.button + n - d) % n).find(|s| live.contains(s));
        last == Some(self.seat)
    }

    /// Whether anyone's bet or raised this hand (the blinds aside).
    pub fn any_raise(&self) -> bool {
        self.actions.iter().any(|a| matches!(a.action, Action::Raise { .. }))
    }

    /// The raises before the flop, in order.
    pub fn preflop_raises(&self) -> impl Iterator<Item = &Played> {
        self.actions.iter().filter(|a| a.street == Street::Preflop && matches!(a.action, Action::Raise { .. }))
    }
}

impl Table {
    /// The spot at `seat`'s turn, as everyone sees it.
    pub fn spot(&self, seat: usize) -> Option<Spot> {
        let hs = self.hand.as_ref()?;
        let legal = self.legal(seat)?;
        Some(Spot {
            seat,
            variant: self.rules.variant,
            limit: self.rules.limit,
            big_blind: self.rules.big_blind,
            table_seats: self.seats.len(),
            street: hs.street,
            board: hs.board.clone(),
            legal,
            button: hs.button,
            players: hs.players.iter().map(|p| Public { seat: p.seat, behind: self.behind(p.seat), street_bet: p.street_bet, total: p.total, folded: p.folded, all_in: p.all_in }).collect(),
            actions: hs.actions.clone(),
        })
    }

    /// What the player in `seat` sees at their turn (no reads: the room adds what it's seen).
    pub fn seen(&self, seat: usize) -> Option<Seen> {
        let spot = self.spot(seat)?;
        let hole = self.hand.as_ref()?.player(seat)?.hole.clone();
        Some(Seen { spot, hole, reads: Vec::new() })
    }
}

/// What a bot sees when it's its turn, boiled down (how Rookies, Regulars and Sharks think).
#[derive(Clone, Debug)]
struct View {
    variant: Variant,
    hole: Vec<Card>,
    board: Vec<Card>,
    street: Street,
    legal: Legal,
    /// All chips in the middle (including this round's).
    pot: u64,
    /// Others still in the hand.
    opponents: usize,
    /// Chips in front of it this round.
    in_front: u64,
    /// Whether it acts last this round (on the button, or the last one left to act).
    in_position: bool,
    /// Whether someone's raised (others' hands are stronger then).
    raised: bool,
    /// Whether it made the last raise before the flop (the one others expect to bet the flop).
    aggressor: bool,
    /// Whether it checked earlier this round (a raise now is a check-raise).
    checked: bool,
    /// A draw now (a flush draw or an open-ended straight draw, with cards to come), and, on the
    /// river, whether it had one on the turn (one that missed).
    draw: bool,
    drew: bool,
}

impl Seen {
    fn view(&self) -> View {
        let sp = &self.spot;
        View {
            variant: sp.variant,
            hole: self.hole.clone(),
            board: sp.board.clone(),
            street: sp.street,
            legal: sp.legal,
            pot: sp.pot(),
            opponents: sp.opponents().count(),
            in_front: sp.me().street_bet,
            in_position: sp.in_position(),
            raised: sp.any_raise(),
            aggressor: sp.preflop_raises().last().is_some_and(|a| a.seat == sp.seat),
            checked: sp.actions.iter().any(|a| a.street == sp.street && a.seat == sp.seat && a.action == Action::Check),
            draw: sp.street < Street::River && sp.street > Street::Preflop && drawing(sp.variant, &self.hole, &sp.board),
            drew: sp.street == Street::River && sp.board.len() == 5 && drawing(sp.variant, &self.hole, &sp.board[..4]),
        }
    }

    /// What it's learned of the player in `seat`.
    pub fn reads_of(&self, seat: usize) -> Option<&Reads> {
        self.reads.iter().find(|(s, _)| *s == seat).map(|(_, r)| r)
    }
}

/// The chance of winning (ties count by share) against `opponents` random hands, dealing the
/// rest of the board out `sims` times.
pub fn equity(variant: Variant, hole: &[Card], board: &[Card], opponents: usize, sims: usize, rng: &mut Stream) -> f64 {
    if opponents == 0 {
        return 1.0;
    }
    let used = CardSet::of(&[hole, board].concat());
    let rest: Vec<Card> = Card::all().filter(|c| !used.contains(*c)).collect();
    let k = hole.len();
    let need = opponents * k + (5 - board.len());
    if need > rest.len() {
        return 1.0 / (opponents + 1) as f64;
    }
    let mut score = 0.0;
    let mut deck = rest.clone();
    for _ in 0..sims {
        // A partial shuffle: just the cards needed.
        for i in 0..need {
            let j = i + rng.below((deck.len() - i) as u32) as usize;
            deck.swap(i, j);
        }
        let mut b = board.to_vec();
        b.extend(&deck[opponents * k..need]);
        let val = |h: &[Card]| -> Value { hand_value(variant, h, &b) };
        let mine = val(hole);
        let mut best_other = Value(0);
        let mut ties = 0;
        for o in 0..opponents {
            let v = val(&deck[o * k..o * k + k]);
            if v > best_other {
                best_other = v;
                ties = 0;
            }
            if v == mine {
                ties += 1;
            }
        }
        if mine > best_other {
            score += 1.0;
        } else if mine == best_other {
            score += 1.0 / (ties + 1) as f64;
        }
    }
    score / sims as f64
}

/// A hand's worth with a board (five or more cards between them).
pub(crate) fn hand_value(variant: Variant, hole: &[Card], board: &[Card]) -> Value {
    match variant {
        Variant::Holdem => eval::evaluate(&[hole, board].concat()),
        Variant::Omaha => eval::omaha_value(hole, board),
    }
}

/// A quick sense of a Hold'em starting hand (the Chen formula), 0 to 1.
pub fn preflop_strength(hole: &[Card]) -> f64 {
    if hole.len() != 2 {
        return 0.5;
    }
    let (a, b) = (hole[0], hole[1]);
    let (hi, lo) = if a.rank() >= b.rank() { (a, b) } else { (b, a) };
    let base = |r: u8| -> f64 {
        match r {
            12 => 10.0,
            11 => 8.0,
            10 => 7.0,
            9 => 6.0,
            r => (r as f64 + 2.0) / 2.0,
        }
    };
    let mut s = base(hi.rank());
    if hi.rank() == lo.rank() {
        s = (s * 2.0).max(5.0);
    } else {
        if hi.suit() == lo.suit() {
            s += 2.0;
        }
        let gap = hi.rank() - lo.rank() - 1;
        s -= match gap {
            0 => 0.0,
            1 => 1.0,
            2 => 2.0,
            3 => 4.0,
            _ => 5.0,
        };
        if gap <= 1 && hi.rank() < 10 {
            s += 1.0;
        }
    }
    // Chen scores run from about -1 (72o) to 20 (AA).
    ((s + 1.0) / 21.0).clamp(0.0, 1.0)
}

/// A draw: four cards of a suit (one of them its own) or four in a row open at both ends (one of
/// them its own), without a straight or better made already. In Omaha, two of its own with
/// the board, as a hand there must be.
fn drawing(variant: Variant, hole: &[Card], board: &[Card]) -> bool {
    if board.len() < 3 {
        return false;
    }
    let pairs: Vec<Vec<Card>> = match variant {
        Variant::Holdem => vec![hole.to_vec()],
        Variant::Omaha => {
            let mut v = Vec::new();
            for i in 0..hole.len() {
                for j in i + 1..hole.len() {
                    v.push(vec![hole[i], hole[j]]);
                }
            }
            v
        }
    };
    pairs.iter().any(|mine| {
        let all: Vec<Card> = mine.iter().chain(board).copied().collect();
        if eval::evaluate(&all).category() >= eval::Category::Straight {
            return false;
        }
        let flush = (0..4u8).any(|s| all.iter().filter(|c| c.suit() == s).count() == 4 && mine.iter().any(|c| c.suit() == s));
        // Ranks 0 (a deuce) to 12 (an ace), the ace low too (-1).
        let has = |r: i32, cards: &[Card]| cards.iter().any(|c| c.rank() as i32 == r || (r == -1 && c.rank() == 12));
        let open = (0..=8).any(|lo: i32| (lo..lo + 4).all(|r| has(r, &all)) && (lo..lo + 4).any(|r| has(r, mine)) && lo >= 0 && lo + 4 <= 12);
        flush || open
    })
}

/// What a bot of `level` does (one that's no one in particular: see [`decide_as`]).
pub fn decide(seen: &Seen, level: Level, rng: &mut Stream) -> Action {
    decide_as(seen, level, &persona::PLAIN, rng)
}

/// What `who` does, playing at `level`: its own way, as well as its level plays. (A Genius
/// keeps its personality toned down: its style shows, but it plays to win.)
pub fn decide_as(seen: &Seen, level: Level, who: &Persona, rng: &mut Stream) -> Action {
    let who = who.in_the_mood(&seen.spot);
    match level {
        Level::Genius => genius::decide(seen, &who.toned(0.5), rng),
        _ => decide_simply(&seen.view(), level, &who, rng),
    }
}

/// How Rookies, Regulars and Sharks decide.
fn decide_simply(view: &View, level: Level, who: &Persona, rng: &mut Stream) -> Action {
    let mut st = level.style();
    // Its own way: looser or tighter, harder or softer, bluffing more or less.
    st.tight /= who.loose.max(0.3);
    st.aggression = (st.aggression * who.aggression).clamp(0.05, 0.97);
    st.bluff = (st.bluff * who.bluff).clamp(0.0, 0.6);
    let l = view.legal;
    let roll = |rng: &mut Stream| rng.below(10_000) as f64 / 10_000.0;
    // Its chances, a little off for the weaker bots, and lower when others showed strength.
    let mut eq = if view.street == Street::Preflop && view.variant == Variant::Holdem {
        // Against several players the best hands still win less often (a loose player thinks
        // less of that than a tight one).
        let s = preflop_strength(&view.hole);
        s.powf((1.0 + 0.35 * view.opponents.saturating_sub(1) as f64) / who.loose.max(0.3))
    } else {
        equity(view.variant, &view.hole, &view.board, view.opponents.max(1), st.sims, rng)
    };
    if view.raised && view.street != Street::Preflop {
        eq *= 0.92;
    }
    eq = (eq + (roll(rng) - 0.5) * 2.0 * st.noise).clamp(0.0, 1.0);
    // The price of a call.
    let call = l.call as f64;
    let pot_odds = if call > 0.0 { call / (view.pot as f64 + call) } else { 0.0 };
    // What a fair share of the pot would be.
    let fair = 1.0 / (view.opponents + 1) as f64;
    let strong = eq > fair * 1.9 * st.tight || eq > 0.72;
    let good = eq > fair * 1.25 * st.tight;
    let position_bonus = if view.in_position { 0.04 } else { 0.0 };

    let raise_to = |frac: f64, rng: &mut Stream| -> u64 {
        // Call, then bet `frac` of the pot that makes (a little more or less, so it doesn't
        // give its hand away), between what's allowed.
        let pot_after = view.pot as f64 + call;
        let wobble = 0.85 + roll(rng) * 0.3;
        let target = view.in_front as f64 + call + pot_after * frac * wobble;
        (target as u64).clamp(l.min_to, l.max_to)
    };

    if strong && l.can_raise && roll(rng) < st.aggression + position_bonus {
        // (A trapper lets a big hand lie low now and then, before the river.)
        if who.trap > 0.0 && view.street != Street::River && roll(rng) < who.trap {
            return if l.can_check { Action::Check } else { Action::Call };
        }
        // Value: bigger when very strong; all-in with the nuts-ish when short.
        let frac = if eq > 0.85 { 1.0 } else { 0.66 } * who.sizing;
        let to = raise_to(frac, rng);
        if to >= l.max_to {
            return Action::AllIn;
        }
        return Action::Raise { to };
    }
    // How it bluffs, the way people do (0.13.26): a maniac far more than a rock, a rookie hardly
    // ever, the better bots a little more.
    let bluffing = who.bluff
        * match level {
            Level::Rookie => 0.05,
            Level::Regular => 1.0,
            Level::Hard => 1.15,
            Level::Shark | Level::Genius => 1.3,
        };
    if l.can_check {
        if l.can_raise {
            // The one who raised before the flop bets the flop whatever it hit (the better the
            // bot, the more often; less into several players).
            let cbet = match level {
                Level::Rookie => 0.35,
                Level::Regular => 0.6,
                Level::Hard => 0.68,
                Level::Shark | Level::Genius => 0.75,
            } * who.aggression.sqrt()
                * 0.8f64.powi(view.opponents.saturating_sub(1) as i32);
            if view.street == Street::Flop && view.aggressor && roll(rng) < cbet.min(0.95) {
                return Action::Raise { to: raise_to(0.5 * who.sizing, rng) };
            }
            // A draw is bet as a semi-bluff.
            if view.draw && roll(rng) < (st.aggression * 0.6 * bluffing.sqrt()).min(0.9) {
                return Action::Raise { to: raise_to(0.6 * who.sizing, rng) };
            }
            // A draw that missed gets fired on the river, heads up.
            if view.drew && view.street == Street::River && view.opponents == 1 && !good && roll(rng) < (0.35 * bluffing).min(0.9) {
                return Action::Raise { to: raise_to(0.66 * who.sizing, rng) };
            }
        }
        // Nobody bet: bet good hands sometimes, bluff now and then (more in position; a rookie
        // hardly ever).
        let light = (st.bluff + position_bonus) * if level == Level::Rookie { 0.4 } else { 1.0 };
        if l.can_raise && (good && roll(rng) < st.aggression * 0.7 || roll(rng) < light) {
            let to = raise_to(0.5 * who.sizing, rng);
            return Action::Raise { to };
        }
        return Action::Check;
    }
    // Facing a bet: a draw raised as a semi-bluff now and then; the stronger bots check-raise a
    // bluff now and then.
    if l.can_raise && view.street >= Street::Flop {
        if view.draw && eq + position_bonus > pot_odds * 0.8 && roll(rng) < (0.22 * bluffing).min(0.8) {
            return Action::Raise { to: raise_to(0.75 * who.sizing, rng) };
        }
        if view.checked && matches!(level, Level::Hard | Level::Shark) && !good && roll(rng) < (st.bluff * 0.6 * who.bluff).min(0.5) {
            return Action::Raise { to: raise_to(0.8 * who.sizing, rng) };
        }
    }
    // Facing a bet: call when the chances beat the price (rookies call more, and so do the
    // loose and the sticky).
    let margin = match level {
        Level::Rookie => -0.08,
        Level::Regular => 0.02,
        Level::Hard => 0.01,
        Level::Shark | Level::Genius => 0.0,
    } - (who.loose - 1.0) * 0.05
        - who.sticky;
    if eq + position_bonus > pot_odds + margin || strong {
        // A bluff-raise now and then, from the shark (and the wilder ones).
        if (level == Level::Shark || who.bluff >= 1.5) && l.can_raise && view.street >= Street::Flop && roll(rng) < st.bluff * 0.4 {
            return Action::Raise { to: raise_to(0.75 * who.sizing, rng) };
        }
        return Action::Call;
    }
    // A cheap call with something to draw to (gamblers find more of them cheap).
    if pot_odds < 0.15 + (who.loose - 1.0).max(0.0) * 0.1 && eq > fair * 0.6 {
        return Action::Call;
    }
    Action::Fold
}

/// The longest a bot takes over a turn: well inside the clock everyone has.
pub const MAX_THINK_MS: u64 = 13_000;
/// And the quickest.
pub const MIN_THINK_MS: u64 = 450;

/// How long a bot takes over `action` (its choice, from what it sees), in milliseconds, the way a
/// person would (0.13.26): from how hard the choice was and what's riding on it, not from what it
/// does, so a long think can end in a fold or a check as well as a bet. Rags are thrown away at
/// once; a close call over a big river bet takes a while whichever way it goes; a trapper now and
/// then acts out a long think over a monster; a new card on the table gets a look first. Each
/// bot at its own pace, snapping off some choices and going into the tank over others as its
/// personality goes, never the same twice (spread like people's times).
pub fn think_ms(seen: &Seen, level: Level, who: &Persona, action: Action, rng: &mut Stream) -> u64 {
    let spot = &seen.spot;
    let roll = |rng: &mut Stream| (rng.below(1_000_000) as f64 + 0.5) / 1_000_000.0;
    let me = spot.me();
    let pot = spot.pot().max(1) as f64;
    let call = spot.legal.call as f64;
    let stack = (me.behind + me.street_bet).max(1) as f64;
    // What's riding on it: the price against the pot, how much of what's left goes in, the street,
    // the crowd.
    let price = (call / pot).min(2.0);
    let commit = (call / me.behind.max(1) as f64).min(1.0);
    let put = match action {
        Action::Raise { to } => (to.saturating_sub(me.street_bet) as f64 / stack).min(1.0),
        Action::AllIn => 1.0,
        Action::Call => commit,
        _ => 0.0,
    };
    // (A bet's size against the pot: how much to put out is a choice too.)
    let size = match action {
        Action::Raise { to } => (to.saturating_sub(me.street_bet) as f64 / pot).min(2.0),
        Action::AllIn => 2.0,
        _ => 0.0,
    };
    let late = match spot.street {
        Street::Preflop => 0.0,
        Street::Flop => 0.2,
        Street::Turn => 0.35,
        Street::River => 0.5,
    };
    let opponents = spot.opponents().count().max(1);
    let crowd = opponents.saturating_sub(1) as f64 * 0.05;
    let riding = (price * 0.4 + commit.max(put) * 0.8 + size * 0.25 + late + crowd).min(2.0) * if spot.street == Street::Preflop { 0.6 } else { 1.0 };
    // How hard it was: how near its hand sits to the line between one move and the next (the
    // price of a call, or a hand worth a bet), and rags not at all.
    let fair = 1.0 / (opponents + 1) as f64;
    let preflop = spot.street == Street::Preflop && spot.variant == Variant::Holdem;
    let eq = if preflop { preflop_strength(&seen.hole) } else { equity(spot.variant, &seen.hole, &spot.board, opponents, 120, rng) };
    let line = if call > 0.0 { call / (pot + call) } else { fair * 1.3 };
    let hard = (-((eq - line) / 0.12).powi(2)).exp();
    if action == Action::Fold && preflop && eq < 0.3 {
        return MIN_THINK_MS;
    }
    let level_k = match level {
        Level::Rookie => 0.82,
        Level::Regular => 0.95,
        Level::Hard => 1.0,
        Level::Shark => 1.05,
        Level::Genius => 1.12,
    };
    // Spread like people's (log-normal).
    let (u1, u2) = (roll(rng), roll(rng));
    let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
    // (Hands move at their own speed: a fold or a check a moment, a call a little more, a bet's
    // size a choice of its own.)
    let base = match action {
        Action::Fold => 520.0,
        Action::Check => 660.0,
        Action::Call => 1_250.0,
        Action::Raise { .. } | Action::AllIn => 1_500.0,
    };
    let mut ms = base * level_k * who.pace * (0.5 + 1.6 * hard * (0.4 + riding) + 0.35 * riding) * (0.38 * z).exp();
    // A new card on the table: a look at it first.
    if spot.street != Street::Preflop && !spot.actions.iter().any(|a| a.street == spot.street) {
        ms += 350.0 + roll(rng) * 750.0;
    }
    // A trapper acting out a long think over a monster it means to slow-play.
    if who.trap > 0.0 && eq > 0.85 && matches!(action, Action::Check | Action::Call) && roll(rng) < who.trap * 0.5 {
        ms += 2_500.0 + roll(rng) * 4_000.0;
    }
    // Now and then a snap (the easier the choice, the likelier), now and then a long think (the
    // harder, and the more riding on it, the likelier), as its personality goes.
    let r = roll(rng);
    if r < who.snaps * (1.0 - hard) * 1.5 {
        ms = 450.0 + roll(rng) * 500.0;
    } else if r > 1.0 - who.tanks * (0.08 + 2.0 * hard * (0.15 + riding)) * if action == Action::Fold { 2.1 } else { 1.0 } {
        ms += 2_500.0 + roll(rng) * 6_000.0 * (0.4 + riding.min(1.8));
    }
    (ms as u64).clamp(MIN_THINK_MS, MAX_THINK_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::parse_cards;
    use crate::fair::{seed_from, shuffle};
    use crate::game::{Event, Rules};

    #[test]
    fn equity_is_about_right() {
        let mut rng = Stream::new(seed_from(1));
        let aa = parse_cards("AhAd").unwrap();
        let e = equity(Variant::Holdem, &aa, &[], 1, 4000, &mut rng);
        assert!((0.82..0.88).contains(&e), "aces against one random hand: {e}");
        let e = equity(Variant::Holdem, &parse_cards("7c2d").unwrap(), &[], 1, 4000, &mut rng);
        assert!((0.30..0.38).contains(&e), "seven-two: {e}");
        let nuts = equity(Variant::Holdem, &parse_cards("AhKh").unwrap(), &parse_cards("QhJhTh2c3d").unwrap(), 3, 300, &mut rng);
        assert_eq!(nuts, 1.0);
    }

    #[test]
    fn chen_orders_hands() {
        let s = |h: &str| preflop_strength(&parse_cards(h).unwrap());
        assert!(s("AhAd") > s("KhKd"));
        assert!(s("KhKd") > s("AhKh"));
        assert!(s("AhKh") > s("AhKd"));
        assert!(s("AhKd") > s("9h8h"));
        assert!(s("9h8h") > s("7c2d"));
    }

    /// Plays a hand at `t` to the end with bots at `levels` (by seat), every action checked
    /// legal; `look` sees each bot's [`Seen`] first.
    fn play_hand(t: &mut Table, hand: u64, deck: Vec<Card>, levels: &[Level], rng: &mut Stream, mut look: impl FnMut(&Table, &Seen)) {
        let mut out = t.start_hand(hand, deck).unwrap();
        loop {
            let next = out.iter().rev().find_map(|(_, e)| match e {
                Event::Turn { seat, .. } => Some(Some(*seat)),
                Event::HandEnded { .. } => Some(None),
                _ => None,
            });
            let Some(Some(seat)) = next else { break };
            let seen = t.seen(seat).unwrap();
            look(t, &seen);
            let a = decide(&seen, levels[seat], rng);
            out = t.act(seat, a).unwrap_or_else(|e| panic!("{a:?} with {:?}: {e}", seen.spot.legal));
        }
    }

    /// Bots play many hands against each other: everything they do is legal, and the better
    /// bots win chips from the weaker over time.
    #[test]
    fn sharks_beat_rookies() {
        let mut rng = Stream::new(seed_from(9));
        let mut shark_total: i64 = 0;
        let levels = [Level::Shark, Level::Rookie, Level::Rookie, Level::Regular];
        let mut by_level = [0i64; 4];
        for game in 0..40u64 {
            let mut t = Table::new(Rules::holdem(5, 10, 4));
            for i in 0..4 {
                t.sit(i, &format!("bot{i}"), "x", 1000).unwrap();
            }
            for hand in 0..60u64 {
                if t.ready().len() < 2 {
                    break;
                }
                play_hand(&mut t, hand, shuffle(seed_from(game * 997 + hand)), &levels, &mut rng, |_, _| {});
            }
            for (i, s) in t.seats.iter().enumerate() {
                let won = s.as_ref().map(|s| s.stack as i64).unwrap_or(0) - 1000;
                by_level[levels[i] as usize] += won;
                if levels[i] == Level::Shark {
                    shark_total += won;
                }
            }
        }
        assert!(shark_total > 0, "the shark should win over 40 games: {shark_total} ({by_level:?})");
        assert!(by_level[Level::Rookie as usize] < 0, "rookies lose: {by_level:?}");
    }

    /// The proof bots can't cheat: at every turn of many hands, every level's decision is
    /// played again with everyone else's cards and the rest of the deck dealt differently, and
    /// comes out exactly the same (same random numbers, same choice). What a bot sees has no
    /// other player's cards in it, and nothing it decides depends on them.
    #[test]
    fn bots_decide_the_same_whatever_others_hold() {
        let mut rng = Stream::new(seed_from(77));
        let mut checked = 0;
        for game in 0..12u64 {
            let mut t = Table::new(Rules::holdem(5, 10, 5));
            for i in 0..5 {
                t.sit(i, &format!("bot{i}"), "x", 1000).unwrap();
            }
            let levels = [Level::Genius, Level::Shark, Level::Regular, Level::Rookie, Level::Genius];
            for hand in 0..8u64 {
                if t.ready().len() < 2 {
                    break;
                }
                play_hand(&mut t, hand, shuffle(seed_from(game * 31 + hand)), &levels, &mut rng, |t, seen| {
                    // The same table with the hidden cards dealt again: others' hole cards and
                    // the deck still to come, shuffled among themselves.
                    let mut other = t.clone();
                    let hs = other.hand.as_mut().unwrap();
                    let mine = hs.player(seen.spot.seat).unwrap().hole.clone();
                    let shown = CardSet::of(&[mine.as_slice(), hs.board.as_slice()].concat());
                    let mut hidden: Vec<Card> = Card::all().filter(|c| !shown.contains(*c)).collect();
                    let mut mix = Stream::new(seed_from(game * 1_000 + hand * 10 + seen.spot.actions.len() as u64));
                    for i in (1..hidden.len()).rev() {
                        let j = mix.below(i as u32 + 1) as usize;
                        hidden.swap(i, j);
                    }
                    let k = mine.len();
                    let mut it = hidden.into_iter();
                    for p in hs.players.iter_mut().filter(|p| p.seat != seen.spot.seat) {
                        p.hole = (0..k).map(|_| it.next().unwrap()).collect();
                    }
                    let again = other.seen(seen.spot.seat).unwrap();
                    assert_eq!(&again, seen, "what a seat sees mustn't change with others' cards");
                    for level in Level::ALL {
                        for who in persona::ALL.iter().chain([&persona::PLAIN]) {
                            let a = decide_as(seen, level, who, &mut Stream::new(seed_from(checked)));
                            let b = decide_as(&again, level, who, &mut Stream::new(seed_from(checked)));
                            assert_eq!(a, b, "{level:?} {} decided differently when others held other cards", who.name);
                        }
                    }
                    checked += 1;
                });
            }
        }
        assert!(checked > 100, "{checked} decisions checked");
    }

    /// Each personality plays its own way: over the same deals, a maniac puts chips in with far
    /// more hands than a nit and raises far more, and a calling station calls far more than a
    /// rock, at every level.
    #[test]
    fn personalities_play_their_own_way() {
        let tally = |name: &str, level: Level| -> (f64, f64, f64) {
            let who = persona::of(name);
            let mut rng = Stream::new(seed_from(1234));
            let (mut played, mut raised, mut called, mut spots, mut facing) = (0, 0, 0, 0, 0);
            for deal in 0..600u64 {
                let mut t = Table::new(Rules::holdem(5, 10, 6));
                for i in 0..6 {
                    t.sit(i, &format!("bot{i}"), "x", 1000).unwrap();
                }
                let out = t.start_hand(1, shuffle(seed_from(deal))).unwrap();
                let Some(seat) = out.iter().rev().find_map(|(_, e)| if let Event::Turn { seat, .. } = e { Some(*seat) } else { None }) else { continue };
                // First to act before the flop: does it play?
                let seen = t.seen(seat).unwrap();
                let a = decide_as(&seen, level, who, &mut rng);
                spots += 1;
                if !matches!(a, Action::Fold | Action::Check) {
                    played += 1;
                }
                if matches!(a, Action::Raise { .. } | Action::AllIn) {
                    raised += 1;
                }
                // Then facing a pot-sized raise from the next seat: does it call?
                let mut t2 = t.clone();
                let opener = seat;
                if t2.act(opener, Action::Raise { to: 40 }).is_ok()
                    && let Some(next) = t2.hand.as_ref().and_then(|h| h.to_act)
                    && let Some(seen2) = t2.seen(next)
                {
                    facing += 1;
                    if !matches!(decide_as(&seen2, level, who, &mut rng), Action::Fold) {
                        called += 1;
                    }
                }
            }
            (played as f64 / spots as f64, raised as f64 / spots as f64, called as f64 / facing.max(1) as f64)
        };
        for level in [Level::Rookie, Level::Regular, Level::Shark, Level::Genius] {
            let (maniac, nit) = (tally("Blaze", level), tally("Dusk", level));
            // (A Genius keeps its style toned down: still clearly looser, by less.)
            let by = if level == Level::Genius { 0.05 } else { 0.12 };
            assert!(maniac.0 > nit.0 * 1.5 && maniac.0 > nit.0 + by, "{level:?}: the maniac plays more hands: {maniac:?} vs {nit:?}");
            assert!(maniac.1 > nit.1 * 1.5, "{level:?}: the maniac raises more: {maniac:?} vs {nit:?}");
            if level != Level::Genius {
                let (station, rock) = (tally("Jinx", level), tally("Flint", level));
                assert!(station.2 > rock.2 + 0.05, "{level:?}: the station calls more: {station:?} vs {rock:?}");
            }
        }
    }

    /// Think times: inside the clock, spread out like people's, and longer for a big call on
    /// the river than for a check before the flop.
    #[test]
    fn bots_take_human_times() {
        let mut t = Table::new(Rules::holdem(5, 10, 3));
        for i in 0..3 {
            t.sit(i, &format!("bot{i}"), "x", 1000).unwrap();
        }
        t.start_hand(1, shuffle(seed_from(3))).unwrap();
        let seat = t.hand.as_ref().unwrap().to_act.unwrap();
        let easy = t.seen(seat).unwrap();
        let mut hard = easy.clone();
        hard.spot.street = Street::River;
        hard.spot.board = crate::cards::parse_cards("2c7d9hJsKc").unwrap();
        hard.spot.legal.can_check = false;
        hard.spot.legal.call = 800;
        hard.spot.players.iter_mut().for_each(|p| p.total = 400);
        let mut rng = Stream::new(seed_from(5));
        let times = |seen: &Seen, a: Action, rng: &mut Stream| -> Vec<u64> { (0..2_000).map(|_| think_ms(seen, Level::Genius, &persona::PLAIN, a, rng)).collect() };
        let (e, h) = (times(&easy, Action::Check, &mut rng), times(&hard, Action::Call, &mut rng));
        let mean = |v: &[u64]| v.iter().sum::<u64>() as f64 / v.len() as f64;
        assert!(e.iter().chain(&h).all(|ms| (MIN_THINK_MS..=MAX_THINK_MS).contains(ms)));
        assert!(mean(&h) > mean(&e) * 1.6, "a big river call takes longer: {} vs {}", mean(&h), mean(&e));
        // Snaps and long thinks both happen.
        assert!(e.iter().any(|ms| *ms < 900) && h.iter().any(|ms| *ms > 7_000));
        // (A choice that's no choice at all goes in the least time a bot takes, as 0.13.28's do;
        // a hard one never takes the same time twice.)
        let distinct: std::collections::BTreeSet<u64> = h.iter().map(|ms| ms / 100).collect();
        assert!(distinct.len() > 40, "times vary: {}", distinct.len());
        // A big call over a big river bet takes a while whichever way it goes: folded too.
        let fold = times(&hard, Action::Fold, &mut rng);
        assert!(mean(&fold) > mean(&e) * 1.4, "a hard fold takes a while: {} vs {}", mean(&fold), mean(&e));
        let at = |name: &str, rng: &mut Stream| (0..2_000).map(|_| think_ms(&hard, Level::Regular, persona::of(name), Action::Call, rng)).sum::<u64>() as f64 / 2_000.0;
        let (quick, slow) = (at("Blaze", &mut rng), at("Moss", &mut rng));
        assert!(slow > quick * 1.8, "Moss takes his time, Blaze doesn't: {slow} vs {quick}");
    }

    /// What a table of bots does over many hands: (each move's think times, the preflop
    /// raiser's flop bets when checked to and its chances, river bets with less than a pair when
    /// checked to and the chances, by seat).
    struct Tally {
        times: Vec<(Action, u64)>,
        cbets: (u32, u32),
        river_bluffs: Vec<(u32, u32)>,
    }

    fn tally(names: &[&str], level: Level, hands: u64, seed: u64) -> Tally {
        let n = names.len();
        let mut rng = Stream::new(seed_from(seed));
        let mut t = Table::new(Rules::holdem(10, 20, n));
        for (i, _) in names.iter().enumerate() {
            t.sit(i, &format!("bot{i}"), "x", 4_000).unwrap();
        }
        let mut out = Tally { times: Vec::new(), cbets: (0, 0), river_bluffs: vec![(0, 0); n] };
        for hand in 0..hands {
            for s in t.seats.iter_mut().flatten() {
                if s.stack < 400 {
                    s.stack = 4_000;
                }
            }
            let mut ev = t.start_hand(hand, shuffle(seed_from(seed * 1_000_003 + hand))).unwrap();
            loop {
                let Some(Some(seat)) = ev.iter().rev().find_map(|(_, e)| match e {
                    Event::Turn { seat, .. } => Some(Some(*seat)),
                    Event::HandEnded { .. } => Some(None),
                    _ => None,
                }) else {
                    break;
                };
                let seen = t.seen(seat).unwrap();
                let who = persona::of(names[seat]);
                let a = decide_as(&seen, level, who, &mut rng);
                out.times.push((a, think_ms(&seen, level, who, a, &mut rng)));
                let sp = &seen.spot;
                let checked_to = sp.legal.can_check && sp.legal.can_raise && !sp.actions.iter().any(|x| x.street == sp.street && matches!(x.action, Action::Raise { .. } | Action::AllIn));
                let bets = matches!(a, Action::Raise { .. } | Action::AllIn);
                if checked_to && sp.street == Street::Flop && sp.preflop_raises().last().is_some_and(|r| r.seat == seat) {
                    out.cbets.1 += 1;
                    out.cbets.0 += bets as u32;
                }
                if checked_to && sp.street == Street::River && hand_value(sp.variant, &seen.hole, &sp.board).category() == eval::Category::HighCard {
                    out.river_bluffs[seat].1 += 1;
                    out.river_bluffs[seat].0 += bets as u32;
                }
                ev = t.act(seat, a).unwrap();
            }
        }
        out
    }

    /// As 0.13.26 made them: how long a bot thinks comes from how hard the choice was, so a long
    /// think ends in a fold as often as in anything (0.13.28's Medium bots: two in five of their
    /// thinks over four seconds were folds, three in ten raises); rags go at once; the preflop
    /// raiser bets the flop most times it's checked to (0.13.28's: 75%); and bluffs go with who
    /// a bot is: a maniac fires far more than a rock, a rookie hardly ever.
    #[test]
    fn bots_think_and_bluff_like_people() {
        let t = tally(&["Lux", "Gale", "Cinder", "Ace", "Flint"], Level::Regular, 1_500, 7);
        let long: Vec<&(Action, u64)> = t.times.iter().filter(|x| x.1 > 4_000).collect();
        let share = |f: &dyn Fn(&Action) -> bool| long.iter().filter(|x| f(&x.0)).count() as f64 / long.len() as f64;
        let folds = share(&|a| *a == Action::Fold);
        let raises = share(&|a| matches!(a, Action::Raise { .. } | Action::AllIn));
        assert!(long.len() > 100 && (0.3..0.55).contains(&folds) && (0.15..0.4).contains(&raises), "long thinks: {} folds {folds:.2} raises {raises:.2}", long.len());
        let fold_ms: Vec<u64> = t.times.iter().filter(|x| x.0 == Action::Fold).map(|x| x.1).collect();
        let instant = fold_ms.iter().filter(|&&ms| ms == MIN_THINK_MS).count() as f64 / fold_ms.len() as f64;
        assert!(instant > 0.5, "most folds are rags thrown away at once: {instant:.2}");
        let cbet = t.cbets.0 as f64 / t.cbets.1.max(1) as f64;
        assert!(t.cbets.1 > 40 && (0.6..0.9).contains(&cbet), "c-bets {:?}", t.cbets);
        let rate = |names: &[&str], level: Level, who: usize| {
            let t = tally(names, level, 1_500, 11);
            let (b, n) = t.river_bluffs[who];
            b as f64 / n.max(1) as f64
        };
        let maniac = rate(&["Blaze", "Ivy", "Sage"], Level::Regular, 0);
        let rock = rate(&["Flint", "Ivy", "Sage"], Level::Regular, 0);
        let rookie = rate(&["Blaze", "Ivy", "Sage"], Level::Rookie, 0);
        assert!(maniac > rock * 2.5 && maniac > rookie * 2.0, "river bluffs: maniac {maniac:.3} rock {rock:.3} rookie maniac {rookie:.3}");
    }
}
