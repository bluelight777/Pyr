//! Spellcheck for the message box, like Discord's: a red squiggle under misspelled words, and
//! corrections when you right-click one. The dictionaries (Hunspell format; see
//! assets/dict/README.md) are built in and checked on this PC by spellbook, so nothing you type
//! goes anywhere, and Windows and Linux behave the same.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::mpsc;

use eframe::egui::{self, Color32, Pos2, Stroke, vec2};
use ingle_engine::state::ServerView;

pub struct Lang {
    pub code: &'static str,
    pub name: &'static str,
    aff: &'static str,
    dic: &'static str,
}

macro_rules! lang {
    ($code:literal, $name:literal) => {
        Lang {
            code: $code,
            name: $name,
            aff: include_str!(concat!("../assets/dict/", $code, ".aff")),
            dic: include_str!(concat!("../assets/dict/", $code, ".dic")),
        }
    };
}

/// The languages to pick from (right-click the message box → Languages).
pub const LANGS: &[Lang] = &[
    lang!("en-US", "English (United States)"),
    lang!("en-GB", "English (United Kingdom)"),
    lang!("es", "Español"),
    lang!("fr", "Français"),
];

/// A language by its code (English (United States) for one we don't have).
pub fn lang(code: &str) -> &'static Lang {
    LANGS.iter().find(|l| l.code == code).unwrap_or(&LANGS[0])
}

/// Chat words no dictionary has, never underlined.
const CHAT_WORDS: &[&str] = &[
    "afk", "ain't", "bc", "brb", "btw", "cuz", "dm", "dms", "dunno", "emote", "emotes", "ez", "fyi", "gg", "ggs", "gif",
    "gifs", "giphy", "gimme", "gl", "glhf", "gonna", "gotta", "hf", "hmu", "idc", "idk", "ikr", "imho", "imo", "ingle",
    "irl", "jk", "kinda", "lemme", "lmao", "lmfao", "lmk", "lol", "nah", "ngl", "noob", "np", "nvm", "omg", "omfg", "pls",
    "plz", "pog", "poggers", "rn", "rofl", "smh", "tbh", "tho", "thx", "ty", "wanna", "wp", "wtf", "xd", "y'all", "ya",
    "yea", "yeah", "yep",
];

enum Dict {
    Loading(mpsc::Receiver<Result<spellbook::Dictionary, String>>),
    Ready(spellbook::Dictionary),
    Failed,
}

/// The loaded dictionary, and what's been checked already (the box is looked at every frame).
#[derive(Default)]
pub struct Speller {
    code: &'static str,
    dict: Option<Dict>,
    seen: HashMap<String, bool>,
    /// Words that are right here though no dictionary has them: people's and roles' names,
    /// words you added, chat words. Built for `extra_for`.
    extra: HashSet<String>,
    extra_for: Option<(String, usize, usize, usize)>,
}

impl Speller {
    /// Use the dictionary for `code`. True once it's loaded: it loads in the background the
    /// first time (a fraction of a second), so typing never waits for it.
    pub fn ready(&mut self, code: &str, ctx: &egui::Context) -> bool {
        let lang = lang(code);
        if self.code != lang.code || self.dict.is_none() {
            self.code = lang.code;
            self.seen.clear();
            let (tx, rx) = mpsc::channel();
            let ctx = ctx.clone();
            let spawned = std::thread::Builder::new().name("ingle-spellcheck".into()).spawn(move || {
                let t = std::time::Instant::now();
                let d = spellbook::Dictionary::new(lang.aff, lang.dic).map_err(|e| e.to_string());
                tracing::info!("spellcheck: {} ready in {} ms", lang.code, t.elapsed().as_millis());
                let _ = tx.send(d);
                ctx.request_repaint();
            });
            self.dict = Some(if spawned.is_ok() { Dict::Loading(rx) } else { Dict::Failed });
        }
        if let Some(Dict::Loading(rx)) = &self.dict {
            self.dict = match rx.try_recv() {
                Ok(Ok(d)) => Some(Dict::Ready(d)),
                Ok(Err(e)) => {
                    tracing::warn!("spellcheck: couldn't read the {} dictionary: {e}", self.code);
                    Some(Dict::Failed)
                }
                Err(mpsc::TryRecvError::Empty) => return false,
                Err(mpsc::TryRecvError::Disconnected) => Some(Dict::Failed),
            };
        }
        matches!(self.dict, Some(Dict::Ready(_)))
    }

