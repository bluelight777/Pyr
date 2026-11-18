//! Prop bets in chat (the rebuild's new casino game): anyone in a channel opens a prop (a
//! question with two to six outcomes), and everyone there bets Pyr Casino chips on it in one
//! shared market. The server keeps everything: the props, every bet, the odds, and settling
//! and paying out (from the casino's own chips, like every other game here).
//!
//! The odds come from a logarithmic market scoring rule (LMSR) market maker, as in Maanix's
//! PlayProps (its `lmsr.rs`, MIT or Apache-2.0): each outcome has shares outstanding `q[i]`
//! and the prop a depth `b`; the cost of the shares is `C(q) = b ln Σ e^(q_i/b)`, a bet pays
//! `C(after) - C(before)`, the price of an outcome is the softmax of `q / b` (the prices sum
//! to 1, so they read as chances), and a winning share pays one chip. The market maker can lose
//! at most `b ln n` on a prop, whatever's bet: whoever opens the prop puts that up (its
//! backing), and gets back what's left of it when it's settled. So a prop never makes chips
//! out of nothing: what winners get comes from the bets and the backing.
//!
//! **The casino's own props (0.15).** Props that open, close and settle by themselves: from
//! the props feed (`pyr_proto::props_feed`: games, prices, the weather, each settled from the
//! source it names) and from the casino's own numbers. Nobody opens or settles those, so the
//! casino backs them (`house`): their odds start where the feed says ([`q_from_prior`]), and
//! the most the casino can lose on one is `b ln(1 / p)` of its least likely outcome
//! ([`house_backing`]). Every prop has a category ([`CATEGORIES`]); members' own are
//! `community` unless they say.

use serde::{Deserialize, Serialize};

/// The most outcomes a prop has.
pub const MAX_OUTCOMES: usize = 6;
/// The smallest bet.
pub const MIN_BET: u64 = 10;
/// The biggest one bet (the market maker takes any; this keeps the arithmetic easy).
pub const MAX_BET: u64 = 100_000_000;
/// How deep a prop's market is (`b`): how far a bet moves the odds. Opening one puts up `b ln n`
/// chips to back it (see [`backing`]).
pub const DEPTHS: [(u64, &str); 3] = [(10_000, "Friendly"), (100_000, "Big"), (1_000_000, "High roller")];
/// The most props open at once: one person's, and one channel's.
pub const MAX_OPEN_EACH: usize = 5;
pub const MAX_OPEN_IN_CHANNEL: usize = 20;
/// The most props a list of every channel's gives (Prop Bets' own window).
pub const MAX_LISTED: usize = 100;
/// How long a prop can stay open unsettled before it's called off (everyone's chips back).
pub const MAX_OPEN_DAYS: i64 = 30;
/// When betting can close: in an hour, a day, a week, or when it's settled (0).
pub const CLOSES_IN: [u64; 4] = [0, 3600, 86_400, 7 * 86_400];

/// The categories props come in (id, name), in the order they're shown. A category the feed
/// brings that isn't here yet shows under its own name ([`category_name`]).
pub const CATEGORIES: [(&str, &str); 14] = [
    ("nfl", "NFL"),
    ("nba", "NBA"),
    ("nhl", "NHL"),
    ("mlb", "MLB"),
    ("cfb", "College Football"),
    ("cbb", "College Basketball"),
    ("topical", "Topical"),
    ("casino", "Pyr Casino"),
    ("crypto", "Crypto"),
    ("weather", "Weather"),
    ("entertainment", "Entertainment"),
    ("science", "Science"),
    ("art", "Art"),
    ("community", "Community"),
];
/// Members' own props, unless they say another.
pub const COMMUNITY: &str = "community";
/// The casino's props about itself: made by each server from its own numbers, not the feed.
pub const CASINO: &str = "casino";

