//! The Pyr Library's list of plugins: fetched from pyr.gg, believed only with
//! Firelit Games' signature (the release key) and within its dates, and kept on this PC so
//! the Plugins page has something to show offline.

use std::path::Path;
use std::time::Duration;

use pyr_proto::library::{self, Library};
use serde::{Deserialize, Serialize};

/// (`index2.json` since 0.8.1: it has skins with shapes of their own, which `index.json`, the
/// list older Pyrs read, leaves out.)
const DEFAULT_URL: &str = "https://pyr.gg/library/index2.json";
/// The newest list seen (Unix seconds it was signed), so an older one is never believed again.
const SEEN_FILE: &str = "library-seen.json";
/// The last list believed, as signed (shown when the website can't be reached).
const CACHE_FILE: &str = "library.json";

/// Where the list is (`PYR_LIBRARY_URL` overrides it, for testing).
pub fn index_url() -> String {
    pyr_proto::env_var("LIBRARY_URL").unwrap_or_else(|| DEFAULT_URL.to_string())
}

/// Where a file in the list is: its address is relative to the list's own.
pub fn file_url(index_url: &str, rel: &str) -> String {
    if rel.starts_with("https://") || rel.starts_with("http://") {
        return rel.to_string();
    }
    match index_url.rfind('/') {
        Some(i) => format!("{}/{}", &index_url[..i], rel.trim_start_matches('/')),
        None => rel.to_string(),
    }
}

#[derive(Serialize, Deserialize, Default)]
struct Seen {
    issued: i64,
}

fn now_secs() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Opens a signed list and checks its dates against what's been seen in `dir`; believed, it
/// becomes the newest seen.
pub fn believe(text: &str, dir: &Path, now: i64) -> Result<Library, String> {
    believe_with(text, crate::update::RELEASE_KEY, dir, now)
}

/// `believe`, with the key to check it against (tests sign with a key of their own).
pub fn believe_with(text: &str, key: &str, dir: &Path, now: i64) -> Result<Library, String> {
    let lib = library::open(text, key)?;
    let path = dir.join(SEEN_FILE);
    let seen: Seen = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    lib.fresh(now, seen.issued)?;
    if lib.issued > seen.issued {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, serde_json::to_vec(&Seen { issued: lib.issued }).unwrap_or_default());
    }
    Ok(lib)
}

/// Fetches the list (and keeps a copy). Run off the UI thread.
pub fn fetch(dir: &Path) -> Result<Library, String> {
    let url = index_url();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(None)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .build()
        .into();
    let text = agent
        .get(&url)
        .header("Cache-Control", "no-cache")
        .call()
        .map_err(|e| format!("couldn't reach the Pyr Library: {e}"))?
        .body_mut()
        .with_config()
        .limit(4 << 20)
        .read_to_string()
        .map_err(|e| format!("couldn't read the plugin list: {e}"))?;
    let lib = believe(&text, dir, now_secs())?;
    let _ = std::fs::write(dir.join(CACHE_FILE), &text);
    Ok(lib)
}

/// The last list believed, if it still is (for showing before, or without, a fetch).
pub fn cached(dir: &Path) -> Option<Library> {
    let text = std::fs::read_to_string(dir.join(CACHE_FILE)).ok()?;
    let lib = library::open(&text, crate::update::RELEASE_KEY).ok()?;
    let seen: Seen = std::fs::read(dir.join(SEEN_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    lib.fresh(now_secs(), seen.issued).ok()?;
    Some(lib)
}

/// For tests: a folder of its own.
#[cfg(test)]
pub fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("pyr-lib-{name}-{}-{}", std::process::id(), pyr_proto::now_ms()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_are_found_next_to_the_list() {
        let idx = "https://pyr.gg/library/index.json";
        assert_eq!(file_url(idx, "files/dice/1.0.0/dice.wasm"), "https://pyr.gg/library/files/dice/1.0.0/dice.wasm");
        assert_eq!(file_url(idx, "https://example.com/x"), "https://example.com/x");
    }

    #[test]
    fn an_older_list_is_never_believed_again() {
        let dir = temp_dir("seen");
        let key = pyr_proto::crypto::Identity::generate();
        let list = |issued| serde_json::to_string(&library::sign(&key, &Library { issued, expires: 10_000, ..Default::default() })).unwrap();
        assert!(believe_with(&list(100), &key.user_id(), &dir, 500).is_ok());
        assert!(believe_with(&list(200), &key.user_id(), &dir, 500).is_ok());
        assert!(believe_with(&list(100), &key.user_id(), &dir, 500).unwrap_err().contains("older"));
        assert!(believe_with(&list(300), &key.user_id(), &dir, 20_000).unwrap_err().contains("past its date"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_list_not_signed_by_firelit_is_not_believed() {
        let dir = temp_dir("sig");
        let other = pyr_proto::crypto::Identity::generate();
        let lib = Library { issued: 10, expires: now_secs() + 1000, plugins: vec![], skins: vec![] };
        let text = serde_json::to_string(&library::sign(&other, &lib)).unwrap();
        assert!(believe(&text, &dir, now_secs()).unwrap_err().contains("isn't signed by Firelit"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
