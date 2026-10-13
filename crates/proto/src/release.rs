//! Signed update manifests: what `firelitgames.com/ingle/latest.json` holds.
//!
//! The app only installs an update whose manifest is signed with the Firelit release key
//! (an Ed25519 key kept offline by whoever publishes releases) and whose download matches the
//! SHA-256 in that manifest. So someone who breaks into the website can't push their own
//! program to everyone's PC.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::b64;
use crate::crypto::{self, Identity};

/// Signed messages start with this, so a release signature can't be mistaken for anything else.
const CONTEXT: &[u8] = b"ingle release manifest v1\n";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Asset {
    pub url: String,
    /// Lowercase hex SHA-256 of the file.
    pub sha256: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Manifest {
    pub version: String,
    /// What's new, shown in the app.
    #[serde(default)]
    pub notes: String,
    /// The Windows setup (.exe, installed silently).
    #[serde(default)]
    pub windows: Option<Asset>,
    /// The Linux package (.deb).
    #[serde(default)]
    pub linux_deb: Option<Asset>,
    /// Every version released so far (so an app can tell how many updates it's behind).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub released: Vec<String>,
}

impl Manifest {
    /// How many released versions are newer than `current` (at least 1 when this one is).
    pub fn missed(&self, current: &str) -> usize {
        let n = self.released.iter().filter(|v| newer(v, current)).count();
        n.max(newer(&self.version, current) as usize)
    }
}

/// The file on the website: the manifest exactly as signed, and its signature.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Signed {
    pub manifest: String,
    pub sig: String,
}

fn message(manifest: &str) -> Vec<u8> {
    [CONTEXT, manifest.as_bytes()].concat()
}

pub fn sign(key: &Identity, manifest: &Manifest) -> Signed {
    let text = serde_json::to_string_pretty(manifest).expect("manifest serializes");
    let sig = key.sign(&message(&text));
    Signed { manifest: text, sig: b64::encode(&sig) }
}

/// Checks the signature against `release_key` (the public key, as a user id) and parses.
pub fn open(signed_json: &str, release_key: &str) -> Result<Manifest, String> {
    let signed: Signed = serde_json::from_str(signed_json).map_err(|e| format!("not a release file: {e}"))?;
    let sig = b64::decode(&signed.sig).map_err(|_| "bad signature encoding".to_string())?;
    if !crypto::verify(release_key, &message(&signed.manifest), &sig) {
        return Err("the release isn't signed with the Firelit release key".into());
    }
    serde_json::from_str(&signed.manifest).map_err(|e| format!("bad manifest: {e}"))
}

/// Whether version `a` is newer than `b` ("0.10.1" > "0.9.3"; missing parts count as 0).
pub fn newer(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.trim().trim_start_matches('v').split(['.', '-', '+']).take(3).map(|p| p.parse().unwrap_or(0)).collect()
    };
    let (mut x, mut y) = (parse(a), parse(b));
    x.resize(3, 0);
    y.resize(3, 0);
    x > y
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_manifest_round_trips_and_rejects_tampering() {
        let key = Identity::generate();
        let m = Manifest {
            version: "0.2.1".into(),
            notes: "Fixes".into(),
            released: vec!["0.1.0".into(), "0.2.0".into(), "0.3.0".into()],
            windows: Some(Asset { url: "https://example.org/a.exe".into(), sha256: sha256_hex(b"x"), size: 1 }),
            linux_deb: None,
        };
        let signed = serde_json::to_string(&sign(&key, &m)).unwrap();
        assert_eq!(open(&signed, &key.user_id()).unwrap(), m);
        // Another key, or a changed manifest, is refused.
        assert!(open(&signed, &Identity::generate().user_id()).is_err());
        let evil = signed.replace("a.exe", "evil.exe");
        assert!(open(&evil, &key.user_id()).is_err());
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("0.2.0", "0.1.0"));
        assert!(newer("1.0", "0.99.99"));
        assert!(!newer("0.2.0", "0.2.0"));
        assert!(!newer("0.1.9", "0.2.0"));
    }

    #[test]
    fn counts_missed_updates() {
        let m = Manifest {
            version: "0.6.3".into(),
            notes: String::new(),
            windows: None,
            linux_deb: None,
            released: ["0.5.0", "0.6.0", "0.6.2", "0.6.3"].iter().map(|s| s.to_string()).collect(),
        };
        assert_eq!(m.missed("0.6.3"), 0);
        assert_eq!(m.missed("0.6.2"), 1);
        assert_eq!(m.missed("0.6.1"), 2);
        assert_eq!(m.missed("0.5.0"), 3);
        // An older server list without the history still says "one".
        let old = Manifest { released: Vec::new(), ..m };
        assert_eq!(old.missed("0.5.0"), 1);
    }
}
