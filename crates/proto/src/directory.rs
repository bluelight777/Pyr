//! Short invite links, `https://pyr.gg/<code>`, like Discord's discord.gg ones.
//!
//! A community's server tells the invite directory at pyr.gg which of its invites open by
//! their code alone: an announcement signed with the server's TLS key, the key its invite
//! links pin, so only that server can list or take back codes for itself. pyr.gg turns a
//! code back into the full invite (`pyr://host/code/fp`) and shows the community's name on
//! its page, in the app and in link previews. The full link needs nobody: it keeps working
//! when pyr.gg is down or a server isn't listed there.

use serde::{Deserialize, Serialize};

/// Where short links live; the directory's API is under `<DIRECTORY>/api/v1/`.
pub const DIRECTORY: &str = "https://pyr.gg";
/// What a server signs: this, then the announcement's JSON.
pub const ANNOUNCE_CONTEXT: &[u8] = b"pyr directory announce v1\n";
/// Length of the codes servers make for their invites (31^8: about 850 billion).
pub const CODE_LEN: usize = 8;
/// Shortest and longest code (a community's own code can be as short as 3).
pub const CODE_MIN: usize = 3;
pub const CODE_MAX: usize = 32;
/// Most invites one community can list.
pub const MAX_CODES: usize = 1000;
/// Longest community name and inviter name the directory keeps (in characters).
pub const NAME_MAX: usize = 100;
pub const INVITER_MAX: usize = 64;
/// The signature schemes servers sign with (their TLS keys are one of these).
pub const SCHEME_ECDSA_P256: &str = "ecdsa-p256-sha256";
pub const SCHEME_ED25519: &str = "ed25519";

/// What codes are made of: lower-case letters and digits without look-alikes (no i, l, o, 0
/// or 1).
pub const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";

/// A short link's address.
pub fn short_url(code: &str) -> String {
    format!("{DIRECTORY}/{code}")
}

/// A new random code for an invite ([`CODE_LEN`] long), never one pyr.gg keeps for itself.
pub fn new_code() -> String {
    loop {
        let code = random_code(CODE_LEN);
        if !is_reserved(&code) {
            return code;
        }
    }
}

/// `len` random characters from [`ALPHABET`], each as likely as the next.
pub fn random_code(len: usize) -> String {
    let mut out = String::with_capacity(len);
    while out.len() < len {
        for b in crate::crypto::random_bytes::<32>() {
            // (248 is 8 × 31: the bytes above it would make the first letters likelier.)
            if b < 248 && out.len() < len {
                out.push(ALPHABET[(b % 31) as usize] as char);
            }
        }
    }
    out
}

/// The code in a short link (`https://pyr.gg/abc12345`, also without `https://`, with
/// `www.`, as `/invite/abc12345` or `/invite-abc12345`, and with anything after a `?` or
/// `#`), lower-cased. None for anything else, a full invite link included.
pub fn short_code(input: &str) -> Option<String> {
    let mut s = input.trim();
    for scheme in ["https://", "http://"] {
        if let Some(rest) = strip_ci(s, scheme) {
            s = rest;
            break;
        }
    }
    if let Some(rest) = strip_ci(s, "www.") {
        s = rest;
    }
    let host = DIRECTORY.split_once("://").map_or(DIRECTORY, |(_, h)| h);
    let s = strip_ci(s, host)?.strip_prefix('/')?;
    let s = s.split(['?', '#']).next().unwrap_or("");
    let s = s.strip_suffix('/').unwrap_or(s);
    let s = strip_ci(s, "invite/").or_else(|| strip_ci(s, "invite-")).unwrap_or(s);
    let code = s.to_ascii_lowercase();
    (is_code(&code) && !is_reserved(&code)).then_some(code)
}

/// `s` without `prefix` (in any case), if it starts with it.
fn strip_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    s.get(..prefix.len()).filter(|p| p.eq_ignore_ascii_case(prefix)).map(|_| &s[prefix.len()..])
}

/// Whether `code` can be a code: 3–32 lower-case letters, digits and hyphens, not starting or
/// ending with a hyphen.
pub fn is_code(code: &str) -> bool {
    let b = code.as_bytes();
    (CODE_MIN..=CODE_MAX).contains(&b.len())
        && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        && b[0] != b'-'
        && b[b.len() - 1] != b'-'
}

