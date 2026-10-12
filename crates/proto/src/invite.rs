//! Invite links: `ingle://host:port/CODE?fp=<base64url sha256(cert)>`.
//!
//! The fingerprint pins the server's self-signed TLS certificate, so connections are
//! authenticated without any certificate authority.

use crate::b64;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteLink {
    pub host: String,
    pub port: u16,
    pub code: Option<String>,
    pub fingerprint: [u8; 32],
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum InviteError {
    #[error("the link is missing the server fingerprint (?fp=...)")]
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
            if s.len() >= scheme.len() && s[..scheme.len()].eq_ignore_ascii_case(&scheme) {
                s = &s[scheme.len()..];
                break;
            }
        }
        let (rest, query) = match s.split_once('?') {
            Some((a, b)) => (a, Some(b)),
            None => (s, None),
        };
        let mut fp = None;
        for pair in query.unwrap_or("").split('&') {
            if let Some(v) = pair.strip_prefix("fp=") {
                fp = Some(b64::decode_array::<32>(v).ok_or(InviteError::BadFingerprint)?);
            }
        }
        let fingerprint = fp.ok_or(InviteError::MissingFingerprint)?;
        let (authority, path) = match rest.split_once('/') {
            Some((a, p)) => (a, p),
            None => (rest, ""),
        };
        let code = path.trim_matches('/');
        let code = if code.is_empty() { None } else { Some(code.to_string()) };
        let (host, port) = split_host_port(authority)?;
        Ok(InviteLink { host, port, code, fingerprint })
    }

    /// `host:port`, with IPv6 hosts bracketed.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn to_url(&self) -> String {
        let code = self.code.as_deref().unwrap_or("");
        format!("{}://{}/{}?fp={}", crate::APP_ID, self.authority(), code, b64::encode(&self.fingerprint))
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
    fn parse_and_format() {
        let fp = [7u8; 32];
        let link = InviteLink { host: "camp.example.com".into(), port: 7766, code: Some("abc123".into()), fingerprint: fp };
        let url = link.to_url();
        assert!(url.starts_with("ingle://camp.example.com:7766/abc123?fp="));
        assert_eq!(InviteLink::parse(&url).unwrap(), link);
        assert_eq!(InviteLink::parse(&format!("  {url}  ")).unwrap(), link);
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
        let fp = b64::encode(&[1u8; 32]);
        assert_eq!(InviteLink::parse(&format!("ingle://host:99999/x?fp={fp}")).unwrap_err(), InviteError::BadPort);
    }
}