/// A category's id as kept: a known one, a new one from the feed as it is, `community` for
/// none (props from before 0.15) or one that isn't a name.
pub fn category_id(id: &str) -> String {
    let id = id.trim().to_ascii_lowercase();
    if !id.is_empty() && id.len() <= 24 && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-') { id } else { COMMUNITY.to_string() }
}

/// A category's name: "College Football"; one that isn't known yet, its id with a capital.
pub fn category_name(id: &str) -> String {
    let id = category_id(id);
    match CATEGORIES.iter().find(|c| c.0 == id) {
        Some(c) => c.1.to_string(),
        None => {
            let mut name = id.replace(['_', '-'], " ");
            if let Some(first) = name.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            name
        }
    }
}

/// The least chance a house prop's outcome starts at (what bounds the casino's risk on it:
/// [`house_backing`]).
pub const MIN_PRIOR: f64 = 0.02;
/// How long after betting closed a house prop waits for its result before it's called off.
pub const HOUSE_WAIT_DAYS: i64 = 10;
/// How many price points a prop keeps of how its odds went.
pub const HISTORY: usize = 96;
/// How deep a house prop's market is unless the server's owner says (one of [`DEPTHS`]).
pub const HOUSE_DEPTH: u64 = 100_000;

/// Starting odds, tidied: none under [`MIN_PRIOR`], adding up to 1 (even odds for ones that
/// make no sense).
pub fn tidy_prior(prior: &[f64], n: usize) -> Vec<f64> {
    let n = n.max(2);
    if prior.len() != n || prior.iter().any(|p| !p.is_finite() || *p <= 0.0) {
        return vec![1.0 / n as f64; n];
    }
    let sum: f64 = prior.iter().sum();
    let mut p: Vec<f64> = prior.iter().map(|x| x / sum).collect();
    // (Raised to the least, the rest scaled down to make room: a few rounds settle it.)
    for _ in 0..8 {
        let low: f64 = p.iter().filter(|x| **x < MIN_PRIOR).map(|x| MIN_PRIOR - x).sum();
        if low <= 1e-12 {
            break;
        }
        let room: f64 = p.iter().filter(|x| **x > MIN_PRIOR).map(|x| x - MIN_PRIOR).sum();
        p = p.iter().map(|x| if *x <= MIN_PRIOR { MIN_PRIOR } else { MIN_PRIOR + (x - MIN_PRIOR) * (1.0 - low / room) }).collect();
    }
    p
}

/// Where a house prop's market starts so its odds read `prior`: `q_i = b ln(p_i / p_max)`
/// (the likeliest outcome at 0; nobody holds these shares, so nobody is paid for them).
pub fn q_from_prior(prior: &[f64], b: f64) -> Vec<f64> {
    let max = prior.iter().cloned().fold(f64::MIN_POSITIVE, f64::max);
    prior.iter().map(|p| b * (p.max(1e-12) / max).ln()).collect()
}

/// The most the casino can lose on a house prop that starts at `prior`: `b ln(1 / p)` of its
/// least likely outcome (what the bets bring in covers the rest, whatever's bet).
pub fn house_backing(prior: &[f64], b: f64) -> u64 {
    let min = prior.iter().cloned().fold(1.0, f64::min).max(1e-12);
    (b * (1.0 / min).ln()).ceil() as u64
}

/// Each outcome's price (its chance): the softmax of `q / b`.
pub fn prices(q: &[f64], b: f64) -> Vec<f64> {
    let max = q.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let exps: Vec<f64> = q.iter().map(|x| ((x - max) / b).exp()).collect();
    let sum: f64 = exps.iter().sum();
    exps.iter().map(|e| e / sum).collect()
}

/// Shares of outcome `i` that `chips` buy: `b ln(1 + (e^(c/b) - 1) / p_i)`, written so it
/// stays exact for bets far bigger than `b` (`b (x - ln p + ln(1 - (1 - p) e^-x))`, x = c/b).
pub fn shares_for(q: &[f64], b: f64, i: usize, chips: f64) -> f64 {
    let p = prices(q, b)[i].clamp(1e-300, 1.0);
    let x = chips / b;
    b * (x - p.ln() + (-(1.0 - p) * (-x).exp()).ln_1p())
}

