//! Invite links: `ingle://host/CODE/FP`, where FP is the start of the base64url SHA-256 of
//! the server's certificate (the port is left out when it's the usual 7766).
//!
//! The fingerprint pins the server's self-signed TLS certificate, so connections are
//! authenticated without any certificate authority. Links carry its first 9 bytes (72 bits):
//! plenty to stop an impostor server (it would need a certificate whose hash starts the same,
//! ~2^72 tries), and the app learns and pins the full one on first connect. The older long
//! form `ingle://host:port/CODE?fp=<all 32 bytes>` is still accepted.

use crate::b64;

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
    #[error("the link is incomplete (it should look like ingle://host/code/xxxxxxxxxxxx)")]
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
        let mut s = input.trim();
        for scheme in [format!("{}://", crate::APP_ID), "https://".into(), "http://".into()] {
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

    /// The short form, `ingle://host/CODE/FP` (the port only when it isn't 7766).
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
        // A multi-byte character right where "ingle://" would end.
        for t in ["ingle:/€abc", "http:/€", "é", "ingle:€€€€€€", "https:/\u{1F525}x"] {
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
        assert_eq!(url.len(), "ingle://2.24.121.231/xykyyqhf/".len() + 12, "{url}");
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
        assert!(l.to_url().starts_with("ingle://camp.example.com:9000/abc/"));
        let long = format!("ingle://2.24.121.231:7766/xykyyqhf?fp={}", b64::encode(&fp));
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
        let l = InviteLink::parse(&format!("ingle://[::1]:9000/x?fp={fp}")).unwrap();
        assert_eq!(l.host, "::1");
        assert_eq!(l.port, 9000);
        assert_eq!(l.authority(), "[::1]:9000");
        let l = InviteLink::parse(&format!("192.168.1.5?fp={fp}")).unwrap();
        assert_eq!(l.port, crate::DEFAULT_PORT);
        assert_eq!(l.code, None);
    }

    #[test]
    fn errors() {
        assert_eq!(InviteLink::parse("ingle://host:1/x").unwrap_err(), InviteError::MissingFingerprint);
        assert_eq!(InviteLink::parse("ingle://host:1/x?fp=zz").unwrap_err(), InviteError::BadFingerprint);
        assert_eq!(InviteLink::parse("ingle://host/x/abc").unwrap_err(), InviteError::BadFingerprint);
        let fp = b64::encode(&[1u8; 32]);
        assert_eq!(InviteLink::parse(&format!("ingle://host:99999/x?fp={fp}")).unwrap_err(), InviteError::BadPort);
    }
}