    /// Names on this server, and words you added, count as spelled right.
    pub fn know(&mut self, s: &ServerView, added: &[String]) {
        let stamp = (s.key.clone(), s.members.len(), s.roles.len(), added.len());
        if self.extra_for.as_ref() == Some(&stamp) {
            return;
        }
        self.extra_for = Some(stamp);
        let mut extra: HashSet<String> = CHAT_WORDS.iter().map(|w| w.to_string()).collect();
        let names = s.members.keys().map(|u| s.member_name(u)).chain(s.roles.iter().map(|r| r.name.clone()));
        for name in names.chain(added.iter().cloned()) {
            for w in name.split(|c: char| !c.is_alphanumeric() && !is_apostrophe(c)) {
                if !w.is_empty() {
                    extra.insert(normalize(w).to_lowercase());
                }
            }
        }
        self.extra = extra;
        self.seen.clear();
    }

    /// Whether `word` is spelled right.
    pub fn correct(&mut self, word: &str) -> bool {
        let w = normalize(word);
        if let Some(&ok) = self.seen.get(&w) {
            return ok;
        }
        let lower = w.to_lowercase();
        let ok = self.extra.contains(&lower)
            || lower.strip_suffix("'s").map(|base| self.extra.contains(base)).unwrap_or(false)
            || match &self.dict {
                Some(Dict::Ready(d)) => d.check(&w),
                _ => true,
            };
        if self.seen.len() > 20_000 {
            self.seen.clear();
        }
        self.seen.insert(w, ok);
        ok
    }

    /// Corrections for `word`, best first (at most five).
    pub fn suggest(&self, word: &str) -> Vec<String> {
        let Some(Dict::Ready(d)) = &self.dict else { return Vec::new() };
        let mut out = Vec::new();
        d.suggest(&normalize(word), &mut out);
        out.dedup();
        out.truncate(5);
        out
    }

    /// The misspelled words in `text`, as char ranges. The word you're in the middle of typing
    /// (the one ending at `typing_at`) waits until you've moved on from it.
    pub fn misspelled(&mut self, text: &str, typing_at: Option<usize>) -> Vec<Range<usize>> {
        let chars: Vec<char> = text.chars().collect();
        words(text)
            .into_iter()
            .filter(|r| Some(r.end) != typing_at)
            .filter(|r| {
                let w: String = chars[r.clone()].iter().collect();
                !self.correct(&w)
            })
            .collect()
    }
}

fn is_apostrophe(c: char) -> bool {
    c == '\'' || c == '’'
}

/// Curly apostrophes as the dictionaries spell them.
fn normalize(word: &str) -> String {
    word.replace('’', "'")
}

