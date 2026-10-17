//! Skins: how Pyr looks and sounds. A skin is a folder with a `skin.json` that sets Pyr's
//! colors, and can bring a picture to show behind the chat, fonts, and its own sounds for
//! Pyr's notifications:
//!
//! ```json
//! { "name": "Blood Moon", "version": "1.0", "summary": "Deep red and black.", "author": "Blue",
//!   "colors": { "main": "#1a0d0f", "side": "#140a0b", "ember": "#d4202c" },
//!   "background": { "file": "background.webp", "show": 0.3, "under": "chat" },
//!   "fonts": { "body": "fonts/Body.ttf", "bold": "fonts/Bold.ttf", "scale": 1.0 },
//!   "sounds": { "message": "sounds/message.ings" } }
//! ```
//!
//! Colors left out stay Campfire's (Pyr's own). Skins are sent in through the Claude
//! connector, looked at and approved like plugins, and listed in the Pyr Library for anyone
//! to use. What's in one is a stranger's work: everything here is checked before Pyr uses
//! it (sizes, kinds of file, a picture's real size, sound files' frames), and the app checks a
//! font parses before it's handed to the text renderer.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::control::{image_dimensions_ok, sniff_image};
use crate::library::valid_path;

/// The file every skin has, at the top of its folder.
pub const MANIFEST: &str = "skin.json";
/// Biggest single file (so each fits one control frame), biggest skin, most files.
pub const MAX_FILE: usize = 700_000;
pub const MAX_TOTAL: usize = 4_000_000;
pub const MAX_FILES: usize = 24;
/// skin.json, and text files (a font's license, a readme).
pub const MAX_TEXT: usize = 64_000;
/// Largest side of the picture.
pub const MAX_PICTURE_SIDE: u32 = 4096;

/// The colors a skin can set (Pyr's palette), and where each shows.
pub const COLORS: &[(&str, &str)] = &[
    ("rail", "the strip of server icons on the far left"),
    ("side", "the channel list and the member list"),
    ("side_dark", "the bars under the channel list (your name, voice)"),
    ("main", "the chat"),
    ("elev", "cards, boxes to type in, and buttons"),
    ("elev2", "a button or card under the pointer"),
    ("hover", "a row under the pointer"),
    ("line", "lines between things"),
    ("shadow_line", "the darker line under headers"),
    ("text", "headings and names"),
    ("body", "message text"),
    ("muted", "secondary text"),
    ("dim", "the faintest text (times, hints)"),
    ("ember", "the accent: main buttons, what's selected"),
    ("ember2", "the accent's lighter shade (highlights)"),
    ("glow", "speaking rings and glows"),
    ("live", "LIVE, alerts and unread badges"),
    ("online", "online dots"),
    ("owner", "the owner's crown"),
    ("admin", "admins' names"),
    ("link", "links in messages"),
    ("stage", "behind video and screen shares"),
    ("tile", "video tiles"),
    ("code_bg", "code blocks"),
    ("selection", "selected text"),
];

/// Pyr's own colors (Campfire), in [`COLORS`]' order.
pub const CAMPFIRE: [[u8; 3]; 25] = [
    [0x15, 0x11, 0x0f],
    [0x1e, 0x19, 0x16],
    [0x1a, 0x15, 0x12],
    [0x26, 0x20, 0x1c],
    [0x2f, 0x28, 0x23],
    [0x3a, 0x31, 0x2b],
    [0x2a, 0x23, 0x1f],
    [0x3b, 0x32, 0x2c],
    [0x14, 0x10, 0x0e],
    [0xee, 0xe6, 0xdf],
    [0xe6, 0xdd, 0xd5],
    [0xa8, 0x99, 0x8c],
    [0x7d, 0x70, 0x67],
    [0xff, 0x7a, 0x3d],
    [0xff, 0xb3, 0x47],
    [0xff, 0xa9, 0x4d],
    [0xe5, 0x48, 0x4d],
    [0x6c, 0xc0, 0x70],
    [0xf5, 0xb0, 0x41],
    [0xec, 0x8f, 0x63],
    [0xff, 0xad, 0x7a],
    [0x10, 0x0d, 0x0b],
    [0x1d, 0x18, 0x15],
    [0x1a, 0x15, 0x12],
    [0x8a, 0x40, 0x1c],
];

