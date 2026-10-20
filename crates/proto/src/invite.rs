//! Invite links: `pyr://host/CODE/FP`, where FP is the start of the base64url SHA-256 of
//! the server's certificate (the port is left out when it's the usual 7766).
//!
//! The fingerprint pins the server's self-signed TLS certificate, so connections are
//! authenticated without any certificate authority. Links carry its first 9 bytes (72 bits):
//! plenty to stop an impostor server (it would need a certificate whose hash starts the same,
//! ~2^72 tries), and the app learns and pins the full one on first connect. The older long
//! form `pyr://host:port/CODE?fp=<all 32 bytes>` is still accepted.

use crate::b64;

/// Where invite links open on the web: pyr.gg, which offers Pyr to anyone
/// who doesn't have it yet and a button that opens the invite in it. A web link can be clicked
/// anywhere links can (chat apps don't make `pyr://` links clickable).
pub const WEB_INVITE: &str = "https://pyr.gg/";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteLink {
    pub host: String,
    pub port: u16,
    pub code: Option<String>,
    /// The certificate's SHA-256; only the first `fp_len` bytes are known from a short link.
    pub fingerprint: [u8; 32],
    pub fp_len: usize,
}

/// Bytes of the fingerprint in a short link (12 characters).
pub const SHORT_FP: usize = 9;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InviteError {
    #[error("the link is incomplete (it should look like pyr://host/code/xxxxxxxxxxxx)")]
    MissingFingerprint,
    #[error("the server fingerprint in the link is not valid")]
    BadFingerprint,
    #[error("the link is missing a host name")]
    MissingHost,
    #[error("the port number in the link is not valid")]
    BadPort,
}

impl InviteLink {
    pub fn parse(input: &str) -> Result<InviteLink, InviteError> {
        // A web link (`https://…/#n=<server>&join=<host>/<code>/<fp>`): the invite inside it.
        if let Some(inner) = web_inner(input) {
            return InviteLink::parse(&inner);
        }
        let mut s = input.trim();
        // (And links made before the rename: see `legacy`.)
        for scheme in [format!("{}://", crate::APP_ID), crate::legacy::SCHEME.to_string(), "https://".into(), "http://".into()] {
            // (`get`: pasted text can have a multi-byte character right where the scheme ends.)
            if s.get(..scheme.len()).is_some_and(|p| p.eq_ignore_ascii_case(&scheme)) {
                s = &s[scheme.len()..];
                break;
            }
        }
        let (rest, query) = match s.split_once('?') {
            Some((a, b)) => (a, Some(b)),
            None => (s, None),
        };
        let mut fp: Option<([u8; 32], usize)> = None;
        for pair in query.unwrap_or("").split('&') {
            if let Some(v) = pair.strip_prefix("fp=") {
                fp = Some((b64::decode_array::<32>(v).ok_or(InviteError::BadFingerprint)?, 32));
            }
        }
        let (authority, path) = match rest.split_once('/') {
            Some((a, p)) => (a, p),
            None => (rest, ""),
        };
        let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if fp.is_none() && parts.len() >= 2 {
            let short = parts.pop().unwrap_or_default();
            let bytes = b64::decode(short).map_err(|_| InviteError::BadFingerprint)?;
            if !(SHORT_FP..=32).contains(&bytes.len()) {
                return Err(InviteError::BadFingerprint);
            }
            let mut full = [0u8; 32];
            full[..bytes.len()].copy_from_slice(&bytes);
            fp = Some((full, bytes.len()));
        }
        let (fingerprint, fp_len) = fp.ok_or(InviteError::MissingFingerprint)?;
        let code = parts.join("/");
        let code = if code.is_empty() || code == "-" { None } else { Some(code) };
        let (host, port) = split_host_port(authority)?;
        Ok(InviteLink { host, port, code, fingerprint, fp_len })
    }

    /// A link for a server whose whole fingerprint is known.
    pub fn new(host: String, port: u16, code: Option<String>, fingerprint: [u8; 32]) -> InviteLink {
        InviteLink { host, port, code, fingerprint, fp_len: 32 }
    }

    /// The link has the whole fingerprint (not just its start).
    pub fn is_full(&self) -> bool {
        self.fp_len >= 32
    }

    /// Whether a certificate with this full fingerprint is the server this link means.
    pub fn matches(&self, full: &[u8; 32]) -> bool {
        full[..self.fp_len] == self.fingerprint[..self.fp_len]
    }

    /// `host:port`, with IPv6 hosts bracketed.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The short form, `pyr://host/CODE/FP` (the port only when it isn't 7766).
    pub fn to_url(&self) -> String {
        let host = if self.port == crate::DEFAULT_PORT {
            if self.host.contains(':') { format!("[{}]", self.host) } else { self.host.clone() }
        } else {
            self.authority()
        };
        let fp = b64::encode(&self.fingerprint[..SHORT_FP.min(self.fp_len)]);
        match self.code.as_deref() {
            Some(code) => format!("{}://{}/{}/{}", crate::APP_ID, host, code, fp),
            // A link with no code still needs the fingerprint as the last part.
            None => format!("{}://{}/-/{}", crate::APP_ID, host, fp),
        }
    }

