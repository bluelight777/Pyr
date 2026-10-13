//! Backups of servers you own, kept on this PC: everything the server stores (messages,
//! channels, members, roles, emoji, pictures and files, and the server's own identity), so a
//! server can be put back exactly as it was on another VPS, or after one lapses.
//!
//! Each server gets a folder (by default in Documents, which Windows often also copies to
//! OneDrive):
//!
//!   <name> (<key>)/
//!     server.json           which server, where it was, when it was last backed up
//!     identity/             server-cert.der, server-key.der (keep these private)
//!     files/<id>            every stored file, fetched once (ids are never reused)
//!     database/latest.db    the newest copy of the database
//!     database/<time>.db    older copies: every hour for two days, then daily for three
//!                           months, then monthly, kept for good
//!
//! To restore, set up a server with "Restore from a backup" (or by hand: copy latest.db to
//! `ingle.db`, files/ and the two identity files into the server's data folder before it
//! first starts).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use ingle_proto::control::{
    BACKUP_CERT, BACKUP_DATABASE, BACKUP_FEATURE, BACKUP_FILE_PREFIX, BACKUP_KEY, BACKUP_MANIFEST, BackupManifest, FILE_PREAMBLE,
    FileReply, FileRequest, Role,
};
use ingle_proto::framing;
use serde::{Deserialize, Serialize};

use crate::Shared;

/// How often a server you own is backed up while you're connected to it.
pub const EVERY: Duration = Duration::from_secs(60 * 60);

/// A backup's `server.json`.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct BackupInfo {
    pub key: String,
    pub name: String,
    pub host: String,
    pub port: u16,
    /// When the newest complete backup finished (ms since 1970).
    pub last_backup: i64,
    pub files: usize,
    pub bytes: u64,
    pub server_version: String,
}

/// Where backups go: the setting, or "Ingle Server Backups" in Documents.
pub fn root(shared: &Shared) -> PathBuf {
    if let Some(d) = shared.config.lock().backup_dir.clone() {
        return d;
    }
    directories::UserDirs::new()
        .and_then(|u| u.document_dir().map(Path::to_path_buf))
        .unwrap_or_else(|| shared.config_dir.clone())
        .join("Ingle Server Backups")
}

/// This server's backup folder.
pub fn folder(shared: &Shared, key: &str, name: &str) -> PathBuf {
    let clean: String = name.chars().filter(|c| c.is_alphanumeric() || " -_'".contains(*c)).take(40).collect();
    let clean = clean.trim();
    let short: String = key.chars().filter(|c| c.is_ascii_alphanumeric()).take(8).collect();
    root(shared).join(if clean.is_empty() { format!("Server ({short})") } else { format!("{clean} ({short})") })
}

/// Reads a backup folder's `server.json`.
pub fn info(dir: &Path) -> Option<BackupInfo> {
    serde_json::from_slice(&std::fs::read(dir.join("server.json")).ok()?).ok()
}

/// Whether I own `key` and it can be backed up.
pub fn can_back_up(shared: &Shared, key: &str) -> bool {
    let st = shared.state.read();
    st.server(key)
        .map(|s| s.features.iter().any(|f| f == BACKUP_FEATURE) && s.me.as_ref().map(|m| m.role == Role::Owner).unwrap_or(false))
        .unwrap_or(false)
}

/// Back up `key` now (if I own it); the result also shows in `AppState::backups`.
pub async fn run(shared: Arc<Shared>, key: String) -> Result<BackupInfo> {
    if !can_back_up(&shared, &key) {
        bail!("Only the server's owner can back it up (and the server needs updating to 0.6.1 or newer).");
    }
    {
        let mut st = shared.state.write();
        let b = st.backups.entry(key.clone()).or_default();
        if b.running {
            bail!("A backup of this server is already running.");
        }
        b.running = true;
    }
    let r = run_inner(&shared, &key).await;
    {
        let mut st = shared.state.write();
        let b = st.backups.entry(key.clone()).or_default();
        b.running = false;
        match &r {
            Ok(i) => {
                b.last_ok = Some(i.last_backup);
                b.error = None;
            }
            Err(e) => b.error = Some(format!("{e:#}")),
        }
    }
    (shared.repaint)();
    r
}