/// Pyr's notification sounds a skin can replace (as [`crate::sound`] files: Opus, 10 s at
/// most), and when each plays.
pub const SOUNDS: &[(&str, &str)] = &[
    ("message", "a new message"),
    ("join", "someone joins your voice channel"),
    ("leave", "someone leaves it"),
    ("stream_start", "someone starts sharing their screen"),
    ("stream_stop", "they stop"),
    ("watch_start", "you start watching a stream"),
    ("watch_stop", "you stop watching"),
    ("ring", "someone's calling you"),
    ("ringback", "you're calling someone and it's ringing"),
];

/// What `skin.json` says. Unknown keys are left alone when a skin is used (a newer skin on an
/// older Pyr), and pointed out by [`check`].
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct Manifest {
    pub name: String,
    pub version: String,
    pub summary: String,
    pub author: String,
    /// Palette name → "#rrggbb".
    pub colors: BTreeMap<String, String>,
    pub background: Option<Background>,
    pub fonts: Fonts,
    /// Sound name → file.
    pub sounds: BTreeMap<String, String>,
}

/// A picture behind the chat (or the whole window), seen through Pyr's colors.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Background {
    pub file: String,
    /// How much of the picture shows through: 0.1 (a hint) to 0.6.
    pub show: f32,
    /// "chat" (behind the chat only) or "window" (behind the channel and member lists too).
    pub under: String,
}

impl Default for Background {
    fn default() -> Background {
        Background { file: String::new(), show: 0.3, under: "chat".into() }
    }
}

impl Background {
    pub const SHOW: std::ops::RangeInclusive<f32> = 0.1..=0.6;

