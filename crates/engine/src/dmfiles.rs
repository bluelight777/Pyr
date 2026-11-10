//! Files of any kind in direct messages and groups (0.13.13, `DM_FILES_FEATURE`): up to 2 GB,
//! end-to-end encrypted like the messages. A file goes in parts of 8 MB
//! (`DM_FILE_PART_BYTES`), each sealed with the file's own key (`sealed::seal_part`) and
//! uploaded on its own, so a dropped connection costs one part at most: it carries on from
//! there once the connection's back. Then a message names the file and its parts (only its
//! conversation can open it, and the server holds only scrambled pieces). Saving one
//! downloads its parts in order, opens each and writes it to the disk as it comes.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use pyr_proto::b64;
use pyr_proto::control::{ChannelId, DM_FILE_NAME, DM_FILES_FEATURE, FILE_PREAMBLE, FileKind, FileReply, FileRequest, MAX_DM_FILE_BYTES, SEALED_FEATURE};
use pyr_proto::framing;
use pyr_proto::sealed::{self, DM_FILE_PART_BYTES, SealedFile};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::Shared;
use crate::server::ServerHandle;
use crate::state::{DmSave, DmSend};

const SEALED_MIME: &str = "application/octet-stream";

/// This Pyr's numbers for sends and saves (to tell them apart, and a save started again from
/// the one cancelled before it).
static NEXT: AtomicU64 = AtomicU64::new(1);

/// What a file is, by its name: for the other end's icon (it's sent as it is, whatever it is).
pub fn mime_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "zip" => "application/zip",
        "pdf" => "application/pdf",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "txt" | "log" | "md" | "csv" => "text/plain",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => SEALED_MIME,
    }
}

/// Whether a file of `size` bytes can be sent, and if not, why.
pub fn check_size(size: u64) -> Result<(), String> {
    if size == 0 {
        return Err("That file is empty.".into());
    }
    if size > MAX_DM_FILE_BYTES {
        return Err(format!("Files can be up to 2 GB (that one is {:.1} GB).", size as f64 / 1e9));
    }
    Ok(())
}

/// Why a part didn't go (or come): the server said no (final), or the connection let it down
/// (worth another try).
enum Fail {
    Refused(String),
    Net(String),
}

async fn put_part(conn: &quinn::Connection, ct: &[u8]) -> Result<String, Fail> {
    let net = |e: anyhow::Error| Fail::Net(format!("{e:#}"));
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| net(e.into()))?;
    let req = FileRequest::Put { kind: FileKind::Sealed, name: DM_FILE_NAME.into(), mime: SEALED_MIME.into(), size: ct.len() as u64 };
    send.write_all(FILE_PREAMBLE).await.map_err(|e| net(e.into()))?;
    send.write_all(&framing::encode(&req)).await.map_err(|e| net(e.into()))?;
    send.write_all(ct).await.map_err(|e| net(e.into()))?;
    send.finish().map_err(|e| net(e.into()))?;
    let reply: FileReply = tokio::time::timeout(Duration::from_secs(300), crate::net::read_frame(&mut recv))
        .await
        .map_err(|_| Fail::Net("the server didn't answer in time".into()))?
        .map_err(net)?
        .ok_or_else(|| Fail::Net("the server didn't answer".into()))?;
    if !reply.ok {
        // (Cut short on the way: another try.)
        return Err(if reply.error.contains("cut short") { Fail::Net(reply.error) } else { Fail::Refused(reply.error) });
    }
    Ok(reply.id)
}

async fn get_part(conn: &quinn::Connection, id: &str) -> Result<Vec<u8>, Fail> {
    let net = |e: anyhow::Error| Fail::Net(format!("{e:#}"));
    let (mut send, mut recv) = conn.open_bi().await.map_err(|e| net(e.into()))?;
    send.write_all(FILE_PREAMBLE).await.map_err(|e| net(e.into()))?;
    send.write_all(&framing::encode(&FileRequest::Get { id: id.to_string() })).await.map_err(|e| net(e.into()))?;
    send.finish().map_err(|e| net(e.into()))?;
    let reply: FileReply = tokio::time::timeout(Duration::from_secs(30), crate::net::read_frame(&mut recv))
        .await
        .map_err(|_| Fail::Net("the server didn't answer in time".into()))?
        .map_err(net)?
        .ok_or_else(|| Fail::Net("the server didn't answer".into()))?;
    if !reply.ok {
        return Err(Fail::Refused(reply.error));
    }
    if reply.size > DM_FILE_PART_BYTES + 16 {
        return Err(Fail::Refused("the server sent the wrong part of it".into()));
    }
    let bytes = tokio::time::timeout(Duration::from_secs(300), recv.read_to_end(reply.size as usize + 1))
        .await
        .map_err(|_| Fail::Net("it took too long to come".into()))?
        .map_err(|e| net(anyhow!(e)))?;
    if bytes.len() as u64 != reply.size {
        return Err(Fail::Net("it didn't arrive whole".into()));
    }
    Ok(bytes)
}

