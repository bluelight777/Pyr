//! Updates, like Discord: Ingle checks firelitgames.com for a newer version, shows a green
//! button when there is one, and on a click downloads it, installs it silently, reopens and
//! rejoins the voice channel you were in.
//!
//! The release file (`latest.json`) must be signed with the Firelit release key and the
//! download must match its SHA-256 (see `ingle_proto::release`), so a hacked website can't
//! push anything else. It must also be within its dates and no older than one this app has
//! already seen (`update-seen.json`), so a hacked website can't hide a newer release by
//! serving an old one either.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ingle_proto::control::ChannelId;
use ingle_proto::release::{self, Asset, Manifest};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

/// Public half of the Firelit release key. The secret half signs `latest.json`.
pub const RELEASE_KEY: &str = "J0EGQDSoDplSzp3aFrK7EA8C5Of4w4D40kaO-g7iJcE";
const DEFAULT_URL: &str = "https://firelitgames.com/ingle/latest.json";
const CHECK_EVERY: Duration = Duration::from_secs(3 * 60 * 60);
const RESUME_FILE: &str = "resume.json";
/// When the newest release file this app has seen was signed (see `check`).
const SEEN_FILE: &str = "update-seen.json";

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn manifest_url() -> String {
    std::env::var("INGLE_UPDATE_URL").unwrap_or_else(|_| DEFAULT_URL.to_string())
}

#[derive(Clone, Debug)]
pub enum Status {
    /// Up to date, or not checked yet.
    Idle,
    Available(Manifest),
    /// Downloading the new version: bytes done, total.
    Downloading(u64, u64),
    /// Downloaded and checked; the app should now start the installer and quit. (The file's
    /// SHA-256 is checked once more right before it's installed.)
    Ready(PathBuf, String),
    Failed(String),
    /// The release file couldn't be trusted to be the current one (past its date, or older
    /// than one seen before): something to tell the person, once.
    Stale(String),
}

pub struct Updater {
    status: Arc<Mutex<Status>>,
    repaint: Arc<dyn Fn() + Send + Sync>,
}

impl Updater {
    /// Checks shortly after start and then every few hours. `dir` is where what's been seen is
    /// kept (Ingle's settings folder).
    pub fn start(repaint: Arc<dyn Fn() + Send + Sync>, dir: PathBuf) -> Updater {
        let status = Arc::new(Mutex::new(Status::Idle));
        let (s, r) = (status.clone(), repaint.clone());
        let _ = std::thread::Builder::new().name("ingle-update".into()).spawn(move || {
            std::thread::sleep(Duration::from_secs(8));
            clean_old_downloads(&download_base(), Duration::from_secs(60 * 60));
            loop {
                match check(&dir) {
                    Ok(Some(m)) => {
                        let mut st = s.lock();
                        // Don't interrupt a download, or undo a failure the person hasn't seen.
                        if matches!(*st, Status::Idle | Status::Available(_)) {
                            tracing::info!("update available: {}", m.version);
                            *st = Status::Available(m);
                            r();
                        }
                    }
                    Ok(None) => tracing::debug!("up to date"),
                    Err(Check::Stale(e)) => {
                        tracing::warn!("update check: {e}");
                        let mut st = s.lock();
                        if matches!(*st, Status::Idle) {
                            *st = Status::Stale(e);
                            r();
                        }
                    }
                    Err(Check::Failed(e)) => tracing::info!("update check failed: {e}"),
                }
                std::thread::sleep(CHECK_EVERY);
            }
        });
        Updater { status, repaint }
    }

    pub fn status(&self) -> Status {
        self.status.lock().clone()
    }

    /// Back to "available" after the person has seen an error.
    pub fn dismiss_error(&self, m: Option<Manifest>) {
        *self.status.lock() = m.map(Status::Available).unwrap_or(Status::Idle);
    }

    /// Download and check the new version in the background.
    pub fn download(&self, manifest: Manifest) {
        let Some(asset) = platform_asset(&manifest).cloned() else { return };
        *self.status.lock() = Status::Downloading(0, asset.size);
        let (s, r) = (self.status.clone(), self.repaint.clone());
        let _ = std::thread::Builder::new().name("ingle-update-dl".into()).spawn(move || {
            let result = fetch(&asset, &manifest.version, |done| {
                *s.lock() = Status::Downloading(done, asset.size);
                r();
            });
            *s.lock() = match result {
                Ok(path) => Status::Ready(path, asset.sha256.to_lowercase()),
                Err(e) => {
                    tracing::warn!("update download failed: {e}");
                    Status::Failed(e)
                }
            };
            r();
        });
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(None)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(60)))
        .build()
        .into()
}

/// Why a check came to nothing.
enum Check {
    /// Couldn't reach the website, or read what it said.
    Failed(String),
    /// What it said is signed, but can't be trusted to be the current release.
    Stale(String),
}

