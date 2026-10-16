//! Updates, like Discord: Ingle checks firelitgames.com for a newer version, shows a green
//! Update button when there is one (see `update_chip`), and on a click downloads it, installs
//! it silently, reopens and rejoins the voice channel you were in.
//!
//! The release file (`latest.json`) must be signed with the Firelit release key and the
//! download must match its SHA-256 (see `ingle_proto::release`), so a hacked website can't
//! push anything else. It must also be within its dates and no older than one this app has
//! already seen (`update-seen.json`), so a hacked website can't hide a newer release by
//! serving an old one either.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// What a check someone asked for found (Check for updates, in the window or the tray icon's
/// menu), to tell them.
#[derive(Clone, Debug, PartialEq)]
pub enum Checked {
    /// This is the newest version.
    UpToDate,
    /// A newer version (the Update button, and the tray icon's Update item, have it now).
    Found(String),
    Failed(String),
}

/// Who asked for a check: the window (told with a toast) or the tray icon's menu (told with a
/// Windows notice, since the window may well be hidden).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Asker {
    Window,
    #[cfg_attr(not(windows), allow(dead_code))]
    Tray,
}

#[derive(Clone)]
pub struct Updater {
    status: Arc<Mutex<Status>>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    /// Where what's been seen is kept (Ingle's settings folder).
    dir: PathBuf,
    /// One check at a time (the regular one, or one someone asked for).
    check_lock: Arc<Mutex<()>>,
    /// A check is running (the updater's arrow turns while it does).
    checking: Arc<AtomicBool>,
    /// What checks people asked for found, waiting to be told.
    answers: Arc<Mutex<Vec<(Asker, Checked)>>>,
}

impl Updater {
    /// Checks shortly after start and then every few hours. `dir` is where what's been seen is
    /// kept (Ingle's settings folder).
    pub fn start(repaint: Arc<dyn Fn() + Send + Sync>, dir: PathBuf) -> Updater {
        let u = Updater {
            status: Arc::new(Mutex::new(Status::Idle)),
            repaint,
            dir,
            check_lock: Arc::new(Mutex::new(())),
            checking: Arc::new(AtomicBool::new(false)),
            answers: Arc::new(Mutex::new(Vec::new())),
        };
        let me = u.clone();
        let _ = std::thread::Builder::new().name("ingle-update".into()).spawn(move || {
            std::thread::sleep(Duration::from_secs(8));
            clean_old_downloads(&download_base(), Duration::from_secs(60 * 60));
            loop {
                let _ = me.check_and_apply(false);
                std::thread::sleep(CHECK_EVERY);
            }
        });
        u
    }

    pub fn status(&self) -> Status {
        self.status.lock().clone()
    }

    /// A check is running.
    pub fn checking(&self) -> bool {
        self.checking.load(Ordering::Relaxed)
    }

    /// Check right away (Check for updates), in the background. What it finds goes to the
    /// Update button as usual, and to [`Updater::take_answers`] for `asker`; then `done` is
    /// called (from the checking thread).
    pub fn check_now(&self, asker: Asker, done: impl FnOnce() + Send + 'static) {
        // Nothing to look for while an update is on its way in.
        if matches!(self.status(), Status::Downloading(..) | Status::Ready(..)) {
            return;
        }
        let me = self.clone();
        let _ = std::thread::Builder::new().name("ingle-update-check".into()).spawn(move || {
            let answer = match me.check_and_apply(true) {
                Ok(Some(version)) => Checked::Found(version),
                Ok(None) => Checked::UpToDate,
                Err(e) => Checked::Failed(e),
            };
            tracing::info!("update check (asked for): {answer:?}");
            me.answers.lock().push((asker, answer));
            (me.repaint)();
            done();
        });
    }

    /// What the checks `asker` asked for found, oldest first (each told once).
    pub fn take_answers(&self, asker: Asker) -> Vec<Checked> {
        let mut all = self.answers.lock();
        let (mine, rest): (Vec<_>, Vec<_>) = all.drain(..).partition(|(a, _)| *a == asker);
        *all = rest;
        mine.into_iter().map(|(_, c)| c).collect()
    }