/// Chips that selling `shares` of outcome `i` back brings: `-b ln(1 + p_i (e^(-s/b) - 1))`,
/// worked out from how the other outcomes stand against this one (`L = ln Σ e^((q_j - q_i)/b)`
/// over the others: `b (ln(1 + e^L) - ln(e^L + e^(-s/b)))`). Written from the price it loses
/// what's left of the others once this one's chance is nearer 1 than a number can say (a bet
/// a hundred times the depth), and 0.15.0 came out with infinity there: a cash-out "worth
/// 18,446,744,073,709,551,615".
pub fn proceeds_for(q: &[f64], b: f64, i: usize, shares: f64) -> f64 {
    let l = ln_sum_exp(q.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, x)| (x - q[i]) / b));
    b * (ln_add_exp(0.0, l) - ln_add_exp(l, -shares / b))
}

/// `ln(e^a + e^b)`, without leaving the logarithms (either may be minus infinity).
fn ln_add_exp(a: f64, b: f64) -> f64 {
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    if lo == f64::NEG_INFINITY { hi } else { hi + (lo - hi).exp().ln_1p() }
}

/// `ln Σ e^x` (minus infinity for none).
fn ln_sum_exp(xs: impl Iterator<Item = f64>) -> f64 {
    xs.fold(f64::NEG_INFINITY, ln_add_exp)
}

/// The most the market maker can lose on a prop of `n` outcomes and depth `b`.
pub fn house_risk(n: usize, b: f64) -> f64 {
    b * (n as f64).ln()
}

/// What opening a prop of `n` outcomes and depth `b` puts up: the most its market maker can pay
/// out over what's bet, so every winning share is paid whatever happens. What's left of it
/// comes back when the prop is settled (or called off).
pub fn backing(n: usize, b: f64) -> u64 {
    house_risk(n.max(2), b).ceil() as u64
}

/// Where a prop stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PropState {
    /// Taking bets.
    #[default]
    Open,
    /// Betting closed (its time came); waiting to be settled.
    Closed,
    /// Settled: `winner` won.
    Settled,
    /// Called off: everyone got back what they'd put in.
    Void,
}

/// What I hold of one outcome: shares, and the chips they cost me.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct Holding {
    pub outcome: usize,
    pub shares: f64,
    pub cost: u64,
}

/// A bet, as everyone in the channel sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct BetLine {
    pub uid: String,
    pub name: String,
    pub outcome: usize,
    /// Chips in (a bet), or out (cashed out: negative).
    pub chips: i64,
    pub at: i64,
}

/// The odds at a moment: each outcome's chance.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PricePoint {
    pub at: i64,
    pub p: Vec<f32>,
}

/// A prop, as one person sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PropView {
    pub id: u64,
    pub channel_id: i64,
    pub author: String,
    pub author_name: String,
    pub question: String,
    pub outcomes: Vec<String>,
    /// The market: depth and shares outstanding (so the app can say what a bet pays).
    pub b: f64,
    pub q: Vec<f64>,
    pub state: PropState,
    pub winner: Option<usize>,
    pub created_at: i64,
    /// When betting closes (0: when it's settled).
    pub closes_at: i64,
    /// Chips bet on it in all, and how many bets.
    pub pool: u64,
    pub bets: u32,
    /// The latest bets (newest first).
    pub latest: Vec<BetLine>,
    /// Mine.
    pub mine: Vec<Holding>,
    /// I may settle it or call it off (its author, or a moderator).
    pub can_settle: bool,
    /// The chips its author put up to back it (see [`backing`]).
    pub backing: u64,
    /// Who settled it, and the biggest winners (name, chips paid), once it's settled.
    pub settled_by: String,
    pub winners: Vec<(String, u64)>,
    /// Its category (0.15, see [`CATEGORIES`]; none from an older server: `community`).
    pub category: String,
    /// The casino's own (0.15): it opened by itself, the casino backs it, and it settles by
    /// itself from `source`. Nobody here settles it (`can_settle`: a manager may call it off).
    pub house: bool,
    /// What it's about, shared by the props of one game or one day, that's name ("Chiefs at
    /// Bills"), and when that starts (ms; 0: not said).
    pub event: String,
    pub event_name: String,
    pub starts_at: i64,
    /// A line about it as it stands: "Sunday 4:25 PM ET", "Q3 5:21 · 17-20".
    pub detail: String,
    /// Whose numbers settle it, and how.
    pub source: String,
    pub rules: String,
    /// What happened, once it's settled or called off: "Bills 27, Chiefs 24".
    pub note: String,
    pub settled_at: i64,
    /// How much it's worth showing first (0: not especially).
    pub featured: u8,
    /// How many people have something on it.
    pub bettors: u32,
    /// How its odds went (oldest first), when one prop is looked up (`PropGet`); a list of
    /// props comes without.
    pub history: Vec<PricePoint>,
}

