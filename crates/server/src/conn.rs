//! One client connection: login handshake, control requests, and media ingest.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use bytes::BytesMut;
use pyr_proto::control::*;
use pyr_proto::media::{self, AudioHeader, VideoHeader, track};
use pyr_proto::sealed::{KeyRef, KeyShare, MAX_KEYS_ASKED, MAX_SHARES, NewShare, Sealed};
use pyr_proto::{MAX_VIDEO_FRAME, PROTOCOL_VERSION, crypto, framing};
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::db::new_invite_code;
use crate::hub::{ConnId, ConnSlot, Hub, ModeratedVoice, Outbox, Reply, frame};

/// Most video frames being received at once on one connection.
const MAX_VIDEO_READS: usize = 24;
/// Longest message `nonce` a client may send (it's echoed to everyone).
const MAX_NONCE_LEN: usize = 64;
/// Usable invites one member may have made at a time.
const MAX_INVITES_PER_MEMBER: usize = 25;
/// Requests per second (and burst) one connection may make, all kinds together.
const REQ_RATE: f64 = 20.0;
const REQ_BURST: f64 = 80.0;
/// Soundboard: a few sounds in a row, then one every couple of seconds.
const SOUND_RATE: f64 = 0.5;
const SOUND_BURST: f64 = 4.0;
/// Ringing someone (and declining): a few in a row, then one every 5 seconds.
const RING_RATE: f64 = 0.2;
const RING_BURST: f64 = 4.0;
/// Idle and back: a Pyr changes it every few minutes at most (one in 10 seconds, after 6).
const IDLE_RATE: f64 = 0.1;
const IDLE_BURST: f64 = 6.0;
/// Making sure of people takes a handful of messages each time (one every 2 seconds, after 20).
const HANDSHAKE_RATE: f64 = 0.5;
const HANDSHAKE_BURST: f64 = 20.0;
/// The most vouches sent back at once.
const VOUCHES_SENT_MAX: usize = 2000;
/// History pages are kept well under the frame limit (long messages add up).
const HISTORY_PAGE_BYTES: usize = 700 * 1024;
/// Who can read a conversation, at most this many at a time (`ChannelReaders`).
const READERS_PAGE: usize = 2000;
/// How an encrypted picture's file is listed (the server doesn't know what it is).
const SEALED_MIME: &str = "application/octet-stream";
/// Video one person may send, in bytes per second and burst (Pyr's sharpest screen share
/// plus a camera is about 1.5 MB/s): frames past this are dropped, not passed on to
/// everyone watching.
const VIDEO_BYTES_PER_SEC: f64 = 5.0 * 1024.0 * 1024.0;
const VIDEO_BURST_BYTES: f64 = 24.0 * 1024.0 * 1024.0;
/// Sound one person may send (their microphone and their stream's sound, 50 packets a second
/// each, up to about 40 KB/s together at Pyr's best quality): packets past this are dropped,
/// not copied to everyone listening.
const AUDIO_PACKETS_PER_SEC: f64 = 200.0;
const AUDIO_PACKET_BURST: f64 = 400.0;
const AUDIO_BYTES_PER_SEC: f64 = 96.0 * 1024.0;
const AUDIO_BURST_BYTES: f64 = 192.0 * 1024.0;

pub async fn handle(hub: Arc<Hub>, incoming: quinn::Incoming, mut slot: ConnSlot) {
    let remote = incoming.remote_address();
    // From the first packet to logging in: a connection that's slower than this gives its
    // place up (see `hub::LOGIN_WITHIN`).
    let deadline = tokio::time::Instant::now() + crate::hub::LOGIN_WITHIN;
    let conn = match tokio::time::timeout_at(deadline, incoming).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            debug!(%remote, "handshake failed: {e}");
            return;
        }
        Err(_) => {
            debug!(%remote, "handshake too slow");
            return;
        }
    };
    slot.handshake_done();
    match session(hub, conn.clone(), &mut slot, deadline).await {
        Ok(()) => debug!(%remote, "connection closed"),
        Err(e) => debug!(%remote, "connection ended: {e:#}"),
    }
    conn.close(quinn::VarInt::from_u32(0), b"bye");
    drop(slot);
}

/// Most of a submission's files sent with `PluginSubmission` (bytes, before base64): what fits
/// one reply with room to spare.
const INLINE_SUBMISSION_BYTES: usize = 600_000;

async fn read_frame<T: serde::de::DeserializeOwned>(recv: &mut quinn::RecvStream) -> Result<Option<T>> {
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = framing::body_len(len)?;
    let mut body = vec![0u8; n];
    recv.read_exact(&mut body).await?;
    Ok(Some(framing::decode(&body)?))
}

/// Whether `sig` is `author_id`'s signature over a submission transcript for `digest` on this
/// connection (the server's nonce, the TLS keying material).
fn signed_by(conn: &quinn::Connection, nonce: &[u8], author_id: &str, digest: &[u8], sig: &str) -> bool {
    let mut exporter = [0u8; 32];
    if conn.export_keying_material(&mut exporter, crypto::SUBMISSION_EXPORTER_LABEL, b"").is_err() {
        return false;
    }
    let Ok(sig) = pyr_proto::b64::decode(sig) else { return false };
    crypto::is_valid_user_id(author_id) && crypto::verify(author_id, &crypto::submission_transcript(nonce, &exporter, author_id, digest), &sig)
}

/// A submission: the rest of its files (`parts`, read only if it would be taken), its
/// signature checked if it has one, then taken in. Returns its id and whether it was signed.
async fn take_submission(
    hub: &Hub,
    conn: &quinn::Connection,
    recv: &mut quinn::RecvStream,
    nonce: &[u8],
    mut sub: PluginSubmission,
) -> std::result::Result<(i64, bool), String> {
    let ip = conn.remote_address().ip();
    hub.may_take_submission(ip)?;
    if sub.parts > MAX_SUBMISSION_PARTS {
        return Err(format!("It comes in {} parts: {MAX_SUBMISSION_PARTS} at most.", sub.parts));
    }
    if sub.parts > 0 {
        let until = tokio::time::Instant::now() + Duration::from_secs(120);
        for _ in 0..sub.parts {
            let part: Option<ClientFrame> = tokio::time::timeout_at(until, read_frame(recv))
                .await
                .map_err(|_| "The rest of it didn't arrive in time.".to_string())?
                .map_err(|e| format!("The rest of it didn't arrive: {e}"))?;
            match part.map(|f| f.req) {
                Some(Request::SubmissionPart { files }) => {
                    if sub.files.len() + files.len() > SUBMISSION_FILES_MAX {
                        return Err("Too many files.".into());
                    }
                    sub.files.extend(files);
                }
                _ => return Err("Expected the rest of its files.".into()),
            }
        }
    }
    // The files that came are the ones it lists (and so the ones its signature covers).
    if !sub.manifest.is_empty() {
        let mut said = sub.manifest.clone();
        said.sort();
        if manifest_of(&sub.files) != said {
            return Err("Its files aren't the ones it lists.".into());
        }
    }
    let signed = !sub.author_id.is_empty();
    if signed && !signed_by(conn, nonce, &sub.author_id, &submission_digest(&sub), &sub.sig) {
        return Err("Its signature doesn't check out.".into());
    }
    let id = hub.take_plugin_submission(ip, sub, signed)?;
    Ok((id, signed))
}

async fn session(hub: Arc<Hub>, conn: quinn::Connection, slot: &mut ConnSlot, deadline: tokio::time::Instant) -> Result<()> {
    let (mut send, mut recv) = tokio::time::timeout_at(deadline, conn.accept_bi()).await.context("no control stream")??;
    // Messages to this app go ahead of the video sent to it (see `media::CONTROL_PRIORITY`).
    if !pyr_proto::ab::old("fair") {
        let _ = send.set_priority(pyr_proto::media::CONTROL_PRIORITY);
    }
    let mut preamble = [0u8; 4];
    tokio::time::timeout_at(deadline, recv.read_exact(&mut preamble)).await.context("no preamble")??;
    if &preamble != pyr_proto::CONTROL_PREAMBLE {
        bail!("not a Pyr client");
    }

    let nonce = crypto::random_bytes::<32>();
    let hello = Event::Hello { proto: PROTOCOL_VERSION, server_name: hub.info().name, nonce: nonce.to_vec() };
    send.write_all(&frame(0, hello)).await?;

    let login: ClientFrame =
        tokio::time::timeout_at(deadline, read_frame(&mut recv)).await.context("login timeout")??.ok_or_else(|| anyhow!("closed"))?;
    let reject = |code: ErrorCode, msg: &str| frame(login.id, Event::error(code, msg));
    // Not logging in, just a bug report (anyone may send one where they're collected).
    if let Request::ReportBug { report } = login.req {
        let reply = match hub.take_bug_report(conn.remote_address().ip(), report) {
            Ok(None) => frame(login.id, Event::Ok),
            Ok(Some(title)) => frame(login.id, Event::IdeaJoined { title }),
            Err(msg) => reject(ErrorCode::RateLimited, &msg),
        };
        send.write_all(&reply).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        return Ok(());
    }
    // Nor this: a plugin or skin sent in for the Pyr Library (where they're collected),
    // with the rest of its files after it if it's big, and signed by its sender if they can
    // send new versions of it later.
    if let Request::SubmitPlugin { submission } = login.req {
        let ip = conn.remote_address().ip();
        let reply = match take_submission(&hub, &conn, &mut recv, &nonce, submission).await {
            Ok((id, true)) => frame(login.id, Event::Submitted { id }),
            Ok((_, false)) => frame(login.id, Event::Ok),
            Err(msg) => {
                debug!(%ip, "submission refused: {msg}");
                reject(ErrorCode::RateLimited, &msg)
            }
        };
        send.write_all(&reply).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        return Ok(());
    }
    // Or asking for one's own (by a Pyr id this connection proves).
    if let Request::MySubmissions { author_id, sig } = login.req {
        let ok = signed_by(&conn, &nonce, &author_id, &my_submissions_digest(), &sig);
        let reply = if !ok {
            reject(ErrorCode::Unauthorized, "That signature doesn't check out.")
        } else {
            match hub.my_submissions(conn.remote_address().ip(), &author_id) {
                Ok(list) => frame(login.id, Event::MySubmissions { list }),
                Err(msg) => reject(ErrorCode::RateLimited, &msg),
            }
        };
        send.write_all(&reply).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        return Ok(());
    }
    let Request::Login { proto, user_id, sig, name, invite, client, caps } = login.req else {
        send.write_all(&reject(ErrorCode::BadRequest, "expected login")).await?;
        bail!("first request was not a login");
    };
    if proto != PROTOCOL_VERSION {
        let msg = format!("This server speaks protocol v{PROTOCOL_VERSION}; your app speaks v{proto}. Please update.");
        send.write_all(&reject(ErrorCode::VersionMismatch, &msg)).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        bail!("protocol mismatch");
    }
    let mut exporter = [0u8; 32];
    conn.export_keying_material(&mut exporter, crypto::LOGIN_EXPORTER_LABEL, b"")
        .map_err(|_| anyhow!("TLS exporter unavailable"))?;
    // Exactly the way we write ids: the same key spelled with padding or spaces would
    // otherwise count as someone else (a way around a ban) or bloat everything it's sent in.
    let canonical = crypto::is_valid_user_id(&user_id)
        && pyr_proto::b64::decode(&user_id).map(|b| pyr_proto::b64::encode(&b) == user_id).unwrap_or(false);
    if !canonical || !crypto::verify(&user_id, &crypto::login_transcript(&nonce, &exporter, &user_id), &sig) {
        send.write_all(&reject(ErrorCode::Unauthorized, "identity proof failed")).await?;
        bail!("bad login signature");
    }
    let name = clean_member_name(&name).unwrap_or_else(|| "Anonymous".to_string());
    let client: String = client.chars().filter(|c| !c.is_control()).take(48).collect();
    if let Err((code, msg)) = hub.authorize(&user_id, &name, invite.as_deref(), conn.remote_address().ip()) {
        send.write_all(&reject(code, &msg)).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        bail!("login refused: {msg}");
    }
    info!(user = %&user_id[..8], %name, %client, remote = %conn.remote_address(), "logged in");
    slot.logged_in();

    let (tx, mut rx) = Outbox::new(conn.clone());
    let queued = tx.queued();
    // Ends when the connection does (once every `Outbox` for it is gone) or a write fails.
    let writer = tokio::spawn(async move {
        while let Some(b) = rx.recv().await {
            if send.write_all(&b).await.is_err() {
                break;
            }
            queued.fetch_sub(b.len(), Ordering::Relaxed);
        }
        let _ = send.finish();
    });

    let conn_id = hub.alloc_conn_id();
    let session_cell = Arc::new(AtomicU32::new(0));
    let groups = caps.iter().any(|c| c == GROUPS_CAP);
    let feedback = caps.iter().any(|c| c == VIDEO_FEEDBACK_CAP);
    let movable = caps.iter().any(|c| c == MOVE_CAP);
    let casino = if caps.iter().any(|c| c == CASINO_013_CAP) {
        pyr_poker::net::CASINO_013
    } else if caps.iter().any(|c| c == CASINO_012_CAP) {
        pyr_poker::net::CASINO_012
    } else {
        pyr_poker::net::CASINO_011
    };
    if let Err((code, msg)) = hub.register(conn_id, &user_id, conn.clone(), tx.clone(), session_cell.clone(), login.id, groups, feedback, movable, casino) {
        tx.send(frame(login.id, Event::error(code, msg.clone())));
        drop(tx);
        let _ = timeout(Duration::from_secs(1), writer).await;
        bail!("not registered: {msg}");
    }

    let dg = tokio::spawn(datagram_loop(hub.clone(), conn.clone(), session_cell.clone()));
    let uni = tokio::spawn(uni_loop(hub.clone(), conn.clone(), session_cell.clone()));
    let files = tokio::spawn(file_loop(hub.clone(), conn.clone(), user_id.clone()));

    let mut ctx = Ctx {
        hub: &hub,
        conn_id,
        uid: user_id.clone(),
        groups,
        casino,
        msg_tokens: Bucket::new(1.5, 10.0),
        react_tokens: Bucket::new(3.0, 15.0),
        req_tokens: Bucket::new(REQ_RATE, REQ_BURST),
        sound_tokens: Bucket::new(SOUND_RATE, SOUND_BURST),
        ring_tokens: Bucket::new(RING_RATE, RING_BURST),
        idle_tokens: Bucket::new(IDLE_RATE, IDLE_BURST),
        handshake_tokens: Bucket::new(HANDSHAKE_RATE, HANDSHAKE_BURST),
    };
    let result = async {
        loop {
            let Some(f) = read_frame::<ClientFrame>(&mut recv).await? else { return Ok(()) };
            let id = f.id;
            let reply = if !ctx.req_tokens.take(cost(&f.req)) {
                Err((ErrorCode::RateLimited, "Slow down a little.".into()))
            } else if let Request::SetVanity { code } = f.req {
                // Answered once pyr.gg has it (other requests carry on meanwhile).
                match ctx.require(perm::MANAGE_SERVER, "give the community its own link") {
                    Err(e) => Err(e),
                    Ok(()) => {
                        let (hub, tx, uid) = (hub.clone(), tx.clone(), ctx.uid.clone());
                        tokio::spawn(async move {
                            let ev = hub.set_vanity(&uid, code).await.unwrap_or_else(|(code, message)| Event::Error { code, message });
                            if id != 0 {
                                tx.send(frame(id, ev));
                            }
                        });
                        continue;
                    }
                }
            } else {
                match ctx.handle(f.req) {
                    // A new invite: answered with its short link once pyr.gg lists it.
                    Ok(ev @ Event::InviteCreated { .. }) if hub.short_links_on() => {
                        let (hub, tx) = (hub.clone(), tx.clone());
                        tokio::spawn(async move {
                            let ev = hub.with_short_link(ev).await;
                            if id != 0 {
                                tx.send(frame(id, ev));
                            }
                        });
                        continue;
                    }
                    r => r,
                }
            };
            let ev = reply.unwrap_or_else(|(code, message)| Event::Error { code, message });
            if id != 0 {
                let mut b = frame(id, ev);
                if b.len() > pyr_proto::MAX_CONTROL_FRAME {
                    b = frame(id, Event::error(ErrorCode::Internal, "That reply was too big to send."));
                }
                tx.send(b);
            }
        }
    }
    .await;

    hub.unregister(conn_id);
    dg.abort();
    uni.abort();
    files.abort();
    drop(tx);
    let _ = timeout(Duration::from_secs(1), writer).await;
    result
}

/// Audio datagrams: rewrite the session id and fan out.
async fn datagram_loop(hub: Arc<Hub>, conn: quinn::Connection, session: Arc<AtomicU32>) {
    let mut targets = Vec::new();
    let (mut packets, mut bytes) = (Bucket::new(AUDIO_PACKETS_PER_SEC, AUDIO_PACKET_BURST), Bucket::new(AUDIO_BYTES_PER_SEC, AUDIO_BURST_BYTES));
    while let Ok(d) = conn.read_datagram().await {
        let s = session.load(Ordering::Relaxed);
        if s == 0 {
            continue;
        }
        // More than anyone's app sends: dropped here, before it's copied to everyone.
        if !packets.take(1.0) || !bytes.take(d.len() as f64) {
            continue;
        }
        let Some(h) = AudioHeader::parse(&d) else { continue };
        if h.track != track::MIC && h.track != track::SCREEN_AUDIO {
            continue;
        }
        hub.audio_targets(s, h.track, &mut targets);
        if targets.is_empty() {
            continue;
        }
        let mut b = BytesMut::from(&d[..]);
        media::set_session(&mut b, s);
        let b = b.freeze();
        for t in &targets {
            let _ = t.send_datagram(b.clone());
        }
    }
}

/// Video frames: one unidirectional stream per frame. Only from someone in voice, a few at
/// a time, and each read no further than its header says.
async fn uni_loop(hub: Arc<Hub>, conn: quinn::Connection, session: Arc<AtomicU32>) {
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_VIDEO_READS));
    let budget = Arc::new(parking_lot::Mutex::new(Bucket::new(VIDEO_BYTES_PER_SEC, VIDEO_BURST_BYTES)));
    while let Ok(mut stream) = conn.accept_uni().await {
        if session.load(Ordering::Relaxed) == 0 {
            let _ = stream.stop(quinn::VarInt::from_u32(0));
            continue;
        }
        let Ok(permit) = slots.clone().try_acquire_owned() else {
            // Too many at once: drop this frame (the sender sends a keyframe when asked).
            let _ = stream.stop(quinn::VarInt::from_u32(0));
            continue;
        };
        let hub = hub.clone();
        let session = session.clone();
        let budget = budget.clone();
        tokio::spawn(async move {
            let _permit = permit;
            let read = async {
                let mut head = [0u8; media::VIDEO_HEADER_LEN];
                stream.read_exact(&mut head).await.ok()?;
                let h = VideoHeader::parse(&head)?;
                let len = h.len as usize;
                if len > MAX_VIDEO_FRAME || !budget.lock().take(len as f64) {
                    return None;
                }
                let mut data = BytesMut::with_capacity(media::VIDEO_HEADER_LEN + len);
                data.extend_from_slice(&head);
                data.resize(media::VIDEO_HEADER_LEN + len, 0);
                stream.read_exact(&mut data[media::VIDEO_HEADER_LEN..]).await.ok()?;
                Some((h, data))
            };
            let Ok(Some((h, mut data))) = timeout(Duration::from_secs(5), read).await else {
                let _ = stream.stop(quinn::VarInt::from_u32(0));
                return;
            };
            let s = session.load(Ordering::Relaxed);
            if s == 0 {
                return;
            }
            media::set_session(&mut data, s);
            hub.forward_video(s, h.track, h.is_keyframe(), data.freeze());
        });
    }
}

/// File streams: uploads and downloads of images (see [`FILE_PREAMBLE`]).
async fn file_loop(hub: Arc<Hub>, conn: quinn::Connection, uid: String) {
    // At most a few transfers at once per connection.
    let slots = Arc::new(tokio::sync::Semaphore::new(4));
    while let Ok((send, recv)) = conn.accept_bi().await {
        let (hub, uid, slots) = (hub.clone(), uid.clone(), slots.clone());
        tokio::spawn(async move {
            let Ok(_permit) = slots.acquire_owned().await else { return };
            if let Err(e) = file_stream(&hub, &uid, send, recv).await {
                debug!("file stream ended: {e:#}");
            }
        });
    }
}

/// Whether file `row` is kept from member `uid`. A picture sent in a direct message (or a
/// group) is only for its people, and one in a private channel only for those who can see it;
/// one not sent yet is only its uploader's, unless it's one of the server's GIFs. (If its
/// message can't be found, nobody gets it: never fail open.)
fn hidden_from(hub: &Hub, uid: &str, row: &crate::db::FileRow) -> bool {
    match row.message_id {
        None => matches!(row.kind, FileKind::Attachment | FileKind::Sealed) && row.owner != uid && !hub.db.is_gif(&row.id),
        Some(m) => match hub.db.get_message(m) {
            None => true,
            Some(msg) => match hub.db.private_members(msg.channel_id) {
                Some(who) => !who.iter().any(|w| w == uid),
                None => hub.db.channel(msg.channel_id).is_some_and(|ch| !hub.sees(uid, &ch)),
            },
        },
    }
}