    /// One check. A newer version goes to the Update button; the newer version's number, if
    /// any, or why the check failed, is returned. (A stale release file is also shown as an
    /// error by the regular checks; one someone asked for just answers with it.)
    fn check_and_apply(&self, asked: bool) -> Result<Option<String>, String> {
        let _one = self.check_lock.lock();
        self.checking.store(true, Ordering::Relaxed);
        (self.repaint)();
        let found = check(&self.dir);
        self.checking.store(false, Ordering::Relaxed);
        let answer = match found {
            Ok(Some(m)) => {
                let version = m.version.clone();
                let mut st = self.status.lock();
                // Don't interrupt a download, or undo a failure the person hasn't seen.
                if matches!(*st, Status::Idle | Status::Available(_)) {
                    tracing::info!("update available: {}", m.version);
                    *st = Status::Available(m);
                }
                Ok(Some(version))
            }
            Ok(None) => {
                tracing::debug!("up to date");
                Ok(None)
            }
            Err(Check::Stale(e)) => {
                tracing::warn!("update check: {e}");
                let mut st = self.status.lock();
                if !asked && matches!(*st, Status::Idle) {
                    *st = Status::Stale(e.clone());
                }
                Err(e)
            }
            Err(Check::Failed(e)) => {
                tracing::info!("update check failed: {e}");
                Err(e)
            }
        };
        (self.repaint)();
        answer
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
        // (Development builds: a pretend update's pretend download; see dev_pretend_check.)
        #[cfg(debug_assertions)]
        if asset.url == "pretend" {
            let _ = std::thread::Builder::new().name("ingle-update-pretend".into()).spawn(move || {
                for i in 0..=60u64 {
                    *s.lock() = Status::Downloading(asset.size * i / 60, asset.size);
                    r();
                    std::thread::sleep(Duration::from_millis(250));
                }
                *s.lock() = Status::Failed("that was a pretend update (INGLE_DEV_FAKE_UPDATE): nothing was downloaded".into());
                r();
            });
            return;
        }
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

/// For the release file: small, so a minute is plenty.
fn agent() -> ureq::Agent {
    agent_with_body_time(Duration::from_secs(60))
}

/// ureq's body timeout is the time for the whole body, not a pause between reads. Given a
/// minute, a 55 MB installer needed a 7 Mbit/s connection, and on anything slower the update
/// "broke off". Allow 50 kB/s instead (about 20 minutes for 55 MB), and a minute more.
fn download_time(size: u64) -> Duration {
    Duration::from_secs(60 + size / 50_000)
}

fn agent_with_body_time(body: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(None)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(body))
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
    #[cfg(debug_assertions)]
    if let Some(pretend) = dev_pretend_check() {
        return pretend;
    }
    let text = agent()
        .get(&manifest_url())
        // The newest one, not a copy a cache kept (someone checking right after a release).
        .header("Cache-Control", "no-cache")
        .call()
        .map_err(|e| Check::Failed(e.to_string()))?
        .body_mut()
        .read_to_string()
        .map_err(|e| Check::Failed(e.to_string()))?;
    let m = release::open(&text, RELEASE_KEY).map_err(Check::Failed)?;
    believe(&m, dir, now_secs()).map_err(Check::Stale)?;
    Ok((release::newer(&m.version, current_version()) && platform_asset(&m).is_some()).then_some(m))
}

/// Development builds only: `INGLE_DEV_FAKE_UPDATE` stands in for the website, to see how the
/// updater looks in each state: `none` (up to date), `fail` (can't check), or a version that's
/// "out" (whose download only pretends, and then fails). Never in release builds.
#[cfg(debug_assertions)]
fn dev_pretend_check() -> Option<Result<Option<Manifest>, Check>> {
    let v = std::env::var("INGLE_DEV_FAKE_UPDATE").ok().filter(|v| !v.is_empty())?;
    std::thread::sleep(Duration::from_millis(1200));
    Some(match v.as_str() {
        "none" => Ok(None),
        "fail" => Err(Check::Failed("pretend: couldn't reach firelitgames.com".into())),
        v => {
            let asset = Some(Asset { url: "pretend".into(), sha256: String::new(), size: 55_000_000 });
            let released = vec![current_version().to_string(), v.to_string()];
            Ok(Some(Manifest { version: v.into(), notes: String::new(), windows: asset.clone(), linux_deb: asset, released, issued: 0, expires: 0 }))
        }
    })
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
    let mut resp = agent_with_body_time(download_time(asset.size))
        .get(&asset.url)
        .call()
        .map_err(|e| format!("couldn't download the update: {e}"))?;
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
        // As root: copy the package somewhere only root can write, check it there, and install
        // that copy. (Checking it here and then having apt read it from a folder we own would
        // leave a moment for something running as us to swap it.) The package's own copy of the
        // script has a password prompt that says what it's for.
        let mut cmd = std::process::Command::new("pkexec");
        if Path::new(LINUX_HELPER).is_file() {
            cmd.arg(LINUX_HELPER);
        } else {
            cmd.args(["/bin/sh", "-c", LINUX_INSTALL, "ingle-update"]);
        }
        let status = cmd
            .arg(path)
            .arg(sha256)
            .status()
            .map_err(|e| format!("couldn't ask for permission to install (pkexec): {e}"))?;
        if status.code() == Some(3) {
            return Err("the downloaded update changed on disk, so it wasn't installed".into());
        }
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

/// Run as root by `install` on Linux, with the package and its SHA-256: copied into a new
/// root-only folder, checked there, installed from there. The package installs it as
/// [`LINUX_HELPER`] (with its own password prompt, `com.firelitgames.ingle.install-update`).
#[cfg_attr(windows, allow(dead_code))]
const LINUX_INSTALL: &str = include_str!("../../../installer/linux/install-update");
#[cfg_attr(windows, allow(dead_code))]
const LINUX_HELPER: &str = "/usr/lib/ingle/install-update";

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn linux_install_checks_the_copy_it_installs() {
        // The script, with apt-get standing in: a matching package gets to "apt-get" (here a
        // stand-in that records its argument); a changed one stops with code 3 before it.
        let dir = std::env::temp_dir().join(format!("ingle-inst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        let fake = dir.join("bin").join("apt-get");
        std::fs::write(&fake, format!("#!/bin/sh\ncp \"$3\" {}/installed.deb\n", dir.display())).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let pkg = dir.join("ingle.deb");
        std::fs::write(&pkg, b"the real package").unwrap();
        let path = format!("{}:{}", dir.join("bin").display(), std::env::var("PATH").unwrap_or_default());
        // Both ways it runs: the package's own copy, and the same script given to sh.
        let helper = concat!(env!("CARGO_MANIFEST_DIR"), "/../../installer/linux/install-update");
        for packaged in [true, false] {
            let run = |sha: &str| {
                let mut cmd = if packaged {
                    std::process::Command::new(helper)
                } else {
                    let mut c = std::process::Command::new("/bin/sh");
                    c.args(["-c", LINUX_INSTALL, "ingle-update"]);
                    c
                };
                cmd.env("PATH", &path).arg(&pkg).arg(sha).status().unwrap()
            };
            let _ = std::fs::remove_file(dir.join("installed.deb"));
            assert_eq!(run(&release::sha256_hex(b"something else")).code(), Some(3), "packaged: {packaged}");
            assert!(!dir.join("installed.deb").exists());
            assert!(run(&release::sha256_hex(b"the real package")).success(), "packaged: {packaged}");
            assert_eq!(std::fs::read(dir.join("installed.deb")).unwrap(), b"the real package");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_slow_connection_has_time_to_download_an_update() {
        // 55 MB at 50 kB/s is 1,100 seconds.
        assert!(download_time(55_000_000) >= Duration::from_secs(1_100));
        assert!(download_time(0) >= Duration::from_secs(60));
    }

    #[test]
    fn answers_go_to_whoever_asked() {
        let u = Updater {
            status: Arc::new(Mutex::new(Status::Idle)),
            repaint: Arc::new(|| {}),
            dir: std::env::temp_dir(),
            check_lock: Arc::new(Mutex::new(())),
            checking: Arc::new(AtomicBool::new(false)),
            answers: Arc::new(Mutex::new(vec![
                (Asker::Tray, Checked::UpToDate),
                (Asker::Window, Checked::Found("9.9.9".into())),
                (Asker::Tray, Checked::Failed("offline".into())),
            ])),
        };
        assert_eq!(u.take_answers(Asker::Tray), vec![Checked::UpToDate, Checked::Failed("offline".into())]);
        assert_eq!(u.take_answers(Asker::Tray), vec![]);
        assert_eq!(u.take_answers(Asker::Window), vec![Checked::Found("9.9.9".into())]);
    }

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
