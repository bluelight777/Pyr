//! One running copy of the app per profile.
//!
//! Clicking an `ingle://` invite link starts the app with that link. If the app is already
//! open, the new copy hands the link to it (through a small file next to the config) and
//! quits, instead of opening a second window logged in as the same person.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOCK: &str = "instance.lock";
const HANDOFF: &str = "handoff";

pub enum Start {
    /// We're the only copy. Keep `lock` alive for as long as the app runs.
    First { lock: Option<File>, handoff: Handoff },
    /// Another copy is running and has been handed our link (if any).
    AlreadyRunning,
}

/// Where the running copy picks up links from later launches.
#[derive(Clone)]
pub struct Handoff {
    dir: PathBuf,
    last_check: Instant,
}

pub fn start(dir: &Path, join: Option<&str>) -> Start {
    let _ = std::fs::create_dir_all(dir);
    let handoff = Handoff { dir: dir.to_path_buf(), last_check: Instant::now() };
    let file = match OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(LOCK)) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("couldn't open the instance lock ({e}); allowing several copies");
            return Start::First { lock: None, handoff };
        }
    };
    match file.try_lock() {
        Ok(()) => {
            // A stale handoff from before we started isn't meant for us.
            let _ = std::fs::remove_file(dir.join(HANDOFF));
            Start::First { lock: Some(file), handoff }
        }
        Err(TryLockError::WouldBlock) => {
            let tmp = dir.join("handoff.tmp");
            let ok = std::fs::write(&tmp, join.unwrap_or("")).and_then(|_| std::fs::rename(&tmp, dir.join(HANDOFF)));
            if let Err(e) = ok {
                tracing::warn!("couldn't hand the link to the running copy: {e}");
            }
            Start::AlreadyRunning
        }
        Err(TryLockError::Error(e)) => {
            tracing::warn!("file locking unavailable ({e}); allowing several copies");
            Start::First { lock: None, handoff }
        }
    }
}

impl Handoff {
    /// Checks (at most twice a second) whether another launch handed us something.
    /// `Some("")` means "just come to the front"; otherwise it's an invite link.
    pub fn poll(&mut self) -> Option<String> {
        if self.last_check.elapsed() < Duration::from_millis(500) {
            return None;
        }
        self.last_check = Instant::now();
        let path = self.dir.join(HANDOFF);
        let text = std::fs::read_to_string(&path).ok()?;
        let _ = std::fs::remove_file(&path);
        Some(text.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_launch_hands_over_its_link() {
        let dir = std::env::temp_dir().join(format!("ingle-instance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let Start::First { lock, handoff: mut first } = start(&dir, None) else { panic!("first launch should win") };
        assert!(matches!(start(&dir, Some("ingle://example.org:7766/abc?fp=x")), Start::AlreadyRunning));
        std::thread::sleep(Duration::from_millis(550));
        assert_eq!(first.poll().as_deref(), Some("ingle://example.org:7766/abc?fp=x"));
        std::thread::sleep(Duration::from_millis(550));
        assert_eq!(first.poll(), None, "a handoff is delivered once");
        assert!(matches!(start(&dir, None), Start::AlreadyRunning));
        std::thread::sleep(Duration::from_millis(550));
        assert_eq!(first.poll().as_deref(), Some(""), "a plain launch just asks for the window");
        drop(lock);
        assert!(matches!(start(&dir, None), Start::First { .. }), "the lock goes away with the app");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
