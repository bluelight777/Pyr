//! Binary media headers.
//!
//! Audio travels in QUIC datagrams; every encoded video frame travels on its own
//! unidirectional QUIC stream (so a stale frame can be abandoned without blocking newer ones).
//!
//! The first [`AAD_LEN`] bytes of both headers are authenticated by the end-to-end
//! encryption; the session field is written by the server and is not authenticated
//! (a wrong value simply makes decryption fail).
//!
//! ```text
//! audio datagram (14-byte header):
//!   0 kind=0xA1 | 1 track | 2 flags | 3 reserved | 4..6 epoch | 6..10 seq | 10..14 session | ciphertext
//! video frame (22-byte header):
//!   0 kind=0xB1 | 1 track | 2 flags | 3 codec | 4..6 epoch | 6..10 seq | 10..14 session
//!   | 14..18 capture time (ms, sender clock) | 18..22 ciphertext length | ciphertext
//! ```

pub const KIND_AUDIO: u8 = 0xA1;
pub const KIND_VIDEO: u8 = 0xB1;
pub const AUDIO_HEADER_LEN: usize = 14;
pub const VIDEO_HEADER_LEN: usize = 22;
/// Bytes of the header covered by the AEAD tag.
pub const AAD_LEN: usize = 10;
/// Byte offset of the session id in both header kinds.
pub const SESSION_OFFSET: usize = 10;

pub mod track {
    pub const MIC: u8 = 0;
    pub const CAMERA: u8 = 1;
    pub const SCREEN: u8 = 2;
    pub const SCREEN_AUDIO: u8 = 3;
}

pub mod flags {
    /// Audio: first packet after silence (receiver should re-buffer).
    pub const TALKSPURT: u8 = 0b0000_0001;
    /// Video: frame is an IDR/keyframe.
    pub const KEYFRAME: u8 = 0b0000_0001;
}

pub const CODEC_H264: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioHeader {
    pub track: u8,
    pub flags: u8,
    pub epoch: u16,
    pub seq: u32,
    pub session: u32,
}

impl AudioHeader {
    pub fn write(&self, out: &mut [u8]) {
        out[0] = KIND_AUDIO;
        out[1] = self.track;
        out[2] = self.flags;
        out[3] = 0;
        out[4..6].copy_from_slice(&self.epoch.to_be_bytes());
        out[6..10].copy_from_slice(&self.seq.to_be_bytes());
        out[10..14].copy_from_slice(&self.session.to_be_bytes());
    }

    pub fn to_bytes(&self) -> [u8; AUDIO_HEADER_LEN] {
        let mut b = [0u8; AUDIO_HEADER_LEN];
        self.write(&mut b);
        b
    }

    pub fn parse(buf: &[u8]) -> Option<AudioHeader> {
        if buf.len() < AUDIO_HEADER_LEN || buf[0] != KIND_AUDIO {
            return None;
        }
        Some(AudioHeader {
            track: buf[1],
            flags: buf[2],
            epoch: u16::from_be_bytes([buf[4], buf[5]]),
            seq: u32::from_be_bytes([buf[6], buf[7], buf[8], buf[9]]),
            session: u32::from_be_bytes([buf[10], buf[11], buf[12], buf[13]]),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VideoHeader {
    pub track: u8,
    pub flags: u8,
    pub codec: u8,
    pub epoch: u16,
    pub seq: u32,
    pub session: u32,
    pub ts_ms: u32,
    pub len: u32,
}

impl VideoHeader {
    pub fn is_keyframe(&self) -> bool {
        self.flags & flags::KEYFRAME != 0
    }

    pub fn write(&self, out: &mut [u8]) {
        out[0] = KIND_VIDEO;
        out[1] = self.track;
        out[2] = self.flags;
        out[3] = self.codec;
        out[4..6].copy_from_slice(&self.epoch.to_be_bytes());
        out[6..10].copy_from_slice(&self.seq.to_be_bytes());
        out[10..14].copy_from_slice(&self.session.to_be_bytes());
        out[14..18].copy_from_slice(&self.ts_ms.to_be_bytes());
        out[18..22].copy_from_slice(&self.len.to_be_bytes());
    }

    pub fn to_bytes(&self) -> [u8; VIDEO_HEADER_LEN] {
        let mut b = [0u8; VIDEO_HEADER_LEN];
        self.write(&mut b);
        b
    }

    pub fn parse(buf: &[u8]) -> Option<VideoHeader> {
        if buf.len() < VIDEO_HEADER_LEN || buf[0] != KIND_VIDEO {
            return None;
        }
        let rd = |i: usize| u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        Some(VideoHeader {
            track: buf[1],
            flags: buf[2],
            codec: buf[3],
            epoch: u16::from_be_bytes([buf[4], buf[5]]),
            seq: rd(6),
            session: rd(10),
            ts_ms: rd(14),
            len: rd(18),
        })
    }
}

/// Overwrite the (unauthenticated) session id in an audio or video header.
pub fn set_session(buf: &mut [u8], session: u32) {
    buf[SESSION_OFFSET..SESSION_OFFSET + 4].copy_from_slice(&session.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_header_roundtrip() {
        let h = AudioHeader { track: 3, flags: flags::TALKSPURT, epoch: 513, seq: 0xdead_beef, session: 77 };
        let b = h.to_bytes();
        assert_eq!(AudioHeader::parse(&b).unwrap(), h);
        let mut b2 = b;
        set_session(&mut b2, 9);
        assert_eq!(AudioHeader::parse(&b2).unwrap().session, 9);
        assert_eq!(&b2[..AAD_LEN], &b[..AAD_LEN], "session must not be inside the AAD");
        assert!(AudioHeader::parse(&b[..13]).is_none());
    }

    #[test]
    fn video_header_roundtrip() {
        let h = VideoHeader {
            track: track::SCREEN,
            flags: flags::KEYFRAME,
            codec: CODEC_H264,
            epoch: 1,
            seq: 12345,
            session: 4,
            ts_ms: 99,
            len: 100_000,
        };
        let b = h.to_bytes();
        let p = VideoHeader::parse(&b).unwrap();
        assert_eq!(p, h);
        assert!(p.is_keyframe());
    }
}