/// The words in `text` worth checking, as char ranges: runs of letters, with apostrophes
/// inside them ("don't"). Links, e-mail addresses, @mentions, #channels, :emoji:, /commands,
/// `code`, words with digits, single letters and other alphabets are left alone.
pub fn words(text: &str) -> Vec<Range<usize>> {
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let mut skip = vec![false; n];
    // Code between backticks (```blocks``` too: an odd number of them opens one).
    let mut code = false;
    for (i, c) in chars.iter().enumerate() {
        if *c == '`' {
            code = !code;
        }
        skip[i] = code || *c == '`';
    }
    // Links and addresses: the whole space-separated piece.
    let mut i = 0;
    while i < n {
        if chars[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && !chars[i].is_whitespace() {
            i += 1;
        }
        let piece: String = chars[start..i].iter().collect::<String>().to_lowercase();
        let dotted = chars[start..i].windows(3).any(|w| w[0].is_alphanumeric() && w[1] == '.' && w[2].is_alphanumeric());
        let email = piece.chars().skip(1).any(|c| c == '@') && piece.contains('.');
        if piece.contains("://") || piece.starts_with("www.") || email || dotted {
            skip[start..i].iter_mut().for_each(|s| *s = true);
        }
    }
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        if !chars[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < n
            && (chars[i].is_alphanumeric() || is_apostrophe(chars[i]) && chars.get(i + 1).map(|c| c.is_alphabetic()).unwrap_or(false))
        {
            i += 1;
        }
        let r = start..i;
        let before = start.checked_sub(1).map(|p| chars[p]);
        let after = chars.get(i).copied();
        let marked = matches!(before, Some('@' | '#' | ':' | '/' | '\\' | '<' | '&' | '$' | '%' | '_')) || after == Some('_');
        let word = &chars[r.clone()];
        let latin = word.iter().all(|c| (*c as u32) <= 0x24f || is_apostrophe(*c));
        if word.len() >= 2 && !marked && latin && !word.iter().any(|c| c.is_numeric()) && !skip[r.clone()].iter().any(|s| *s) {
            out.push(r);
        }
    }
    out
}

/// The checkable word at char position `at` (a cursor position, so its end counts too).
pub fn word_at(text: &str, at: usize) -> Option<Range<usize>> {
    words(text).into_iter().find(|r| r.start <= at && at <= r.end)
}

/// Red wavy lines under `bad` (char ranges of the galley's text), the galley drawn at `origin`.
pub fn paint_squiggles(painter: &egui::Painter, galley: &egui::Galley, origin: Pos2, bad: &[Range<usize>]) {
    if bad.is_empty() {
        return;
    }
    let stroke = Stroke::new(1.2_f32, Color32::from_rgb(0xf2, 0x4b, 0x4f));
    let mut idx = 0usize;
    for row in &galley.rows {
        // (left, right, baseline) of the run being underlined in this row.
        let mut run: Option<(f32, f32, f32)> = None;
        for g in &row.glyphs {
            if bad.iter().any(|r| r.contains(&idx)) {
                let x0 = row.pos.x + g.pos.x;
                let x1 = x0 + g.advance_width;
                let y = row.pos.y + g.pos.y;
                run = Some(match run {
                    Some((a, _, y)) => (a, x1, y),
                    None => (x0, x1, y),
                });
            } else if let Some((a, b, y)) = run.take() {
                squiggle(painter, origin + vec2(a, y + 3.0), b - a, stroke);
            }
            idx += 1;
        }
        if let Some((a, b, y)) = run.take() {
            squiggle(painter, origin + vec2(a, y + 3.0), b - a, stroke);
        }
        if row.ends_with_newline {
            idx += 1;
        }
    }
}

fn squiggle(painter: &egui::Painter, start: Pos2, width: f32, stroke: Stroke) {
    let mut pts = Vec::new();
    let mut x = 0.0;
    let mut up = false;
    while x <= width + 0.01 {
        pts.push(start + vec2(x, if up { -1.0 } else { 1.0 }));
        up = !up;
        x += 2.0;
    }
    if pts.len() >= 2 {
        painter.add(egui::Shape::line(pts, stroke));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(text: &str) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        words(text).into_iter().map(|r| chars[r].iter().collect()).collect()
    }

    #[test]
    fn picks_out_the_words_to_check() {
        assert_eq!(found("Hello wrld, don't panic!"), ["Hello", "wrld", "don't", "panic"]);
        assert_eq!(found("it’s fine"), ["it’s", "fine"]);
        // Left alone: mentions, channels, emoji codes, commands, links, addresses, code, digits,
        // single letters, other alphabets, snake_case.
        assert!(found("@Kainn #genral :smilee: /giphy https://exmple.com/pth www.foo.bar a@b.cd").is_empty());
        assert!(found("`codde` ```blokc``` h3llo a I привет some_thing exmple.com").is_empty(), "{:?}", found("`codde` ```blokc``` h3llo a I привет some_thing exmple.com"));
        assert_eq!(found("after `code` wrod"), ["after", "wrod"]);
    }

    #[test]
    fn finds_the_word_under_the_pointer() {
        let t = "say helo now";
        assert_eq!(word_at(t, 4), Some(4..8));
        assert_eq!(word_at(t, 8), Some(4..8));
        assert_eq!(word_at(t, 6), Some(4..8));
        assert_eq!(word_at("a  b", 2), None);
    }

    #[test]
    fn every_dictionary_loads_and_checks() {
        for l in LANGS {
            let d = spellbook::Dictionary::new(l.aff, l.dic).unwrap_or_else(|e| panic!("{}: {e}", l.code));
            let (good, bad) = match l.code {
                "es" => ("perro", "perrro"),
                "fr" => ("bonjour", "bonjourr"),
                "en-GB" => ("colour", "colur"),
                _ => ("color", "colur"),
            };
            assert!(d.check(good), "{}: {good}", l.code);
            assert!(!d.check(bad), "{}: {bad}", l.code);
        }
    }

    #[test]
    fn underlines_misspellings_but_not_names_or_the_word_being_typed() {
        let ctx = egui::Context::default();
        let mut sp = Speller::default();
        let t = std::time::Instant::now();
        while !sp.ready("en-US", &ctx) {
            assert!(t.elapsed().as_secs() < 30, "dictionary never loaded");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let mut s = ServerView::new("k".into(), "Camp".into(), String::new());
        s.members.insert(
            "u1".into(),
            ingle_proto::control::Member { user_id: "u1".into(), name: "Kainzor".into(), online: true, ..Default::default() },
        );
        sp.know(&s, &["Brethren".to_string()]);
        let text = "Kainzor's helo, the Brethren gg wrld teh";
        let chars: Vec<char> = text.chars().collect();
        let bad: Vec<String> = sp.misspelled(text, None).into_iter().map(|r| chars[r].iter().collect()).collect();
        assert_eq!(bad, ["helo", "wrld", "teh"]);
        // Still typing "teh": not yet.
        let bad = sp.misspelled(text, Some(chars.len()));
        assert_eq!(bad.len(), 2);
        let sug = sp.suggest("helo");
        assert!(sug.iter().any(|w| w == "hello"), "{sug:?}");
        assert!(sp.suggest("teh").iter().any(|w| w == "the"));
        // Capitals at the start of a sentence are fine; wrong ones aren't.
        assert!(sp.correct("Hello") && sp.correct("HELLO") && !sp.correct("hELLO"));
    }
}