async fn run_inner(shared: &Arc<Shared>, key: &str) -> Result<BackupInfo> {
    let (name, entry) = {
        let st = shared.state.read();
        let name = st.server(key).map(|s| s.info.name.clone()).unwrap_or_default();
        let entry = shared.config.lock().servers.iter().find(|s| s.key == key).cloned();
        (name, entry)
    };
    let conn = shared
        .servers
        .lock()
        .get(key)
        .and_then(|h| h.connection())
        .ok_or_else(|| anyhow!("Not connected to the server right now."))?;
    let dir = folder(shared, key, &name);
    for sub in ["identity", "files", "database"] {
        std::fs::create_dir_all(dir.join(sub)).with_context(|| format!("making the backup folder {}", dir.display()))?;
    }
    let manifest: BackupManifest =
        serde_json::from_slice(&get(&conn, BACKUP_MANIFEST).await.context("the list of what to back up")?).context("reading the list")?;

    // The server's identity (rarely changes, but cheap).
    for part in [BACKUP_CERT, BACKUP_KEY] {
        let bytes = get(&conn, part).await.with_context(|| format!("the server's {part}"))?;
        write_private(&dir.join("identity").join(part), &bytes)?;
    }
    // Files: only the new ones.
    let mut bytes_total = 0u64;
    for (id, size) in &manifest.files {
        bytes_total += size;
        if !(id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())) {
            continue;
        }
        let path = dir.join("files").join(id);
        if std::fs::metadata(&path).map(|m| m.len() == *size).unwrap_or(false) {
            continue;
        }
        match get(&conn, &format!("{BACKUP_FILE_PREFIX}{id}")).await {
            Ok(b) => write_atomic(&path, &b)?,
            // Deleted on the server meanwhile: fine.
            Err(e) if e.to_string().contains("gone") => {}
            Err(e) => return Err(e.context(format!("file {id}"))),
        }
    }
    // The database, last (so it never points at files the backup doesn't have yet).
    let db = get(&conn, BACKUP_DATABASE).await.context("the database")?;
    if !db.starts_with(b"SQLite format 3\0") {
        bail!("the server sent something that isn't a database");
    }
    bytes_total += db.len() as u64;
    let now = chrono::Local::now();
    let stamp = now.format("%Y-%m-%d %H-%M").to_string();
    write_private(&dir.join("database").join(format!("{stamp}.db")), &db)?;
    write_private(&dir.join("database").join("latest.db"), &db)?;
    prune(&dir.join("database"));

    let info = BackupInfo {
        key: key.to_string(),
        name: if manifest.name.is_empty() { name } else { manifest.name.clone() },
        host: entry.as_ref().map(|e| e.host.clone()).unwrap_or_default(),
        port: entry.as_ref().map(|e| e.port).unwrap_or_default(),
        last_backup: ingle_proto::now_ms(),
        files: manifest.files.len(),
        bytes: bytes_total,
        server_version: manifest.version.clone(),
    };
    write_atomic(&dir.join("server.json"), &serde_json::to_vec_pretty(&info)?)?;
    std::fs::write(
        dir.join("READ ME.txt"),
        "This is a backup of an Ingle server you own: every message, channel, member, role, emoji, picture and file,\n\
         and the server's identity (the identity folder: keep it private; it's what makes a restored server the same server).\n\n\
         To put the server back (on a new VPS, say): in Ingle, + (Add a server) > Set up a server > Restore from a backup,\n\
         and pick this folder. Or by hand, before the server first starts: copy database/latest.db to <data folder>/ingle.db,\n\
         files/ to <data folder>/files/, and identity/* into <data folder>/.\n",
    )
    .ok();
    tracing::info!(server = %info.name, files = info.files, "backed up to {}", dir.display());
    Ok(info)
}