/// How someone did on props over some days: what they won less what they put in, on props
/// settled and bets cashed out in that time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PropLeader {
    pub uid: String,
    pub name: String,
    pub net: i64,
    /// Chips put in, props bet on, the ones that paid, and the biggest single win.
    pub staked: u64,
    pub props: u32,
    pub wins: u32,
    pub biggest: i64,
}

/// The server's best at props over `days` (0: since ever), and me among them (`rank`: 0 when
/// I've nothing settled in that time).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PropStats {
    pub days: u32,
    pub leaders: Vec<PropLeader>,
    pub me: PropLeader,
    pub rank: u32,
    pub players: u32,
}

/// The casino's own props on a server: off until whoever manages the server turns them on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PropAuto {
    pub on: bool,
    /// The categories turned off (the rest are on, new ones the feed brings too).
    pub off: Vec<String>,
    /// How deep their markets are (one of [`DEPTHS`]).
    pub depth: u64,
    /// I may change it (Manage server).
    pub can_set: bool,
    /// The feed as this server last read it: when (ms; 0: not yet), how many props it had, and
    /// what went wrong if something did.
    pub feed_at: i64,
    pub feed_props: u32,
    pub feed_error: String,
}

/// What to set of [`PropAuto`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PropAutoSet {
    pub on: bool,
    pub off: Vec<String>,
    pub depth: u64,
}

impl PropView {
    pub fn prices(&self) -> Vec<f64> {
        prices(&self.q, self.b)
    }

    /// Whether bets are taken now (`now`: ms).
    pub fn open_at(&self, now: i64) -> bool {
        self.state == PropState::Open && (self.closes_at == 0 || now < self.closes_at)
    }

    /// What my shares of each outcome are worth if sold now.
    pub fn worth(&self, h: &Holding) -> f64 {
        proceeds_for(&self.q, self.b, h.outcome, h.shares)
    }

    /// Its category's id (`community` for a prop from before categories).
    pub fn category_id(&self) -> String {
        category_id(&self.category)
    }
}

/// A prop's question and outcomes, checked and tidied: 5 to 150 characters for the question,
/// two to six different outcomes of up to 30 (Yes and No when none are given).
pub fn check_new(question: &str, outcomes: &[String]) -> Result<(String, Vec<String>), String> {
    let clean = |s: &str| s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect::<String>().split_whitespace().collect::<Vec<_>>().join(" ");
    let question = clean(question);
    let mut outcomes: Vec<String> = outcomes.iter().map(|o| clean(o)).filter(|o| !o.is_empty()).collect();
    if outcomes.is_empty() {
        outcomes = vec!["Yes".into(), "No".into()];
    }
    if !(5..=150).contains(&question.chars().count()) {
        return Err("Write a question of 5 to 150 characters, like: Will it rain on Saturday?".into());
    }
    if !(2..=MAX_OUTCOMES).contains(&outcomes.len()) {
        return Err("A prop has 2 to 6 outcomes.".into());
    }
    if outcomes.iter().any(|o| o.chars().count() > 30) {
        return Err("Keep each outcome to 30 characters.".into());
    }
    for (i, o) in outcomes.iter().enumerate() {
        if outcomes[..i].iter().any(|p| p.eq_ignore_ascii_case(o)) {
            return Err("Each outcome has to be different.".into());
        }
    }
    Ok((question, outcomes))
}