    /// Shortens a link a server handed out (older servers make the long form).
    pub fn shorten(link: &str) -> String {
        InviteLink::parse(link).map(|l| l.to_url()).unwrap_or_else(|_| link.to_string())
    }

    /// The web form, `https://pyr.gg/#n=<server name>&join=<host>/<code>/<fp>`.
    /// Everything after the `#` stays in the browser (browsers never send it to the website),
    /// where the page reads it to show the server's name and an Open invite in Pyr button.
    pub fn to_web_url(&self, server_name: &str) -> String {
        let link = self.to_url();
        let rest = link.strip_prefix(&format!("{}://", crate::APP_ID)).unwrap_or(&link);
        // (Brackets around an IPv6 address would end the link in some chat apps.)
        let rest = rest.replace('[', "%5B").replace(']', "%5D");
        let name: String = server_name.trim().chars().filter(|c| !c.is_control()).take(60).collect();
        if name.is_empty() {
            format!("{WEB_INVITE}#join={rest}")
        } else {
            format!("{WEB_INVITE}#n={}&join={rest}", percent_encode(&name))
        }
    }

    /// The web form of a link a server handed out (it's left as it is if it isn't a link).
    pub fn web(link: &str, server_name: &str) -> String {
        InviteLink::parse(link).map(|l| l.to_web_url(server_name)).unwrap_or_else(|_| link.to_string())
    }
}

/// The `pyr://` link inside a web invite link, if `input` is one (see
/// [`InviteLink::to_web_url`]). Only a link with `join=` after its `#` counts.
fn web_inner(input: &str) -> Option<String> {
    let s = input.trim();
    let (_, fragment) = s.split_once('#')?;
    let at = fragment.find("join=")?;
    // `join=` is the last part, so it's either the start or follows `&`.
    if at > 0 && !fragment[..at].ends_with('&') {
        return None;
    }
    let rest = percent_decode(&fragment[at + "join=".len()..]);
    let scheme = format!("{}://", crate::APP_ID);
    let rest = rest.strip_prefix(&scheme).or_else(|| rest.strip_prefix(crate::legacy::SCHEME)).unwrap_or(&rest);
    (!rest.is_empty()).then(|| format!("{scheme}{rest}"))
}