async fn file_stream(hub: &Hub, uid: &str, mut send: quinn::SendStream, mut recv: quinn::RecvStream) -> Result<()> {
    let mut preamble = [0u8; 4];
    timeout(Duration::from_secs(10), recv.read_exact(&mut preamble)).await??;
    if &preamble != FILE_PREAMBLE {
        bail!("not a file stream");
    }
    let req: FileRequest = timeout(Duration::from_secs(10), read_frame(&mut recv)).await??.ok_or_else(|| anyhow!("no request"))?;
    let reply = |r: FileReply| framing::encode(&r);
    let fail = |msg: &str| reply(FileReply { ok: false, id: String::new(), mime: String::new(), size: 0, error: msg.to_string() });
    match req {
        FileRequest::Get { id } => {
            let (Some(row), Some(path)) = (hub.db.file(&id), hub.file_path(&id)) else {
                send.write_all(&fail("That file is gone.")).await?;
                let _ = send.finish();
                return Ok(());
            };
            let private = hidden_from(hub, uid, &row);
            let bytes = if private { Vec::new() } else { tokio::fs::read(&path).await.unwrap_or_default() };
            if bytes.len() as u64 != row.size || private {
                send.write_all(&fail("That file is gone.")).await?;
                let _ = send.finish();
                return Ok(());
            }
            send.write_all(&reply(FileReply { ok: true, id, mime: row.mime, size: row.size, error: String::new() })).await?;
            send.write_all(&bytes).await?;
            let _ = send.finish();
        }
        FileRequest::Backup { item } => {
            backup_item(hub, uid, &item, &mut send).await?;
        }
        FileRequest::Put { kind, name, mime: _, size } => {
            let limit = match kind {
                FileKind::Attachment => MAX_ATTACHMENT_BYTES,
                // (Its tag on top.)
                FileKind::Sealed => MAX_ATTACHMENT_BYTES + 16,
                FileKind::Avatar => MAX_AVATAR_BYTES,
                FileKind::Sound => pyr_proto::sound::MAX_BYTES as u64,
            };
            if size == 0 || size > limit {
                let msg = if kind == FileKind::Sound {
                    format!("Sounds can be up to {} KB.", limit >> 10)
                } else {
                    format!("Images can be up to {} MB.", limit >> 20)
                };
                send.write_all(&fail(&msg)).await?;
                let _ = send.finish();
                return Ok(());
            }
            if kind == FileKind::Sound {
                return sound_put(hub, uid, &name, size, send, recv).await;
            }
            let slot = match hub.reserve_upload(uid, size) {
                Ok(s) => s,
                Err(msg) => {
                    send.write_all(&fail(&msg)).await?;
                    let _ = send.finish();
                    return Ok(());
                }
            };
            let bytes = timeout(Duration::from_secs(300), recv.read_to_end(size as usize + 1)).await??;
            if bytes.len() as u64 != size {
                send.write_all(&fail("The upload was cut short.")).await?;
                let _ = send.finish();
                return Ok(());
            }
            if kind == FileKind::Sealed {
                // Ciphertext: kept as it is, under a random id; what it is, only the Pyrs that
                // open its message know (and they check it's a picture).
                let id: String = crypto::random_bytes::<16>().iter().map(|b| format!("{b:02x}")).collect();
                let path = hub.file_path(&id).ok_or_else(|| anyhow!("bad id"))?;
                let tmp = path.with_extension("part");
                tokio::fs::write(&tmp, &bytes).await?;
                tokio::fs::rename(&tmp, &path).await?;
                hub.db.insert_file(&crate::db::FileRow {
                    id: id.clone(),
                    owner: uid.to_string(),
                    kind,
                    name: String::new(),
                    mime: SEALED_MIME.to_string(),
                    size,
                    width: 0,
                    height: 0,
                    message_id: None,
                })?;
                slot.stored();
                send.write_all(&reply(FileReply { ok: true, id, mime: SEALED_MIME.to_string(), size, error: String::new() })).await?;
                let _ = send.finish();
                let _ = timeout(Duration::from_secs(10), send.stopped()).await;
                return Ok(());
            }
            let Some(mime) = sniff_image(&bytes) else {
                send.write_all(&fail("Only PNG, JPEG, GIF and WebP images can be sent.")).await?;
                let _ = send.finish();
                return Ok(());
            };
            let (width, height) = image_size(&bytes).unwrap_or((0, 0));
            // Pictures that claim to be enormous are refused: decoding one would make every
            // app that shows it run out of memory.
            if !image_dimensions_ok(&bytes) {
                send.write_all(&fail("That image is too large (at most 8192 pixels across).")).await?;
                let _ = send.finish();
                return Ok(());
            }
            let id = match kind {
                // Profile pictures are named by their content, so an app can tell whether the
                // server already has its current one.
                FileKind::Avatar => avatar_id(uid, &bytes),
                // (Sounds are handled by `sound_put` before this.)
                FileKind::Attachment | FileKind::Sound | FileKind::Sealed => crypto::random_bytes::<16>().iter().map(|b| format!("{b:02x}")).collect(),
            };
            let path = hub.file_path(&id).ok_or_else(|| anyhow!("bad id"))?;
            let tmp = path.with_extension("part");
            tokio::fs::write(&tmp, &bytes).await?;
            tokio::fs::rename(&tmp, &path).await?;
            let name: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\')).take(100).collect();
            hub.db.insert_file(&crate::db::FileRow {
                id: id.clone(),
                owner: uid.to_string(),
                kind,
                name: if name.trim().is_empty() { "image".into() } else { name },
                mime: mime.to_string(),
                size,
                width,
                height,
                message_id: None,
            })?;
            slot.stored();
            send.write_all(&reply(FileReply { ok: true, id, mime: mime.to_string(), size, error: String::new() })).await?;
            let _ = send.finish();
        }
    }
    // Let the reply reach the other side before the stream is dropped.
    let _ = timeout(Duration::from_secs(10), send.stopped()).await;
    Ok(())
}

/// A soundboard sound (`FileKind::Sound`): checked (it has to be a sound file Pyr can play,
/// see `pyr_proto::sound`) and named by its content, so the same sound is kept once however
/// often it's sent.
async fn sound_put(hub: &Hub, uid: &str, name: &str, size: u64, mut send: quinn::SendStream, mut recv: quinn::RecvStream) -> Result<()> {
    let reply = |r: FileReply| framing::encode(&r);
    let fail = |msg: &str| reply(FileReply { ok: false, id: String::new(), mime: String::new(), size: 0, error: msg.to_string() });
    let slot = match hub.reserve_upload(uid, size) {
        Ok(s) => s,
        Err(msg) => {
            send.write_all(&fail(&msg)).await?;
            let _ = send.finish();
            return Ok(());
        }
    };
    let bytes = timeout(Duration::from_secs(120), recv.read_to_end(size as usize + 1)).await??;
    if bytes.len() as u64 != size {
        send.write_all(&fail("The upload was cut short.")).await?;
        let _ = send.finish();
        return Ok(());
    }
    if pyr_proto::sound::check(&bytes).is_none() {
        send.write_all(&fail("That isn't a sound Pyr can play.")).await?;
        let _ = send.finish();
        return Ok(());
    }
    const MIME: &str = "audio/x-pyr-sound";
    let id = pyr_proto::sound::sound_id(&bytes);
    let have = hub.db.file(&id).is_some_and(|f| f.kind == FileKind::Sound) && hub.file_path(&id).is_some_and(|p| p.is_file());
    if !have {
        let path = hub.file_path(&id).ok_or_else(|| anyhow!("bad id"))?;
        let tmp = path.with_extension("part");
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        let name: String = name.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\')).take(100).collect();
        hub.db.insert_file(&crate::db::FileRow {
            id: id.clone(),
            owner: uid.to_string(),
            kind: FileKind::Sound,
            name: if name.trim().is_empty() { "sound".into() } else { name },
            mime: MIME.to_string(),
            size,
            width: 0,
            height: 0,
            message_id: None,
        })?;
        slot.stored();
    }
    send.write_all(&reply(FileReply { ok: true, id, mime: MIME.to_string(), size, error: String::new() })).await?;
    let _ = send.finish();
    let _ = timeout(Duration::from_secs(10), send.stopped()).await;
    Ok(())
}

/// A soundboard sound's name: 1–32 characters, no control characters.
fn clean_sound_name(name: &str) -> Option<String> {
    let n: String = name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_string();
    (!n.is_empty() && n.chars().count() <= MAX_SOUND_NAME).then_some(n)
}

/// What a sound shows with it: one emoji, a few characters at most (or nothing).
fn clean_sound_emoji(e: &str) -> Option<String> {
    let e = e.trim();
    (e.len() <= 80 && !e.chars().any(|c| c.is_control() || c.is_whitespace())).then(|| e.to_string())
}

struct Ctx<'a> {
    hub: &'a Hub,
    conn_id: ConnId,
    uid: String,
    /// The app knows group conversations (`GROUPS_CAP`).
    groups: bool,
    /// How much of Pyr Casino the app knows (`pyr_poker::net::CASINO_011` to `CASINO_013`: see
    /// `CASINO_012_CAP` and `CASINO_013_CAP`).
    casino: u8,
    /// Messages and edits.
    msg_tokens: Bucket,
    react_tokens: Bucket,
    /// Every request (weighted by [`cost`]).
    req_tokens: Bucket,
    /// Soundboard sounds played.
    sound_tokens: Bucket,
    /// Rings and declined calls (direct messages' calls).
    ring_tokens: Bucket,
    /// Idle and back again (`SetIdle`): everyone hears of each change.
    idle_tokens: Bucket,
    /// Messages for other members' Pyrs (`Handshake`).
    handshake_tokens: Bucket,
}

/// A token bucket: `rate` per second, holding at most `burst`.
struct Bucket {
    tokens: f64,
    rate: f64,
    burst: f64,
    at: Instant,
}

impl Bucket {
    fn new(rate: f64, burst: f64) -> Bucket {
        Bucket { tokens: burst, rate, burst, at: Instant::now() }
    }

    fn take(&mut self, n: f64) -> bool {
        let now = Instant::now();
        self.tokens = (self.tokens + now.duration_since(self.at).as_secs_f64() * self.rate).min(self.burst);
        self.at = now;
        if self.tokens < n {
            return false;
        }
        self.tokens -= n;
        true
    }
}

/// How much of the request allowance a request uses: database-heavy or things that grow the
/// database cost more.
fn cost(req: &Request) -> f64 {
    match req {
        Request::Ping { .. } | Request::Typing { .. } | Request::VoiceState { .. } => 0.5,
        Request::FetchHistory { .. } | Request::GetVouches { .. } | Request::GetVouchesForMe {} | Request::ListInvites {} => 3.0,
        Request::E2eeKeys { keys, .. } => 1.0 + keys.len() as f64 / 16.0,
        Request::ShareKeys { shares, .. } => 1.0 + shares.len() as f64 / 50.0,
        Request::GetKeys { keys, .. } => 1.0 + keys.len() as f64 / 20.0,
        Request::ChannelReaders { .. } => 2.0,
        Request::CreateInvite { .. }
        | Request::CreateBotInvite { .. }
        | Request::RevokeInvite { .. }
        | Request::SetVanity { .. }
        | Request::SetShortLinks { .. }
        | Request::FriendRequest { .. }
        | Request::OpenDm { .. }
        | Request::AddEmoji { .. }
        | Request::AddSound { .. }
        | Request::AddGif { .. }
        | Request::CreateChannel { .. }
        | Request::SetChannelAccess { .. }
        | Request::CreateRole { .. }
        | Request::SetAvatar { .. }
        | Request::SetServerIcon { .. }
        | Request::SetGiphyKey { .. }
        | Request::SetKlipyKey { .. }
        | Request::UpdateProfile { .. }
        | Request::UpdateServer { .. }
        | Request::CreateGroup { .. }
        | Request::AddToGroup { .. }
        | Request::RenameGroup { .. }
        | Request::SetVouches { .. }
        | Request::Block { .. } => 4.0,
        _ => 1.0,
    }
}

/// A member id as we write them (anything else can't be anyone).
fn valid_uid(uid: &str) -> Result<(), (ErrorCode, String)> {
    if crypto::is_valid_user_id(uid) { Ok(()) } else { Err((ErrorCode::NotFound, "No such member.".into())) }
}

/// A Unicode emoji reaction: no letters, digits (except in keycaps), spaces or controls, and
/// short.
fn valid_unicode_reaction(e: &str) -> bool {
    let keycap = e.contains('\u{20e3}');
    !e.is_empty()
        && e.len() <= 32
        && e.chars().all(|c| {
            if c.is_ascii() {
                keycap && (c.is_ascii_digit() || c == '#' || c == '*')
            } else {
                !c.is_control() && !c.is_whitespace() && !is_invisible(c) || matches!(c, '\u{200d}' | '\u{fe0f}' | '\u{e0020}'..='\u{e007f}')
            }
        })
}

fn bad<T>(msg: &str) -> Result<T, (ErrorCode, String)> {
    Err((ErrorCode::BadRequest, msg.to_string()))
}

/// An encrypted message as the server takes it: well formed, and with its author's own key
/// (what's inside only the Pyrs that open it can check).
fn check_sealed(sealed: Option<Sealed>, uid: &str) -> Result<Sealed, (ErrorCode, String)> {
    match sealed {
        None => Err((ErrorCode::BadRequest, "Messages here are end-to-end encrypted: update Pyr to send them.".into())),
        Some(s) if !s.well_formed() || s.key.owner != uid => bad("that encrypted message isn't well formed"),
        Some(s) => Ok(s),
    }
}

