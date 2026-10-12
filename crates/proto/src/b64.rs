//! URL-safe, unpadded base64 helpers (used for keys, signatures and binary blobs in JSON).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

pub fn encode(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

pub fn decode(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    URL_SAFE_NO_PAD.decode(s.trim().trim_end_matches('='))
}

/// Decode into a fixed-size array.
pub fn decode_array<const N: usize>(s: &str) -> Option<[u8; N]> {
    let v = decode(s).ok()?;
    <[u8; N]>::try_from(v.as_slice()).ok()
}

/// `#[serde(with = "ingle_proto::b64::bytes")]` for `Vec<u8>` fields.
pub mod bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::encode(v))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        super::decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn roundtrip() {
        let data = [0u8, 1, 2, 250, 251, 255];
        let s = super::encode(&data);
        assert!(!s.contains('='));
        assert_eq!(super::decode(&s).unwrap(), data);
        assert_eq!(super::decode_array::<6>(&s).unwrap(), data);
        assert!(super::decode_array::<5>(&s).is_none());
    }
}
