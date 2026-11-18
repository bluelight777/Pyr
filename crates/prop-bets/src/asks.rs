//! What's asked of Pyr: a line on the output Pyr reads (`@pyr {json}`), one function each so
//! the tests can hold them to what Pyr reads (`crates/app/src/plugins/programs.rs`).

use std::io::Write;

use serde_json::{Value, json};

/// Asks Pyr for something.
pub fn ask(v: Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "@pyr {v}");
    let _ = out.flush();
}

/// This window takes only what changed each time (0.15's Pyr; an older one never minds).
pub fn hello() -> Value {
    json!({"op": "props_hello", "v": 2})
}
pub fn bet(id: u64, outcome: usize, chips: u64, min_shares: f64) -> Value {
    json!({"op": "prop_bet", "id": id, "outcome": outcome, "chips": chips, "min_shares": min_shares})
}
pub fn cash_out(id: u64, outcome: usize) -> Value {
    json!({"op": "prop_cash_out", "id": id, "outcome": outcome})
}
pub fn open(channel: i64, question: &str, outcomes: &[String], depth: u64, closes_in: u64, category: &str) -> Value {
    json!({"op": "prop_open", "channel": channel, "question": question, "outcomes": outcomes, "depth": depth, "closes_in": closes_in, "category": category})
}
pub fn settle(id: u64, outcome: usize) -> Value {
    json!({"op": "prop_settle", "id": id, "outcome": outcome})
}
pub fn void(id: u64) -> Value {
    json!({"op": "prop_void", "id": id})
}
pub fn refresh() -> Value {
    json!({"op": "props_refresh"})
}
/// One prop looked up, with how its odds went.
pub fn look(id: u64) -> Value {
    json!({"op": "prop_look", "id": id})
}
/// The server's best at props over so many days (0: since ever).
pub fn stats(days: u32) -> Value {
    json!({"op": "prop_stats", "days": days})
}
/// The casino's own props on the server: turned on or off, which categories, how deep.
pub fn auto(on: bool, off: &[String], depth: u64) -> Value {
    json!({"op": "prop_auto", "set": {"on": on, "off": off, "depth": depth}})
}
/// A prop's card posted in the channel showing in Pyr.
pub fn share(id: u64) -> Value {
    json!({"op": "prop_share", "id": id})
}
/// A parlay placed: its legs (each prop and the outcome picked), the stake, and what it was
/// shown to pay, less a little.
pub fn parlay(legs: &[(u64, usize)], chips: u64, min_pays: f64) -> Value {
    json!({"op": "parlay_place", "legs": legs.iter().map(|l| json!([l.0, l.1])).collect::<Vec<_>>(), "chips": chips, "min_pays": min_pays})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_it_asks_is_what_pyr_reads() {
        // (Pyr's side reads these very lines: `prop_bets_window_asks_for_bets_and_props` in
        // crates/app/src/plugins/programs.rs.)
        assert_eq!(bet(12, 1, 500, 980.5).to_string(), r#"{"chips":500,"id":12,"min_shares":980.5,"op":"prop_bet","outcome":1}"#);
        assert_eq!(cash_out(12, 0), json!({"op": "prop_cash_out", "id": 12, "outcome": 0}));
        assert_eq!(open(3, "Rain?", &["Yes".into(), "No".into()], 10_000, 3600, "weather"), json!({"op": "prop_open", "channel": 3, "question": "Rain?", "outcomes": ["Yes", "No"], "depth": 10_000, "closes_in": 3600, "category": "weather"}));
        assert_eq!(settle(12, 0), json!({"op": "prop_settle", "id": 12, "outcome": 0}));
        assert_eq!(void(12), json!({"op": "prop_void", "id": 12}));
        assert_eq!(refresh(), json!({"op": "props_refresh"}));
        assert_eq!(hello(), json!({"op": "props_hello", "v": 2}));
        assert_eq!(look(7), json!({"op": "prop_look", "id": 7}));
        assert_eq!(stats(7), json!({"op": "prop_stats", "days": 7}));
        assert_eq!(auto(true, &["weather".into()], 100_000), json!({"op": "prop_auto", "set": {"on": true, "off": ["weather"], "depth": 100_000}}));
        assert_eq!(share(7), json!({"op": "prop_share", "id": 7}));
        assert_eq!(parlay(&[(12, 1), (14, 0), (30, 3)], 500, 3890.5).to_string(), r#"{"chips":500,"legs":[[12,1],[14,0],[30,3]],"min_pays":3890.5,"op":"parlay_place"}"#);
    }
}