impl Ctx<'_> {
    fn role(&self) -> Role {
        self.hub.db.get_user(&self.uid).map(|u| u.role).unwrap_or(Role::Member)
    }

    /// What this member may do: everything for admins and the owner, otherwise what their
    /// roles allow.
    fn perms(&self) -> u64 {
        if self.role().is_admin() {
            return perm::ALL;
        }
        let mine = self.hub.db.member_roles(&self.uid);
        self.hub.db.roles().iter().filter(|r| mine.contains(&r.id)).fold(0, |acc, r| acc | r.perms)
    }

    /// Someone else who's a member here.
    fn other_member(&self, uid: &str) -> Result<(), (ErrorCode, String)> {
        if uid == self.uid {
            return Err((ErrorCode::BadRequest, "That's you.".into()));
        }
        match self.hub.db.get_user(uid) {
            Some(u) if !u.banned => Ok(()),
            _ => Err((ErrorCode::NotFound, "They aren't a member of this server.".into())),
        }
    }

    /// Tells both people where they now stand with each other.
    fn tell_relation(&self, other: &str) {
        let db = &self.hub.db;
        self.hub.send_to_user(&self.uid, Event::RelationUpdated { user_id: other.to_string(), kind: db.relation(&self.uid, other) });
        self.hub.send_to_user(other, Event::RelationUpdated { user_id: self.uid.clone(), kind: db.relation(other, &self.uid) });
    }

    fn require(&self, p: u64, what: &str) -> Result<(), (ErrorCode, String)> {
        if self.can(p) { Ok(()) } else { Err((ErrorCode::Forbidden, format!("You don't have permission to {what}."))) }
    }

    /// Roles are given out (made, changed, handed to people or taken back) by Manage roles,
    /// but never by Pyr Casino's Game Masters or Ambassadors: only the server's owner names
    /// them, and they can't hand themselves or others power through a server role either.
    fn may_give_out_roles(&self) -> Result<(), (ErrorCode, String)> {
        self.require(perm::MANAGE_ROLES, "manage roles")?;
        if self.role() != Role::Owner && crate::poker::Poker::role(self.hub, &self.uid).is_some() {
            return Err((ErrorCode::Forbidden, "Pyr Casino's staff don't give out roles: only the server's owner does.".into()));
        }
        Ok(())
    }

    fn can(&self, p: u64) -> bool {
        self.perms() & p == p
    }

    fn require_owner(&self, what: &str) -> Result<(), (ErrorCode, String)> {
        if self.role() == Role::Owner { Ok(()) } else { Err((ErrorCode::Forbidden, format!("Only the server's owner can {what}."))) }
    }

    /// Where someone stands: the owner above admins above every custom role (by its
    /// position); members with no roles at the bottom.
    fn rank_of(&self, uid: &str) -> i64 {
        match self.hub.db.get_user(uid).map(|u| u.role) {
            Some(Role::Owner) => i64::MAX,
            Some(Role::Admin) => i64::MAX - 1,
            _ => {
                let mine = self.hub.db.member_roles(uid);
                self.hub.db.roles().iter().filter(|r| mine.contains(&r.id)).map(|r| r.position as i64).max().unwrap_or(i64::MIN)
            }
        }
    }

    /// Roles at or above your own highest one are out of your reach (unless you're an admin).
    fn can_manage_role(&self, r: &RoleDef) -> bool {
        self.role().is_admin() || (r.position as i64) < self.rank_of(&self.uid)
    }

    /// A text channel, or a direct message or group I'm in.
    fn can_see(&self, channel_id: ChannelId) -> Result<(), (ErrorCode, String)> {
        let db = &self.hub.db;
        if let Some(ch) = db.channel(channel_id) {
            // (A private channel only for the roles it's for, and admins: see `Channel::visible_to`.)
            return if self.hub.sees(&self.uid, &ch) { Ok(()) } else { Err((ErrorCode::NotFound, "No such channel.".into())) };
        }
        if let Some((a, b)) = db.dm_members(channel_id) {
            if a == self.uid || b == self.uid {
                return Ok(());
            }
        } else if let Some(g) = db.group(channel_id) {
            if self.groups && g.members.contains(&self.uid) {
                return Ok(());
            }
        }
        Err((ErrorCode::NotFound, "No such channel.".into()))
    }

    /// Where I can send messages: a channel I may post in, or a conversation I'm in.
    fn can_post(&self, channel_id: ChannelId) -> Result<(), (ErrorCode, String)> {
        self.can_see(channel_id)?;
        if let Some(ch) = self.hub.db.channel(channel_id) {
            let (admin, roles) = self.hub.access_of(&self.uid);
            if !ch.postable_by(admin, &roles) {
                return Err((ErrorCode::Forbidden, "You don't have permission to send messages in this channel.".into()));
            }
        }
        Ok(())
    }

    /// Whether someone here is a bot.
    fn is_bot(&self, uid: &str) -> bool {
        self.hub.db.get_user(uid).is_some_and(|u| u.bot)
    }

    /// Roles that exist here, each once (a channel's access lists).
    fn real_roles(&self, ids: &[RoleId]) -> Vec<RoleId> {
        let all = self.hub.db.roles();
        let mut v: Vec<RoleId> = ids.iter().copied().filter(|id| all.iter().any(|r| r.id == *id)).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The others in a private conversation I'm in (a direct message's other person, or a
    /// group's other members), for its call. Not someone I've blocked or who blocked me (in a
    /// direct message).
    fn call_others(&self, channel_id: ChannelId) -> Result<Vec<UserId>, (ErrorCode, String)> {
        let db = &self.hub.db;
        if db.dm_members(channel_id).is_some() {
            return self.dm_partner(channel_id).map(|o| vec![o]);
        }
        match db.group(channel_id) {
            Some(g) if self.groups && g.members.contains(&self.uid) => Ok(g.members.into_iter().filter(|m| *m != self.uid).collect()),
            _ => Err((ErrorCode::NotFound, "No such conversation.".into())),
        }
    }

    /// Who may be put in a group by me: members here (not banned), not me, and not someone
    /// either of us blocked.
    fn group_candidates(&self, uids: &[UserId]) -> Result<Vec<UserId>, (ErrorCode, String)> {
        let db = &self.hub.db;
        let mut out: Vec<UserId> = Vec::new();
        for u in uids {
            valid_uid(u)?;
            if *u == self.uid || out.contains(u) {
                continue;
            }
            match db.get_user(u) {
                Some(x) if x.bot => return Err((ErrorCode::Forbidden, format!("{} is a bot: bots don't have conversations.", x.name))),
                Some(x) if !x.banned => {}
                _ => return Err((ErrorCode::NotFound, "Someone you picked isn't a member of this server.".into())),
            }
            if db.blocked_between(&self.uid, u) {
                return Err((ErrorCode::Forbidden, "You can't add someone you've blocked (or who blocked you).".into()));
            }
            out.push(u.clone());
        }
        Ok(out)
    }

    /// Tells everyone in a group how it is now (and who left, that they're out of it).
    fn tell_group(&self, channel_id: ChannelId, left: Option<&str>) {
        let hub = self.hub;
        match hub.db.group(channel_id) {
            Some(g) => {
                hub.group_call_members(channel_id, &g.members, left);
                hub.send_to_group_members(&g.members.clone(), Event::GroupUpserted { group: g });
            }
            None => hub.group_call_members(channel_id, &[], left),
        }
        if let Some(who) = left {
            hub.send_to_group_members(&[who.to_string()], Event::GroupRemoved { channel_id });
        }
    }

    fn take_message_token(&mut self) -> Result<(), (ErrorCode, String)> {
        if !self.msg_tokens.take(1.0) {
            return Err((ErrorCode::RateLimited, "You're sending messages too fast.".into()));
        }
        Ok(())
    }

    /// The other person in a direct message I'm in, for its call: not blocked either way.
    fn dm_partner(&self, channel_id: ChannelId) -> Result<UserId, (ErrorCode, String)> {
        let db = &self.hub.db;
        let other = match db.dm_members(channel_id) {
            Some((a, b)) if a == self.uid => b,
            Some((a, b)) if b == self.uid => a,
            _ => return Err((ErrorCode::NotFound, "No such conversation.".into())),
        };
        if db.blocked_between(&self.uid, &other) {
            return Err((ErrorCode::Forbidden, "You can't call this person.".into()));
        }
        Ok(other)
    }

    /// In a direct message, refuse if either has blocked the other.
    fn not_blocked_in(&self, channel_id: ChannelId) -> Result<(), (ErrorCode, String)> {
        let db = &self.hub.db;
        if let Some((x, y)) = db.dm_members(channel_id) {
            let other = if x == self.uid { y } else { x };
            if db.blocked_between(&self.uid, &other) {
                return Err((ErrorCode::Forbidden, "You can't message this person.".into()));
            }
        }
        Ok(())
    }

    fn handle(&mut self, req: Request) -> Reply {
        let hub = self.hub;
        let db = &hub.db;
        // Bots chat where their roles let them, and that's all: no voice, conversations,
        // calls, friends or invites, and they aren't people to make sure of, vouch for, or
        // send bug reports and plugins for.
        if matches!(
            req,
            Request::OpenDm { .. }
                | Request::CreateGroup { .. }
                | Request::AddToGroup { .. }
                | Request::Ring { .. }
                | Request::DeclineCall { .. }
                | Request::FriendRequest { .. }
                | Request::FriendAccept { .. }
                | Request::CreateInvite { .. }
                | Request::CreateBotInvite { .. }
                | Request::ListInvites {}
                | Request::RevokeInvite { .. }
                | Request::SetVanity { .. }
                | Request::SetShortLinks { .. }
                | Request::VoiceJoin { .. }
                | Request::Handshake { .. }
                | Request::SetVouches { .. }
                | Request::GetVouchesForMe {}
                | Request::ReportBug { .. }
                | Request::Poker { .. }
                | Request::SubmitPlugin { .. }
                | Request::SubmissionPart { .. }
        ) && self.is_bot(&self.uid)
        {
            return Err((ErrorCode::Forbidden, "Bots can't do that.".into()));
        }
        match req {
            Request::Login { .. } => bad("already logged in"),
            Request::Ping { t } => Ok(Event::Pong { t }),

            Request::SendMessage { channel_id, content: _, reply_to, nonce, attachments, sealed } => {
                self.take_message_token()?;
                if nonce.len() > MAX_NONCE_LEN {
                    return bad("bad message nonce");
                }
                if attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE {
                    return bad("too many attachments");
                }
                // Messages are end-to-end encrypted here: none are taken in the clear.
                let sealed = check_sealed(sealed, &self.uid)?;
                for id in &attachments {
                    match db.file(id) {
                        Some(f) if f.owner == self.uid && f.kind == FileKind::Sealed && f.message_id.is_none() => {}
                        _ => return bad("an attachment is missing; try sending it again"),
                    }
                }
                self.can_post(channel_id)?;
                self.not_blocked_in(channel_id)?;
                let reply_to = reply_to.filter(|id| db.get_message(*id).map(|m| m.channel_id == channel_id).unwrap_or(false));
                let message = db
                    .insert_message_with(channel_id, &self.uid, "", reply_to, &attachments, Some(&sealed))
                    .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_in(channel_id, Event::MessageCreated { message, nonce });
                Ok(Event::Ok)
            }
            Request::EditMessage { message_id, content: _, sealed } => {
                self.take_message_token()?;
                let sealed = check_sealed(sealed, &self.uid)?;
                let Some(m) = db.get_message(message_id) else { return Err((ErrorCode::NotFound, "gone".into())) };
                if m.author != self.uid {
                    return Err((ErrorCode::Forbidden, "You can only edit your own messages.".into()));
                }
                // (Not in a group you've left.)
                self.can_see(m.channel_id)?;
                self.not_blocked_in(m.channel_id)?;
                if let Ok(Some(message)) = db.edit_message(message_id, "", Some(&sealed)) {
                    hub.broadcast_in(message.channel_id, Event::MessageUpdated { message });
                }
                Ok(Event::Ok)
            }
            Request::DeleteMessage { message_id } => {
                let Some(m) = db.get_message(message_id) else { return Ok(Event::Ok) };
                // In direct messages only the author can; elsewhere moderators can too.
                let in_dm = db.private_members(m.channel_id).is_some();
                if m.author != self.uid && (in_dm || self.perms() & perm::MANAGE_MESSAGES == 0 || self.can_see(m.channel_id).is_err()) {
                    return Err((ErrorCode::Forbidden, "You can't delete that message.".into()));
                }
                let files = db.delete_message(message_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.wipe_files(&files);
                hub.broadcast_in(m.channel_id, Event::MessageDeleted { channel_id: m.channel_id, message_id });
                Ok(Event::Ok)
            }
            Request::React { message_id, emoji, add } => {
                if !self.react_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "You're reacting too fast.".into()));
                }
                let emoji = emoji.trim().to_string();
                // A server emoji (`<:name:id>`, which must exist here; kept as the server
                // writes it) or a real emoji.
                let emoji = if emoji.starts_with('<') && emoji.ends_with('>') && emoji.len() <= 120 {
                    let id = emoji.trim_end_matches('>').rsplit(':').next().unwrap_or("");
                    match db.emojis().into_iter().find(|e| e.id == id) {
                        Some(e) if e.animated => format!("<a:{}:{}>", e.name, e.id),
                        Some(e) => format!("<:{}:{}>", e.name, e.id),
                        None => return bad("not an emoji on this server"),
                    }
                } else if valid_unicode_reaction(&emoji) {
                    emoji
                } else {
                    return bad("not an emoji");
                };
                let Some(m) = db.get_message(message_id) else { return Err((ErrorCode::NotFound, "gone".into())) };
                self.can_see(m.channel_id)?;
                if add {
                    self.not_blocked_in(m.channel_id)?;
                }
                if add && db.distinct_reactions(message_id) >= 20 {
                    return bad("too many different reactions");
                }
                db.set_reaction(message_id, &emoji, &self.uid, add).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(message) = db.get_message(message_id) {
                    hub.broadcast_in(message.channel_id, Event::MessageUpdated { message });
                }
                Ok(Event::Ok)
            }
            Request::PublishMessageKey { key } => {
                if key.user != self.uid || key.verified().is_none() {
                    return bad("that isn't your message key");
                }
                db.set_message_key(&key).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                // Ready for keys: what others asked for that this Pyr has.
                hub.send_pending_wants(&self.uid, self.conn_id);
                Ok(Event::Ok)
            }
            Request::ChannelReaders { channel_id, after } => {
                self.can_see(channel_id)?;
                let mut all = hub.readers(channel_id).unwrap_or_default();
                all.sort();
                all.dedup();
                if let Some(after) = after {
                    all.retain(|u| *u > after);
                }
                let more = all.len() > READERS_PAGE;
                all.truncate(READERS_PAGE);
                let mut keys = db.message_keys(&all);
                let (mut readers, mut keyless) = (Vec::new(), Vec::new());
                for u in all {
                    match keys.remove(&u) {
                        Some(k) => readers.push(k),
                        None => keyless.push(u),
                    }
                }
                Ok(Event::Readers { channel_id, readers, keyless, more })
            }
            Request::ShareKeys { channel_id, shares } => {
                self.can_see(channel_id)?;
                if shares.len() > MAX_SHARES || !shares.iter().all(|s| s.well_formed()) {
                    return bad("bad key shares");
                }
                // (Receivers need my message key to open them.)
                let Some(mine) = db.message_keys(std::slice::from_ref(&self.uid)).remove(&self.uid) else {
                    return bad("publish your message key first");
                };
                let readers: HashSet<UserId> = hub.readers(channel_id).unwrap_or_default().into_iter().collect();
                // Only to people who can read here, and only keys that are mine or I was given.
                let mut known: HashMap<KeyRef, bool> = HashMap::new();
                let ok: Vec<NewShare> = shares
                    .into_iter()
                    .filter(|s| readers.contains(&s.to))
                    .filter(|s| *known.entry(s.key.clone()).or_insert_with(|| db.has_key(channel_id, &s.key, &self.uid)))
                    .collect();
                let kept = db.add_shares(channel_id, &self.uid, &ok).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                let mut to: HashMap<UserId, Vec<KeyShare>> = HashMap::new();
                for s in kept {
                    to.entry(s.to.clone()).or_default().push(KeyShare { key: s.key, from: mine.clone(), nonce: s.nonce, ct: s.ct });
                }
                for (uid, shares) in to {
                    hub.send_to_user(&uid, Event::KeyShares { channel_id, shares });
                }
                Ok(Event::Ok)
            }
            Request::GetKeys { channel_id, keys } => {
                self.can_see(channel_id)?;
                if keys.len() > MAX_KEYS_ASKED || !keys.iter().all(|k| k.well_formed()) {
                    return bad("bad keys");
                }
                let shares = db.shares_for(channel_id, &self.uid, &keys);
                let mut missing: Vec<KeyRef> = keys.into_iter().filter(|k| !shares.iter().any(|s| &s.key == k)).collect();
                missing.sort();
                missing.dedup();
                if !missing.is_empty() {
                    db.add_wants(channel_id, &self.uid, &missing).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                    if let Some(by) = db.message_keys(std::slice::from_ref(&self.uid)).remove(&self.uid) {
                        hub.ask_holders(channel_id, &by, &missing);
                    }
                }
                Ok(Event::KeyShares { channel_id, shares })
            }
            Request::FetchHistory { channel_id, before, limit } => {
                self.can_see(channel_id)?;
                let (mut messages, mut complete) = db.history(channel_id, before, limit);
                // Long messages add up: keep the page well inside one frame, dropping the
                // oldest (the app asks for those next).
                let size = |m: &Message| {
                    m.content.len()
                        + m.sealed.as_ref().map(|s| s.ct.len() + 300).unwrap_or(0)
                        + 400
                        + m.attachments.iter().map(|a| a.name.len() + 160).sum::<usize>()
                        + m.reactions.iter().map(|r| r.emoji.len() + 24 + r.users.iter().map(|u| u.len() + 3).sum::<usize>()).sum::<usize>()
                };
                let mut total: usize = messages.iter().map(size).sum();
                while total > HISTORY_PAGE_BYTES && messages.len() > 1 {
                    total -= size(&messages.remove(0));
                    complete = false;
                }
                Ok(Event::History { channel_id, messages, complete })
            }
            Request::Typing { channel_id } => match self.can_post(channel_id).and_then(|_| self.not_blocked_in(channel_id)) {
                Ok(()) => hub.typing(self.conn_id, &self.uid, channel_id),
                Err(_) => Ok(Event::Ok),
            },
            Request::SetIdle { idle } => {
                if !self.idle_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "Slow down a little.".into()));
                }
                hub.set_idle(self.conn_id, idle);
                Ok(Event::Ok)
            }
            Request::SetDnd { dnd } => {
                if !self.idle_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "Slow down a little.".into()));
                }
                hub.set_dnd(self.conn_id, dnd);
                Ok(Event::Ok)
            }
            Request::Handshake { to, data } => {
                if !self.handshake_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "Slow down a little.".into()));
                }
                if data.len() > pyr_proto::handshake::HANDSHAKE_MAX {
                    return bad("that's too long");
                }
                valid_uid(&to)?;
                if to == self.uid {
                    return bad("that's you");
                }
                if !crate::db::user_is_member(db, &to) || !hub.is_online(&to) {
                    return Err((ErrorCode::NotFound, "They aren't online here right now.".into()));
                }
                // Between people where one blocked the other it goes nowhere, and nothing says so.
                if !db.blocked_between(&self.uid, &to) {
                    hub.send_to_user(&to, Event::Handshake { from: self.uid.clone(), data });
                }
                Ok(Event::Ok)
            }
            Request::SetVouches { vouches } => {
                if vouches.len() > MAX_VOUCHES {
                    return bad("that's more vouches than a server keeps");
                }
                if vouches.iter().any(|v| v.by != self.uid || !v.valid() || !crate::db::user_is_member(db, &v.for_uid)) {
                    return bad("a vouch that isn't yours, isn't signed, or isn't for a member here");
                }
                db.set_vouches(&self.uid, &vouches).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                Ok(Event::Ok)
            }
            Request::GetVouches { by } => {
                if by.len() > MAX_VOUCHES_ASKED {
                    return bad("that's too many people at once");
                }
                Ok(Event::Vouches { vouches: db.vouches_by(&by, VOUCHES_SENT_MAX) })
            }
            Request::GetVouchesForMe {} => Ok(Event::Vouches { vouches: db.vouches_for(&self.uid, VOUCHES_SENT_MAX) }),
            Request::OpenDm { user_id } => {
                valid_uid(&user_id)?;
                if user_id == self.uid {
                    return bad("you can't message yourself");
                }
                match db.get_user(&user_id) {
                    Some(u) if u.bot => return Err((ErrorCode::Forbidden, "Bots don't have conversations: talk to them in a channel.".into())),
                    Some(u) if !u.banned => {}
                    _ => return Err((ErrorCode::NotFound, "They aren't a member of this server.".into())),
                }
                if db.blocked_between(&self.uid, &user_id) {
                    return Err((ErrorCode::Forbidden, "You can't message this person.".into()));
                }
                let (id, new) = db.open_dm(&self.uid, &user_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if new {
                    let theirs = Dm { id, with: self.uid.clone(), last_message: None, last_from: None };
                    hub.send_to_user(&user_id, Event::DmOpened { dm: theirs });
                }
                let mine = db.dms_of(&self.uid).into_iter().find(|d| d.id == id);
                let (last_message, last_from) = mine.map(|d| (d.last_message, d.last_from)).unwrap_or((None, None));
                Ok(Event::DmOpened { dm: Dm { id, with: user_id, last_message, last_from } })
            }

            Request::CreateChannel { name, kind, private, view_roles } => {
                self.require(perm::MANAGE_CHANNELS, "manage channels")?;
                let Some(name) = clean_channel_name(&name, kind) else { return bad("channel name is empty") };
                let view_roles = self.real_roles(&view_roles);
                let channel = db.create_channel_with(&name, kind, private, &view_roles).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.channel_changed(&channel);
                Ok(Event::Ok)
            }
            Request::MoveChannel { channel_id, before } => {
                self.require(perm::MANAGE_CHANNELS, "manage channels")?;
                match db.move_channel(channel_id, before).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    None => Err((ErrorCode::NotFound, "No such channel.".into())),
                    Some(changed) => {
                        for ch in &changed {
                            hub.channel_changed(ch);
                        }
                        Ok(Event::Ok)
                    }
                }
            }
            Request::SetChannelAccess { channel_id, private, view_roles, read_only, post_roles } => {
                self.require(perm::MANAGE_CHANNELS, "manage channels")?;
                if db.channel(channel_id).is_none() {
                    return Err((ErrorCode::NotFound, "No such channel.".into()));
                }
                let (view_roles, post_roles) = (self.real_roles(&view_roles), self.real_roles(&post_roles));
                match db.set_channel_access(channel_id, private, &view_roles, read_only, &post_roles) {
                    Ok(Some(channel)) => hub.channel_changed(&channel),
                    Ok(None) => {}
                    Err(e) => return Err((ErrorCode::Internal, e.to_string())),
                }
                Ok(Event::Ok)
            }
            Request::UpdateChannel { channel_id, name, topic } => {
                self.require(perm::MANAGE_CHANNELS, "manage channels")?;
                let Some(ch) = db.channel(channel_id) else { return Err((ErrorCode::NotFound, "No such channel.".into())) };
                let name = match name {
                    Some(n) => Some(clean_channel_name(&n, ch.kind).ok_or((ErrorCode::BadRequest, "empty name".into()))?),
                    None => None,
                };
                let topic = topic.map(|t| t.chars().filter(|c| !c.is_control()).take(300).collect::<String>());
                if let Ok(Some(channel)) = db.update_channel(channel_id, name.as_deref(), topic.as_deref()) {
                    hub.channel_changed(&channel);
                }
                Ok(Event::Ok)
            }
            Request::DeleteChannel { channel_id } => {
                self.require(perm::MANAGE_CHANNELS, "manage channels")?;
                // Only the server's own channels: never a direct message or group (nor its call).
                if db.channel(channel_id).is_none() {
                    return Ok(Event::Ok);
                }
                hub.evict_channel(channel_id);
                let (existed, files) = db.delete_channel(channel_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.wipe_files(&files);
                if existed {
                    hub.broadcast(Event::ChannelDeleted { channel_id });
                }
                Ok(Event::Ok)
            }
            Request::CreateInvite { max_uses, expires_secs, reuse } => {
                let max_uses = max_uses.filter(|n| *n > 0);
                let listed = hub.short_links_on();
                // One of mine with the same settings, if it still works (like Discord's Invite
                // People, which doesn't make a new link each time it's opened).
                if reuse {
                    if let Some(i) = db.reusable_invite(&self.uid, max_uses, expires_secs, listed) {
                        return Ok(invite_created(hub, &i.code, i.expires_at, i.max_uses, i.uses));
                    }
                }
                if db.invites_by(&self.uid) >= MAX_INVITES_PER_MEMBER {
                    return bad("You've made a lot of invite links already. Use one of those (Server Settings → Invites lists them).");
                }
                // (pyr.gg's short links are made of the code itself: https://pyr.gg/<code>.)
                let code = pyr_proto::directory::new_code();
                let expires_at = expires_secs.map(|s| pyr_proto::now_ms() + (s.min(365 * 86400) as i64) * 1000);
                db.create_invite(&code, &self.uid, max_uses, expires_at, Role::Member)
                    .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if listed {
                    db.set_listed(&code, true).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                }
                Ok(invite_created(hub, &code, expires_at, max_uses, 0))
            }
            Request::ListInvites {} => Ok(hub.invites_event(&self.uid, self.can(perm::MANAGE_SERVER))),
            Request::RevokeInvite { code } => {
                let Some(i) = db.invite(&code) else { return Err((ErrorCode::NotFound, "That invite is gone already.".into())) };
                if i.created_by != self.uid {
                    self.require(perm::MANAGE_SERVER, "take back other people's invites")?;
                }
                if i.grants == Role::Owner {
                    return bad("Owner links can't be taken back here.");
                }
                if db.get_meta("vanity").as_deref() == Some(code.as_str()) {
                    return bad("That's the community's own link: change it in Server Settings → Invites.");
                }
                db.delete_invite(&code).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if i.listed {
                    hub.invites_changed();
                }
                Ok(Event::Ok)
            }
            // (Answered in the loop above, once pyr.gg has it.)
            Request::SetVanity { .. } => bad("not here"),
            Request::SetShortLinks { on } => {
                self.require_owner("turn short links on or off")?;
                if on && hub.directory().is_none() {
                    let why = hub.short_links_off_why.get().cloned().unwrap_or_else(|| "Short links can't work for this server.".into());
                    return bad(&why);
                }
                db.set_meta("short_links", if on { "1" } else { "0" }).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.invites_changed();
                Ok(hub.invites_event(&self.uid, true))
            }
            Request::CreateBotInvite {} => {
                self.require(perm::MANAGE_SERVER, "add bots")?;
                if db.invites_by(&self.uid) >= MAX_INVITES_PER_MEMBER {
                    return bad("You've made a lot of invite links already. Use one of those.");
                }
                // One bot, within a week.
                let code = format!("{BOT_INVITE_PREFIX}{}", new_invite_code(16));
                let expires_at = pyr_proto::now_ms() + 7 * 86_400_000;
                db.create_bot_invite(&code, &self.uid, expires_at).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                Ok(invite_created(hub, &code, Some(expires_at), Some(1), 0))
            }
            Request::SetRole { user_id, role } => {
                valid_uid(&user_id)?;
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the owner can change roles.".into()));
                }
                if user_id == self.uid || role == Role::Owner {
                    return bad("ownership can't be changed here");
                }
                match db.get_user(&user_id) {
                    Some(u) if u.bot && role.is_admin() => return Err((ErrorCode::Forbidden, "Bots can't be admins: give them roles instead.".into())),
                    Some(u) if !u.banned && u.role != Role::Owner => {}
                    _ => return Err((ErrorCode::NotFound, "No such member.".into())),
                }
                db.set_role(&user_id, role).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(member) = hub.member(&user_id) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                // Admins see every channel; members only theirs.
                hub.access_changed(&user_id);
                Ok(Event::Ok)
            }
            Request::Kick { user_id } | Request::Ban { user_id } if user_id == self.uid => bad("you can't remove yourself"),
            Request::Kick { user_id } | Request::Ban { user_id } if !crypto::is_valid_user_id(&user_id) => {
                Err((ErrorCode::NotFound, "No such member.".into()))
            }
            Request::Kick { user_id } => self.remove(user_id, false),
            Request::Ban { user_id } => self.remove(user_id, true),
            Request::UpdateProfile { name } => {
                let Some(name) = clean_member_name(&name) else { return bad("Pick another name (not empty, not @everyone or @here).") };
                db.touch_user(&self.uid, &name).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(member) = hub.member(&self.uid) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                Ok(Event::Ok)
            }
            Request::UpdateServer { name, description } => {
                self.require(perm::MANAGE_SERVER, "change the server's settings")?;
                if let Some(n) = name.and_then(|n| clean_name(&n, 48)) {
                    let _ = db.set_meta("name", &n);
                    // (pyr.gg shows the name with its short links.)
                    hub.invites_changed();
                }
                if let Some(d) = description {
                    let d: String = d.chars().filter(|c| !c.is_control()).take(300).collect();
                    let _ = db.set_meta("description", &d);
                }
                hub.broadcast_info();
                Ok(Event::Ok)
            }

            Request::VoiceJoin { channel_id, e2ee_pub, e2ee_sig, self_mute, self_deaf } => {
                hub.join_voice(self.conn_id, channel_id, e2ee_pub, e2ee_sig, self_mute, self_deaf)
            }
            Request::VoiceLeave => hub.leave_voice(self.conn_id),
            Request::VoiceState { self_mute, self_deaf } => hub.set_voice_state(self.conn_id, self_mute, self_deaf),
            Request::Publish { track } => hub.publish(self.conn_id, track),
            Request::Unpublish { track } => hub.unpublish(self.conn_id, track),
            Request::Subscribe { session, track } => hub.subscribe(self.conn_id, session, track, true),
            Request::Unsubscribe { session, track } => hub.subscribe(self.conn_id, session, track, false),
            Request::RequestKeyframe { session, track } => hub.request_keyframe(self.conn_id, session, track),
            Request::E2eeKeys { epoch, keys } => {
                if keys.len() > 256 {
                    return bad("too many keys");
                }
                hub.route_e2ee(self.conn_id, epoch, keys)
            }
            // A private conversation's call: ring the others (only from inside the call).
            Request::Ring { channel_id, video, moved_from } => {
                let others = self.call_others(channel_id)?;
                if hub.voice_channel_of(self.conn_id) != Some(channel_id) {
                    return bad("Join the call first.");
                }
                if !self.ring_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "That's a lot of ringing. Give them a moment.".into()));
                }
                // Moved from a direct message's call (people were added to it): said only to the
                // other person in it, and only when the caller has just left that call, so
                // nobody can be pulled out of a call they're in with someone else.
                let was_in = moved_from
                    .filter(|m| hub.just_left_call(&self.uid, *m))
                    .and_then(|m| db.dm_members(m).map(|(a, b)| (m, [a, b])))
                    .filter(|(_, who)| who.contains(&self.uid));
                // (Not those in the call already: on their other devices it would ring for nothing.)
                let here = hub.in_call(channel_id);
                for o in others.iter().filter(|o| !here.contains(*o)) {
                    let moved_from = was_in.as_ref().filter(|(_, who)| who.contains(o)).map(|(m, _)| *m);
                    hub.send_to_user(o, Event::Ringing { channel_id, from: self.uid.clone(), video, moved_from });
                }
                Ok(Event::Ok)
            }
            Request::DeclineCall { channel_id } => {
                let others = self.call_others(channel_id)?;
                if !self.ring_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "Slow down a little.".into()));
                }
                let ev = Event::CallDeclined { channel_id, by: self.uid.clone() };
                for o in &others {
                    hub.send_to_user(o, ev.clone());
                }
                hub.send_to_user(&self.uid, ev);
                Ok(Event::Ok)
            }

            // ---- group conversations
            Request::CreateGroup { members } => {
                if !self.groups {
                    return bad("This app doesn't know group conversations yet: update Pyr.");
                }
                let others = self.group_candidates(&members)?;
                if others.len() < 2 {
                    return bad("A group needs at least two other people (for one, send them a direct message).");
                }
                if others.len() + 1 > MAX_GROUP {
                    return bad(&format!("A group can have up to {MAX_GROUP} people."));
                }
                if db.groups_owned(&self.uid) >= 200 {
                    return bad("You've started a lot of groups already. Leave some first.");
                }
                let id = db.create_group(&self.uid, &others).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                let g = db.group(id).ok_or((ErrorCode::Internal, "the group went missing".into()))?;
                hub.send_to_group_members(&g.members.iter().filter(|m| **m != self.uid).cloned().collect::<Vec<_>>(), Event::GroupUpserted { group: g.clone() });
                // (Answered with it, and told to my other devices too.)
                hub.send_to_group_members_except(&self.uid, self.conn_id, Event::GroupUpserted { group: g.clone() });
                Ok(Event::GroupUpserted { group: g })
            }
            Request::AddToGroup { channel_id, members } => {
                let g = db.group(channel_id).filter(|g| self.groups && g.members.contains(&self.uid)).ok_or((ErrorCode::NotFound, "No such group.".into()))?;
                let new: Vec<UserId> = self.group_candidates(&members)?.into_iter().filter(|u| !g.members.contains(u)).collect();
                if new.is_empty() {
                    return Ok(Event::Ok);
                }
                if g.members.len() + new.len() > MAX_GROUP {
                    return bad(&format!("A group can have up to {MAX_GROUP} people."));
                }
                db.add_group_members(channel_id, &new).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                self.tell_group(channel_id, None);
                Ok(Event::Ok)
            }
            Request::LeaveGroup { channel_id } => {
                match db.group(channel_id) {
                    Some(g) if g.members.contains(&self.uid) => {}
                    _ => return Ok(Event::Ok),
                }
                let files = db.leave_group(channel_id, &self.uid).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.wipe_files(&files);
                self.tell_group(channel_id, Some(&self.uid.clone()));
                Ok(Event::Ok)
            }
            Request::RenameGroup { channel_id, name } => {
                if db.group(channel_id).filter(|g| self.groups && g.members.contains(&self.uid)).is_none() {
                    return Err((ErrorCode::NotFound, "No such group.".into()));
                }
                let name = clean_name(&name, 64).unwrap_or_default();
                db.rename_group(channel_id, &name).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                self.tell_group(channel_id, None);
                Ok(Event::Ok)
            }

            // ---- roles
            Request::CreateRole { name, color, perms, hoist } => {
                self.may_give_out_roles()?;
                let Some(name) = clean_name(&name, 32) else { return bad("role name is empty") };
                if db.roles().len() >= 100 {
                    return bad("this server has the most roles it can have (100)");
                }
                // You can't hand out powers you don't have.
                let perms = perms & perm::ALL;
                if perms & !self.perms() != 0 {
                    return Err((ErrorCode::Forbidden, "You can't create a role with permissions you don't have.".into()));
                }
                let role = db.create_role(&name, color & 0xffffff, perms, hoist).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::RoleUpserted { role });
                Ok(Event::Ok)
            }
            Request::UpdateRole { role_id, name, color, perms, hoist, shift } => {
                self.may_give_out_roles()?;
                let Some(mut role) = db.role(role_id) else { return Err((ErrorCode::NotFound, "No such role.".into())) };
                if !self.can_manage_role(&role) {
                    return Err((ErrorCode::Forbidden, "That role is at or above your own highest role.".into()));
                }
                if let Some(n) = name {
                    role.name = clean_name(&n, 32).ok_or((ErrorCode::BadRequest, "role name is empty".into()))?;
                }
                if let Some(c) = color {
                    role.color = c & 0xffffff;
                }
                if let Some(p) = perms {
                    let p = p & perm::ALL;
                    // Only permissions you have yourself can be switched on or off.
                    if (p ^ role.perms) & !self.perms() != 0 {
                        return Err((ErrorCode::Forbidden, "You can only change permissions you have yourself.".into()));
                    }
                    role.perms = p;
                }
                if let Some(h) = hoist {
                    role.hoist = h;
                }
                let mut changed = vec![];
                if shift != 0 {
                    // Swap places with the neighbour above or below.
                    let all = db.roles(); // highest first
                    if let Some(i) = all.iter().position(|r| r.id == role_id) {
                        let j = if shift > 0 { i.checked_sub(1) } else { Some(i + 1).filter(|j| *j < all.len()) };
                        if let Some(j) = j {
                            let mut other = all[j].clone();
                            if !self.can_manage_role(&other) {
                                return Err((ErrorCode::Forbidden, "You can't move a role above your own highest role.".into()));
                            }
                            std::mem::swap(&mut role.position, &mut other.position);
                            if role.position == other.position {
                                role.position += shift.signum();
                            }
                            changed.push(other);
                        }
                    }
                }
                changed.push(role);
                for r in changed {
                    db.save_role(&r).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                    hub.broadcast(Event::RoleUpserted { role: r });
                }
                Ok(Event::Ok)
            }
            Request::DeleteRole { role_id } => {
                self.may_give_out_roles()?;
                let Some(role) = db.role(role_id) else { return Ok(Event::Ok) };
                if !self.can_manage_role(&role) {
                    return Err((ErrorCode::Forbidden, "That role is at or above your own highest role.".into()));
                }
                let had = db.delete_role(role_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::RoleDeleted { role_id });
                for uid in &had {
                    if let Some(member) = hub.member(uid) {
                        hub.broadcast(Event::MemberUpserted { member });
                    }
                    // Channels only that role could see are gone for them.
                    hub.access_changed(uid);
                }
                Ok(Event::Ok)
            }
            Request::SetMemberRoles { user_id, roles } => {
                self.may_give_out_roles()?;
                let Some(target) = db.get_user(&user_id).filter(|u| !u.banned) else {
                    return Err((ErrorCode::NotFound, "No such member.".into()));
                };
                if user_id != self.uid && !self.role().is_admin() && self.rank_of(&user_id) >= self.rank_of(&self.uid) {
                    return Err((ErrorCode::Forbidden, "You can only change the roles of people below you.".into()));
                }
                if target.role == Role::Owner && self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the owner can change the owner's roles.".into()));
                }
                let all = db.roles();
                let before = db.member_roles(&user_id);
                let mut after: Vec<RoleId> = roles.into_iter().filter(|id| all.iter().any(|r| r.id == *id)).collect();
                after.sort();
                after.dedup();
                // Every role being added or taken away must be below your own highest one, and
                // you can only hand out roles whose powers you have yourself.
                let mine = self.perms();
                for id in before.iter().filter(|r| !after.contains(r)).chain(after.iter().filter(|r| !before.contains(r))) {
                    if let Some(r) = all.iter().find(|r| r.id == *id) {
                        if !self.can_manage_role(r) {
                            return Err((ErrorCode::Forbidden, format!("\"{}\" is at or above your own highest role.", r.name)));
                        }
                        if !before.contains(id) && r.perms & !mine != 0 {
                            return Err((ErrorCode::Forbidden, format!("\"{}\" has permissions you don't have.", r.name)));
                        }
                    }
                }
                db.set_member_roles(&user_id, &after).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(member) = hub.member(&user_id) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                // What they can see may have changed with their roles.
                hub.access_changed(&user_id);
                Ok(Event::Ok)
            }
            Request::SetAvatar { file_id } => {
                if let Some(id) = &file_id {
                    match db.file(id) {
                        Some(f) if f.owner == self.uid && f.kind == FileKind::Avatar => {}
                        _ => return bad("upload the picture first"),
                    }
                }
                let old = db.set_avatar(&self.uid, file_id.as_deref()).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(old) = old.filter(|o| Some(o) != file_id.as_ref()) {
                    if db.release_avatar(&old) {
                        hub.wipe_files(&[old]);
                    }
                }
                if let Some(member) = hub.member(&self.uid) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                Ok(Event::Ok)
            }
            Request::FriendRequest { user_id } => {
                valid_uid(&user_id)?;
                self.other_member(&user_id)?;
                if self.is_bot(&user_id) {
                    return Err((ErrorCode::Forbidden, "Bots can't be friends.".into()));
                }
                if db.outgoing_requests(&self.uid) >= 200 {
                    return bad("You have a lot of friend requests waiting already.");
                }
                if db.blocked_between(&self.uid, &user_id) {
                    return Err((ErrorCode::Forbidden, "You can't add this person as a friend.".into()));
                }
                let ie = |e: anyhow::Error| (ErrorCode::Internal, e.to_string());
                match db.relation(&self.uid, &user_id) {
                    Some(RelationKind::Friend | RelationKind::Outgoing) => {}
                    Some(RelationKind::Incoming) => {
                        db.clear_relation(&self.uid, &user_id, &[0]).map_err(ie)?;
                        db.set_relation(&self.uid, &user_id, 1).map_err(ie)?;
                        db.set_relation(&user_id, &self.uid, 1).map_err(ie)?;
                    }
                    _ => db.set_relation(&self.uid, &user_id, 0).map_err(ie)?,
                }
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::FriendAccept { user_id } => {
                valid_uid(&user_id)?;
                if db.relation(&self.uid, &user_id) != Some(RelationKind::Incoming) {
                    return bad("There's no friend request from them.");
                }
                let ie = |e: anyhow::Error| (ErrorCode::Internal, e.to_string());
                db.clear_relation(&self.uid, &user_id, &[0]).map_err(ie)?;
                db.set_relation(&self.uid, &user_id, 1).map_err(ie)?;
                db.set_relation(&user_id, &self.uid, 1).map_err(ie)?;
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::FriendRemove { user_id } => {
                valid_uid(&user_id)?;
                db.clear_relation(&self.uid, &user_id, &[0, 1]).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::Block { user_id } => {
                valid_uid(&user_id)?;
                if user_id == self.uid {
                    return bad("You can't block yourself.");
                }
                let ie = |e: anyhow::Error| (ErrorCode::Internal, e.to_string());
                db.clear_relation(&self.uid, &user_id, &[0, 1]).map_err(ie)?;
                db.set_relation(&self.uid, &user_id, 2).map_err(ie)?;
                // A call between them ends with it.
                hub.end_dm_call(&self.uid, &user_id);
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::Unblock { user_id } => {
                valid_uid(&user_id)?;
                db.unblock(&self.uid, &user_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                self.tell_relation(&user_id);
                Ok(Event::Ok)
            }
            Request::AddEmoji { name, file_id, animated } => {
                self.require(perm::MANAGE_EMOJI, "add emoji")?;
                if !valid_emoji_name(&name) {
                    return bad("Emoji names are 2–32 letters, numbers or underscores.");
                }
                match db.file(&file_id) {
                    Some(f) if f.owner == self.uid && f.kind == FileKind::Avatar => {}
                    _ => return bad("upload the picture first"),
                }
                let all = db.emojis();
                if all.len() >= MAX_CUSTOM_EMOJI {
                    return bad(&format!("This server already has {MAX_CUSTOM_EMOJI} emoji, the most it can have."));
                }
                if all.iter().any(|e| e.name.eq_ignore_ascii_case(&name)) {
                    return bad(&format!("There's already an emoji called :{name}:."));
                }
                let e = CustomEmoji { id: file_id, name, animated, by: self.uid.clone() };
                if !db.add_emoji(&e).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return bad("That picture is already one of this server's emoji.");
                }
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::AddGif { name, file_id } => {
                self.require(perm::MANAGE_EMOJI, "add GIFs")?;
                let name = name.trim().to_string();
                if name.is_empty() || name.chars().count() > MAX_GIF_NAME || name.contains(['<', '>', ':']) {
                    return bad(&format!("GIF names are 1–{MAX_GIF_NAME} characters, without < > or :."));
                }
                let f = match db.file(&file_id) {
                    Some(f) if f.owner == self.uid && f.kind == FileKind::Attachment && f.message_id.is_none() => f,
                    _ => return bad("upload the picture first"),
                };
                let all = db.gifs();
                if all.len() >= MAX_SERVER_GIFS {
                    return bad(&format!("This server already has {MAX_SERVER_GIFS} GIFs, the most it can have."));
                }
                if all.iter().any(|g| g.name.eq_ignore_ascii_case(&name)) {
                    return bad(&format!("There's already a GIF called {name}."));
                }
                let g = ServerGif { id: file_id, name, width: f.width, height: f.height, by: self.uid.clone() };
                if !db.add_gif(&g).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return bad("That picture is already one of this server's GIFs.");
                }
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::RemoveGif { id } => {
                self.require(perm::MANAGE_EMOJI, "remove GIFs")?;
                if db.remove_gif(&id).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    hub.wipe_files(&[id]);
                    hub.broadcast_info();
                }
                Ok(Event::Ok)
            }
            Request::RenameGif { id, name } => {
                self.require(perm::MANAGE_EMOJI, "rename GIFs")?;
                let name = name.trim().to_string();
                if name.is_empty() || name.chars().count() > MAX_GIF_NAME || name.contains(['<', '>', ':']) {
                    return bad(&format!("GIF names are 1–{MAX_GIF_NAME} characters, without < > or :."));
                }
                if db.gifs().iter().any(|g| g.id != id && g.name.eq_ignore_ascii_case(&name)) {
                    return bad(&format!("There's already a GIF called {name}."));
                }
                db.rename_gif(&id, &name).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::RemoveEmoji { id } => {
                self.require(perm::MANAGE_EMOJI, "remove emoji")?;
                if db.remove_emoji(&id).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    if db.release_avatar(&id) {
                        hub.wipe_files(&[id]);
                    }
                    hub.broadcast_info();
                }
                Ok(Event::Ok)
            }
            Request::RenameEmoji { id, name } => {
                self.require(perm::MANAGE_EMOJI, "rename emoji")?;
                if !valid_emoji_name(&name) {
                    return bad("Emoji names are 2–32 letters, numbers or underscores.");
                }
                if db.emojis().iter().any(|e| e.id != id && e.name.eq_ignore_ascii_case(&name)) {
                    return bad(&format!("There's already an emoji called :{name}:."));
                }
                db.rename_emoji(&id, &name).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::AddSound { file_id, name, emoji, volume } => {
                self.require(perm::MANAGE_EMOJI, "add sounds")?;
                let Some(name) = clean_sound_name(&name) else {
                    return bad(&format!("Sound names are 1–{MAX_SOUND_NAME} characters."));
                };
                let Some(emoji) = clean_sound_emoji(&emoji) else { return bad("That emoji won't do.") };
                let Some(f) = db.file(&file_id).filter(|f| f.kind == FileKind::Sound) else { return bad("upload the sound first") };
                let Some(length_ms) =
                    hub.file_path(&f.id).and_then(|p| std::fs::read(p).ok()).and_then(|b| pyr_proto::sound::check(&b)).map(|s| s.length_ms())
                else {
                    return bad("upload the sound first");
                };
                let all = db.sounds();
                if all.len() >= MAX_SERVER_SOUNDS {
                    return bad(&format!("This server already has {MAX_SERVER_SOUNDS} sounds, the most it can have."));
                }
                let s = ServerSound { id: file_id, name, emoji, volume: volume.clamp(0.0, 1.0), by: self.uid.clone(), length_ms };
                if !db.add_sound(&s).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return bad("That sound is already on this server's soundboard.");
                }
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::EditSound { id, name, emoji, volume } => {
                self.require(perm::MANAGE_EMOJI, "change sounds")?;
                let name = match name {
                    Some(n) => Some(clean_sound_name(&n).ok_or((ErrorCode::BadRequest, format!("Sound names are 1–{MAX_SOUND_NAME} characters.")))?),
                    None => None,
                };
                let emoji = match emoji {
                    Some(e) => Some(clean_sound_emoji(&e).ok_or((ErrorCode::BadRequest, "That emoji won't do.".to_string()))?),
                    None => None,
                };
                let volume = volume.map(|v| v.clamp(0.0, 1.0));
                if !db.edit_sound(&id, name.as_deref(), emoji.as_deref(), volume).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return Err((ErrorCode::NotFound, "That sound is gone.".into()));
                }
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::RemoveSound { id } => {
                self.require(perm::MANAGE_EMOJI, "remove sounds")?;
                if db.remove_sound(&id).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    hub.wipe_files(&[id]);
                    hub.broadcast_info();
                }
                Ok(Event::Ok)
            }
            Request::Poker { msg } => {
                let can_manage = self.can(perm::MANAGE_SERVER);
                match hub.poker.handle_from(hub, &self.uid, can_manage, self.casino, msg) {
                    // (An older app gets it as it can read it, or just an Ok.)
                    Ok(Some(msg)) => match msg.for_app(self.casino) {
                        Some(msg) => Ok(Event::Poker { msg }),
                        None => Ok(Event::Ok),
                    },
                    Ok(None) => Ok(Event::Ok),
                    Err(e) => Err((ErrorCode::BadRequest, e)),
                }
            }
            Request::PlaySound { sound, name, emoji, volume } => {
                if !self.sound_tokens.take(1.0) {
                    return Err((ErrorCode::RateLimited, "Easy on the sounds: try again in a moment.".into()));
                }
                let known = pyr_proto::sound::is_builtin(&sound) || db.file(&sound).is_some_and(|f| f.kind == FileKind::Sound);
                if !known {
                    return Err((ErrorCode::NotFound, "upload the sound first".into()));
                }
                let name: String = name.chars().filter(|c| !c.is_control()).take(MAX_SOUND_NAME).collect();
                let emoji = clean_sound_emoji(&emoji).unwrap_or_default();
                hub.play_sound(self.conn_id, sound, name, emoji, volume.clamp(0.0, 1.0))
            }
            Request::SetServerIcon { file_id } => {
                self.require(perm::MANAGE_SERVER, "change the server's icon")?;
                if let Some(id) = &file_id {
                    match db.file(id) {
                        Some(f) if f.owner == self.uid && f.kind == FileKind::Avatar => {}
                        _ => return bad("upload the picture first"),
                    }
                }
                let old = db.get_meta("icon").filter(|i| !i.is_empty());
                db.set_meta("icon", file_id.as_deref().unwrap_or("")).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(old) = old.filter(|o| Some(o) != file_id.as_ref()) {
                    if db.release_avatar(&old) {
                        hub.wipe_files(&[old]);
                    }
                }
                hub.broadcast_info();
                // (Its invite pages at pyr.gg show the icon.)
                hub.invites_changed();
                Ok(Event::Ok)
            }
            Request::ListBugs => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can read bug reports.".into()));
                }
                // As many as fit in one reply (logs make them big), what's waiting first: a pile
                // of reports (even a flood from strangers) never stops the list loading.
                let mut reports = db.bugs(500);
                let size = |b: &BugEntry| b.report.title.len() + b.report.body.len() + b.report.log.len() + b.report.os.len() + 300;
                let mut total = 0usize;
                let mut keep = 0usize;
                for b in &reports {
                    total += size(b);
                    if total > HISTORY_PAGE_BYTES && keep > 0 {
                        break;
                    }
                    keep += 1;
                }
                reports.truncate(keep);
                Ok(Event::Bugs { reports })
            }
            Request::SetBugStatus { id, status } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can approve bug reports.".into()));
                }
                if !db.set_bug_status(id, status).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return Err((ErrorCode::NotFound, "That report is gone.".into()));
                }
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::SetBugReports { on } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can do that.".into()));
                }
                db.set_meta("bug_reports", if on { "1" } else { "" }).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::ReportBug { .. } => bad("send bug reports instead of logging in"),
            Request::SubmitPlugin { .. } | Request::SubmissionPart { .. } => bad("send plugins and skins instead of logging in"),
            Request::MySubmissions { .. } => bad("ask for your own submissions instead of logging in"),
            Request::ListPluginSubmissions => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can see plugins sent in.".into()));
                }
                Ok(Event::PluginSubmissions { list: db.submissions(1000) })
            }
            Request::GetPluginSubmission { id } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can see plugins sent in.".into()));
                }
                let Some((entry, files)) = db.submission(id) else {
                    return Err((ErrorCode::NotFound, "That plugin is gone.".into()));
                };
                let files: Vec<SourceFile> = serde_json::from_str(&files).unwrap_or_default();
                // As much as fits one reply comes along; the rest (a skin's bigger files) is
                // fetched one by one.
                let (mut inline, mut more, mut room) = (Vec::new(), Vec::new(), INLINE_SUBMISSION_BYTES);
                let manifest = manifest_of(&files);
                for f in files {
                    let d = manifest.iter().find(|m| m.path == f.path).cloned().unwrap_or_default();
                    if (d.size as usize) <= room {
                        room -= d.size as usize;
                        inline.push(f);
                    } else {
                        more.push(d);
                    }
                }
                Ok(Event::PluginSubmission { entry, files: inline, more })
            }
            Request::GetSubmissionFile { id, path } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can see what's sent in.".into()));
                }
                let Some((_, files)) = db.submission(id) else {
                    return Err((ErrorCode::NotFound, "That's gone.".into()));
                };
                let files: Vec<SourceFile> = serde_json::from_str(&files).unwrap_or_default();
                match files.into_iter().find(|f| f.path == path) {
                    Some(f) => Ok(Event::SubmissionFile { id, path, data: f.data }),
                    None => Err((ErrorCode::NotFound, "There's no such file in it.".into())),
                }
            }
            Request::SetPluginSubmissionStatus { id, status, note } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can approve plugins.".into()));
                }
                let mut note: String = note.chars().filter(|c| !c.is_control() || *c == '\n').collect::<String>().trim().to_string();
                while note.len() > SUBMISSION_NOTE_MAX {
                    note.pop();
                }
                if !db.set_submission_status(id, status, &note).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    return Err((ErrorCode::NotFound, "That plugin is gone.".into()));
                }
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::SetPluginSubmissions { on } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can do that.".into()));
                }
                db.set_meta("plugin_submissions", if on { "1" } else { "" }).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::SetGiphyKey { key } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can set up GIF search.".into()));
                }
                let key = key.trim();
                if key.len() > 64 || !key.bytes().all(|b| b.is_ascii_alphanumeric()) {
                    return bad("that doesn't look like a GIPHY API key (letters and numbers, from developers.giphy.com)");
                }
                db.set_meta("giphy_key", key).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::SetKlipyKey { key } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the server's owner can set up GIF search.".into()));
                }
                // (It goes in the path of every KLIPY address: letters, numbers, - and _ only.)
                let key = key.trim();
                if key.len() > 128 || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                    return bad("that doesn't look like a KLIPY app key (letters and numbers, from partner.klipy.com)");
                }
                db.set_meta("klipy_key", key).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast_info();
                Ok(Event::Ok)
            }
            Request::ServerMute { user_id, muted } => {
                self.require(perm::MUTE_MEMBERS, "server mute people")?;
                self.may_moderate(&user_id, if muted { "server mute" } else { "undo the server mute of" }, muted)?;
                hub.set_moderated(ModeratedVoice::Muted, &user_id, muted)
            }
            Request::ServerDeafen { user_id, deafened } => {
                self.require(perm::DEAFEN_MEMBERS, "server deafen people")?;
                self.may_moderate(&user_id, if deafened { "server deafen" } else { "undo the server deafen of" }, deafened)?;
                hub.set_moderated(ModeratedVoice::Deafened, &user_id, deafened)
            }
            // (Like Discord's: Move members is all it takes, whoever they are.)
            Request::MoveMember { user_id, channel_id } => {
                self.require(perm::MOVE_MEMBERS, "move people between voice channels")?;
                if user_id.len() > 200 {
                    return bad("bad user id");
                }
                hub.move_member(&self.uid, &user_id, channel_id)
            }
            Request::Unknown => Err((ErrorCode::BadRequest, "This server is older than your app and can't do that yet.".into())),
        }
    }

    /// Server mute or deafen (`on`), or undo it: someone below you (like kicking). Yourself
    /// too, but only the owner can undo their own: a moderator muted by someone above them stays
    /// muted until someone above them says otherwise.
    fn may_moderate(&self, user_id: &str, what: &str, on: bool) -> Result<(), (ErrorCode, String)> {
        if user_id == self.uid {
            if on || self.role() == Role::Owner {
                return Ok(());
            }
            let undo = what.trim_start_matches("undo the ").trim_end_matches(" of");
            return Err((ErrorCode::Forbidden, format!("You can't undo your own {undo}: ask someone with a higher role.")));
        }
        self.other_member(user_id)?;
        if self.rank_of(user_id) >= self.rank_of(&self.uid) {
            return Err((ErrorCode::Forbidden, format!("You can't {what} someone with the same or a higher role.")));
        }
        Ok(())
    }

    fn remove(&self, user_id: UserId, ban: bool) -> Reply {
        if ban {
            self.require(perm::BAN, "ban members")?;
        } else {
            self.require(perm::KICK, "kick members")?;
        }
        if self.rank_of(&user_id) >= self.rank_of(&self.uid) {
            return Err((ErrorCode::Forbidden, "You can't remove someone with the same or a higher role.".into()));
        }
        if let Err(e) = self.hub.db.remove_user(&user_id, ban) {
            warn!("remove_user failed: {e}");
            return Err((ErrorCode::Internal, e.to_string()));
        }
        let reason = if ban { "You were banned from this server." } else { "You were kicked from this server." };
        self.hub.eject(&user_id, reason);
        // (Their invites went with them, and pyr.gg's count of members changed.)
        self.hub.invites_changed();
        Ok(Event::Ok)
    }
}