/// Waits for this server's connection (it may be dropped and coming back), while `go` says to.
async fn connection(h: &ServerHandle, go: &impl Fn() -> bool, waiting: &impl Fn(Option<String>)) -> Option<quinn::Connection> {
    let mut said = false;
    loop {
        if !go() {
            return None;
        }
        if let Some(c) = h.connection().filter(|c| c.close_reason().is_none()) {
            if said {
                waiting(None);
            }
            return Some(c);
        }
        if !said {
            waiting(Some("Waiting for the connection to come back…".into()));
            said = true;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

/// Starts sending the file at `path` in this direct message or group.
pub(crate) fn send(shared: &Arc<Shared>, key: &str, h: ServerHandle, channel: ChannelId, path: PathBuf) -> Result<u64, String> {
    {
        let st = shared.state.read_recursive();
        let s = st.server(key).ok_or("Not connected to that server.")?;
        if !s.has(DM_FILES_FEATURE) {
            return Err("This server needs updating before files can be sent on it.".into());
        }
        if !s.has(SEALED_FEATURE) {
            return Err("This server doesn't encrypt messages, so files can't be sent on it.".into());
        }
        if s.dm(channel).is_none() && s.group(channel).is_none() {
            return Err("Files can only be sent in direct messages and groups.".into());
        }
    }
    let meta = std::fs::metadata(&path).map_err(|e| format!("Couldn't read that file: {e}"))?;
    if !meta.is_file() {
        return Err("That isn't a file (a folder can be zipped first).".into());
    }
    check_size(meta.len())?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let size = meta.len();
    if let Some(s) = shared.state.write().server_mut(key) {
        s.dm_sends.push(DmSend { id, channel_id: channel, name: name.clone(), size, sent: 0, waiting: None, failed: None });
    }
    (shared.repaint)();
    let shared = shared.clone();
    let key = key.to_string();
    shared.rt.clone().spawn(async move {
        if let Err(e) = upload(&shared, &key, &h, id, channel, &path, &name, size).await {
            tracing::warn!(file = %name, "a file for a direct message didn't go: {e}");
            if let Some(s) = shared.state.write().server_mut(&key) {
                if let Some(d) = s.dm_sends.iter_mut().find(|d| d.id == id) {
                    d.failed = Some(e);
                    d.waiting = None;
                }
            }
        }
        (shared.repaint)();
    });
    Ok(id)
}

/// Stops sending a file (or dismisses one that stopped).
pub(crate) fn cancel(shared: &Shared, key: &str, id: u64) {
    if let Some(s) = shared.state.write().server_mut(key) {
        s.dm_sends.retain(|d| d.id != id);
    }
    (shared.repaint)();
}

#[allow(clippy::too_many_arguments)]
async fn upload(shared: &Arc<Shared>, key: &str, h: &ServerHandle, id: u64, channel: ChannelId, path: &Path, name: &str, size: u64) -> Result<(), String> {
    // Still wanted (not cancelled, and the server's still one of mine).
    let go = || shared.state.read_recursive().server(key).is_some_and(|s| s.dm_sends.iter().any(|d| d.id == id));
    let update = |f: &dyn Fn(&mut DmSend)| {
        if let Some(s) = shared.state.write().server_mut(key) {
            if let Some(d) = s.dm_sends.iter_mut().find(|d| d.id == id) {
                f(d);
            }
        }
        (shared.repaint)();
    };
    let waiting = |why: Option<String>| update(&|d| d.waiting = why.clone());
    let file_key = sealed::new_file_key();
    let n = sealed::part_count(size);
    let mut file = tokio::fs::File::open(path).await.map_err(|e| format!("Couldn't read {name} (was it changed or moved?): {e}"))?;
    let mut parts: Vec<String> = Vec::with_capacity(n as usize);
    for i in 0..n {
        if !go() {
            return Ok(());
        }
        let at = i * DM_FILE_PART_BYTES;
        let len = DM_FILE_PART_BYTES.min(size - at) as usize;
        let mut buf = vec![0u8; len];
        let read = async {
            file.seek(std::io::SeekFrom::Start(at)).await?;
            file.read_exact(&mut buf).await
        };
        read.await.map_err(|e| format!("Couldn't read {name} (was it changed or moved?): {e}"))?;
        let ct = sealed::seal_part(&file_key, i, i + 1 == n, &buf);
        drop(buf);
        let mut attempt = 0u32;
        let part = loop {
            let Some(conn) = connection(h, &go, &waiting).await else { return Ok(()) };
            match put_part(&conn, &ct).await {
                Ok(part) => break part,
                Err(Fail::Refused(e)) => return Err(e),
                Err(Fail::Net(e)) => {
                    attempt += 1;
                    tracing::info!(attempt, "dm file: a part failed, trying again: {e}");
                    if attempt >= 3 {
                        waiting(Some("The connection is having trouble: trying again…".into()));
                    }
                    tokio::time::sleep(Duration::from_millis((500u64 << attempt.min(5)).min(15_000))).await;
                }
            }
        };
        parts.push(part);
        let sent = (at + len as u64).min(size);
        update(&|d| {
            d.sent = sent;
            d.waiting = None;
        });
    }
    // Everything's there: the message that says what it is.
    let file = SealedFile {
        id: parts[0].clone(),
        name: name.to_string(),
        mime: mime_for(name).to_string(),
        size,
        width: 0,
        height: 0,
        key: b64::encode(&file_key),
        parts: parts.clone(),
    };
    let nonce = b64::encode(&pyr_proto::crypto::random_bytes::<9>());
    let mut attempt = 0u32;
    loop {
        if connection(h, &go, &waiting).await.is_none() {
            return Ok(());
        }
        match crate::sealed::send_file(shared, key, h, channel, file.clone(), nonce.clone()).await {
            Ok(()) => break,
            // (The connection dropped as it went: again, once it's back. The server takes the
            // same parts again: they aren't anyone's message yet.)
            Err(e) if e.contains("dropped") || e.contains("not connected") => {
                attempt += 1;
                tokio::time::sleep(Duration::from_millis((500u64 << attempt.min(5)).min(15_000))).await;
            }
            Err(e) => return Err(e),
        }
    }
    // Mine can be saved here too.
    shared.sealed.lock().add_doc(file_key, parts);
    if let Some(s) = shared.state.write().server_mut(key) {
        s.dm_sends.retain(|d| d.id != id);
    }
    Ok(())
}

/// Starts saving a file from a direct message to `path` (picked by the person).
pub(crate) fn save(shared: &Arc<Shared>, key: &str, h: ServerHandle, file_id: &str, path: PathBuf) -> Result<(), String> {
    let (size, expired) = {
        let st = shared.state.read_recursive();
        let s = st.server(key).ok_or("Not connected to that server.")?;
        if s.dm_saves.get(file_id).is_some_and(|d| !d.finished && d.failed.is_none()) {
            return Err("It's being saved already.".into());
        }
        let a = s.logs.values().flat_map(|l| l.messages.iter()).flat_map(|m| m.attachments.iter()).find(|a| a.id == file_id && a.parts > 0).ok_or("That file isn't here.")?;
        (a.size, a.expired)
    };
    if expired {
        return Err("That file has expired: files are kept 7 days.".into());
    }
    let (file_key, parts) = shared.sealed.lock().doc(file_id).ok_or("That file can't be opened here.")?;
    let job = NEXT.fetch_add(1, Ordering::Relaxed);
    if let Some(s) = shared.state.write().server_mut(key) {
        s.dm_saves.insert(file_id.to_string(), DmSave { job, path: path.clone(), size, done: 0, finished: false, waiting: None, failed: None });
    }
    (shared.repaint)();
    let shared = shared.clone();
    let key = key.to_string();
    let file_id = file_id.to_string();
    shared.rt.clone().spawn(async move {
        let r = download(&shared, &key, &h, &file_id, job, &path, size, &file_key, &parts).await;
        let mut st = shared.state.write();
        let mine = st.server_mut(&key).and_then(|s| s.dm_saves.get_mut(&file_id)).filter(|d| d.job == job);
        match (r, mine) {
            (Ok(true), Some(d)) => {
                d.finished = true;
                d.done = size;
                d.waiting = None;
            }
            (Err(e), Some(d)) => {
                tracing::warn!("a file from a direct message couldn't be saved: {e}");
                d.failed = Some(e);
                d.waiting = None;
                let _ = std::fs::remove_file(&path);
            }
            // Cancelled: what was written goes.
            (Ok(false), _) | (Err(_), None) => {
                let _ = std::fs::remove_file(&path);
            }
            (Ok(true), None) => {}
        }
        drop(st);
        (shared.repaint)();
    });
    Ok(())
}

/// Stops saving a file (what was written goes), or forgets one saved or that stopped.
pub(crate) fn dismiss_save(shared: &Shared, key: &str, file_id: &str) {
    if let Some(s) = shared.state.write().server_mut(key) {
        s.dm_saves.remove(file_id);
    }
    (shared.repaint)();
}

/// Downloads and opens each part in order, writing it out: true when it's all there, false
/// when it was cancelled.
#[allow(clippy::too_many_arguments)]
async fn download(shared: &Arc<Shared>, key: &str, h: &ServerHandle, file_id: &str, job: u64, path: &Path, size: u64, file_key: &[u8; 32], parts: &[String]) -> Result<bool, String> {
    let go = || shared.state.read_recursive().server(key).and_then(|s| s.dm_saves.get(file_id)).is_some_and(|d| d.job == job && !d.finished && d.failed.is_none());
    let update = |f: &dyn Fn(&mut DmSave)| {
        if let Some(d) = shared.state.write().server_mut(key).and_then(|s| s.dm_saves.get_mut(file_id)).filter(|d| d.job == job) {
            f(d);
        }
        (shared.repaint)();
    };
    let waiting = |why: Option<String>| update(&|d| d.waiting = why.clone());
    let mut out = tokio::fs::File::create(path).await.map_err(|e| format!("Couldn't save there: {e}"))?;
    let n = parts.len() as u64;
    if n != sealed::part_count(size) {
        return Err("That file can't be opened here.".into());
    }
    let mut written = 0u64;
    for (i, part) in parts.iter().enumerate() {
        let i = i as u64;
        let mut attempt = 0u32;
        let ct = loop {
            let Some(conn) = connection(h, &go, &waiting).await else { return Ok(false) };
            match get_part(&conn, part).await {
                Ok(ct) => break ct,
                Err(Fail::Refused(e)) if e.contains("gone") => return Err("That file has expired: files are kept 7 days.".into()),
                Err(Fail::Refused(e)) => return Err(e),
                Err(Fail::Net(e)) => {
                    attempt += 1;
                    tracing::info!(attempt, "dm file: a part didn't come, trying again: {e}");
                    if attempt >= 3 {
                        waiting(Some("The connection is having trouble: trying again…".into()));
                    }
                    tokio::time::sleep(Duration::from_millis((500u64 << attempt.min(5)).min(15_000))).await;
                }
            }
        };
        let plain = sealed::open_part(file_key, i, i + 1 == n, &ct).ok_or("A part of the file didn't check out: it was changed or damaged on the way.")?;
        let want = DM_FILE_PART_BYTES.min(size - i * DM_FILE_PART_BYTES);
        if plain.len() as u64 != want {
            return Err("The file came out a different size than was sent.".into());
        }
        out.write_all(&plain).await.map_err(|e| format!("Couldn't write the file (is the disk full?): {e}"))?;
        written += plain.len() as u64;
        update(&|d| {
            d.done = written;
            d.waiting = None;
        });
        if !go() {
            return Ok(false);
        }
    }
    out.flush().await.map_err(|e| format!("Couldn't write the file (is the disk full?): {e}"))?;
    out.sync_all().await.map_err(|e| format!("Couldn't write the file (is the disk full?): {e}"))?;
    if written != size {
        return Err("The file came out a different size than was sent.".into());
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_say_what_they_are_by_their_names() {
        assert_eq!(mime_for("Trip.MOV"), "video/quicktime");
        assert_eq!(mime_for("notes.txt"), "text/plain");
        assert_eq!(mime_for("archive.tar.gz"), SEALED_MIME);
        assert_eq!(mime_for("README"), SEALED_MIME);
    }

    #[test]
    fn a_file_has_to_have_something_in_it_and_fit() {
        assert!(check_size(0).is_err());
        assert!(check_size(1).is_ok());
        assert!(check_size(MAX_DM_FILE_BYTES).is_ok());
        assert_eq!(check_size(3_000_000_000).unwrap_err(), "Files can be up to 2 GB (that one is 3.0 GB).");
    }
}