/// `/prop Will it rain on Saturday? | Yes | No`: the question and outcomes typed after it.
pub fn parse_command(args: &str) -> (String, Vec<String>) {
    let mut parts = args.split('|').map(|s| s.trim().to_string());
    let question = parts.next().unwrap_or_default();
    (question, parts.filter(|o| !o.is_empty()).collect())
}

/// A prop's link in chat (the message that shows it as a card): `pyr://prop/<id>`.
pub fn link(id: u64) -> String {
    format!("pyr://prop/{id}")
}

/// The prop a link is for.
pub fn id_in_link(word: &str) -> Option<u64> {
    let w = word.trim().trim_matches(|c: char| matches!(c, '<' | '>' | '(' | ')' | '.' | ','));
    let rest = w.get(..11).filter(|p| p.eq_ignore_ascii_case("pyr://prop/")).map(|_| &w[11..])?;
    rest.trim_end_matches('/').parse().ok().filter(|id| *id > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(q: &[f64], b: f64) -> f64 {
        let max = q.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        max + b * q.iter().map(|x| ((x - max) / b).exp()).sum::<f64>().ln()
    }

    #[test]
    fn prices_add_up_to_one() {
        let p = prices(&[10.0, -40.0, 75.0], 150.0);
        assert!((p.iter().sum::<f64>() - 1.0).abs() < 1e-12);
        assert_eq!(prices(&[0.0, 0.0], 10_000.0), vec![0.5, 0.5]);
    }

    #[test]
    fn a_bet_costs_what_the_cost_function_says() {
        for &(b, chips) in &[(150.0, 80.0), (10_000.0, 2_500.0), (10_000.0, 5_000_000.0), (1_000_000.0, 100_000_000.0)] {
            let mut q = vec![20.0, 0.0, -30.0];
            let before = cost(&q, b);
            let s = shares_for(&q, b, 1, chips);
            q[1] += s;
            let paid = cost(&q, b) - before;
            assert!((paid - chips).abs() / chips < 1e-9, "b {b}, chips {chips}: paid {paid}");
        }
    }

    #[test]
    fn selling_back_returns_what_was_paid() {
        let (b, mut q) = (10_000.0, vec![0.0, 0.0]);
        let s = shares_for(&q, b, 0, 4_000.0);
        q[0] += s;
        assert!((proceeds_for(&q, b, 0, s) - 4_000.0).abs() < 1e-6);
    }

    /// As it happened on 0.15.0 (Nov 18): 100,000,000 on a 76% favourite, in a market a million
    /// deep. It pays 100,274,436 if it wins, and it's worth what it cost: not infinity.
    #[test]
    fn a_bet_far_past_the_depth_sells_back_for_what_it_cost() {
        let b = 1_000_000.0;
        let mut q = q_from_prior(&[0.76, 0.24], b);
        let s = shares_for(&q, b, 0, 100_000_000.0);
        assert_eq!(s.floor(), 100_274_436.0);
        q[0] += s;
        assert_eq!(prices(&q, b)[0], 1.0, "its chance is nearer 1 than a number can say");
        let back = proceeds_for(&q, b, 0, s);
        assert!((back - 100_000_000.0).abs() < 1e-3, "{back}");
        // Half of it now, the rest after: the same in all.
        let first = proceeds_for(&q, b, 0, s / 2.0);
        q[0] -= s / 2.0;
        let rest = proceeds_for(&q, b, 0, s / 2.0);
        assert!((first + rest - 100_000_000.0).abs() < 1e-3, "{first} + {rest}");
        // Whatever the odds and however big the bet: what was paid comes back, a share never
        // sells for more than a chip, and nothing sells for less than nothing.
        for prior in [vec![0.5, 0.5], vec![0.02, 0.98], vec![0.98, 0.02], vec![0.1, 0.2, 0.3, 0.4], vec![0.02, 0.02, 0.02, 0.02, 0.02, 0.9]] {
            for b in [10_000.0, 100_000.0, 1_000_000.0] {
                for chips in [10.0, 777.0, 40_000.0, 5_000_000.0, 100_000_000.0] {
                    for i in 0..prior.len() {
                        let mut q = q_from_prior(&prior, b);
                        let s = shares_for(&q, b, i, chips);
                        q[i] += s;
                        let back = proceeds_for(&q, b, i, s);
                        assert!(back.is_finite() && (back - chips).abs() <= chips * 1e-9 + 1e-6, "{prior:?}, b {b}, {chips} on {i}: {back} back");
                        for part in [s * 0.001, s * 0.5, s] {
                            let got = proceeds_for(&q, b, i, part);
                            assert!(got >= 0.0 && got <= part + 1e-6, "{prior:?}, b {b}, {chips} on {i}: {part} shares sell for {got}");
                        }
                        // (And someone else's shares of another outcome, on a market pushed that far.)
                        let j = (i + 1) % prior.len();
                        let got = proceeds_for(&q, b, j, 5_000.0);
                        assert!(got.is_finite() && got >= 0.0 && got <= 5_000.0, "{got}");
                    }
                }
            }
        }
    }

    #[test]
    fn betting_raises_the_odds_and_a_share_costs_under_a_chip() {
        let (b, q) = (10_000.0, vec![0.0, 0.0]);
        let s = shares_for(&q, b, 0, 1_000.0);
        assert!(s > 1_000.0 && s < 2_000.0, "{s}");
        assert!(prices(&[s, 0.0], b)[0] > 0.5);
    }

    #[test]
    fn a_huge_bet_stays_finite_and_the_house_risk_is_bounded() {
        let (b, q) = (10_000.0, vec![0.0, 0.0]);
        let s = shares_for(&q, b, 0, MAX_BET as f64);
        assert!(s.is_finite());
        // Paying out every share of the outcome bet on costs the house at most b ln 2.
        assert!(s - MAX_BET as f64 <= house_risk(2, b) + 1e-6, "{}", s - MAX_BET as f64);
    }

    /// Whatever's bet, and whichever way it goes, what the bets brought in and the backing cover
    /// every winning share.
    #[test]
    fn the_backing_covers_every_winning_share() {
        for &(n, b) in &[(2usize, 10_000.0), (3, 10_000.0), (6, 100_000.0)] {
            let mut q = vec![0.0; n];
            let mut taken = 0.0;
            for (k, chips) in [5_000.0, 120_000.0, 30.0, 9_000_000.0, 777.0].iter().enumerate() {
                let i = (k * 7 + 3) % n;
                let s = shares_for(&q, b, i, *chips);
                q[i] += s;
                taken += chips;
            }
            for w in 0..n {
                assert!(q[w] <= taken + backing(n, b) as f64 + 1e-6, "n {n}, b {b}, {w} wins: owed {}, held {}", q[w], taken + backing(n, b) as f64);
            }
        }
        assert_eq!(backing(2, 10_000.0), 6_932);
    }

    #[test]
    fn props_are_checked() {
        assert_eq!(check_new("Will it rain?", &[]).unwrap().1, vec!["Yes".to_string(), "No".to_string()]);
        assert!(check_new("Hm?", &[]).is_err());
        assert!(check_new("Who wins tonight?", &["A".into(), "a".into()]).is_err());
        assert!(check_new("Who wins tonight?", &(0..7).map(|i| i.to_string()).collect::<Vec<_>>()).is_err());
        assert_eq!(parse_command("Will Maanix hit a billion? | Yes | No | Maybe "), ("Will Maanix hit a billion?".to_string(), vec!["Yes".to_string(), "No".into(), "Maybe".into()]));
    }

    #[test]
    fn a_house_prop_starts_at_the_feed_s_odds_and_its_risk_is_bounded() {
        let b = 100_000.0;
        let prior = tidy_prior(&[0.42, 0.58], 2);
        let q = q_from_prior(&prior, b);
        let p = prices(&q, b);
        assert!((p[0] - 0.42).abs() < 1e-9 && (p[1] - 0.58).abs() < 1e-9, "{p:?}");
        // Whatever's bet and whichever way it goes, the bets and the backing cover every share
        // sold (the shares the market starts with are nobody's).
        for prior in [vec![0.42, 0.58], vec![0.02, 0.98], vec![0.1, 0.2, 0.3, 0.4], vec![0.5, 0.5]] {
            let n = prior.len();
            let start = q_from_prior(&prior, b);
            let mut q = start.clone();
            let mut taken = 0.0;
            for (k, chips) in [5_000.0, 120_000.0, 30.0, 9_000_000.0, 777.0, 40_000_000.0].iter().enumerate() {
                let i = (k * 5 + 1) % n;
                q[i] += shares_for(&q, b, i, *chips);
                taken += chips;
            }
            for w in 0..n {
                let owed = q[w] - start[w];
                assert!(owed <= taken + house_backing(&prior, b) as f64 + 1e-6, "{prior:?}, {w} wins: owed {owed}, held {}", taken + house_backing(&prior, b) as f64);
            }
        }
        assert_eq!(house_backing(&[0.5, 0.5], 10_000.0), backing(2, 10_000.0), "even odds: what a member's prop puts up");
        // Odds that make no sense, or too long: tidied.
        assert_eq!(tidy_prior(&[0.0, 1.0], 2), vec![0.5, 0.5]);
        assert_eq!(tidy_prior(&[0.3], 2), vec![0.5, 0.5]);
        let t = tidy_prior(&[0.001, 0.999], 2);
        assert!((t[0] - MIN_PRIOR).abs() < 1e-9 && (t.iter().sum::<f64>() - 1.0).abs() < 1e-9, "{t:?}");
        let t = tidy_prior(&[0.001, 0.002, 0.497, 0.5], 4);
        assert!(t.iter().all(|p| *p >= MIN_PRIOR - 1e-9) && (t.iter().sum::<f64>() - 1.0).abs() < 1e-9, "{t:?}");
        let t = tidy_prior(&[2.0, 6.0], 2);
        assert_eq!(t, vec![0.25, 0.75]);
    }

    #[test]
    fn categories_have_names_and_new_ones_show_under_their_own() {
        assert_eq!(category_name("cfb"), "College Football");
        assert_eq!(category_name(""), "Community");
        assert_eq!(category_name("space_news"), "Space news");
        assert_eq!(category_id("NFL"), "nfl");
        assert_eq!(category_id("not a name!"), COMMUNITY);
        // A prop from an older server (no category, no house): read as it was.
        let old: PropView = serde_json::from_str(r#"{"id":3,"channel_id":1,"author":"a","author_name":"A","question":"Rain?","outcomes":["Yes","No"],"b":10000.0,"q":[0.0,0.0],"state":"open","winner":null,"created_at":1,"closes_at":0,"pool":0,"bets":0,"latest":[],"mine":[],"can_settle":true,"backing":6932,"settled_by":"","winners":[]}"#).unwrap();
        assert_eq!((old.category_id().as_str(), old.house, old.history.len()), ("community", false, 0));
    }

    #[test]
    fn props_are_found_by_their_links() {
        assert_eq!(id_in_link(&link(42)), Some(42));
        assert_eq!(id_in_link("<pyr://prop/7>"), Some(7));
        assert_eq!(id_in_link("pyr://prop/x"), None);
        assert_eq!(id_in_link("pyr://play/pyropoly/ABCD"), None);
    }
}
