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
//! How long a bot takes over a turn ([`think_ms`]) comes from the spot everyone can see and the
//! bot's own pace, never from its cards: its timing gives nothing away.
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
    Rookie,
    Regular,
    Shark,
    Genius,
}

impl Level {
    pub const ALL: [Level; 4] = [Level::Rookie, Level::Regular, Level::Shark, Level::Genius];

    pub fn name(self) -> &'static str {
        match self {
            Level::Rookie => "Rookie",
            Level::Regular => "Regular",
            Level::Shark => "Shark",
            Level::Genius => "Genius",
        }
    }

    /// The level shown on a bot's seat (people show theirs).
    pub fn look_level(self) -> u32 {
        match self {
            Level::Rookie => 3,
            Level::Regular => 12,
            Level::Shark => 30,
            Level::Genius => 50,
        }
    }

    /// What a 0.11 app can be told (it doesn't know Genius: a Shark, to it).
    pub fn for_older_app(self) -> Level {
        match self {
            Level::Genius => Level::Shark,
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
    if l.can_check {
        // Nobody bet: bet good hands sometimes, bluff now and then (more in position).
        if l.can_raise && (good && roll(rng) < st.aggression * 0.7 || roll(rng) < st.bluff + position_bonus) {
            let to = raise_to(0.5 * who.sizing, rng);
            return Action::Raise { to };
        }
        return Action::Check;
    }
    // Facing a bet: call when the chances beat the price (rookies call more, and so do the
    // loose and the sticky).
    let margin = match level {
        Level::Rookie => -0.08,
        Level::Regular => 0.02,
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

/// How long a bot takes to make `action` (its choice at `spot`), in milliseconds, the way a
/// person would: a fold or a check before the flop often in a moment; a call that costs a lot,
/// a raise (how much?) or an all in taking a while; a look at a new card first; later streets
/// and crowded pots slower. Each bot at its own pace, snapping off some choices and going into
/// the tank over others now and then as its personality goes, and never the same twice (spread
/// like people's times). It shows nothing of its cards that the move itself, made a moment
/// later, doesn't.
pub fn think_ms(spot: &Spot, level: Level, who: &Persona, action: Action, rng: &mut Stream) -> u64 {
    let roll = |rng: &mut Stream| (rng.below(1_000_000) as f64 + 0.5) / 1_000_000.0;
    let me = spot.me();
    let pot = spot.pot().max(1) as f64;
    let call = spot.legal.call as f64;
    let stack = (me.behind + me.street_bet).max(1) as f64;
    // What's riding on it: the price against the pot, and against what's left.
    let price = (call / pot).min(2.0);
    let commit = (call / me.behind.max(1) as f64).min(1.0);
    let late = match spot.street {
        Street::Preflop => 0.0,
        Street::Flop => 0.25,
        Street::Turn => 0.4,
        Street::River => 0.6,
    };
    let crowd = spot.opponents().count().saturating_sub(1) as f64 * 0.06;
    // The choice: a fold or a check is quick, a call takes some thought, a bet or a raise more
    // (how much?), all in the most. (Base time, and how heavy it is.)
    let (base, weight, snappy, tanky) = match action {
        Action::Fold if spot.street == Street::Preflop => (620.0, 0.12 + price * 0.25 + commit * 0.3, 2.2, 0.3),
        Action::Fold => (950.0, 0.35 + price * 0.5 + commit * 0.6 + late, 1.3, 0.8),
        Action::Check => (800.0, 0.25 + late * 0.6 + crowd, 1.8, 0.4),
        Action::Call => (1_150.0, 0.5 + price * 0.55 + commit * 1.3 + late + crowd, 1.0, 1.3),
        Action::Raise { to } => {
            let size = (to.saturating_sub(me.street_bet) as f64 / pot).min(3.0);
            (1_400.0, 0.55 + size * 0.3 + (to as f64 / stack).min(1.0) * 0.7 + late + crowd, 0.7, 1.1)
        }
        Action::AllIn => (1_650.0, 0.85 + commit * 0.6 + late + crowd, 0.6, 1.6),
    };
    // Each level its own way: rookies quick and impulsive, geniuses steadier.
    let level_k = match level {
        Level::Rookie => 0.82,
        Level::Regular => 0.95,
        Level::Shark => 1.05,
        Level::Genius => 1.12,
    };
    // Spread like people's (log-normal).
    let (u1, u2) = (roll(rng), roll(rng));
    let z = (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos();
    let mut ms = base * level_k * who.pace * (0.55 + weight) * (0.38 * z).exp();
    // A new card on the table: a look at it first.
    if spot.street != Street::Preflop && !spot.actions.iter().any(|a| a.street == spot.street) {
        ms += 350.0 + roll(rng) * 750.0;
    }
    // Now and then a snap, now and then a long think: how often depends on who it is, and on
    // the choice (snapping off a fold, tanking over a big call).
    let r = roll(rng);
    if r < who.snaps * snappy {
        ms = 450.0 + roll(rng) * 500.0;
    } else if r > 1.0 - who.tanks * tanky {
        ms += 2_500.0 + roll(rng) * 6_000.0 * (0.4 + weight.min(1.8));
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
    /// the river than for a check before the flop. The spot and the choice decide them (the
    /// choice is shown a moment later anyway): there's no way to pass anyone's cards in.
    #[test]
    fn bots_take_human_times() {
        let mut t = Table::new(Rules::holdem(5, 10, 3));
        for i in 0..3 {
            t.sit(i, &format!("bot{i}"), "x", 1000).unwrap();
        }
        t.start_hand(1, shuffle(seed_from(3))).unwrap();
        let seat = t.hand.as_ref().unwrap().to_act.unwrap();
        let easy = t.spot(seat).unwrap();
        let mut hard = easy.clone();
        hard.street = Street::River;
        hard.legal.can_check = false;
        hard.legal.call = 800;
        hard.players.iter_mut().for_each(|p| p.total = 400);
        let mut rng = Stream::new(seed_from(5));
        let times = |spot: &Spot, a: Action, rng: &mut Stream| -> Vec<u64> { (0..2_000).map(|_| think_ms(spot, Level::Genius, &persona::PLAIN, a, rng)).collect() };
        let (e, h) = (times(&easy, Action::Check, &mut rng), times(&hard, Action::Call, &mut rng));
        let mean = |v: &[u64]| v.iter().sum::<u64>() as f64 / v.len() as f64;
        assert!(e.iter().chain(&h).all(|ms| (MIN_THINK_MS..=MAX_THINK_MS).contains(ms)));
        assert!(mean(&h) > mean(&e) * 1.6, "a big river call takes longer: {} vs {}", mean(&h), mean(&e));
        // Snaps and long thinks both happen.
        assert!(e.iter().any(|ms| *ms < 900) && h.iter().any(|ms| *ms > 7_000));
        let distinct: std::collections::BTreeSet<u64> = e.iter().map(|ms| ms / 100).collect();
        assert!(distinct.len() > 20, "times vary");
        // A fold before the flop is mostly quick; a shove takes a while; and a fast bot is
        // faster than a slow one at the same spot.
        let fold = times(&easy, Action::Fold, &mut rng);
        let shove = times(&hard, Action::AllIn, &mut rng);
        assert!(mean(&fold) < 1_400.0 && mean(&shove) > 2.0 * mean(&fold), "fold {} shove {}", mean(&fold), mean(&shove));
        let at = |name: &str, rng: &mut Stream| (0..2_000).map(|_| think_ms(&hard, Level::Regular, persona::of(name), Action::Call, rng)).sum::<u64>() as f64 / 2_000.0;
        let (quick, slow) = (at("Blaze", &mut rng), at("Moss", &mut rng));
        assert!(slow > quick * 1.8, "Moss takes his time, Blaze doesn't: {slow} vs {quick}");
    }
}