    pub fn whole_window(&self) -> bool {
        self.under == "window"
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Fonts {
    /// For text (TrueType or OpenType).
    pub body: Option<String>,
    /// For headings and names (the body font is used if there's none).
    pub bold: Option<String>,
    /// Size next to Pyr's own font: 0.8 to 1.25 (fonts differ in how big they look).
    pub scale: f32,
}

impl Default for Fonts {
    fn default() -> Fonts {
        Fonts { body: None, bold: None, scale: 1.0 }
    }
}

impl Fonts {
    pub const SCALE: std::ops::RangeInclusive<f32> = 0.8..=1.25;
}

/// "#rrggbb" → RGB.
pub fn parse_color(s: &str) -> Option<[u8; 3]> {
    let h = s.trim().strip_prefix('#')?;
    if h.len() != 6 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(h, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn color_hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

impl Manifest {
    /// The whole palette: the skin's colors, Campfire's for the rest (in [`COLORS`]' order).
    pub fn palette(&self) -> [[u8; 3]; 25] {
        let mut p = CAMPFIRE;
        for (i, (name, _)) in COLORS.iter().enumerate() {
            if let Some(c) = self.colors.get(*name).and_then(|v| parse_color(v)) {
                p[i] = c;
            }
        }
        p
    }

    /// Every file it uses (picture, fonts, sounds), with what each is for.
    pub fn uses(&self) -> Vec<(&str, Use)> {
        let mut out = Vec::new();
        if let Some(b) = &self.background {
            out.push((b.file.as_str(), Use::Picture));
        }
        for f in [&self.fonts.body, &self.fonts.bold].into_iter().flatten() {
            out.push((f.as_str(), Use::Font));
        }
        for f in self.sounds.values() {
            out.push((f.as_str(), Use::Sound));
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    Picture,
    Font,
    Sound,
}

/// The palette index of a color name.
pub fn color_index(name: &str) -> Option<usize> {
    COLORS.iter().position(|(n, _)| *n == name)
}

/// WCAG contrast between two colors (1 to 21).
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    fn lum(c: [u8; 3]) -> f32 {
        let ch = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])
    }
    let (x, y) = (lum(a), lum(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// A font file's kind from its first bytes: TrueType or OpenType (not a collection, WOFF or
/// anything else).
pub fn is_font(b: &[u8]) -> bool {
    b.len() > 12 && (b.starts_with(&[0, 1, 0, 0]) || b.starts_with(b"OTTO") || b.starts_with(b"true"))
}

/// Text files a skin may carry (a font's license, a readme): name and contents.
fn is_text(path: &str, b: &[u8]) -> bool {
    let lower = path.to_ascii_lowercase();
    (lower.ends_with(".txt") || lower.ends_with(".md")) && b.len() <= MAX_TEXT && std::str::from_utf8(b).is_ok()
}

/// A picture of the skin for the Library (optional).
pub fn is_preview(path: &str) -> bool {
    matches!(path, "preview.png" | "preview.jpg" | "preview.webp")
}

/// What checking a skin found: what's wrong (it can't be used or sent in until it's fixed),
/// and what's worth a look.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub manifest: Manifest,
    pub problems: Vec<String>,
    pub notes: Vec<String>,
}

impl Report {
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

/// Checks a skin's files (path, bytes): the same checks wherever a skin goes (the connector
/// before sending, the server taking it in, the owner's app, the Library tool, the app before
/// using it). Fonts are only told by their first bytes here; parsing one is up to the app.
pub fn check(files: &[(String, Vec<u8>)]) -> Report {
    let mut r = Report::default();
    let mut total = 0usize;
    let mut paths = std::collections::HashSet::new();
    if files.len() > MAX_FILES {
        r.problems.push(format!("A skin has {MAX_FILES} files at most (this one has {}).", files.len()));
    }
    for (path, bytes) in files {
        if !valid_path(path) {
            r.problems.push(format!("{path:?}: names are letters, digits, dots, dashes and underscores, in at most three folders deep."));
        }
        if !paths.insert(path.as_str()) {
            r.problems.push(format!("{path}: in it twice."));
        }
        if bytes.len() > MAX_FILE {
            r.problems.push(format!("{path}: {} KB; a file can be {} KB at most.", bytes.len() / 1000, MAX_FILE / 1000));
        }
        total += bytes.len();
    }
    if total > MAX_TOTAL {
        r.problems.push(format!("The skin is {} KB; {} KB at most.", total / 1000, MAX_TOTAL / 1000));
    }
    let find = |p: &str| files.iter().find(|(path, _)| path == p).map(|(_, b)| b.as_slice());
    let Some(raw) = find(MANIFEST) else {
        r.problems.push("There's no skin.json at the top of the skin's folder.".into());
        return r;
    };
    if raw.len() > MAX_TEXT {
        r.problems.push(format!("skin.json is {} KB; {} KB at most.", raw.len() / 1000, MAX_TEXT / 1000));
        return r;
    }
    let value: serde_json::Value = match serde_json::from_slice(raw) {
        Ok(v) => v,
        Err(e) => {
            r.problems.push(format!("skin.json isn't readable JSON: {e}"));
            return r;
        }
    };
    unknown_keys(&value, &mut r.problems);
    let m: Manifest = match serde_json::from_value(value) {
        Ok(m) => m,
        Err(e) => {
            r.problems.push(format!("skin.json: {e}"));
            return r;
        }
    };
    let name = m.name.trim();
    if name.is_empty() || name.chars().count() > 40 || name.chars().any(|c| c.is_control()) {
        r.problems.push("The skin needs a name (40 characters at most).".into());
    }
    if m.version.trim().is_empty() || m.version.len() > 20 {
        r.problems.push("The skin needs a version, like 1.0 (20 characters at most).".into());
    }
    if m.summary.chars().count() > 200 {
        r.problems.push("The summary is one line: 200 characters at most.".into());
    }
    if m.author.chars().count() > 80 {
        r.problems.push("The author is 80 characters at most.".into());
    }
    for (k, v) in &m.colors {
        if color_index(k).is_none() {
            r.problems.push(format!("colors: there's no color called {k:?} (see the list in the guide)."));
        } else if parse_color(v).is_none() {
            r.problems.push(format!("colors.{k}: {v:?} isn't a color written as #rrggbb."));
        }
    }
    // Text people read has to stand out from what it's on.
    let p = m.palette();
    let at = |n: &str| p[color_index(n).expect("palette name")];
    for (fg, bg, min, what) in [
        ("body", "main", 4.5, "Message text"),
        ("text", "main", 4.5, "Names and headings"),
        ("text", "side", 4.5, "Channel names"),
        ("muted", "main", 3.0, "Secondary text"),
    ] {
        let c = contrast(at(fg), at(bg));
        if c < min {
            r.problems.push(format!("{what} ({fg}) on {bg} is too hard to read: contrast {c:.1}, it needs {min} at least."));
        }
    }
    if contrast(at("dim"), at("main")) < 2.0 {
        r.notes.push("dim on main is very faint (times and hints use it).".into());
    }
    if let Some(b) = &m.background {
        if !Background::SHOW.contains(&b.show) {
            r.problems.push(format!("background.show is {}: it goes from 0.1 (a hint) to 0.6.", b.show));
        }
        if b.under != "chat" && b.under != "window" {
            r.problems.push(format!("background.under is \"chat\" or \"window\", not {:?}.", b.under));
        }
    }
    if !Fonts::SCALE.contains(&m.fonts.scale) {
        r.problems.push(format!("fonts.scale is {}: it goes from 0.8 to 1.25.", m.fonts.scale));
    }
    for k in m.sounds.keys() {
        if !SOUNDS.iter().any(|(n, _)| n == k) {
            r.problems.push(format!("sounds: Pyr has no sound called {k:?} (see the list in the guide)."));
        }
    }
    // Every file it names is there and is what it's said to be.
    let uses = m.uses();
    for (path, what) in &uses {
        let Some(b) = find(path) else {
            r.problems.push(format!("{path} is named in skin.json but isn't in the skin."));
            continue;
        };
        match what {
            Use::Picture => match sniff_image(b) {
                Some("image/png" | "image/jpeg" | "image/webp") if image_dimensions_ok(b) => {
                    if let Some((w, h)) = crate::control::image_size(b) {
                        if w > MAX_PICTURE_SIDE || h > MAX_PICTURE_SIDE {
                            r.problems.push(format!("{path}: {w}×{h}; {MAX_PICTURE_SIDE} pixels a side at most."));
                        }
                    }
                }
                _ => r.problems.push(format!("{path}: the picture has to be a PNG, JPEG or WebP.")),
            },
            Use::Font => {
                if !is_font(b) {
                    r.problems.push(format!("{path}: fonts are TrueType or OpenType files (.ttf or .otf)."));
                }
            }
            Use::Sound => {
                if crate::sound::packets(b).is_none() {
                    r.problems.push(format!("{path}: not a Pyr sound file (the Claude connector makes one from an MP3, WAV, OGG or FLAC)."));
                }
            }
        }
    }
    // And nothing else rides along but text (a font's license) and a preview picture.
    for (path, b) in files {
        if path == MANIFEST || uses.iter().any(|(p, _)| p == path) {
            continue;
        }
        if is_preview(path) {
            if !matches!(sniff_image(b), Some("image/png" | "image/jpeg" | "image/webp")) || !image_dimensions_ok(b) {
                r.problems.push(format!("{path}: the preview has to be a PNG, JPEG or WebP picture."));
            }
            continue;
        }
        if !is_text(path, b) {
            r.problems.push(format!("{path}: skin.json doesn't use it (only text files, like a font's license, and preview.png can come along)."));
        }
    }
    if m.fonts.body.is_some() || m.fonts.bold.is_some() {
        let licensed = files.iter().any(|(p, b)| {
            let (name, text) = (p.to_ascii_lowercase(), String::from_utf8_lossy(b).to_ascii_lowercase());
            is_text(p, b) && (["licen", "ofl", "copying"].iter().any(|w| name.contains(w)) || text.contains("license") || text.contains("licence"))
        });
        if !licensed {
            r.notes.push("Fonts usually come with a license that has to travel with them (OFL.txt, say): add it if yours does.".into());
        }
    }
    r.manifest = m;
    r
}

/// Keys skin.json doesn't know (a typo would otherwise be quietly ignored).
fn unknown_keys(v: &serde_json::Value, problems: &mut Vec<String>) {
    let known = |obj: &serde_json::Value, keys: &[&str], at: &str, problems: &mut Vec<String>| {
        if let Some(o) = obj.as_object() {
            for k in o.keys() {
                if !keys.contains(&k.as_str()) {
                    problems.push(format!("skin.json{at}: {k:?} isn't something a skin sets."));
                }
            }
        }
    };
    known(v, &["name", "version", "summary", "author", "colors", "background", "fonts", "sounds"], "", problems);
    known(&v["background"], &["file", "show", "under"], " background", problems);
    known(&v["fonts"], &["body", "bold", "scale"], " fonts", problems);
}

/// A new skin's skin.json: Campfire's colors written out, to change.
pub fn starter(name: &str, author: &str) -> String {
    let colors: serde_json::Map<String, serde_json::Value> =
        COLORS.iter().zip(CAMPFIRE).map(|((n, _), c)| (n.to_string(), serde_json::Value::String(color_hex(c)))).collect();
    let doc = serde_json::json!({
        "name": name,
        "version": "1.0",
        "summary": "",
        "author": author,
        "colors": colors,
    });
    serde_json::to_string_pretty(&doc).unwrap_or_default() + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        b.extend_from_slice(&w.to_be_bytes());
        b.extend_from_slice(&h.to_be_bytes());
        b.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        b
    }

    fn sound() -> Vec<u8> {
        crate::sound::write(1, &[vec![0xf8, 0xff, 0xfe]])
    }

    fn skin(json: &str, more: Vec<(&str, Vec<u8>)>) -> Vec<(String, Vec<u8>)> {
        let mut v = vec![(MANIFEST.to_string(), json.as_bytes().to_vec())];
        v.extend(more.into_iter().map(|(p, b)| (p.to_string(), b)));
        v
    }

    #[test]
    fn campfire_itself_passes() {
        let r = check(&skin(&starter("Campfire", "Firelit Games"), vec![]));
        assert!(r.ok(), "{:?}", r.problems);
        assert_eq!(r.manifest.palette(), CAMPFIRE);
        assert_eq!(COLORS.len(), CAMPFIRE.len());
    }

    #[test]
    fn a_whole_skin_passes_and_its_colours_count() {
        let json = r##"{ "name": "Blood Moon", "version": "1.0", "summary": "Red and black.", "author": "Blue",
            "colors": { "main": "#1a0d0f", "ember": "#d4202c" },
            "background": { "file": "bg.png", "show": 0.3, "under": "window" },
            "fonts": { "body": "fonts/Body.ttf", "scale": 1.1 },
            "sounds": { "message": "sounds/message.ings", "ring": "sounds/ring.ings" } }"##;
        let font = [vec![0, 1, 0, 0], vec![0; 40]].concat();
        let files = skin(
            json,
            vec![
                ("bg.png", png(1920, 1080)),
                ("fonts/Body.ttf", font),
                ("fonts/OFL.txt", b"SIL Open Font License".to_vec()),
                ("sounds/message.ings", sound()),
                ("sounds/ring.ings", sound()),
                ("preview.png", png(640, 400)),
            ],
        );
        let r = check(&files);
        assert!(r.ok(), "{:?}", r.problems);
        assert!(r.notes.is_empty(), "{:?}", r.notes);
        let p = r.manifest.palette();
        assert_eq!(p[color_index("main").unwrap()], [0x1a, 0x0d, 0x0f]);
        assert_eq!(p[color_index("side").unwrap()], CAMPFIRE[1], "left out: Campfire's");
        assert!(r.manifest.background.as_ref().unwrap().whole_window());
    }

    #[test]
    fn what_a_skin_cant_be() {
        let bad = |json: &str, more: Vec<(&str, Vec<u8>)>| {
            let r = check(&skin(json, more));
            assert!(!r.ok(), "{json} passed");
            r.problems.join(" | ")
        };
        let base = |extra: &str| if extra.is_empty() { r#"{ "name": "X", "version": "1" }"#.to_string() } else { format!(r#"{{ "name": "X", "version": "1", {extra} }}"#) };
        assert!(bad(&base(r##""colors": { "mian": "#000000" }"##), vec![]).contains("mian"));
        assert!(bad(&base(r##""colors": { "main": "red" }"##), vec![]).contains("#rrggbb"));
        assert!(bad(&base(r#""colours": {}"#), vec![]).contains("colours"));
        // Unreadable: grey text on grey.
        assert!(bad(&base(r##""colors": { "body": "#555555", "main": "#444444" }"##), vec![]).contains("hard to read"));
        // Files named but missing, or not what they say.
        assert!(bad(&base(r#""background": { "file": "bg.png" }"#), vec![]).contains("isn't in the skin"));
        assert!(bad(&base(r#""background": { "file": "bg.png" }"#), vec![("bg.png", b"GIF89a....".to_vec())]).contains("PNG, JPEG or WebP"));
        assert!(bad(&base(r#""background": { "file": "bg.png" }"#), vec![("bg.png", png(9000, 10))]).contains("PNG, JPEG or WebP"));
        assert!(bad(&base(r#""background": { "file": "bg.png", "show": 0.9 }"#), vec![("bg.png", png(10, 10))]).contains("0.6"));
        assert!(bad(&base(r#""fonts": { "body": "a.ttf" }"#), vec![("a.ttf", b"MZ not a font at all".to_vec())]).contains("TrueType"));
        assert!(bad(&base(r#""sounds": { "message": "m.mp3" }"#), vec![("m.mp3", b"ID3 mp3".to_vec())]).contains("Pyr sound"));
        assert!(bad(&base(r#""sounds": { "boom": "m.ings" }"#), vec![("m.ings", sound())]).contains("boom"));
        // Nothing else rides along.
        assert!(bad(&base(""), vec![("run.exe", b"MZ".to_vec())]).contains("doesn't use it"));
        assert!(bad(&base(""), vec![("../x.txt", b"x".to_vec())]).contains("folders deep"));
        assert!(bad(&base(""), vec![("big.txt", vec![b'a'; MAX_FILE + 1])]).contains("at most"));
        assert!(bad(r#"{ "version": "1" }"#, vec![]).contains("name"));
        assert!(bad("not json", vec![]).contains("JSON"));
        assert!(!check(&[]).ok());
    }

    #[test]
    fn fonts_without_a_licence_get_a_note() {
        let json = r#"{ "name": "X", "version": "1", "fonts": { "bold": "B.otf" } }"#;
        let r = check(&skin(json, vec![("B.otf", [b"OTTO".to_vec(), vec![0; 40]].concat())]));
        assert!(r.ok(), "{:?}", r.problems);
        assert!(r.notes.iter().any(|n| n.contains("license")));
    }

    #[test]
    fn contrast_is_wcags() {
        assert!((contrast([0, 0, 0], [255, 255, 255]) - 21.0).abs() < 0.01);
        assert!((contrast([10, 20, 30], [10, 20, 30]) - 1.0).abs() < 0.001);
        assert_eq!(parse_color("#FF7a3D"), Some([0xff, 0x7a, 0x3d]));
        assert_eq!(parse_color("ff7a3d"), None);
        assert_eq!(color_hex([0xff, 0x7a, 0x3d]), "#ff7a3d");
    }
}