/// The answer to `CreateInvite` (and `CreateBotInvite`), before any short link.
fn invite_created(hub: &Hub, code: &str, expires_at: Option<i64>, max_uses: Option<u32>, uses: u32) -> Event {
    Event::InviteCreated { code: code.to_string(), link: hub.invite_link(code), short: None, short_problem: None, expires_at, max_uses, uses, vanity: None, vanity_link: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Db;

    fn uid() -> String {
        crypto::Identity::generate().user_id()
    }

    /// An encrypted message as far as the server can tell (it can't check more).
    fn sealed_by(uid: &str) -> Option<Sealed> {
        Some(Sealed {
            key: KeyRef { owner: uid.to_string(), id: pyr_proto::sealed::new_key_id() },
            nonce: pyr_proto::b64::encode(&[1u8; 12]),
            ct: pyr_proto::b64::encode(&[2u8; 40]),
            sig: pyr_proto::b64::encode(&[3u8; 64]),
        })
    }

    fn hub() -> Arc<Hub> {
        let dir = std::env::temp_dir().join(format!("pyr-conn-test-{}", crypto::random_bytes::<6>().iter().map(|b| format!("{b:02x}")).collect::<String>()));
        Hub::new(Db::open_in_memory().unwrap(), [7u8; 32], "example.org".into(), 7777, dir)
    }

    fn ctx<'a>(hub: &'a Hub, uid: &str) -> Ctx<'a> {
        Ctx {
            hub,
            conn_id: 1,
            uid: uid.to_string(),
            groups: true,
            casino: pyr_poker::net::CASINO_013,
            msg_tokens: Bucket::new(1.5, 10.0),
            react_tokens: Bucket::new(3.0, 15.0),
            req_tokens: Bucket::new(REQ_RATE, REQ_BURST),
            sound_tokens: Bucket::new(SOUND_RATE, SOUND_BURST),
            ring_tokens: Bucket::new(RING_RATE, RING_BURST),
            idle_tokens: Bucket::new(IDLE_RATE, IDLE_BURST),
            handshake_tokens: Bucket::new(HANDSHAKE_RATE, HANDSHAKE_BURST),
        }
    }

    fn forbidden(r: Reply) -> bool {
        matches!(r, Err((ErrorCode::Forbidden, _)))
    }

    #[test]
    fn only_the_owner_names_casino_staff_and_staff_give_out_no_roles() {
        use pyr_poker::net::{Role as Staff, ToServer};
        let hub = hub();
        let (owner, admin, gm, amb, bob) = (uid(), uid(), uid(), uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&admin, "Admin", Role::Admin).unwrap();
        hub.db.insert_user(&gm, "Game Master", Role::Admin).unwrap();
        hub.db.insert_user(&amb, "Ambassador", Role::Member).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let managers = hub.db.create_role("Role managers", 0, perm::MANAGE_ROLES, false).unwrap();
        hub.db.set_member_roles(&amb, &[managers.id]).unwrap();
        let set = |who: &str, them: &str, role: Option<Staff>| hub.poker.handle(&hub, who, true, ToServer::SetRole { uid: them.into(), role });
        // (This is the home server: the Owner of the Game's, where staff are named.)
        let _ = hub.poker.home_for_tests.set(hub.fingerprint);
        // The owner names Game Masters and Ambassadors; an admin can't, nor can a Game Master.
        assert!(set(&owner, &gm, Some(Staff::GameMaster)).is_ok());
        assert!(set(&owner, &amb, Some(Staff::Ambassador)).is_ok());
        assert!(set(&admin, &bob, Some(Staff::GameMaster)).unwrap_err().contains("Owner"));
        assert!(set(&gm, &bob, Some(Staff::GameMaster)).unwrap_err().contains("Owner"));
        assert!(set(&gm, &bob, Some(Staff::Ambassador)).is_err());
        assert!(set(&gm, &amb, None).is_err(), "nor take a role away");
        assert_eq!(crate::poker::Poker::role(&hub, &bob), None);
        // Staff hand out no server roles either, whatever their powers.
        let helpers = hub.db.create_role("Helpers", 0, 0, false).unwrap();
        for who in [&gm, &amb] {
            let mut c = ctx(&hub, who);
            assert!(forbidden(c.handle(Request::SetMemberRoles { user_id: bob.clone(), roles: vec![helpers.id] })));
            assert!(forbidden(c.handle(Request::CreateRole { name: "Mine".into(), color: 0, perms: 0, hoist: false })));
            assert!(forbidden(c.handle(Request::UpdateRole { role_id: helpers.id, name: Some("Renamed".into()), color: None, perms: None, hoist: None, shift: 0 })));
            assert!(forbidden(c.handle(Request::DeleteRole { role_id: helpers.id })));
            assert!(forbidden(c.handle(Request::SetRole { user_id: bob.clone(), role: Role::Admin })));
        }
        assert!(hub.db.member_roles(&bob).is_empty());
        // An admin who isn't staff still manages roles; so does the owner.
        assert!(ctx(&hub, &admin).handle(Request::SetMemberRoles { user_id: bob.clone(), roles: vec![helpers.id] }).is_ok());
        assert!(ctx(&hub, &owner).handle(Request::SetMemberRoles { user_id: bob.clone(), roles: vec![] }).is_ok());
        // What the Staff page offers: naming staff and the log, to the owner only.
        let view = |who: &str| match hub.poker.handle(&hub, who, true, ToServer::Staff) {
            Ok(Some(pyr_poker::net::ToApp::Staff { view })) => view,
            other => panic!("{other:?}"),
        };
        assert!(view(&owner).owner);
        assert!(!view(&admin).owner && !view(&gm).owner);
        assert!(view(&gm).log.is_empty() && !view(&owner).log.is_empty());
    }

    #[test]
    fn bots_like_discords() {
        let hub = hub();
        let (owner, manager, bob, claude) = (uid(), uid(), uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&manager, "Manager", Role::Member).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let managers = hub.db.create_role("Managers", 0, perm::MANAGE_SERVER, false).unwrap();
        hub.db.set_member_roles(&manager, &[managers.id]).unwrap();
        let ip: std::net::IpAddr = "203.0.113.7".parse().unwrap();
        // Only who can manage the server adds bots.
        assert!(forbidden(ctx(&hub, &bob).handle(Request::CreateBotInvite {})));
        let code = match ctx(&hub, &manager).handle(Request::CreateBotInvite {}) {
            Ok(Event::InviteCreated { code, link, .. }) => {
                assert!(link.contains(&code));
                code
            }
            other => panic!("{other:?}"),
        };
        assert!(code.starts_with(BOT_INVITE_PREFIX));
        // Whatever joins with it is a bot, a plain member otherwise; it's good for one.
        assert!(hub.authorize(&claude, "Claude", Some(&code), ip).is_ok());
        let u = hub.db.get_user(&claude).unwrap();
        assert!(u.bot && u.role == Role::Member);
        assert!(hub.db.members().iter().any(|m| m.user_id == claude && m.bot));
        assert!(!hub.db.members().iter().any(|m| m.user_id == bob && m.bot));
        assert!(hub.member(&claude).is_some_and(|m| m.bot));
        assert!(hub.authorize(&uid(), "Another", Some(&code), ip).is_err(), "used up");
        // Someone already here who uses one is still who they were.
        let code2 = match ctx(&hub, &owner).handle(Request::CreateBotInvite {}) {
            Ok(Event::InviteCreated { code, .. }) => code,
            other => panic!("{other:?}"),
        };
        assert!(hub.authorize(&bob, "Bob", Some(&code2), ip).is_ok());
        assert!(!hub.db.get_user(&bob).unwrap().bot);
        // A bot chats, and that's all.
        let mut c = ctx(&hub, &claude);
        let general = hub.db.create_channel("general", ChannelKind::Text).unwrap();
        let lounge = hub.db.create_channel("lounge", ChannelKind::Voice).unwrap();
        let send = |c: &mut Ctx, ch: ChannelId, text: &str| {
            c.handle(Request::SendMessage { channel_id: ch, content: text.into(), reply_to: None, nonce: String::new(), attachments: vec![], sealed: sealed_by(&c.uid) })
        };
        assert!(send(&mut c, general.id, "Hello! I'm Claude.").is_ok());
        assert!(forbidden(c.handle(Request::OpenDm { user_id: bob.clone() })));
        assert!(forbidden(c.handle(Request::CreateGroup { members: vec![bob.clone()] })));
        assert!(forbidden(c.handle(Request::CreateInvite { max_uses: None, expires_secs: None, reuse: false })));
        assert!(forbidden(c.handle(Request::CreateBotInvite {})));
        assert!(forbidden(c.handle(Request::FriendRequest { user_id: bob.clone() })));
        let (key, sig) = {
            let id = crypto::Identity::generate();
            let pk = [9u8; 32];
            (pyr_proto::b64::encode(&pk), pyr_proto::b64::encode(&id.sign(&pk)))
        };
        assert!(forbidden(c.handle(Request::VoiceJoin { channel_id: lounge.id, e2ee_pub: key, e2ee_sig: sig, self_mute: false, self_deaf: false })));
        // Nobody starts a conversation with one, befriends it or makes it an admin.
        let mut b = ctx(&hub, &bob);
        assert!(forbidden(b.handle(Request::OpenDm { user_id: claude.clone() })));
        assert!(forbidden(b.handle(Request::CreateGroup { members: vec![claude.clone(), owner.clone()] })));
        assert!(forbidden(b.handle(Request::FriendRequest { user_id: claude.clone() })));
        assert!(forbidden(ctx(&hub, &owner).handle(Request::SetRole { user_id: claude.clone(), role: Role::Admin })));
        assert!(!hub.db.get_user(&claude).unwrap().role.is_admin());
        // It sees what its roles let it see, like anyone.
        let helpers = hub.db.create_role("Helpers", 0, 0, false).unwrap();
        let mut o = ctx(&hub, &owner);
        assert!(o.handle(Request::CreateChannel { name: "helpers".into(), kind: ChannelKind::Text, private: true, view_roles: vec![helpers.id] }).is_ok());
        let private = hub.db.channels().into_iter().find(|ch| ch.name == "helpers").unwrap();
        assert!(c.can_see(private.id).is_err());
        assert!(send(&mut c, private.id, "hi").is_err());
        assert!(o.handle(Request::SetMemberRoles { user_id: claude.clone(), roles: vec![helpers.id] }).is_ok());
        assert!(send(&mut c, private.id, "Glad to help.").is_ok());
        // And it can be removed like anyone.
        assert!(o.handle(Request::Kick { user_id: claude.clone() }).is_ok());
        assert!(hub.db.get_user(&claude).is_none());
    }

    #[test]
    fn private_and_read_only_channels() {
        let hub = hub();
        let (owner, admin, mod_, bob) = (uid(), uid(), uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&admin, "Admin", Role::Admin).unwrap();
        hub.db.insert_user(&mod_, "Mod", Role::Member).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let mods = hub.db.create_role("Mods", 0, perm::MANAGE_CHANNELS, false).unwrap();
        let news = hub.db.create_role("News team", 0, 0, false).unwrap();
        hub.db.set_member_roles(&mod_, &[mods.id]).unwrap();
        let mut o = ctx(&hub, &owner);
        // A private channel only Mods (and admins) can see.
        assert!(o.handle(Request::CreateChannel { name: "mod chat".into(), kind: ChannelKind::Text, private: true, view_roles: vec![mods.id, 999] }).is_ok());
        let private = hub.db.channels().into_iter().find(|c| c.name == "mod chat").unwrap();
        assert!(private.private && private.view_roles == vec![mods.id], "unknown roles are dropped");
        let send = |c: &mut Ctx, ch: ChannelId, text: &str| {
            c.handle(Request::SendMessage { channel_id: ch, content: text.into(), reply_to: None, nonce: String::new(), attachments: vec![], sealed: sealed_by(&c.uid) })
        };
        let (mut a, mut m, mut b) = (ctx(&hub, &admin), ctx(&hub, &mod_), ctx(&hub, &bob));
        assert!(send(&mut m, private.id, "mods only").is_ok());
        assert!(send(&mut a, private.id, "admins see everything").is_ok());
        assert!(send(&mut b, private.id, "let me in").is_err());
        assert!(b.handle(Request::FetchHistory { channel_id: private.id, before: None, limit: 50 }).is_err());
        assert!(b.can_see(private.id).is_err() && m.can_see(private.id).is_ok() && a.can_see(private.id).is_ok());
        // Who gets what's said in it.
        let mut seen = hub.db.viewers(&private);
        seen.sort();
        let mut want = vec![owner.clone(), admin.clone(), mod_.clone()];
        want.sort();
        assert_eq!(seen, want);
        // A moderator who can't see it can't delete in it either.
        let mid = match hub.db.history(private.id, None, 10).0.first() {
            Some(m) => m.id,
            None => panic!("no message"),
        };
        let deleters = hub.db.create_role("Deleters", 0, perm::MANAGE_MESSAGES, false).unwrap();
        hub.db.set_member_roles(&bob, &[deleters.id]).unwrap();
        assert!(forbidden(b.handle(Request::DeleteMessage { message_id: mid })));
        // Bob gets the role: he can see it now.
        hub.db.set_member_roles(&bob, &[mods.id]).unwrap();
        assert!(send(&mut b, private.id, "thanks").is_ok());
        hub.db.set_member_roles(&bob, &[]).unwrap();
        assert!(b.can_see(private.id).is_err());

        // A read-only announcements channel: everyone reads, only the News team posts.
        let general = hub.db.create_channel("announcements", ChannelKind::Text).unwrap();
        // (Only who can manage channels sets it.)
        assert!(forbidden(b.handle(Request::SetChannelAccess { channel_id: general.id, private: false, view_roles: vec![], read_only: true, post_roles: vec![] })));
        assert!(m.handle(Request::SetChannelAccess { channel_id: general.id, private: false, view_roles: vec![], read_only: true, post_roles: vec![news.id] }).is_ok());
        assert!(b.can_see(general.id).is_ok());
        assert!(matches!(send(&mut b, general.id, "hi"), Err((ErrorCode::Forbidden, _))));
        assert!(b.handle(Request::FetchHistory { channel_id: general.id, before: None, limit: 50 }).is_ok());
        hub.db.set_member_roles(&bob, &[news.id]).unwrap();
        assert!(send(&mut b, general.id, "news!").is_ok());
        assert!(send(&mut a, general.id, "admins can always post").is_ok());
        assert!(!hub.db.channel(general.id).unwrap().private);
        // Making it private hides it from Bob (he isn't a Mod).
        assert!(o.handle(Request::SetChannelAccess { channel_id: general.id, private: true, view_roles: vec![mods.id], read_only: false, post_roles: vec![] }).is_ok());
        assert!(b.can_see(general.id).is_err());
    }

    #[test]
    fn vouches_are_kept_only_when_theyre_yours_signed_and_for_members() {
        let hub = hub();
        let blue = crypto::Identity::generate();
        let (b, mara, stranger) = (blue.user_id(), uid(), uid());
        hub.db.insert_user(&b, "Blue", Role::Owner).unwrap();
        hub.db.insert_user(&mara, "Mara", Role::Member).unwrap();
        let mut c = ctx(&hub, &b);
        let good = Vouch::new(&blue, &mara);
        assert!(c.handle(Request::SetVouches { vouches: vec![good.clone()] }).is_ok());
        // For someone who isn't a member, someone else's, or not signed by who it says: no.
        assert!(c.handle(Request::SetVouches { vouches: vec![Vouch::new(&blue, &stranger)] }).is_err());
        let other = crypto::Identity::generate();
        assert!(c.handle(Request::SetVouches { vouches: vec![Vouch::new(&other, &mara)] }).is_err());
        assert!(c.handle(Request::SetVouches { vouches: vec![Vouch { sig: vec![0; 64], ..good.clone() }] }).is_err());
        // Anyone here can get them.
        let mut m = ctx(&hub, &mara);
        match m.handle(Request::GetVouches { by: vec![b.clone()] }) {
            Ok(Event::Vouches { vouches }) => assert_eq!(vouches, vec![good.clone()]),
            other => panic!("{other:?}"),
        }
        // Mara asks who made sure of her: Blue.
        match m.handle(Request::GetVouchesForMe {}) {
            Ok(Event::Vouches { vouches }) => assert_eq!(vouches, vec![good.clone()]),
            other => panic!("{other:?}"),
        }
        // Setting them again replaces them.
        assert!(c.handle(Request::SetVouches { vouches: vec![] }).is_ok());
        assert!(matches!(m.handle(Request::GetVouches { by: vec![b.clone()] }), Ok(Event::Vouches { vouches }) if vouches.is_empty()));
        assert!(matches!(m.handle(Request::GetVouchesForMe {}), Ok(Event::Vouches { vouches }) if vouches.is_empty()));
        // Messages for someone's Pyr only go to members who are here, and never too big.
        assert!(matches!(c.handle(Request::Handshake { to: mara.clone(), data: "{}".into() }), Err((ErrorCode::NotFound, _))));
        assert!(c.handle(Request::Handshake { to: mara.clone(), data: "x".repeat(pyr_proto::handshake::HANDSHAKE_MAX + 1) }).is_err());
        assert!(c.handle(Request::Handshake { to: b, data: "{}".into() }).is_err(), "not to yourself");
    }

    #[test]
    fn server_gifs_need_the_permission_and_are_for_everyone() {
        let hub = hub();
        let (owner, keeper, bob) = (uid(), uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&keeper, "Keeper", Role::Member).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let keepers = hub.db.create_role("Emoji keepers", 0, perm::MANAGE_EMOJI, false).unwrap();
        hub.db.set_member_roles(&keeper, &[keepers.id]).unwrap();
        // Pictures uploaded and not sent yet.
        let upload = |who: &str, id: &str| {
            hub.db
                .insert_file(&crate::db::FileRow {
                    id: id.into(),
                    owner: who.into(),
                    kind: FileKind::Attachment,
                    name: "party.gif".into(),
                    mime: "image/gif".into(),
                    size: 100,
                    width: 64,
                    height: 48,
                    message_id: None,
                })
                .unwrap();
        };
        let (a, bobs, c2) = ("a".repeat(32), "b".repeat(32), "c".repeat(32));
        upload(&keeper, &a);
        upload(&bob, &bobs);
        upload(&keeper, &c2);
        let add = |c: &mut Ctx, name: &str, file: &str| c.handle(Request::AddGif { name: name.into(), file_id: file.into() });
        let (mut k, mut b) = (ctx(&hub, &keeper), ctx(&hub, &bob));
        // Only a role with "Manage emoji, GIFs and sounds".
        assert!(forbidden(add(&mut b, "mine", &bobs)));
        // Names are 1–40 characters, without < > or : (they go in messages as <g:name:id>).
        assert!(add(&mut k, " ", &a).is_err());
        assert!(add(&mut k, "a<b", &a).is_err());
        assert!(add(&mut k, "a:b", &a).is_err());
        assert!(add(&mut k, &"x".repeat(MAX_GIF_NAME + 1), &a).is_err());
        // Only your own picture, not sent yet.
        assert!(add(&mut k, "bobs", &bobs).is_err());
        assert!(hidden_from(&hub, &bob, &hub.db.file(&a).unwrap()), "not sent yet: only its uploader's");
        assert!(matches!(add(&mut k, "party cat", &a), Ok(Event::Ok)));
        let gifs = hub.db.gifs();
        assert_eq!(gifs.len(), 1);
        assert_eq!((gifs[0].name.as_str(), gifs[0].width, gifs[0].height, gifs[0].by.as_str()), ("party cat", 64, 48, keeper.as_str()));
        // Now everyone here has it: in the server's info, and its picture.
        assert_eq!(hub.info().gifs, gifs);
        assert!(!hidden_from(&hub, &bob, &hub.db.file(&a).unwrap()));
        assert!(hidden_from(&hub, &keeper, &hub.db.file(&bobs).unwrap()), "other pictures not sent yet stay private");
        // Each name and picture once.
        assert!(add(&mut k, "PARTY CAT", &c2).is_err());
        assert!(add(&mut k, "again", &a).is_err());
        // It can't be sent as an attachment: deleting that message would take it from everyone.
        let general = hub.db.create_channel("general", ChannelKind::Text).unwrap();
        let send = |c: &mut Ctx, file: &str| {
            c.handle(Request::SendMessage { channel_id: general.id, content: "look".into(), reply_to: None, nonce: String::new(), attachments: vec![file.into()], sealed: sealed_by(&c.uid) })
        };
        assert!(send(&mut k, &a).is_err());
        assert!(send(&mut k, &c2).is_err(), "pictures are sent encrypted now (FileKind::Sealed)");
        // Renamed (not to another's name), then removed, its picture with it.
        assert!(matches!(add(&mut k, "dance", &"d".repeat(32)), Err(_)), "no such upload");
        assert!(forbidden(b.handle(Request::RenameGif { id: a.clone(), name: "mine".into() })));
        assert!(k.handle(Request::RenameGif { id: a.clone(), name: "a:b".into() }).is_err());
        assert!(k.handle(Request::RenameGif { id: a.clone(), name: "dance".into() }).is_ok());
        assert_eq!(hub.db.gifs()[0].name, "dance");
        assert!(forbidden(b.handle(Request::RemoveGif { id: a.clone() })));
        assert!(k.handle(Request::RemoveGif { id: a.clone() }).is_ok());
        assert!(hub.db.gifs().is_empty() && hub.info().gifs.is_empty());
        assert!(hub.db.file(&a).is_none());
    }

    #[test]
    fn role_managers_cant_hand_out_powers_they_lack() {
        let hub = hub();
        let (owner, bob, carol) = (uid(), uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        hub.db.insert_user(&carol, "Carol", Role::Member).unwrap();
        // "Role manager" first, then "Moderator" (new roles go to the bottom, so it's below).
        let manager = hub.db.create_role("Role manager", 0, perm::MANAGE_ROLES, false).unwrap();
        let moderator = hub.db.create_role("Moderator", 0, perm::KICK | perm::BAN, false).unwrap();
        let helper = hub.db.create_role("Helper", 0, 0, false).unwrap();
        hub.db.set_member_roles(&bob, &[manager.id]).unwrap();
        let mut b = ctx(&hub, &bob);
        // Giving himself (or Carol) the Moderator role would give powers he doesn't have.
        assert!(forbidden(b.handle(Request::SetMemberRoles { user_id: bob.clone(), roles: vec![manager.id, moderator.id] })));
        assert!(forbidden(b.handle(Request::SetMemberRoles { user_id: carol.clone(), roles: vec![moderator.id] })));
        // A role with nothing he lacks is fine.
        assert!(b.handle(Request::SetMemberRoles { user_id: carol.clone(), roles: vec![helper.id] }).is_ok());
        // Nor can he switch Kick/Ban off (or on) on a lower role.
        let r = b.handle(Request::UpdateRole { role_id: moderator.id, name: None, color: None, perms: Some(perm::KICK), hoist: None, shift: 0 });
        assert!(forbidden(r));
        // The owner can.
        let mut o = ctx(&hub, &owner);
        assert!(o.handle(Request::SetMemberRoles { user_id: carol.clone(), roles: vec![moderator.id] }).is_ok());
    }

    #[test]
    fn server_mute_and_deafen_need_the_permission_and_a_lower_role() {
        let hub = hub();
        let (owner, modr, bob) = (uid(), uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&modr, "Mod", Role::Member).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let muter = hub.db.create_role("Voice mod", 0, perm::MUTE_MEMBERS, false).unwrap();
        hub.db.set_member_roles(&modr, &[muter.id]).unwrap();
        let mute = |c: &mut Ctx, who: &str, on: bool| c.handle(Request::ServerMute { user_id: who.into(), muted: on });
        let deafen = |c: &mut Ctx, who: &str, on: bool| c.handle(Request::ServerDeafen { user_id: who.into(), deafened: on });
        // Bob has neither permission.
        let mut b = ctx(&hub, &bob);
        assert!(forbidden(mute(&mut b, &modr, true)));
        assert!(forbidden(deafen(&mut b, &modr, true)));
        // The voice mod can mute (not deafen) someone below him, and himself, but not the owner.
        let mut m = ctx(&hub, &modr);
        assert!(mute(&mut m, &bob, true).is_ok());
        assert!(forbidden(deafen(&mut m, &bob, true)));
        assert!(forbidden(mute(&mut m, &owner, true)));
        assert!(mute(&mut m, &modr, true).is_ok());
        // ...but not undo it (someone above him has to).
        assert!(forbidden(mute(&mut m, &modr, false)));
        assert!(m.handle(Request::ServerMute { user_id: "x".repeat(5000), muted: true }).is_err());
        // The owner can do both to anyone. It's kept (it lasts through restarts and rejoins).
        let mut o = ctx(&hub, &owner);
        assert!(deafen(&mut o, &bob, true).is_ok());
        assert!(deafen(&mut o, &owner, true).is_ok());
        let muted = hub.moderated(ModeratedVoice::Muted);
        let deaf = hub.moderated(ModeratedVoice::Deafened);
        assert!(muted.contains(&bob) && muted.contains(&modr) && !muted.contains(&owner));
        assert!(deaf.contains(&bob) && deaf.contains(&owner) && !deaf.contains(&modr));
        // Taken off again (deafening isn't muting: Bob's mute stays), the voice mod's by the
        // owner. The owner can undo their own.
        assert!(deafen(&mut o, &bob, false).is_ok());
        assert!(mute(&mut o, &modr, false).is_ok());
        assert!(deafen(&mut o, &owner, false).is_ok());
        assert!(!hub.moderated(ModeratedVoice::Deafened).contains(&bob));
        assert!(hub.moderated(ModeratedVoice::Muted).contains(&bob));
        assert!(!hub.moderated(ModeratedVoice::Muted).contains(&modr));
    }

    #[test]
    fn a_ban_turns_off_the_invite_they_came_with() {
        let hub = hub();
        let owner = uid();
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        let ip: std::net::IpAddr = "198.51.100.4".parse().unwrap();
        hub.db.create_invite("public-link", &owner, None, None, Role::Member).unwrap();
        hub.db.create_invite("other-link", &owner, None, None, Role::Member).unwrap();
        let (troll, friend) = (uid(), uid());
        assert!(hub.authorize(&troll, "Troll", Some("public-link"), ip).is_ok());
        assert!(hub.authorize(&friend, "Friend", Some("other-link"), ip).is_ok());
        assert!(ctx(&hub, &owner).handle(Request::Ban { user_id: troll.clone() }).is_ok());
        // A new identity can't come back in through the link the troll used...
        assert!(hub.authorize(&uid(), "Troll again", Some("public-link"), ip).is_err());
        assert_eq!(hub.db.invite_role("public-link"), None);
        // ...and other links still work. Kicking (not banning) leaves links alone.
        assert!(hub.db.invite_role("other-link").is_some());
        assert!(ctx(&hub, &owner).handle(Request::Kick { user_id: friend.clone() }).is_ok());
        assert!(hub.db.invite_role("other-link").is_some());
    }

    #[test]
    fn gif_keys_and_the_bug_count_go_only_where_theyre_needed() {
        let hub = hub();
        let (owner, member) = (uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&member, "Pat", Role::Member).unwrap();
        assert!(ctx(&hub, &owner).handle(Request::SetGiphyKey { key: "abc123".into() }).is_ok());
        assert!(ctx(&hub, &owner).handle(Request::SetKlipyKey { key: "kl-1_2".into() }).is_ok());
        hub.db.add_bug(&BugReport { title: "crash".into(), ..Default::default() }).unwrap();
        let (o, m) = (hub.info_for(&owner), hub.info_for(&member));
        // GIPHY and KLIPY want each app to search them itself: members' apps get the keys...
        assert_eq!((o.giphy_key.as_str(), o.klipy_key.as_str()), ("abc123", "kl-1_2"));
        assert_eq!((m.giphy_key.as_str(), m.klipy_key.as_str()), ("abc123", "kl-1_2"));
        // ...strangers don't, and only the owner is told how many bug reports are waiting.
        assert!(hub.info_for(&uid()).giphy_key.is_empty());
        assert_eq!((o.new_bug_reports, m.new_bug_reports), (1, 0));
    }

    #[test]
    fn bug_list_always_loads() {
        let hub = hub();
        let owner = uid();
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        // Lots of big reports (a flood from strangers, say), some already looked at.
        for i in 0..60 {
            let r = BugReport { title: format!("report {i}"), body: "b".repeat(BUG_BODY_MAX), log: "l".repeat(BUG_LOG_MAX), ..Default::default() };
            let id = hub.db.add_bug(&r).unwrap();
            if i % 2 == 0 {
                hub.db.set_bug_status(id, BugStatus::Approved).unwrap();
            }
        }
        let reports = match ctx(&hub, &owner).handle(Request::ListBugs) {
            Ok(Event::Bugs { reports }) => reports,
            other => panic!("{other:?}"),
        };
        // It fits in one reply, with what's waiting first.
        assert!(!reports.is_empty());
        let b = frame(1, Event::Bugs { reports: reports.clone() });
        assert!(b.len() <= pyr_proto::MAX_CONTROL_FRAME, "{} bytes", b.len());
        assert!(reports.iter().all(|r| r.status == BugStatus::New), "the new ones come first");
        assert_eq!(reports[0].report.title, "report 59", "newest first");
    }

    #[test]
    fn plugins_sent_in_are_checked_and_kept_for_the_owner() {
        let hub = hub();
        let (owner, member) = (uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&member, "Member", Role::Member).unwrap();
        let ip: std::net::IpAddr = "203.0.113.9".parse().unwrap();
        let file = |path: &str, data: &[u8]| SourceFile { path: path.into(), data: pyr_proto::b64::encode(data) };
        let good = || PluginSubmission {
            name: "Dice\u{7}".into(),
            kind: "addon".into(),
            version: "1.0.0".into(),
            summary: "Rolls dice".into(),
            author: "Mara".into(),
            files: vec![file("Cargo.toml", b"[package]"), file("src/lib.rs", b"fn main() {}")],
            ..Default::default()
        };
        // Not collected yet.
        assert!(hub.take_plugin_submission(ip, good(), false).unwrap_err().contains("doesn't collect"));
        assert!(forbidden(ctx(&hub, &member).handle(Request::SetPluginSubmissions { on: true })));
        assert!(matches!(ctx(&hub, &owner).handle(Request::SetPluginSubmissions { on: true }), Ok(Event::Ok)));
        // Paths that leave the folder, no Cargo.toml, too big, not a plugin kind.
        let mut bad = good();
        bad.files.push(file("../escape.rs", b"x"));
        assert!(hub.take_plugin_submission(ip, bad, false).is_err());
        let mut bad = good();
        bad.files[0].path = "C:/Windows/x".into();
        assert!(hub.take_plugin_submission("203.0.113.10".parse().unwrap(), bad, false).is_err());
        let mut bad = good();
        bad.files.remove(0);
        assert!(hub.take_plugin_submission("203.0.113.11".parse().unwrap(), bad, false).unwrap_err().contains("Cargo.toml"));
        let mut bad = good();
        bad.files.push(file("assets/big.bin", &vec![0u8; SUBMISSION_MAX]));
        assert!(hub.take_plugin_submission("203.0.113.12".parse().unwrap(), bad, false).unwrap_err().contains("bigger"));
        let mut bad = good();
        bad.kind = "script".into();
        assert!(hub.take_plugin_submission("203.0.113.13".parse().unwrap(), bad, false).is_err());
        // A good one is kept (its name without the control character); six an hour per place.
        let id = hub.take_plugin_submission("203.0.113.14".parse().unwrap(), good(), false).unwrap();
        hub.take_plugin_submission("203.0.113.14".parse().unwrap(), good(), false).unwrap();
        hub.take_plugin_submission("203.0.113.14".parse().unwrap(), good(), false).unwrap();
        for _ in 0..3 {
            hub.take_plugin_submission("203.0.113.15".parse().unwrap(), good(), false).unwrap();
        }
        for place in ["203.0.113.14", "203.0.113.15"] {
            hub.take_plugin_submission(place.parse().unwrap(), good(), false).unwrap();
            hub.take_plugin_submission(place.parse().unwrap(), good(), false).unwrap();
            hub.take_plugin_submission(place.parse().unwrap(), good(), false).unwrap();
        }
        assert!(hub.take_plugin_submission("203.0.113.14".parse().unwrap(), good(), false).unwrap_err().contains("try again"));
        assert!(hub.may_take_submission("203.0.113.14".parse().unwrap()).is_err());
        assert!(hub.may_take_submission("203.0.113.16".parse().unwrap()).is_ok());
        // Only the owner sees them, and the count.
        assert!(forbidden(ctx(&hub, &member).handle(Request::ListPluginSubmissions)));
        assert_eq!(hub.info_for(&member).new_plugin_submissions, 0);
        assert_eq!(hub.info_for(&owner).new_plugin_submissions, 12);
        let list = match ctx(&hub, &owner).handle(Request::ListPluginSubmissions) {
            Ok(Event::PluginSubmissions { list }) => list,
            other => panic!("{other:?}"),
        };
        assert_eq!(list.len(), 12);
        let e = list.iter().find(|e| e.id == id).unwrap();
        assert_eq!((e.name.as_str(), e.files, e.size), ("Dice", 2, 21));
        match ctx(&hub, &owner).handle(Request::GetPluginSubmission { id }) {
            Ok(Event::PluginSubmission { files, more, .. }) => {
                assert_eq!(files, good().files);
                assert!(more.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert!(forbidden(ctx(&hub, &member).handle(Request::GetPluginSubmission { id })));
        ctx(&hub, &owner).handle(Request::SetPluginSubmissionStatus { id, status: SubmissionStatus::Approved, note: String::new() }).unwrap();
        assert_eq!(hub.info_for(&owner).new_plugin_submissions, 11);
        assert!(matches!(
            ctx(&hub, &owner).handle(Request::SetPluginSubmissionStatus { id: 999, status: SubmissionStatus::Rejected, note: String::new() }),
            Err((ErrorCode::NotFound, _))
        ));
    }

    /// A skin's files, by `author` (in its skin.json).
    fn skin_files(author: &str) -> Vec<SourceFile> {
        let file = |path: &str, data: &[u8]| SourceFile { path: path.into(), data: pyr_proto::b64::encode(data) };
        let json = r##"{ "name": "Blood Moon", "version": "1.0", "summary": "Red and black.", "author": "AUTHOR",
            "colors": { "main": "#1a0d0f", "ember": "#d4202c" }, "sounds": { "message": "sounds/message.ings" } }"##
            .replace("AUTHOR", author);
        let sound = pyr_proto::sound::write(1, &[vec![0xf8, 0xff, 0xfe]]);
        vec![file("skin.json", json.as_bytes()), file("sounds/message.ings", &sound), file("big.txt", &vec![b'a'; 60_000])]
    }

    #[test]
    fn skins_and_new_versions_from_the_same_sender() {
        let hub = hub();
        let owner = uid();
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        ctx(&hub, &owner).handle(Request::SetPluginSubmissions { on: true }).unwrap();
        let ip = |n: u8| -> std::net::IpAddr { format!("198.51.100.{n}").parse().unwrap() };
        let (mara, eve) = (crypto::Identity::generate().user_id(), crypto::Identity::generate().user_id());
        // (Each under their own author name: see `author_names_are_kept_by_whoever_had_them_first`.)
        let skin = |by: &str| {
            let author = if by == eve { "Eve" } else { "Mara" };
            PluginSubmission { name: "Blood Moon".into(), kind: SKIN_KIND.into(), version: "1.0".into(), author_id: by.into(), files: skin_files(author), ..Default::default() }
        };
        // A skin is checked as Pyr checks skins.
        let mut bad = skin(&mara);
        bad.files[0].data = pyr_proto::b64::encode(br##"{ "name": "X", "version": "1", "colors": { "body": "#333333", "main": "#303030" } }"##);
        assert!(hub.take_plugin_submission(ip(1), bad, true).unwrap_err().contains("hard to read"));
        let mut bad = skin(&mara);
        bad.files.push(SourceFile { path: "tool.exe".into(), data: pyr_proto::b64::encode(b"MZ") });
        assert!(hub.take_plugin_submission(ip(2), bad, true).unwrap_err().contains("doesn't use it"));
        // Signed: the sender is kept. Not signed: nobody is, and it can't be a new version.
        let first = hub.take_plugin_submission(ip(3), skin(&mara), true).unwrap();
        let anon = hub.take_plugin_submission(ip(4), skin(&eve), false).unwrap();
        let mut sneaky = skin("");
        sneaky.update_of = first;
        assert!(hub.take_plugin_submission(ip(5), sneaky, false).unwrap_err().contains("signed"));
        // Only its sender can send a new version, of the same kind.
        let mut by_eve = skin(&eve);
        by_eve.update_of = first;
        assert!(hub.take_plugin_submission(ip(6), by_eve, true).unwrap_err().contains("someone else"));
        let mut onto_anon = skin(&eve);
        onto_anon.update_of = anon;
        assert!(hub.take_plugin_submission(ip(7), onto_anon, true).unwrap_err().contains("someone else"));
        let mut as_plugin = skin(&mara);
        as_plugin.kind = "addon".into();
        as_plugin.update_of = first;
        as_plugin.files = vec![SourceFile { path: "Cargo.toml".into(), data: pyr_proto::b64::encode(b"[package]") }];
        assert!(hub.take_plugin_submission(ip(8), as_plugin, true).unwrap_err().contains("is a skin"));
        let mut v2 = skin(&mara);
        v2.version = "1.1".into();
        v2.update_of = first;
        v2.files[2] = SourceFile { path: "OFL.txt".into(), data: pyr_proto::b64::encode(b"SIL Open Font License") };
        let second = hub.take_plugin_submission(ip(9), v2, true).unwrap();
        // The owner sees skins counted apart, and what the new version changed.
        assert_eq!(hub.info_for(&owner).new_skin_submissions, 3);
        assert_eq!(hub.info_for(&owner).new_plugin_submissions, 0);
        let list = match ctx(&hub, &owner).handle(Request::ListPluginSubmissions) {
            Ok(Event::PluginSubmissions { list }) => list,
            other => panic!("{other:?}"),
        };
        let e = list.iter().find(|e| e.id == second).unwrap();
        assert_eq!((e.update_of, e.author_id.as_str(), e.kind.as_str()), (first, mara.as_str(), SKIN_KIND));
        assert_eq!(e.changes, "Added: OFL.txt. Taken out: big.txt.");
        assert!(list.iter().find(|e| e.id == anon).unwrap().author_id.is_empty());
        // A note goes to the sender with the status; they see their own, nobody else's.
        ctx(&hub, &owner)
            .handle(Request::SetPluginSubmissionStatus { id: first, status: SubmissionStatus::Rejected, note: "Make the red darker.\u{7}".into() })
            .unwrap();
        let mine = hub.my_submissions(ip(10), &mara).unwrap();
        assert_eq!(mine.iter().map(|e| e.id).collect::<Vec<_>>(), vec![second, first]);
        assert_eq!(mine[1].note, "Make the red darker.");
        assert_eq!(mine[1].status, SubmissionStatus::Rejected);
        assert!(hub.my_submissions(ip(10), &eve).unwrap().is_empty(), "eve signed none");
        assert!(hub.my_submissions(ip(10), "").unwrap().is_empty());
        for _ in 0..27 {
            hub.my_submissions(ip(10), &mara).unwrap();
        }
        assert!(hub.my_submissions(ip(10), &mara).is_err(), "30 asks an hour from one place");
        // One sender can't flood from many places either: six tries an hour (refused ones count),
        // and mara has made five.
        let mut more = skin(&mara);
        more.update_of = second;
        hub.take_plugin_submission(ip(20), more.clone(), true).unwrap();
        assert!(hub.take_plugin_submission(ip(21), more, true).unwrap_err().contains("a lot"));
        hub.take_plugin_submission(ip(22), skin(&eve), true).unwrap();
    }

    #[test]
    fn author_names_are_kept_by_whoever_had_them_first() {
        let hub = hub();
        let owner = crypto::Identity::generate().user_id();
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        ctx(&hub, &owner).handle(Request::SetPluginSubmissions { on: true }).unwrap();
        let ip = |n: u8| -> std::net::IpAddr { format!("192.0.2.{n}").parse().unwrap() };
        let (mara, eve) = (crypto::Identity::generate().user_id(), crypto::Identity::generate().user_id());
        let file = |path: &str, data: &[u8]| SourceFile { path: path.into(), data: pyr_proto::b64::encode(data) };
        let plugin = |by: &str, author: &str| PluginSubmission {
            name: "Poker".into(),
            kind: "addon".into(),
            version: "0.1.0".into(),
            author: author.into(),
            author_id: by.into(),
            files: vec![file("Cargo.toml", b"[package]"), file("src/lib.rs", b"")],
            ..Default::default()
        };
        // Mara sends one in first, signed: "Maanix" is hers.
        hub.take_plugin_submission(ip(1), plugin(&mara, "Maanix"), true).unwrap();
        assert_eq!(hub.db.author_name(&author_key("Maanix")).map(|(n, who)| (n, who == mara)), Some(("Maanix".to_string(), true)));
        // Nobody else can use it, or one that only looks different: case, spaces and dashes,
        // a Cyrillic а, a capital I for an l, 0 for o.
        // (Each from someone new, somewhere new: six tries an hour each.)
        for (i, name) in ["Maanix", "maanix", " MAANIX ", "Maa-nix", "Mааnix", "MaanIx", "Maan1x"].into_iter().enumerate() {
            let someone = crypto::Identity::generate().user_id();
            let err = hub.take_plugin_submission(ip(100 + i as u8), plugin(&someone, name), true).unwrap_err();
            assert!(err.contains("is taken"), "{name}: {err}");
        }
        // Not signed, it can't be told from her either.
        assert!(hub.take_plugin_submission(ip(3), plugin("", "Maanix"), false).unwrap_err().contains("is taken"));
        // She can go on using it, and others their own names (signed: theirs from then on).
        hub.take_plugin_submission(ip(4), plugin(&mara, "maanix"), true).unwrap();
        hub.take_plugin_submission(ip(5), plugin(&eve, "Eve"), true).unwrap();
        assert!(hub.take_plugin_submission(ip(6), plugin(&mara, "Eve"), true).unwrap_err().contains("is taken"));
        // Not signed: taken in, but the name isn't anyone's.
        hub.take_plugin_submission(ip(7), plugin("", "Sam"), false).unwrap();
        assert!(hub.db.author_name(&author_key("Sam")).is_none());
        // A skin goes by the author in its skin.json too.
        let mut skin = plugin(&eve, "");
        skin.kind = SKIN_KIND.into();
        skin.name = "Blood Moon".into();
        skin.files = skin_files("Maanix");
        assert!(hub.take_plugin_submission(ip(8), skin.clone(), true).unwrap_err().contains("is taken"));
        skin.files = skin_files("Eve");
        hub.take_plugin_submission(ip(9), skin, true).unwrap();
        // Firelit Games' names are the owner's alone.
        for (i, name) in ["Firelit Games", "firelit", "The Firelit Crew", "Pyr"].into_iter().enumerate() {
            let someone = crypto::Identity::generate().user_id();
            let err = hub.take_plugin_submission(ip(120 + i as u8), plugin(&someone, name), true).unwrap_err();
            assert!(err.contains("Firelit Games' own"), "{name}: {err}");
        }
        hub.take_plugin_submission(ip(11), plugin(&owner, "Firelit Games"), true).unwrap();
        // Names given out before this server knew about them: whoever sent first has them.
        let db = &hub.db;
        db.refill_author_names();
        assert!(db.author_name(&author_key("Maanix")).is_some_and(|(_, who)| who == mara));
        assert!(db.author_name(&author_key("Eve")).is_some_and(|(_, who)| who == eve));
        assert!(db.author_name(&author_key("Sam")).is_none(), "not signed: nobody's");
    }

    #[test]
    fn author_names_that_look_alike_are_one() {
        let same = |a: &str, b: &str| author_key(a) == author_key(b);
        assert!(same("Firelit Games", "firelit-games"));
        assert!(same("Firelit Games", "FireIit Gаmes"), "a capital i, a Cyrillic а");
        assert!(same("Firelit Games", "Ｆｉｒｅｌｉｔ Games"), "full width");
        assert!(same("Bob", "B0b"));
        assert!(same("Modern", "Modem"), "rn and m");
        assert!(same("Zoë", "Zoe"));
        assert!(!same("Mara", "Maria"));
        assert!(!same("Sam", "Pam"));
        assert_eq!(author_key("\u{200b}  \u{202e}"), "", "nothing but invisible characters is no name");
    }

    #[test]
    fn a_big_skins_files_come_one_by_one() {
        let hub = hub();
        let owner = uid();
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        ctx(&hub, &owner).handle(Request::SetPluginSubmissions { on: true }).unwrap();
        let file = |path: &str, data: &[u8]| SourceFile { path: path.into(), data: pyr_proto::b64::encode(data) };
        // A picture and a font too big to come along together with everything else.
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\x07\x80\0\0\x04\x38\x08\x06\0\0\0".to_vec();
        png.resize(650_000, 0);
        let mut font = vec![0, 1, 0, 0];
        font.resize(300_000, 7);
        let json = br#"{ "name": "Big", "version": "1", "background": { "file": "bg.png" }, "fonts": { "body": "f.ttf" } }"#;
        let files = vec![file("skin.json", json), file("bg.png", &png), file("f.ttf", &font), file("OFL.txt", b"SIL Open Font License")];
        let sub = PluginSubmission { name: "Big".into(), kind: SKIN_KIND.into(), version: "1".into(), files: files.clone(), ..Default::default() };
        let id = hub.take_plugin_submission("198.51.100.1".parse().unwrap(), sub, false).unwrap();
        match ctx(&hub, &owner).handle(Request::GetPluginSubmission { id }) {
            Ok(Event::PluginSubmission { files: inline, more, .. }) => {
                assert_eq!(inline.len() + more.len(), files.len());
                assert_eq!(more.iter().map(|d| d.path.as_str()).collect::<Vec<_>>(), vec!["bg.png"]);
                let total: usize = inline.iter().map(|f| pyr_proto::b64::decode(&f.data).unwrap().len()).sum();
                assert!(total <= INLINE_SUBMISSION_BYTES);
                match ctx(&hub, &owner).handle(Request::GetSubmissionFile { id, path: "bg.png".into() }) {
                    Ok(Event::SubmissionFile { data, .. }) => assert_eq!(pyr_proto::b64::decode(&data).unwrap(), png),
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(ctx(&hub, &owner).handle(Request::GetSubmissionFile { id, path: "nope.txt".into() }), Err((ErrorCode::NotFound, _))));
    }

    #[test]
    fn new_members_get_a_small_share_of_storage() {
        let hub = hub();
        let (old, new) = (uid(), uid());
        hub.db.insert_user(&old, "Old", Role::Member).unwrap();
        hub.db.insert_user(&new, "New", Role::Member).unwrap();
        hub.db.set_joined_at(&old, pyr_proto::now_ms() - 10 * 24 * 3600 * 1000).unwrap();
        let mb = |n: u64| n << 20;
        // A newcomer gets 100 MB; someone who's been here a while, a tenth of the server's 5 GB.
        assert!(hub.reserve_upload(&new, mb(150)).is_err());
        assert!(hub.reserve_upload(&new, mb(90)).is_ok());
        assert!(hub.reserve_upload(&old, mb(400)).is_ok());
        assert!(hub.reserve_upload(&old, mb(600)).is_err(), "a tenth of the server is the most");
    }

    #[test]
    fn one_place_cant_take_every_slot() {
        use crate::hub::{Admit, MAX_CONNECTIONS_PER_IP, MAX_CONNECTIONS_PER_NETWORK, MAX_WAITING_PER_IP, MAX_WAITING_PER_NETWORK};
        let hub = hub();
        let ip = |s: String| -> std::net::IpAddr { s.parse().unwrap() };
        let take = |a: std::net::IpAddr| match hub.admit(a, true) {
            (Admit::Accept, Some(s)) => Some(s),
            _ => None,
        };
        // A /64 is one place, whatever address in it: only so many can wait to log in at once...
        let mut slots: Vec<ConnSlot> = (0..MAX_WAITING_PER_IP).map(|i| take(ip(format!("2001:db8:1:1::{:x}", i + 1))).unwrap()).collect();
        assert!(take(ip("2001:db8:1:1:ffff:ffff:ffff:ffff".into())).is_none(), "the /64 has as many waiting as it may");
        // ...and once they're in, the place may have more, up to its own cap.
        for s in &mut slots {
            s.logged_in();
        }
        while slots.len() < MAX_CONNECTIONS_PER_IP {
            let mut s = take(ip(format!("2001:db8:1:1::{:x}", slots.len() + 100))).unwrap();
            s.logged_in();
            slots.push(s);
        }
        assert!(take(ip("2001:db8:1:1::abcd".into())).is_none(), "the /64 is full");
        // Other /64s in the same /48 count towards the /48's cap (waiting ones, then all).
        let mut wide: Vec<ConnSlot> = Vec::new();
        for i in 0..MAX_WAITING_PER_NETWORK {
            wide.push(take(ip(format!("2001:db8:1:{:x}::1", i + 2))).expect("another /64"));
        }
        assert!(take(ip("2001:db8:1:ff::1".into())).is_none(), "the /48 has as many waiting as it may");
        for s in &mut wide {
            s.logged_in();
        }
        while slots.len() + wide.len() < MAX_CONNECTIONS_PER_NETWORK {
            let mut s = take(ip(format!("2001:db8:1:{:x}::1", wide.len() + 100))).expect("room in the /48");
            s.logged_in();
            wide.push(s);
        }
        assert!(take(ip("2001:db8:1:fff::1".into())).is_none(), "the /48 is full");
        // Someone else is fine: another network, or an IPv4 address.
        let other = take(ip("2001:db8:2::1".into()));
        assert!(other.is_some());
        assert!(take(ip("203.0.113.9".into())).is_some());
        // Leaving frees the places up.
        drop(wide);
        assert!(take(ip("2001:db8:1:ff::1".into())).is_some());
    }

    #[test]
    fn echoed_fields_are_bounded_and_ids_checked() {
        let hub = hub();
        let (owner, bob) = (uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let ch = hub.db.create_channel("general", ChannelKind::Text).unwrap();
        let mut b = ctx(&hub, &bob);
        let send = |c: &mut Ctx, nonce: String| {
            c.handle(Request::SendMessage { channel_id: ch.id, content: "hi".into(), reply_to: None, nonce, attachments: vec![], sealed: sealed_by(&c.uid) })
        };
        assert!(send(&mut b, "x".repeat(MAX_NONCE_LEN + 1)).is_err(), "a huge nonce would be echoed to everyone");
        assert!(send(&mut b, "n1".into()).is_ok());
        // Ids that can't be anyone are refused before they reach the database or a broadcast.
        let mut o = ctx(&hub, &owner);
        assert!(o.handle(Request::Ban { user_id: "x".repeat(5000) }).is_err());
        assert!(b.handle(Request::Block { user_id: "y".repeat(5000) }).is_err());
        assert!(b.handle(Request::FriendRequest { user_id: format!("{owner}=") }).is_err());
        // Names that would read as @everyone or show as nothing.
        assert!(b.handle(Request::UpdateProfile { name: "@everyone".into() }).is_err());
        assert!(b.handle(Request::UpdateProfile { name: "\u{200b}\u{3164}".into() }).is_err());
        assert!(b.handle(Request::UpdateProfile { name: "Bo\u{202e}b".into() }).is_ok());
        assert_eq!(hub.db.get_user(&bob).unwrap().name, "Bob");
    }

    #[test]
    fn reactions_are_real_emoji_and_custom_ones_are_rewritten() {
        let hub = hub();
        let (owner, bob) = (uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&bob, "Bob", Role::Member).unwrap();
        let ch = hub.db.create_channel("general", ChannelKind::Text).unwrap();
        let mid = hub.db.insert_message(ch.id, &owner, "hello", None).unwrap().id;
        let id = "0123456789abcdef0123456789abcdef".to_string();
        hub.db.add_emoji(&CustomEmoji { id: id.clone(), name: "party".into(), animated: false, by: owner.clone() }).unwrap();
        let mut b = ctx(&hub, &bob);
        let react = |c: &mut Ctx, e: &str| c.handle(Request::React { message_id: mid, emoji: e.into(), add: true });
        assert!(react(&mut b, "👍🏽").is_ok());
        assert!(react(&mut b, "1\u{fe0f}\u{20e3}").is_ok());
        assert!(react(&mut b, "<<>>").is_err());
        assert!(react(&mut b, "a").is_err());
        assert!(react(&mut b, "\u{200b}").is_err());
        assert!(react(&mut b, &format!("<:anything at all here:{id}>")).is_ok());
        let m = hub.db.get_message(mid).unwrap();
        assert!(m.reactions.iter().any(|r| r.emoji == format!("<:party:{id}>")), "{:?}", m.reactions);
        assert!(!m.reactions.iter().any(|r| r.emoji.contains("anything")));
        assert!(react(&mut b, "<:nope:ffffffffffffffffffffffffffffffff>").is_err());
    }

    #[test]
    fn every_picker_emoji_is_a_valid_reaction() {
        let index: serde_json::Value = serde_json::from_str(include_str!("../../app/assets/emoji/index.json")).unwrap();
        let mut bad = Vec::new();
        let mut n = 0;
        for g in index["groups"].as_array().unwrap() {
            for it in g["items"].as_array().unwrap() {
                let mut all = vec![it["e"].as_str().unwrap().to_string()];
                if let Some(t) = it["t"].as_array() {
                    for v in t {
                        all.push(v[0].as_str().unwrap_or_else(|| v.as_str().unwrap()).to_string());
                    }
                }
                for e in all {
                    n += 1;
                    if !super::valid_unicode_reaction(&e) {
                        bad.push(format!("{} {:?}", it["n"], e.chars().map(|c| format!("{:x}", c as u32)).collect::<Vec<_>>()));
                    }
                }
            }
        }
        assert!(n > 2500, "{n}");
        assert!(bad.is_empty(), "{} of {n} refused: {:?}", bad.len(), &bad[..bad.len().min(30)]);
    }

    #[test]
    fn messages_are_taken_only_encrypted_and_keys_go_only_to_who_can_read() {
        use pyr_proto::sealed::MessageKeys;
        let hub = hub();
        let (ia, ib, ic) = (crypto::Identity::generate(), crypto::Identity::generate(), crypto::Identity::generate());
        let (a, b, c) = (ia.user_id(), ib.user_id(), ic.user_id());
        hub.db.insert_user(&a, "A", Role::Owner).unwrap();
        hub.db.insert_user(&b, "B", Role::Member).unwrap();
        hub.db.insert_user(&c, "C", Role::Member).unwrap();
        let (dm, _) = hub.db.open_dm(&a, &b).unwrap();
        let (mut ca, mut cb, mut cc) = (ctx(&hub, &a), ctx(&hub, &b), ctx(&hub, &c));
        let send = |x: &mut Ctx, sealed: Option<Sealed>| {
            x.handle(Request::SendMessage { channel_id: dm, content: "hi".into(), reply_to: None, nonce: String::new(), attachments: vec![], sealed })
        };
        // Nothing in the clear, nor with someone else's key.
        assert!(send(&mut ca, None).is_err());
        assert!(send(&mut ca, sealed_by(&b)).is_err());
        assert!(send(&mut ca, sealed_by(&a)).is_ok());
        let m = hub.db.history(dm, None, 10).0.pop().unwrap();
        assert!(m.sealed.is_some() && m.content == pyr_proto::sealed::PLACEHOLDER, "older Pyrs see a note");
        // Message keys: only one's own.
        let (ka, kb, kc) = (MessageKeys::derive(&ia), MessageKeys::derive(&ib), MessageKeys::derive(&ic));
        assert!(cb.handle(Request::PublishMessageKey { key: ka.publication(&ia) }).is_err());
        for (x, k, id) in [(&mut ca, &ka, &ia), (&mut cb, &kb, &ib), (&mut cc, &kc, &ic)] {
            assert!(x.handle(Request::PublishMessageKey { key: k.publication(id) }).is_ok());
        }
        // Who reads a direct message: its two people (and nobody else may ask).
        let Ok(Event::Readers { readers, keyless, more, .. }) = ca.handle(Request::ChannelReaders { channel_id: dm, after: None }) else { panic!() };
        let mut who: Vec<_> = readers.iter().map(|r| r.user.clone()).collect();
        who.sort();
        let mut want = vec![a.clone(), b.clone()];
        want.sort();
        assert_eq!((who, keyless.is_empty(), more), (want, true, false));
        assert!(cc.handle(Request::ChannelReaders { channel_id: dm, after: None }).is_err());
        // A key given to B and (wrongly) to C: only B's is kept.
        let kref = KeyRef { owner: a.clone(), id: pyr_proto::sealed::new_key_id() };
        let secret = crypto::random_bytes::<32>();
        let to = |uid: &str, them: &MessageKeys, kr: &KeyRef| ka.wrap(&a, uid, &them.public(), dm, kr, &secret).unwrap();
        assert!(ca.handle(Request::ShareKeys { channel_id: dm, shares: vec![to(&b, &kb, &kref), to(&c, &kc, &kref)] }).is_ok());
        let Ok(Event::KeyShares { shares, .. }) = cb.handle(Request::GetKeys { channel_id: dm, keys: vec![kref.clone()] }) else { panic!() };
        assert_eq!(shares.len(), 1);
        assert_eq!(kb.unwrap(&b, dm, &shares[0]), Some(secret));
        assert!(cc.handle(Request::GetKeys { channel_id: dm, keys: vec![kref.clone()] }).is_err());
        // Nobody gives out a key they weren't given.
        let other = KeyRef { owner: a.clone(), id: pyr_proto::sealed::new_key_id() };
        let fake = kb.wrap(&b, &a, &ka.public(), dm, &other, &secret).unwrap();
        assert!(cb.handle(Request::ShareKeys { channel_id: dm, shares: vec![fake] }).is_ok());
        assert!(hub.db.shares_for(dm, &a, std::slice::from_ref(&other)).is_empty());
        // A key A gave only to themself: B asks for it, and A's Pyr hears so when it's next here.
        let mine = KeyRef { owner: a.clone(), id: pyr_proto::sealed::new_key_id() };
        assert!(ca.handle(Request::ShareKeys { channel_id: dm, shares: vec![ka.wrap(&a, &a, &ka.public(), dm, &mine, &secret).unwrap()] }).is_ok());
        let Ok(Event::KeyShares { shares, .. }) = cb.handle(Request::GetKeys { channel_id: dm, keys: vec![mine.clone()] }) else { panic!() };
        assert!(shares.is_empty());
        assert_eq!(hub.db.wants_for_holder(&a, 10), vec![(dm, b.clone(), mine.clone())]);
        assert!(hub.db.wants_for_holder(&c, 10).is_empty());
        // Given: no longer wanted.
        assert!(ca.handle(Request::ShareKeys { channel_id: dm, shares: vec![to(&b, &kb, &mine)] }).is_ok());
        assert!(hub.db.wants_for_holder(&a, 10).is_empty());
    }

    #[test]
    fn history_pages_stay_inside_one_frame() {
        let hub = hub();
        let a = uid();
        hub.db.insert_user(&a, "A", Role::Owner).unwrap();
        let ch = hub.db.create_channel("general", ChannelKind::Text).unwrap();
        // 100 messages of 4000 four-byte characters: 1.6 MB, more than a frame.
        let big: String = "😀".repeat(pyr_proto::MAX_MESSAGE_CHARS);
        for _ in 0..100 {
            hub.db.insert_message(ch.id, &a, &big, None).unwrap();
        }
        let mut c = ctx(&hub, &a);
        let Ok(ev) = c.handle(Request::FetchHistory { channel_id: ch.id, before: None, limit: 100 }) else { panic!() };
        let Event::History { messages, complete, .. } = &ev else { panic!() };
        assert!(!complete);
        assert!(messages.len() < 100 && !messages.is_empty());
        assert!(frame(1, ev.clone()).len() < pyr_proto::MAX_CONTROL_FRAME);
    }

    #[test]
    fn groups_are_only_for_their_members() {
        let hub = hub();
        let people: Vec<String> = (0..12).map(|_| uid()).collect();
        for (i, p) in people.iter().enumerate() {
            hub.db.insert_user(p, &format!("P{i}"), Role::Member).unwrap();
        }
        let (a, b, c, out) = (&people[0], &people[1], &people[2], &people[3]);
        let mut ca = ctx(&hub, a);
        // At least two others, at most ten in all, members only, and nobody blocked.
        assert!(ca.handle(Request::CreateGroup { members: vec![b.clone()] }).is_err());
        assert!(ca.handle(Request::CreateGroup { members: people[1..11].to_vec() }).is_err());
        assert!(ca.handle(Request::CreateGroup { members: vec![b.clone(), uid()] }).is_err());
        hub.db.set_relation(c, a, 2).unwrap();
        assert!(forbidden(ca.handle(Request::CreateGroup { members: vec![b.clone(), c.clone()] })));
        hub.db.unblock(c, a).unwrap();
        let Ok(Event::GroupUpserted { group }) = ca.handle(Request::CreateGroup { members: vec![b.clone(), c.clone(), b.clone()] }) else { panic!() };
        assert_eq!(group.members.len(), 3, "{:?}", group.members);
        assert_eq!(&group.owner, a);
        let g = group.id;
        // Its members read and write in it; nobody else can, nor an app that doesn't know groups.
        let mut cb = ctx(&hub, b);
        assert!(cb.handle(Request::SendMessage { channel_id: g, content: "hi all".into(), reply_to: None, nonce: String::new(), attachments: vec![], sealed: sealed_by(b) }).is_ok());
        let mut co = ctx(&hub, out);
        assert!(co.handle(Request::FetchHistory { channel_id: g, before: None, limit: 50 }).is_err());
        assert!(co.handle(Request::SendMessage { channel_id: g, content: "let me in".into(), reply_to: None, nonce: String::new(), attachments: vec![], sealed: sealed_by(out) }).is_err());
        assert!(co.handle(Request::AddToGroup { channel_id: g, members: vec![out.clone()] }).is_err());
        assert!(co.handle(Request::RenameGroup { channel_id: g, name: "mine now".into() }).is_err());
        let mut old_app = ctx(&hub, c);
        old_app.groups = false;
        assert!(old_app.handle(Request::FetchHistory { channel_id: g, before: None, limit: 50 }).is_err());
        assert!(ca.handle(Request::FetchHistory { channel_id: g, before: None, limit: 50 }).is_ok());
        // Grows up to ten; a name; leaving.
        assert!(cb.handle(Request::AddToGroup { channel_id: g, members: people[3..11].to_vec() }).is_err(), "eleven is too many");
        assert!(cb.handle(Request::AddToGroup { channel_id: g, members: vec![out.clone()] }).is_ok());
        assert!(hub.db.group(g).unwrap().members.contains(out));
        assert!(cb.handle(Request::RenameGroup { channel_id: g, name: "  Raid   night ".into() }).is_ok());
        assert_eq!(hub.db.group(g).unwrap().name, "Raid night");
        assert!(co.handle(Request::LeaveGroup { channel_id: g }).is_ok());
        assert!(co.handle(Request::FetchHistory { channel_id: g, before: None, limit: 50 }).is_err(), "gone with leaving");
        // A moderator can't delete it (nor end its call) as if it were a channel.
        let boss = uid();
        hub.db.insert_user(&boss, "Boss", Role::Owner).unwrap();
        assert!(ctx(&hub, &boss).handle(Request::DeleteChannel { channel_id: g }).is_ok());
        assert!(hub.db.group(g).is_some());
        // Whoever started it leaving passes it on (and it stops counting as theirs).
        assert_eq!(hub.db.groups_owned(a), 1);
        assert!(ca.handle(Request::LeaveGroup { channel_id: g }).is_ok());
        assert_eq!(hub.db.groups_owned(a), 0);
        assert_eq!(&hub.db.group(g).unwrap().owner, b, "the one who's been in it longest");
        // Its messages go with the last one out.
        for p in [b, c] {
            let mut cp = ctx(&hub, p);
            assert!(cp.handle(Request::LeaveGroup { channel_id: g }).is_ok());
        }
        assert!(hub.db.group(g).is_none());
        assert!(hub.db.history(g, None, 50).0.is_empty());
    }

    #[test]
    fn blocked_people_cant_edit_react_or_type_in_dms() {
        let hub = hub();
        let (a, b) = (uid(), uid());
        hub.db.insert_user(&a, "A", Role::Member).unwrap();
        hub.db.insert_user(&b, "B", Role::Member).unwrap();
        let (dm, _) = hub.db.open_dm(&a, &b).unwrap();
        let mid = hub.db.insert_message(dm, &b, "hi", None).unwrap().id;
        hub.db.set_relation(&a, &b, 2).unwrap();
        let mut cb = ctx(&hub, &b);
        assert!(forbidden(cb.handle(Request::EditMessage { message_id: mid, content: "something mean".into(), sealed: sealed_by(&b) })));
        assert!(forbidden(cb.handle(Request::React { message_id: mid, emoji: "😠".into(), add: true })));
        assert_eq!(hub.db.get_message(mid).unwrap().content, "hi");
    }

    #[test]
    fn invites_capped_and_requests_rate_limited() {
        let hub = hub();
        let a = uid();
        hub.db.insert_user(&a, "A", Role::Member).unwrap();
        let mut c = ctx(&hub, &a);
        for _ in 0..MAX_INVITES_PER_MEMBER {
            assert!(c.handle(Request::CreateInvite { max_uses: None, expires_secs: None, reuse: false }).is_ok());
        }
        assert!(c.handle(Request::CreateInvite { max_uses: None, expires_secs: None, reuse: false }).is_err());
        let mut bucket = Bucket::new(REQ_RATE, REQ_BURST);
        let allowed = (0..1000).filter(|_| bucket.take(1.0)).count();
        assert!(allowed < 120, "{allowed}");
        // Kicking someone retires the invites they made.
        hub.db.remove_user(&a, false).unwrap();
        assert_eq!(hub.db.invites_by(&a), 0);
    }

    /// Invites like Discord's: Invite People hands back the same link while it lasts, people
    /// see and take back their own (managers everyone's), and the community's own code and
    /// short links are the owner's to change.
    #[test]
    fn invites_reused_listed_and_taken_back() {
        let hub = hub();
        let (owner, a, b) = (uid(), uid(), uid());
        hub.db.insert_user(&owner, "Owner", Role::Owner).unwrap();
        hub.db.insert_user(&a, "A", Role::Member).unwrap();
        hub.db.insert_user(&b, "B", Role::Member).unwrap();
        let made = |uid: &str, max_uses: Option<u32>, expires_secs: Option<u64>| match ctx(&hub, uid).handle(Request::CreateInvite { max_uses, expires_secs, reuse: true }) {
            Ok(Event::InviteCreated { code, link, short, expires_at, max_uses, .. }) => (code, link, short, expires_at, max_uses),
            other => panic!("{other:?}"),
        };
        let week = Some(7 * 86_400);
        let (first, link, short, expires_at, _) = made(&a, None, week);
        assert_eq!(first.len(), pyr_proto::directory::CODE_LEN, "short enough to be a short link's code");
        assert!(link.contains(&format!("/{first}/")) && short.is_none(), "no directory here: no short link");
        assert!(expires_at.is_some_and(|e| e > pyr_proto::now_ms() + 6 * 86_400_000));
        // The same settings again: the same invite. Others: a new one.
        assert_eq!(made(&a, None, week).0, first);
        let once = made(&a, Some(1), week).0;
        assert_ne!(once, first);
        assert_ne!(made(&a, None, None).0, first);
        assert_ne!(made(&b, None, week).0, first, "someone else's own");
        // Used up, it isn't handed back.
        hub.db.redeem_invite(&once).unwrap();
        assert_ne!(made(&a, Some(1), week).0, once);
        // Listed: one's own, or everyone's for managers (never owner links).
        hub.db.create_invite("owner-secret", "server", Some(1), None, Role::Owner).unwrap();
        let codes = |uid: &str| match ctx(&hub, uid).handle(Request::ListInvites {}) {
            Ok(Event::Invites { invites, short_links, .. }) => {
                assert!(!short_links);
                invites.into_iter().map(|i| (i.code, i.created_by)).collect::<Vec<_>>()
            }
            other => panic!("{other:?}"),
        };
        let mine = codes(&a);
        assert!(mine.iter().all(|(_, by)| by == &a) && mine.len() == 3, "{mine:?}");
        let all = codes(&owner);
        assert_eq!(all.len(), 4, "{all:?}");
        assert!(!all.iter().any(|(c, _)| c.starts_with("owner-")));
        // Taken back: by its maker, or a manager; not by another member.
        assert!(forbidden(ctx(&hub, &b).handle(Request::RevokeInvite { code: first.clone() })));
        assert!(ctx(&hub, &a).handle(Request::RevokeInvite { code: first.clone() }).is_ok());
        assert!(hub.db.invite(&first).is_none());
        assert!(ctx(&hub, &a).handle(Request::RevokeInvite { code: first }).is_err(), "gone already");
        let bs = codes(&b)[0].0.clone();
        assert!(ctx(&hub, &owner).handle(Request::RevokeInvite { code: bs }).is_ok());
        assert!(ctx(&hub, &owner).handle(Request::RevokeInvite { code: "owner-secret".into() }).is_err());
        // Short links are the owner's to turn on (and can't be here: no directory).
        assert!(forbidden(ctx(&hub, &a).handle(Request::SetShortLinks { on: false })));
        assert!(ctx(&hub, &owner).handle(Request::SetShortLinks { on: true }).is_err());
        assert!(ctx(&hub, &owner).handle(Request::SetShortLinks { on: false }).is_ok());
        // Bots can't touch invites.
        let bot = uid();
        hub.db.insert_member(&bot, "Bot", Role::Member, true).unwrap();
        assert!(forbidden(ctx(&hub, &bot).handle(Request::ListInvites {})));
    }

    #[test]
    fn invites_and_bans_at_login() {
        let hub = hub();
        let ip: std::net::IpAddr = "203.0.113.5".parse().unwrap();
        let (a, b) = (uid(), uid());
        hub.db.create_invite("fivetimes1", "server", Some(5), None, Role::Member).unwrap();
        hub.authorize(&a, "A", Some("fivetimes1"), ip).unwrap();
        // Reconnecting with the same invite doesn't use it up.
        for _ in 0..10 {
            hub.authorize(&a, "A", Some("fivetimes1"), ip).unwrap();
        }
        hub.authorize(&b, "B", Some("fivetimes1"), ip).unwrap();
        // A ban sticks: joining again with an invite doesn't undo it. (The invite they joined
        // with is gone too: see a_ban_turns_off_the_invite_they_came_with.)
        hub.db.remove_user(&b, true).unwrap();
        assert_eq!(hub.db.invite_role("fivetimes1"), None);
        hub.db.create_invite("another1", "server", Some(5), None, Role::Member).unwrap();
        assert!(matches!(hub.authorize(&b, "B", Some("another1"), ip), Err((ErrorCode::Banned, _))));
        assert!(!hub.db.insert_user(&b, "B", Role::Member).unwrap());
        // Guessing invite codes gets throttled per place.
        for i in 0..BAD_INVITES {
            assert!(matches!(hub.authorize(&uid(), "X", Some(&format!("guess{i}")), ip), Err((ErrorCode::InviteInvalid, _))));
        }
        assert!(matches!(hub.authorize(&uid(), "X", Some("another1"), ip), Err((ErrorCode::RateLimited, _))));
        let other: std::net::IpAddr = "198.51.100.7".parse().unwrap();
        assert!(hub.authorize(&uid(), "Y", Some("another1"), other).is_ok());
        // An IPv6 /64 is one place: the other addresses in it don't start the count again.
        for i in 0..BAD_INVITES {
            let ip6: std::net::IpAddr = format!("2001:db8:1:2::{:x}", i + 1).parse().unwrap();
            assert!(matches!(hub.authorize(&uid(), "Z", Some(&format!("g6{i}")), ip6), Err((ErrorCode::InviteInvalid, _))));
        }
        let ip6: std::net::IpAddr = "2001:db8:1:2::ffff".parse().unwrap();
        assert!(matches!(hub.authorize(&uid(), "Z", Some("another1"), ip6), Err((ErrorCode::RateLimited, _))));
        let next_door: std::net::IpAddr = "2001:db8:1:3::1".parse().unwrap();
        assert!(hub.authorize(&uid(), "W", Some("another1"), next_door).is_ok());
    }

    const BAD_INVITES: u32 = 10;

    #[test]
    fn canonical_ids_only() {
        let id = uid();
        assert!(crypto::is_valid_user_id(&id));
        assert!(!crypto::is_valid_user_id(&format!("{id}=")));
        assert!(!crypto::is_valid_user_id(&format!(" {id}")));
    }

    #[test]
    fn decompression_bombs_are_refused() {
        // A GIF that says it's 60000x60000 (with one tiny frame): the canvas alone would be
        // gigabytes.
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&60000u16.to_le_bytes());
        gif.extend_from_slice(&60000u16.to_le_bytes());
        gif.extend_from_slice(&[0, 0, 0, 0x2c, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 0x4c, 1, 0, 0x3b]);
        assert!(!image_dimensions_ok(&gif));
        // Many big frames: too much in all.
        let mut anim = b"GIF89a".to_vec();
        anim.extend_from_slice(&4000u16.to_le_bytes());
        anim.extend_from_slice(&4000u16.to_le_bytes());
        anim.extend_from_slice(&[0, 0, 0]);
        for _ in 0..60 {
            anim.extend_from_slice(&[0x2c, 0, 0, 0, 0, 0xa0, 0x0f, 0xa0, 0x0f, 0, 2, 2, 0x4c, 1, 0]);
        }
        anim.push(0x3b);
        assert!(!image_dimensions_ok(&anim));
        // A normal small GIF is fine.
        let mut ok = b"GIF89a".to_vec();
        ok.extend_from_slice(&64u16.to_le_bytes());
        ok.extend_from_slice(&64u16.to_le_bytes());
        ok.extend_from_slice(&[0, 0, 0, 0x2c, 0, 0, 0, 0, 64, 0, 64, 0, 0, 2, 2, 0x4c, 1, 0, 0x3b]);
        assert!(image_dimensions_ok(&ok));
    }
}

/// One part of a backup (see [`pyr_proto::control::BACKUP_MANIFEST`]), for the owner only:
/// it's everything on the server, direct messages and the server's own key included.
async fn backup_item(hub: &Hub, uid: &str, item: &str, send: &mut quinn::SendStream) -> Result<()> {
    use pyr_proto::control::{BACKUP_CERT, BACKUP_DATABASE, BACKUP_FILE_PREFIX, BACKUP_KEY, BACKUP_MANIFEST, BackupManifest};
    let reply = |r: FileReply| framing::encode(&r);
    let fail = |msg: &str| reply(FileReply { ok: false, id: String::new(), mime: String::new(), size: 0, error: msg.to_string() });
    if hub.member(uid).map(|m| m.role) != Some(Role::Owner) {
        send.write_all(&fail("Only the server's owner can back it up.")).await?;
        let _ = send.finish();
        return Ok(());
    }
    let data_dir = hub.files_dir.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    let bytes: Result<Vec<u8>, String> = match item {
        BACKUP_MANIFEST => {
            let m = BackupManifest {
                version: env!("CARGO_PKG_VERSION").to_string(),
                made_at: pyr_proto::now_ms(),
                name: hub.info().name,
                files: hub.db.all_files().into_iter().filter(|(id, _)| hub.file_path(id).map(|p| p.is_file()).unwrap_or(false)).collect(),
            };
            serde_json::to_vec(&m).map_err(|e| e.to_string())
        }
        BACKUP_DATABASE => {
            let tag: String = pyr_proto::crypto::random_bytes::<8>().iter().map(|b| format!("{b:02x}")).collect();
            let tmp = data_dir.join(format!("backup-{tag}.db"));
            // Quick (it copies pages), and nothing may write meanwhile anyway.
            let r = hub.db.snapshot(&tmp).and_then(|()| Ok(std::fs::read(&tmp)?));
            crate::hub::wipe_file(&tmp);
            r.map_err(|e| format!("{e:#}"))
        }
        BACKUP_CERT | BACKUP_KEY => tokio::fs::read(data_dir.join(item)).await.map_err(|e| e.to_string()),
        other => match other.strip_prefix(BACKUP_FILE_PREFIX).and_then(|id| hub.file_path(id)) {
            Some(path) => tokio::fs::read(&path).await.map_err(|_| "That file is gone.".to_string()),
            None => Err("Unknown backup item.".to_string()),
        },
    };
    match bytes {
        Ok(b) => {
            send.write_all(&reply(FileReply { ok: true, id: item.to_string(), mime: String::new(), size: b.len() as u64, error: String::new() }))
                .await?;
            send.write_all(&b).await?;
        }
        Err(e) => {
            tracing::warn!("backup of {item} failed: {e}");
            send.write_all(&fail(&format!("The server couldn't read its {item}: {e}"))).await?;
        }
    }
    let _ = send.finish();
    Ok(())

}
