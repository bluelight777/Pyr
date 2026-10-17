//! Shared wire protocol for Pyr.
//!
//! * `control` – JSON control messages exchanged on the long-lived control stream
//!   (login, text chat, channels, voice state, E2EE key exchange).
//! * `media` – compact binary headers for audio datagrams and video frames.
//! * `crypto` – identities (Ed25519), login proof, and end-to-end media encryption.
//! * `invite` – invite links that pin the server's TLS certificate.
//! * `udp` (feature `quic`) – QUIC endpoints with a plain-UDP fallback.

pub mod ab;
pub mod b64;
pub mod control;
pub mod crypto;
pub mod framing;
pub mod handshake;
pub mod invite;
pub mod library;
pub mod media;
pub mod release;
pub mod skin;
pub mod sound;
#[cfg(feature = "quic")]
pub mod udp;
pub mod words;

/// Product name shown to people. Change this (and the crate/binary names) to rebrand.
pub const APP_NAME: &str = "Pyr";
/// Lower-case identifier used for the URL scheme and config directories.
pub const APP_ID: &str = "pyr";
/// What Pyr was called before 0.8.0. Still understood: `ingle://` invite links, `INGLE_*`
/// settings in the environment, and the old folders (copied over on the first start, see
/// `pyr_engine::config::migrate_from_ingle`).
pub const OLD_APP_NAME: &str = "Ingle";
pub const OLD_APP_ID: &str = "ingle";
/// QUIC application protocol identifier (unchanged by the rename: every Pyr and Ingle speaks
/// it, and so do their servers; the same goes for the labels in `crypto` and `handshake`).
pub const ALPN: &[u8] = b"ingle/1";
/// Bumped whenever the control protocol changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;
/// First bytes a client writes on its control stream (QUIC only reveals a stream once
/// data flows on it, and this lets the server reject non-Pyr clients early).
pub const CONTROL_PREAMBLE: &[u8; 4] = b"ING\x01";
/// Default UDP port for servers.
pub const DEFAULT_PORT: u16 = 7766;
/// Maximum length (in chars) of a chat message.
pub const MAX_MESSAGE_CHARS: usize = 4000;
/// Maximum size of one control frame.
pub const MAX_CONTROL_FRAME: usize = 1 << 20;
/// Maximum size of one encoded video frame.
pub const MAX_VIDEO_FRAME: usize = 8 << 20;

/// A setting from the environment: `PYR_<name>`, or else what Pyr called it before 0.8.0,
/// `INGLE_<name>`.
pub fn env_var(name: &str) -> Option<String> {
    std::env::var(format!("PYR_{name}")).ok().or_else(|| std::env::var(format!("INGLE_{name}")).ok())
}

/// Whether `PYR_<name>` (or `INGLE_<name>`) is set at all.
pub fn env_set(name: &str) -> bool {
    std::env::var_os(format!("PYR_{name}")).is_some() || std::env::var_os(format!("INGLE_{name}")).is_some()
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