/// Everything but letters, digits and `-._~` as %XX (UTF-8).
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// %XX back to bytes (anything that isn't a valid escape is kept as it is).
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |c: u8| (c as char).to_digit(16);
        if b[i] == b'%' && i + 2 < b.len() {
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn split_host_port(authority: &str) -> Result<(String, u16), InviteError> {
    if authority.is_empty() {
        return Err(InviteError::MissingHost);
    }
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, after) = rest.split_once(']').ok_or(InviteError::MissingHost)?;
        let port = match after.strip_prefix(':') {
            Some(p) => p.parse().map_err(|_| InviteError::BadPort)?,
            None => crate::DEFAULT_PORT,
        };
        return Ok((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => {
            if h.is_empty() {
                return Err(InviteError::MissingHost);
            }
            Ok((h.to_string(), p.parse().map_err(|_| InviteError::BadPort)?))
        }
        _ => Ok((authority.to_string(), crate::DEFAULT_PORT)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn odd_text_is_no_link_not_a_crash() {
        // A multi-byte character right where "pyr://" would end.
        for t in ["pyr:/€abc", "http:/€", "é", "pyr:€€€€€€", "https:/\u{1F525}x"] {
            let _ = InviteLink::parse(t);
        }
    }

    #[test]
    fn short_links() {
        let mut fp = [0u8; 32];
        for (i, b) in fp.iter_mut().enumerate() {
            *b = i as u8 * 7 + 3;
        }
        let link = InviteLink::new("2.24.121.231".into(), 7766, Some("xykyyqhf".into()), fp);
        let url = link.to_url();
        assert_eq!(url.len(), "pyr://2.24.121.231/xykyyqhf/".len() + 12, "{url}");
        let back = InviteLink::parse(&url).unwrap();
        assert_eq!((back.host.as_str(), back.port, back.code.as_deref(), back.fp_len), ("2.24.121.231", 7766, Some("xykyyqhf"), SHORT_FP));
        assert!(back.matches(&fp) && !back.is_full());
        let mut other = fp;
        other[3] ^= 1;
        assert!(!back.matches(&other));
        other = fp;
        other[20] ^= 1;
        assert!(back.matches(&other), "only the first bytes are pinned by a short link");
        // Other ports stay in the link; the long form still works and shortens.
        let l = InviteLink::new("camp.example.com".into(), 9000, Some("abc".into()), fp);
        assert!(l.to_url().starts_with("pyr://camp.example.com:9000/abc/"));
        let long = format!("pyr://2.24.121.231:7766/xykyyqhf?fp={}", b64::encode(&fp));
        let p = InviteLink::parse(&long).unwrap();
        assert!(p.is_full() && p.matches(&fp));
        assert_eq!(InviteLink::shorten(&long), url);
        // No code.
        let l = InviteLink::new("::1".into(), 7766, None, fp);
        let p = InviteLink::parse(&l.to_url()).unwrap();
        assert_eq!((p.host.as_str(), p.code.as_deref()), ("::1", None));
    }

    #[test]
    fn ipv6_and_defaults() {
        let fp = b64::encode(&[1u8; 32]);
        let l = InviteLink::parse(&format!("pyr://[::1]:9000/x?fp={fp}")).unwrap();
        assert_eq!(l.host, "::1");
        assert_eq!(l.port, 9000);
        assert_eq!(l.authority(), "[::1]:9000");
        let l = InviteLink::parse(&format!("192.168.1.5?fp={fp}")).unwrap();
        assert_eq!(l.port, crate::DEFAULT_PORT);
        assert_eq!(l.code, None);
    }

    #[test]
    fn web_links_carry_the_invite_and_the_name() {
        let mut fp = [0u8; 32];
        fp[0] = 9;
        let link = InviteLink::new("2.24.121.231".into(), 7766, Some("xykyyqhf".into()), fp);
        let web = link.to_web_url("Brethren of Blood");
        assert_eq!(web, format!("https://pyr.gg/#n=Brethren%20of%20Blood&join=2.24.121.231/xykyyqhf/{}", b64::encode(&fp[..SHORT_FP])));
        // Pasted into Join a server, it's the same invite.
        assert_eq!(InviteLink::parse(&web).unwrap(), InviteLink::parse(&link.to_url()).unwrap());
        assert_eq!(InviteLink::web(&link.to_url(), "Camp"), link.to_web_url("Camp"));
        // Names that need escaping, no name, another port, IPv6, and the whole link escaped.
        let odd = link.to_web_url("Rock & Roll #1 🔥");
        assert!(odd.contains("#n=Rock%20%26%20Roll%20%231%20%F0%9F%94%A5&join="), "{odd}");
        assert_eq!(InviteLink::parse(&odd).unwrap().code.as_deref(), Some("xykyyqhf"));
        assert!(link.to_web_url("  ").ends_with(&format!("#join=2.24.121.231/xykyyqhf/{}", b64::encode(&fp[..SHORT_FP]))));
        let v6 = InviteLink::new("::1".into(), 9000, Some("abc".into()), fp);
        let w = v6.to_web_url("x");
        assert!(w.contains("join=%5B::1%5D:9000/abc/"), "{w}");
        let back = InviteLink::parse(&w).unwrap();
        assert_eq!((back.host.as_str(), back.port), ("::1", 9000));
        let escaped = format!("https://pyr.gg/#join=pyr%3A%2F%2F2.24.121.231%2Fxykyyqhf%2F{}", b64::encode(&fp[..SHORT_FP]));
        assert_eq!(InviteLink::parse(&escaped).unwrap().host, "2.24.121.231");
        // Not web invites: a plain https link to a server, and a page link without join=.
        assert!(web_inner("https://example.com/x#notjoin=1").is_none());
        assert!(web_inner("https://example.com/x#a=b&join=").is_none());
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }

    #[test]
    fn errors() {
        assert_eq!(InviteLink::parse("pyr://host:1/x").unwrap_err(), InviteError::MissingFingerprint);
        assert_eq!(InviteLink::parse("pyr://host:1/x?fp=zz").unwrap_err(), InviteError::BadFingerprint);
        assert_eq!(InviteLink::parse("pyr://host/x/abc").unwrap_err(), InviteError::BadFingerprint);
        let fp = b64::encode(&[1u8; 32]);
        assert_eq!(InviteLink::parse(&format!("pyr://host:99999/x?fp={fp}")).unwrap_err(), InviteError::BadPort);
    }

    /// Links made before the rename keep working, in the app's form and the web's (new links
    /// are `pyr://` and pyr.gg).
    #[test]
    fn links_from_before_the_rename_still_work() {
        use crate::legacy::{SCHEME, WEB_INVITE};
        let fp = [9u8; 32];
        let new = InviteLink::new("play.example.com".into(), 7766, Some("abc".into()), fp);
        let short = b64::encode(&fp[..SHORT_FP]);
        for old in [
            format!("{SCHEME}play.example.com/abc/{short}"),
            format!("{}play.example.com/abc/{short}", SCHEME.to_uppercase()),
            format!("{WEB_INVITE}#n=Game%20Night&join=play.example.com/abc/{short}"),
            format!("{WEB_INVITE}#join={SCHEME}play.example.com/abc/{short}"),
        ] {
            let l = InviteLink::parse(&old).unwrap_or_else(|e| panic!("{old}: {e}"));
            assert_eq!((l.host.as_str(), l.port, l.code.as_deref()), ("play.example.com", 7766, Some("abc")), "{old}");
            assert!(l.matches(&fp), "{old}");
        }
        assert!(new.to_url().starts_with("pyr://"));
        assert!(new.to_web_url("Game Night").starts_with("https://pyr.gg/#n=Game%20Night&join=play.example.com/abc/"));
    }
}
