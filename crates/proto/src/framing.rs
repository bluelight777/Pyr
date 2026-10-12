//! Length-prefixed JSON frames for the control stream: `u32 big-endian length || JSON`.

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Serialize a value into one frame (length prefix included).
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    let json = serde_json::to_vec(value).expect("control messages always serialize");
    let mut out = Vec::with_capacity(4 + json.len());
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(&json);
    out
}

/// Read the body length from a 4-byte prefix, rejecting oversized frames.
pub fn body_len(prefix: [u8; 4]) -> Result<usize, FrameError> {
    let n = u32::from_be_bytes(prefix) as usize;
    if n > crate::MAX_CONTROL_FRAME {
        Err(FrameError::TooLarge(n))
    } else {
        Ok(n)
    }
}

/// Parse a frame body.
pub fn decode<T: DeserializeOwned>(body: &[u8]) -> Result<T, FrameError> {
    serde_json::from_slice(body).map_err(|e| FrameError::Json(e.to_string()))
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("control frame too large ({0} bytes)")]
    TooLarge(usize),
    #[error("malformed control frame: {0}")]
    Json(String),
}

#[cfg(test)]
mod tests {
    use crate::control::{ClientFrame, Request};

    #[test]
    fn frame_roundtrip() {
        let f = ClientFrame { id: 7, req: Request::Ping { t: 42 } };
        let bytes = super::encode(&f);
        let len = super::body_len(bytes[..4].try_into().unwrap()).unwrap();
        assert_eq!(len, bytes.len() - 4);
        let back: ClientFrame = super::decode(&bytes[4..]).unwrap();
        assert_eq!(back.id, 7);
        assert!(matches!(back.req, Request::Ping { t: 42 }));
    }
}
