//! The casino's chat: one room for everyone in Pyr Casino on this server, to type to each other
//! from the floor or any table. The last lines are kept (across restarts, in the database's
//! meta table); lines are short, plain, and not too often; anyone can take down their own,
//! and staff and the server's managers anyone's.

use std::collections::{HashMap, VecDeque};

use pyr_poker::net::ChatLine;

/// Lines kept (and sent to an app opening the chat).
pub const KEPT: usize = 150;
/// The longest line (characters).
pub const LONGEST: usize = 300;
/// The least time between someone's lines, and how many they can say in `WINDOW_MS`.
const GAP_MS: u64 = 600;
const WINDOW_MS: u64 = 10_000;
const PER_WINDOW: usize = 8;
/// Where it's kept.
pub const META_KEY: &str = "casino_chat";

#[derive(Default)]
pub struct ChatState {
    lines: VecDeque<ChatLine>,
    next: u64,
    loaded: bool,
    /// When each person last spoke (lately).
    said: HashMap<String, VecDeque<u64>>,
}

/// A line as it's kept: spaces for anything that isn't printable, runs of spaces as one,
/// trimmed, at most `LONGEST` characters.
pub fn clean(text: &str) -> String {
    let mut out = String::new();
    let mut space = false;
    for c in text.chars() {
        let c = if c.is_control() || c.is_whitespace() { ' ' } else { c };
        if c == ' ' {
            if !space && !out.is_empty() {
                out.push(' ');
            }
            space = true;
        } else {
            out.push(c);
            space = false;
        }
    }
    out.trim_end().chars().take(LONGEST).collect()
}

impl ChatState {
    /// What was kept, the first time it's needed.
    pub fn load(&mut self, saved: Option<String>) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        if let Some(lines) = saved.and_then(|s| serde_json::from_str::<Vec<ChatLine>>(&s).ok()) {
            self.next = lines.iter().map(|l| l.id).max().unwrap_or(0);
            self.lines = lines.into_iter().rev().take(KEPT).rev().collect();
        }
    }

    pub fn loaded(&self) -> bool {
        self.loaded
    }

    /// What's kept, to save.
    pub fn saved(&self) -> String {
        serde_json::to_string(&self.lines.iter().collect::<Vec<_>>()).unwrap_or_else(|_| "[]".into())
    }

    pub fn recent(&self) -> Vec<ChatLine> {
        self.lines.iter().cloned().collect()
    }

    /// Someone says something.
    pub fn say(&mut self, uid: &str, name: &str, text: &str, now: u64) -> Result<ChatLine, String> {
        let text = clean(text);
        if text.is_empty() {
            return Err("say something first".into());
        }
        if self.said.len() > 4_096 {
            self.said.retain(|_, t| t.back().is_some_and(|x| now.saturating_sub(*x) < WINDOW_MS));
        }
        let times = self.said.entry(uid.to_string()).or_default();
        while times.front().is_some_and(|t| now.saturating_sub(*t) >= WINDOW_MS) {
            times.pop_front();
        }
        if times.back().is_some_and(|t| now.saturating_sub(*t) < GAP_MS) || times.len() >= PER_WINDOW {
            return Err("slow down a little: a few seconds between lines".into());
        }
        times.push_back(now);
        self.next += 1;
        let line = ChatLine { id: self.next, uid: uid.to_string(), name: name.chars().take(40).collect(), text, at_ms: now };
        self.lines.push_back(line.clone());
        while self.lines.len() > KEPT {
            self.lines.pop_front();
        }
        Ok(line)
    }

    /// Takes a line down: someone's own, or anyone's for `moderator`.
    pub fn delete(&mut self, uid: &str, moderator: bool, id: u64) -> Result<(), String> {
        let i = self.lines.iter().position(|l| l.id == id).ok_or("that line's gone already")?;
        if self.lines[i].uid != uid && !moderator {
            return Err("only staff can take down someone else's line".into());
        }
        self.lines.remove(i);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_are_kept_clean_short_and_not_too_fast() {
        let mut c = ChatState::default();
        c.load(None);
        assert_eq!(clean("  hi\n\tthere\u{7}  you  "), "hi there you");
        assert!(c.say("a", "Ann", "   ", 0).is_err());
        let l = c.say("a", "Ann", &"x".repeat(1_000), 0).unwrap();
        assert_eq!(l.text.chars().count(), LONGEST);
        assert!(c.say("a", "Ann", "again", 100).is_err(), "too soon");
        assert!(c.say("b", "Bo", "hello", 100).is_ok(), "someone else isn't held up");
        // Eight in ten seconds at most (the first, at 0, among them).
        let mut t = 1_000;
        let mut ok = 1;
        for _ in 0..12 {
            if c.say("a", "Ann", "spam", t).is_ok() {
                ok += 1;
            }
            t += 700;
        }
        assert_eq!(ok, PER_WINDOW, "{ok} lines");
        // Kept: the last KEPT, ids going up; and back the same after saving.
        for i in 0..400u64 {
            let _ = c.say(&format!("u{i}"), "x", "line", 100_000 + i);
        }
        assert_eq!(c.recent().len(), KEPT);
        let mut again = ChatState::default();
        again.load(Some(c.saved()));
        assert_eq!(again.recent(), c.recent());
        let next = again.say("z", "Z", "after", 1_000_000).unwrap();
        assert!(next.id > c.recent().last().unwrap().id);
    }

    #[test]
    fn my_own_lines_come_down_and_staff_can_take_down_any() {
        let mut c = ChatState::default();
        c.load(None);
        let a = c.say("a", "Ann", "mine", 0).unwrap();
        let b = c.say("b", "Bo", "his", 0).unwrap();
        assert!(c.delete("a", false, b.id).is_err());
        c.delete("a", false, a.id).unwrap();
        c.delete("mod", true, b.id).unwrap();
        assert!(c.recent().is_empty());
        assert!(c.delete("a", false, a.id).is_err());
    }
}