/// Pages of pyr.gg itself and names people would take for official: no community gets these.
const RESERVED: &[&str] = &[
    "about", "account", "admin", "alpha", "android", "anthropic", "api", "app", "apps", "assets", "beta", "blog", "bot", "bots",
    "claude", "css", "dev", "discord", "doc", "docs", "download", "downloads", "faq", "favicon", "firelit", "fonts", "get",
    "help", "home", "icons", "images", "img", "index", crate::legacy::APP_ID, "install", "invite", "invites", "ios", "js", "latest", "legal",
    "library", "linux", "login", "mac", "macos", "mail", "mod", "moderator", "mods", "news", "null", "official", "og",
    "owner", "plugin", "plugins", "privacy", "pyr", "register", "release", "releases", "robots", "root", "security", "server",
    "servers", "settings", "setup", "signin", "signup", "sitemap", "skin", "skins", "staff", "static", "status", "support",
    "team", "terms", "test", "undefined", "update", "updates", "vps", "windows", "www",
];
/// Beginnings kept the same way (`owner-` and `bot-` also mean something in invite codes).
const RESERVED_STARTS: &[&str] = &["owner", "bot-", "invite", "api-", "pyr", "firelit", "official", "support", "admin", "staff", "well-known"];

/// Whether pyr.gg keeps `code` for itself.
pub fn is_reserved(code: &str) -> bool {
    RESERVED.contains(&code) || RESERVED_STARTS.iter().any(|p| code.starts_with(p))
}

/// Why a community's own code can't be had, if it can't (it must also be free: the directory
/// says).
pub fn vanity_problem(code: &str) -> Option<&'static str> {
    if !is_code(code) {
        Some("Use 3 to 32 letters, digits and hyphens (not a hyphen first or last).")
    } else if is_reserved(code) {
        Some("That one is kept for pyr.gg itself.")
    } else {
        None
    }
}

/// What a server tells the directory: where it is, what it's called, and every invite that
/// opens by its code alone. Sent whole each time (the directory makes its list match).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Announce {
    /// When it was made (ms since 1970): the directory only takes one newer than the last.
    pub ts: i64,
    pub host: String,
    pub port: u16,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub members: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<u32>,
    #[serde(default)]
    pub codes: Vec<ListedCode>,
    /// The community's own code (`pyr.gg/<vanity>`), if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vanity: Option<String>,
    /// Its icon's SHA-256 (hex), if it has one (shown on its invite pages and link previews).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_sha: Option<String>,
    /// The icon itself (a PNG, base64url, at most [`ICON_MAX`] bytes): only when the
    /// directory asked for it (see [`AnnounceReply::want_icon`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// The biggest icon the directory takes (a server's icon is a PNG of at most 256×256).
pub const ICON_MAX: usize = 256 * 1024;

