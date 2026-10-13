//! Soundboard sound files ([`crate::control::SOUNDBOARD_FEATURE`]): Opus at 48 kHz in 20 ms
//! frames, in a container small enough for the server to check without decoding anything.
//!
//! ```text
//! "INGS" | version u8 (1) | channels u8 (1 or 2) | frames u16 LE
//! then for each frame: length u16 LE | one Opus packet
//! ```
//!
//! Every app plays the file itself when someone plays a sound, so everyone hears it at full
//! quality (not through anyone's microphone).

pub const MAGIC: &[u8; 4] = b"INGS";
pub const VERSION: u8 = 1;
/// One frame (one Opus packet) of sound.
pub const FRAME_MS: u32 = 20;
/// The longest a sound can be (Discord's can be 5.2 s).
pub const MAX_MS: u32 = 10_000;
pub const MAX_FRAMES: usize = (MAX_MS / FRAME_MS) as usize;
/// The biggest a sound file can be (10 s of stereo at 128 kbps is about 160 KB).
pub const MAX_BYTES: usize = 384 * 1024;
/// Opus's own limit for one packet.
const MAX_PACKET: usize = 1275;
const HEADER: usize = 8;

/// What a sound file holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoundFormat {
    pub channels: u8,
    pub frames: usize,
}

impl SoundFormat {
    pub fn length_ms(&self) -> u32 {
        self.frames as u32 * FRAME_MS
    }
}

/// The Opus packets in a sound file, if it's a well-formed one within the limits.
pub fn packets(b: &[u8]) -> Option<(SoundFormat, Vec<&[u8]>)> {
    if b.len() < HEADER || b.len() > MAX_BYTES || &b[..4] != MAGIC || b[4] != VERSION {
        return None;
    }
    let channels = b[5];
    let frames = u16::from_le_bytes([b[6], b[7]]) as usize;
    if !(1..=2).contains(&channels) || frames == 0 || frames > MAX_FRAMES {
        return None;
    }
    let mut out = Vec::with_capacity(frames);
    let mut i = HEADER;
    for _ in 0..frames {
        let len = u16::from_le_bytes([*b.get(i)?, *b.get(i + 1)?]) as usize;
        i += 2;
        if len == 0 || len > MAX_PACKET || i + len > b.len() {
            return None;
        }
        out.push(&b[i..i + len]);
        i += len;
    }
    // Nothing after the last frame.
    (i == b.len()).then_some((SoundFormat { channels, frames }, out))
}

/// Whether `b` is a sound file Ingle can play (what the server checks on upload).
pub fn check(b: &[u8]) -> Option<SoundFormat> {
    packets(b).map(|(f, _)| f)
}

/// A sound file from Opus packets (20 ms each, 48 kHz).
pub fn write(channels: u8, packets: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER + packets.iter().map(|p| p.len() + 2).sum::<usize>());
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(channels);
    out.extend_from_slice(&(packets.len() as u16).to_le_bytes());
    for p in packets {
        out.extend_from_slice(&(p.len() as u16).to_le_bytes());
        out.extend_from_slice(p);
    }
    out
}

/// A sound file's id on a server: the first 16 bytes of SHA-256 of it, in hex. Named by
/// content, so an app knows whether a server already has a sound before sending it there.
pub fn sound_id(file: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(b"ingle-sound\0");
    h.update(file);
    h.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Built-in sounds every app has (`PlaySound` names them `builtin:<id>`).
pub const BUILTIN_PREFIX: &str = "builtin:";

/// Whether `id` could be a built-in sound's (apps that don't know it just don't play it).
pub fn is_builtin(id: &str) -> bool {
    id.strip_prefix(BUILTIN_PREFIX)
        .is_some_and(|n| (1..=32).contains(&n.len()) && n.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_limits() {
        let p = vec![vec![1u8, 2, 3], vec![4u8; 200]];
        let f = write(2, &p);
        let (fmt, got) = packets(&f).unwrap();
        assert_eq!(fmt, SoundFormat { channels: 2, frames: 2 });
        assert_eq!(fmt.length_ms(), 40);
        assert_eq!(got, vec![&p[0][..], &p[1][..]]);

        // Cut short, padded, wrong magic, too long, empty packets: refused.
        assert!(check(&f[..f.len() - 1]).is_none());
        let mut padded = f.clone();
        padded.push(0);
        assert!(check(&padded).is_none());
        let mut bad = f.clone();
        bad[0] = b'X';
        assert!(check(&bad).is_none());
        let long: Vec<Vec<u8>> = (0..MAX_FRAMES + 1).map(|_| vec![9u8]).collect();
        assert!(check(&write(1, &long)).is_none());
        assert!(check(&write(1, &long[..MAX_FRAMES])).is_some());
        assert!(check(&write(1, &[vec![]])).is_none());
        assert!(check(&write(3, &p)).is_none());
    }

    #[test]
    fn ids_follow_the_content() {
        let a = write(1, &[vec![1, 2, 3]]);
        let b = write(1, &[vec![1, 2, 4]]);
        assert_eq!(sound_id(&a), sound_id(&a.clone()));
        assert_ne!(sound_id(&a), sound_id(&b));
        assert_eq!(sound_id(&a).len(), 32);
    }

    #[test]
    fn builtin_names() {
        assert!(is_builtin("builtin:sad-trombone"));
        assert!(!is_builtin("builtin:"));
        assert!(!is_builtin("builtin:../etc"));
        assert!(!is_builtin("Builtin:quack"));
        assert!(!is_builtin("quack"));
    }
}