/// One part of a backup from the server.
async fn get(conn: &quinn::Connection, item: &str) -> Result<Vec<u8>> {
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(FILE_PREAMBLE).await?;
    send.write_all(&framing::encode(&FileRequest::Backup { item: item.to_string() })).await?;
    send.finish()?;
    let reply: FileReply = tokio::time::timeout(Duration::from_secs(60), crate::net::read_frame(&mut recv))
        .await
        .map_err(|_| anyhow!("the server didn't answer"))??
        .ok_or_else(|| anyhow!("the server didn't answer"))?;
    if !reply.ok {
        bail!("{}", reply.error);
    }
    if reply.size > 8 << 30 {
        bail!("too big ({} bytes)", reply.size);
    }
    let bytes = tokio::time::timeout(Duration::from_secs(1800), recv.read_to_end(reply.size as usize + 1))
        .await
        .map_err(|_| anyhow!("the download took too long"))??;
    if bytes.len() as u64 != reply.size {
        bail!("the download was cut short");
    }
    Ok(bytes)
}

fn write_atomic(path: &Path, data: &[u8]) -> Result<()> {
    let tmp = path.with_extension("partial");
    std::fs::write(&tmp, data).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    write_atomic(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Keeps every copy from the last two days, then one a day for three months, then one a
/// month for good.
fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let mut copies: Vec<(chrono::NaiveDateTime, PathBuf)> = rd
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let p = e.path();
            let stem = p.file_stem()?.to_str()?.to_string();
            let t = chrono::NaiveDateTime::parse_from_str(&stem, "%Y-%m-%d %H-%M").ok()?;
            Some((t, p))
        })
        .collect();
    copies.sort_by(|a, b| b.0.cmp(&a.0));
    let now = chrono::Local::now().naive_local();
    let mut seen_days = std::collections::HashSet::new();
    let mut seen_months = std::collections::HashSet::new();
    for (t, p) in copies {
        let age = now - t;
        let keep = if age < chrono::Duration::days(2) {
            true
        } else if age < chrono::Duration::days(92) {
            seen_days.insert(t.date())
        } else {
            seen_months.insert((t.format("%Y-%m").to_string(), ()))
        };
        if !keep {
            let _ = std::fs::remove_file(&p);
        }
    }
}

/// While connected to a server I own: back it up now and then every hour.
pub fn spawn_schedule(shared: Arc<Shared>, key: String, conn: quinn::Connection) {
    shared.rt.clone().spawn(async move {
        // Give the app a moment to settle after connecting.
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            if conn.close_reason().is_some() {
                return;
            }
            if can_back_up(&shared, &key) {
                let due = {
                    let st = shared.state.read();
                    let last = st.backups.get(&key).and_then(|b| b.last_ok).or_else(|| {
                        let name = st.server(&key).map(|s| s.info.name.clone()).unwrap_or_default();
                        info(&folder(&shared, &key, &name)).map(|i| i.last_backup)
                    });
                    last.map(|t| ingle_proto::now_ms() - t >= EVERY.as_millis() as i64).unwrap_or(true)
                };
                if due {
                    let failing = shared.state.read().backups.get(&key).map(|b| b.error.is_some()).unwrap_or(false);
                    if let Err(e) = run(shared.clone(), key.clone()).await {
                        tracing::warn!("backup failed: {e:#}");
                        // Said once per run of failures, not every few minutes.
                        if !failing {
                            let name = shared.state.read().server(&key).map(|s| s.info.name.clone()).unwrap_or_default();
                            crate::errors::report(format!("Couldn't back up {name}: {e:#}"));
                        }
                    }
                }
            }
            tokio::time::sleep(Duration::from_secs(5 * 60)).await;
        }
    });
}
