//! Shared wire protocol for Ingle.
//!
//! * `control` – JSON control messages exchanged on the long-lived control stream
//!   (login, text chat, channels, voice state, E2EE key exchange).
//! * `media` – compact binary headers for audio datagrams and video frames.
//! * `crypto` – identities (Ed25519), login proof, and end-to-end media encryption.
//! * `invite` – invite links that pin the server's TLS certificate.
//! * `udp` (feature `quic`) – QUIC endpoints with a plain-UDP fallback.

pub mod b64;
pub mod control;
pub mod crypto;
pub mod framing;
pub mod invite;
pub mod media;
pub mod release;
pub mod sound;
#[cfg(feature = "quic")]
pub mod udp;

/// Product name shown to people. Change this (and the crate/binary names) to rebrand.
pub const APP_NAME: &str = "Ingle";
/// Lower-case identifier used for the URL scheme, config directories and ALPN.
pub const APP_ID: &str = "ingle";
/// QUIC application protocol identifier.
pub const ALPN: &[u8] = b"ingle/1";
/// Bumped whenever the control protocol changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;
/// First bytes a client writes on its control stream (QUIC only reveals a stream once
/// data flows on it, and this lets the server reject non-Ingle clients early).
pub const CONTROL_PREAMBLE: &[u8; 4] = b"ING\x01";
/// Default UDP port for servers.
pub const DEFAULT_PORT: u16 = 7766;
/// Maximum length (in chars) of a chat message.
pub const MAX_MESSAGE_CHARS: usize = 4000;
/// Maximum size of one control frame.
pub const MAX_CONTROL_FRAME: usize = 1 << 20;
/// Maximum size of one encoded video frame.
pub const MAX_VIDEO_FRAME: usize = 8 << 20;

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
