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
/// How long a prop can stay open unsettled before it's called off (everyone's chips back).
pub const MAX_OPEN_DAYS: i64 = 30;
/// When betting can close: in an hour, a day, a week, or when it's settled (0).
pub const CLOSES_IN: [u64; 4] = [0, 3600, 86_400, 7 * 86_400];

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

/// Chips that selling `shares` of outcome `i` back brings: `-b ln(1 + p_i (e^(-s/b) - 1))`.
pub fn proceeds_for(q: &[f64], b: f64, i: usize, shares: f64) -> f64 {
    let p = prices(q, b)[i];
    -b * (p * (-shares / b).exp_m1()).ln_1p()
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

/// A prop, as one person sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
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
    fn props_are_found_by_their_links() {
        assert_eq!(id_in_link(&link(42)), Some(42));
        assert_eq!(id_in_link("<pyr://prop/7>"), Some(7));
        assert_eq!(id_in_link("pyr://prop/x"), None);
        assert_eq!(id_in_link("pyr://play/pyropoly/ABCD"), None);
    }
}