#[derive(Serialize, Deserialize, Default)]
struct Seen {
    /// When the newest release file seen was signed (Unix seconds).
    issued: i64,
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The newer release, if there is one for this platform.
fn check(dir: &Path) -> Result<Option<Manifest>, Check> {
    let text = agent()
        .get(&manifest_url())
        .call()
        .map_err(|e| Check::Failed(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| Check::Failed(e.to_string()))?;
    let m = release::open(&text, RELEASE_KEY).map_err(Check::Failed)?;
    believe(&m, dir, now_secs()).map_err(Check::Stale)?;
    Ok((release::newer(&m.version, current_version()) && platform_asset(&m).is_some()).then_some(m))
}

/// A signed release file is only believed if it's within its dates and no older than the
/// newest one seen before; then it's the newest one seen.
fn believe(m: &Manifest, dir: &Path, now: i64) -> Result<(), String> {
    let path = dir.join(SEEN_FILE);
    let seen: Seen = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    m.fresh(now, seen.issued)?;
    if m.issued > seen.issued {
        let _ = std::fs::write(&path, serde_json::to_vec(&Seen { issued: m.issued }).unwrap_or_default());
    }
    Ok(())
}

pub fn platform_asset(m: &Manifest) -> Option<&Asset> {
    if cfg!(windows) {
        m.windows.as_ref()
    } else if cfg!(target_os = "linux") {
        m.linux_deb.as_ref()
    } else {
        None
    }
}

/// Where update downloads go: this user's own folders (on Linux, not the shared /tmp).
fn download_base() -> PathBuf {
    #[cfg(unix)]
    {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(std::env::temp_dir)
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir()
    }
}

/// Removes download folders left by earlier updates (an installer can't remove itself while
/// it runs). Only our own folders, and only ones older than `min_age`, so never the one an
/// installer may still be running from.
fn clean_old_downloads(base: &Path, min_age: Duration) {
    let Ok(entries) = std::fs::read_dir(base) else { return };
    for e in entries.flatten() {
        if !e.file_name().to_str().is_some_and(|n| n.starts_with("ingle-update")) {
            continue;
        }
        // A real folder (not a link to somewhere else).
        let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
        let old = meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age >= min_age);
        if meta.is_dir() && old {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

/// A new folder only this user can get into, for the download. (Not a fixed name in the
/// shared temp folder: on Linux another account could put its own file there, and the
/// package is installed as root.)
fn private_dir() -> Result<PathBuf, String> {
    let base = download_base();
    let _ = std::fs::create_dir_all(&base);
    let name: String = ingle_proto::crypto::random_bytes::<12>().iter().map(|b| format!("{b:02x}")).collect();
    let dir = base.join(format!("ingle-update-{name}"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        // Fails if anything (a file, a link someone planted) already has that name.
        std::fs::DirBuilder::new().mode(0o700).create(&dir).map_err(|e| format!("couldn't make a download folder: {e}"))?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir(&dir).map_err(|e| format!("couldn't make a download folder: {e}"))?;
    Ok(dir)
}

/// The SHA-256 of a file, read fresh from disk.
fn file_sha256(path: &Path) -> Option<String> {
    std::fs::read(path).ok().map(|b| release::sha256_hex(&b))
}

fn fetch(asset: &Asset, version: &str, mut progress: impl FnMut(u64)) -> Result<PathBuf, String> {
    let ext = if cfg!(windows) { "exe" } else { "deb" };
    let version: String = version.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')).take(32).collect();
    let mut resp = agent().get(&asset.url).call().map_err(|e| format!("couldn't download the update: {e}"))?;
    let mut reader = resp.body_mut().with_config().limit(asset.size + 1).reader();
    let mut bytes = Vec::with_capacity(asset.size as usize);
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("the download broke off: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        progress(bytes.len() as u64);
    }
    if bytes.len() as u64 != asset.size || release::sha256_hex(&bytes) != asset.sha256.to_lowercase() {
        return Err("the download doesn't match the release (damaged or tampered with), so it wasn't installed".into());
    }
    let dir = private_dir()?;
    let path = dir.join(format!("ingle-{version}.{ext}"));
    {
        use std::io::Write;
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).map_err(|e| e.to_string())?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// Where you were, so the updated Ingle can put you back.
#[derive(Serialize, Deserialize)]
pub struct Resume {
    pub server_key: String,
    pub channel_id: Option<ChannelId>,
    pub saved_at: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

pub fn save_resume(config_dir: &Path, server_key: &str, channel_id: Option<ChannelId>) {
    let r = Resume { server_key: server_key.to_string(), channel_id, saved_at: now() };
    if let Ok(text) = serde_json::to_string(&r) {
        let _ = std::fs::write(config_dir.join(RESUME_FILE), text);
    }
}

/// Read once at start: only fresh (within 10 minutes), and removed so it's used once.
pub fn take_resume(config_dir: &Path) -> Option<Resume> {
    let path = config_dir.join(RESUME_FILE);
    let text = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let r: Resume = serde_json::from_str(&text).ok()?;
    (now().saturating_sub(r.saved_at) < 600).then_some(r)
}

/// Starts installing the downloaded update. On Windows the silent setup replaces the files
/// once we've quit and then reopens Ingle (/RELAUNCH); on Linux the package is installed
/// with the system's password prompt (pkexec) and Ingle is reopened right after. The caller
/// quits the app when this returns Ok.
pub fn install(path: &Path, sha256: &str) -> Result<(), String> {
    // The very file about to run must still be the one that was checked.
    if file_sha256(path).as_deref() != Some(sha256) {
        return Err("the downloaded update changed on disk, so it wasn't installed".into());
    }
    #[cfg(windows)]
    {
        std::process::Command::new(path)
            .args(["/S", "/RELAUNCH"])
            .spawn()
            .map_err(|e| format!("couldn't start the installer: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let status = std::process::Command::new("pkexec")
            .args(["apt-get", "install", "-y"])
            .arg(path)
            .status()
            .map_err(|e| format!("couldn't ask for permission to install (pkexec): {e}"))?;
        if !status.success() {
            return Err("the update wasn't installed (cancelled, or the package manager is busy)".into());
        }
        // The package replaced our program file, so /proc/self/exe now reads "... (deleted)".
        let exe = std::env::current_exe()
            .ok()
            .map(|p| PathBuf::from(p.to_string_lossy().trim_end_matches(" (deleted)").to_string()))
            .unwrap_or_else(|| PathBuf::from("/usr/bin/ingle"));
        // Reopen once this copy has quit (only one copy runs at a time).
        std::process::Command::new("sh")
            .args(["-c", "sleep 2; exec \"$0\"", &exe.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("installed, but couldn't reopen Ingle: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_or_stale_release_file_is_not_believed() {
        let dir = std::env::temp_dir().join(format!("ingle-seen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let m = |issued, expires| Manifest {
            version: "0.8.0".into(),
            notes: String::new(),
            windows: None,
            linux_deb: None,
            released: Vec::new(),
            issued,
            expires,
        };
        // The current one is believed, and remembered.
        assert!(believe(&m(1_000, 9_000), &dir, 2_000).is_ok());
        // An older one served later (the website replaying it) isn't, nor is one past its date.
        assert!(believe(&m(500, 9_000), &dir, 2_000).is_err());
        assert!(believe(&m(0, 0), &dir, 2_000).is_err(), "nor an undated one, once a dated one was seen");
        assert!(believe(&m(1_000, 9_000), &dir, 9_500).is_err());
        // A newer one is, and then the old one is out for good.
        assert!(believe(&m(3_000, 9_000), &dir, 4_000).is_ok());
        assert!(believe(&m(1_000, 9_000), &dir, 4_000).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn downloads_go_to_a_new_private_folder() {
        let a = private_dir().unwrap();
        let b = private_dir().unwrap();
        assert_ne!(a, b);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
        }
        // A file swapped after the check is never installed.
        let f = a.join("ingle-9.9.9.deb");
        std::fs::write(&f, b"real").unwrap();
        let good = release::sha256_hex(b"real");
        std::fs::write(&f, b"evil").unwrap();
        assert!(install(&f, &good).unwrap_err().contains("changed"));
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }

    #[test]
    fn old_downloads_are_cleaned_up_but_nothing_else() {
        let base = std::env::temp_dir().join(format!("ingle-clean-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("ingle-update-0123abcd")).unwrap();
        std::fs::write(base.join("ingle-update-0123abcd").join("ingle-9.9.9.exe"), b"x").unwrap();
        std::fs::create_dir_all(base.join("ingle-update")).unwrap();
        std::fs::create_dir_all(base.join("someone-else")).unwrap();
        std::fs::write(base.join("ingle-update-file"), b"not a folder").unwrap();
        // Too new to touch yet.
        clean_old_downloads(&base, Duration::from_secs(3600));
        assert!(base.join("ingle-update-0123abcd").exists());
        // Old enough: our folders go, everything else stays.
        clean_old_downloads(&base, Duration::ZERO);
        assert!(!base.join("ingle-update-0123abcd").exists());
        assert!(!base.join("ingle-update").exists());
        assert!(base.join("someone-else").exists());
        assert!(base.join("ingle-update-file").exists());
        #[cfg(unix)]
        {
            // A link named like ours is left alone, and so is what it points at.
            std::os::unix::fs::symlink(base.join("someone-else"), base.join("ingle-update-link")).unwrap();
            clean_old_downloads(&base, Duration::ZERO);
            assert!(base.join("someone-else").exists());
        }
        let _ = std::fs::remove_dir_all(&base);
    }
}