/// An icon's SHA-256, as [`Announce::icon_sha`] says it.
pub fn icon_sha(png: &[u8]) -> String {
    use sha2::Digest;
    sha2::Sha256::digest(png).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `bytes` is a PNG the directory takes for an icon.
pub fn icon_ok(bytes: &[u8]) -> bool {
    bytes.len() <= ICON_MAX && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// Where a short link's community icon is (a PNG), for its page and link previews.
pub fn icon_url(code: &str) -> String {
    format!("{DIRECTORY}/api/v1/icons/{code}.png")
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct ListedCode {
    pub code: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// The name of whoever made it, for "… invited you".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inviter: Option<String>,
}

/// An announcement as sent (`POST /api/v1/announce`).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct SignedAnnounce {
    /// The server's certificate (base64url DER): its SHA-256 is the fingerprint its invite
    /// links carry, and its key checks the signature.
    pub cert: String,
    /// [`SCHEME_ECDSA_P256`] or [`SCHEME_ED25519`].
    pub scheme: String,
    /// The [`Announce`] JSON, exactly as signed (after [`ANNOUNCE_CONTEXT`]).
    pub payload: String,
    /// base64url.
    pub sig: String,
}

/// The directory's answer to an announcement.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct AnnounceReply {
    /// Codes another community has already: not listed for this one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub taken: Vec<String>,
    /// Why the community's own code couldn't be had (taken, or kept for pyr.gg).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vanity_refused: Option<String>,
    /// Why the announcement wasn't taken at all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The directory doesn't have the icon in `icon_sha`: send it (with the next announcement).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub want_icon: bool,
}

/// What a code opens (`GET /api/v1/invites/<code>`).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Lookup {
    pub code: String,
    /// The community's name.
    pub name: String,
    /// The full invite, `pyr://host/code/fp`: what joins.
    pub link: String,
    #[serde(default)]
    pub members: Option<u32>,
    #[serde(default)]
    pub online: Option<u32>,
    #[serde(default)]
    pub inviter: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    /// The community's own code rather than one person's invite.
    #[serde(default)]
    pub vanity: bool,
    /// The community has an icon (at [`icon_url`]).
    #[serde(default)]
    pub icon: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_links_and_only_short_links() {
        for (input, code) in [
            ("https://pyr.gg/qrslmnov", "qrslmnov"),
            ("pyr.gg/qrslmnov", "qrslmnov"),
            ("  HTTPS://WWW.PYR.GG/QrsLmnov/  ", "qrslmnov"),
            ("http://pyr.gg/invite/qrslmnov", "qrslmnov"),
            ("https://pyr.gg/invite-qrslmnov", "qrslmnov"),
            ("https://pyr.gg/qrslmnov?utm=x#y", "qrslmnov"),
            ("https://pyr.gg/brethren-of-blood", "brethren-of-blood"),
            ("https://pyr.gg/abc", "abc"),
        ] {
            assert_eq!(short_code(input).as_deref(), Some(code), "{input}");
        }
        for input in [
            "https://pyr.gg/",
            "https://pyr.gg",
            "https://pyr.gg/#n=Camp&join=2.24.121.231/abc/eIbQ7wcA9qSj",
            "pyr://2.24.121.231/abc/eIbQ7wcA9qSj",
            "https://pyr.gg/library/index.json",
            "https://pyr.gg/latest.json",
            "https://pyr.gg/invite",
            "https://pyr.gg/download",
            "https://pyr.gg/ab",
            "https://pyr.gg/-abc",
            "https://pyr.gg/abc_def",
            "https://pyr.gg.evil.example/abc12345",
            "https://notpyr.gg/abc12345",
            "https://example.com/abc12345",
            "pyr.gg/€abc",
            "é",
        ] {
            assert_eq!(short_code(input), None, "{input}");
        }
        assert_eq!(short_url("qrslmnov"), "https://pyr.gg/qrslmnov");
        assert_eq!(short_code(&short_url("qrslmnov")).as_deref(), Some("qrslmnov"));
    }

    #[test]
    fn codes_and_own_codes() {
        assert!(is_code("abc") && is_code("a-b") && is_code(&"x".repeat(32)));
        assert!(!is_code("ab") && !is_code(&"x".repeat(33)) && !is_code("Abc") && !is_code("ab-") && !is_code("a b") && !is_code("ünï"));
        assert_eq!(vanity_problem("brethren"), None);
        assert!(vanity_problem("download").is_some() && vanity_problem("pyr-support").is_some() && vanity_problem("owner-x").is_some());
        assert!(vanity_problem("bot-abc").is_some() && vanity_problem("x").is_some() && vanity_problem("Caps").is_some());
        // New codes: 8 characters from the alphabet, never kept ones, and every character used.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..3000 {
            let c = new_code();
            assert!(c.len() == CODE_LEN && c.bytes().all(|b| ALPHABET.contains(&b)) && is_code(&c) && !is_reserved(&c), "{c}");
            seen.extend(c.bytes());
        }
        assert_eq!(seen.len(), ALPHABET.len());
        assert!(is_reserved("pyrabcde") && is_reserved("staffabc"), "kept beginnings a made code could have: new_code skips those");
        assert_eq!(random_code(40).len(), 40);
    }

    #[test]
    fn announcements_read_back() {
        let a = Announce {
            ts: 5,
            host: "2.24.121.231".into(),
            port: 7766,
            name: "Brethren of Blood".into(),
            members: Some(42),
            online: None,
            codes: vec![ListedCode { code: "qrslmnov".into(), expires_at: Some(9), inviter: Some("Blue".into()) }],
            vanity: Some("brethren".into()),
            icon_sha: Some(icon_sha(b"x")),
            icon: None,
        };
        let json = serde_json::to_string(&a).unwrap();
        assert!(!json.contains("online") && !json.contains("\"icon\""), "{json}");
        // (A directory from before icons reads it as ever.)
        assert!(json.contains("\"icon_sha\":\"2d711642b726b04401627ca9fbac32f5c8530fb1903cc4db02258717921a4881\""), "{json}");
        assert_eq!(serde_json::from_str::<Announce>(&json).unwrap(), a);
        let r: AnnounceReply = serde_json::from_str("{}").unwrap();
        assert_eq!(r, AnnounceReply::default());
    }
}
