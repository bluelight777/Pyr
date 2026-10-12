//! One client connection: login handshake, control requests, and media ingest.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use bytes::{Bytes, BytesMut};
use ingle_proto::control::*;
use ingle_proto::media::{self, AudioHeader, VideoHeader, track};
use ingle_proto::{MAX_MESSAGE_CHARS, MAX_VIDEO_FRAME, PROTOCOL_VERSION, crypto, framing};
use tokio::sync::mpsc;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::db::new_invite_code;
use crate::hub::{ConnId, Hub, Reply, frame};

pub async fn handle(hub: Arc<Hub>, incoming: quinn::Incoming) {
    let remote = incoming.remote_address();
    let conn = match incoming.await {
        Ok(c) => c,
        Err(e) => {
            debug!(%remote, "handshake failed: {e}");
            return;
        }
    };
    match session(hub, conn.clone()).await {
        Ok(()) => debug!(%remote, "connection closed"),
        Err(e) => debug!(%remote, "connection ended: {e:#}"),
    }
    conn.close(quinn::VarInt::from_u32(0), b"bye");
}

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

async fn session(hub: Arc<Hub>, conn: quinn::Connection) -> Result<()> {
    let (mut send, mut recv) = timeout(Duration::from_secs(10), conn.accept_bi()).await.context("no control stream")??;
    let mut preamble = [0u8; 4];
    timeout(Duration::from_secs(10), recv.read_exact(&mut preamble)).await.context("no preamble")??;
    if &preamble != ingle_proto::CONTROL_PREAMBLE {
        bail!("not an Ingle client");
    }

    let nonce = crypto::random_bytes::<32>();
    let hello = Event::Hello { proto: PROTOCOL_VERSION, server_name: hub.info().name, nonce: nonce.to_vec() };
    send.write_all(&frame(0, hello)).await?;

    let login: ClientFrame =
        timeout(Duration::from_secs(20), read_frame(&mut recv)).await.context("login timeout")??.ok_or_else(|| anyhow!("closed"))?;
    let reject = |code: ErrorCode, msg: &str| frame(login.id, Event::error(code, msg));
    let Request::Login { proto, user_id, sig, name, invite, client } = login.req else {
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
    if !crypto::verify(&user_id, &crypto::login_transcript(&nonce, &exporter, &user_id), &sig) {
        send.write_all(&reject(ErrorCode::Unauthorized, "identity proof failed")).await?;
        bail!("bad login signature");
    }
    let name = clean_name(&name, 32).unwrap_or_else(|| "Anonymous".to_string());
    if let Err((code, msg)) = hub.authorize(&user_id, &name, invite.as_deref()) {
        send.write_all(&reject(code, &msg)).await?;
        let _ = send.finish();
        tokio::time::sleep(Duration::from_millis(300)).await;
        bail!("login refused: {msg}");
    }
    info!(user = %&user_id[..8], %name, %client, remote = %conn.remote_address(), "logged in");

    let (tx, mut rx) = mpsc::unbounded_channel::<Bytes>();
    let writer = tokio::spawn(async move {
        while let Some(b) = rx.recv().await {
            if send.write_all(&b).await.is_err() {
                break;
            }
        }
        let _ = send.finish();
    });

    let conn_id = hub.alloc_conn_id();
    let session_cell = Arc::new(AtomicU32::new(0));
    hub.register(conn_id, &user_id, conn.clone(), tx.clone(), session_cell.clone(), login.id);

    let dg = tokio::spawn(datagram_loop(hub.clone(), conn.clone(), session_cell.clone()));
    let uni = tokio::spawn(uni_loop(hub.clone(), conn.clone(), session_cell.clone()));

    let mut ctx = Ctx { hub: &hub, conn_id, uid: user_id.clone(), msg_tokens: 10.0, last_refill: Instant::now() };
    let result = async {
        loop {
            let Some(f) = read_frame::<ClientFrame>(&mut recv).await? else { return Ok(()) };
            let id = f.id;
            let reply = ctx.handle(f.req);
            let ev = reply.unwrap_or_else(|(code, message)| Event::Error { code, message });
            if id != 0 {
                let _ = tx.send(frame(id, ev));
            }
        }
    }
    .await;

    hub.unregister(conn_id);
    dg.abort();
    uni.abort();
    drop(tx);
    let _ = timeout(Duration::from_secs(1), writer).await;
    result
}

/// Audio datagrams: rewrite the session id and fan out.
async fn datagram_loop(hub: Arc<Hub>, conn: quinn::Connection, session: Arc<AtomicU32>) {
    let mut targets = Vec::new();
    while let Ok(d) = conn.read_datagram().await {
        let s = session.load(Ordering::Relaxed);
        if s == 0 {
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

/// Video frames: one unidirectional stream per frame.
async fn uni_loop(hub: Arc<Hub>, conn: quinn::Connection, session: Arc<AtomicU32>) {
    while let Ok(mut stream) = conn.accept_uni().await {
        let hub = hub.clone();
        let session = session.clone();
        tokio::spawn(async move {
            let Ok(Ok(data)) =
                timeout(Duration::from_secs(5), stream.read_to_end(MAX_VIDEO_FRAME + media::VIDEO_HEADER_LEN)).await
            else {
                return;
            };
            let s = session.load(Ordering::Relaxed);
            if s == 0 {
                return;
            }
            let Some(h) = VideoHeader::parse(&data) else { return };
            if data.len() != media::VIDEO_HEADER_LEN + h.len as usize {
                return;
            }
            let mut b = BytesMut::from(&data[..]);
            media::set_session(&mut b, s);
            hub.forward_video(s, h.track, h.is_keyframe(), b.freeze());
        });
    }
}

struct Ctx<'a> {
    hub: &'a Hub,
    conn_id: ConnId,
    uid: String,
    msg_tokens: f64,
    last_refill: Instant,
}

fn bad<T>(msg: &str) -> Result<T, (ErrorCode, String)> {
    Err((ErrorCode::BadRequest, msg.to_string()))
}

fn clean_content(raw: &str) -> Option<String> {
    let s = raw.trim_matches(|c: char| c.is_whitespace());
    if s.is_empty() || s.chars().count() > MAX_MESSAGE_CHARS {
        return None;
    }
    // Strip control characters except newlines and tabs.
    Some(s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect())
}

impl Ctx<'_> {
    fn role(&self) -> Role {
        self.hub.db.get_user(&self.uid).map(|u| u.role).unwrap_or(Role::Member)
    }

    fn require_admin(&self) -> Result<(), (ErrorCode, String)> {
        if self.role().is_admin() { Ok(()) } else { Err((ErrorCode::Forbidden, "Only admins can do that.".into())) }
    }

    fn take_message_token(&mut self) -> Result<(), (ErrorCode, String)> {
        let now = Instant::now();
        let dt = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;
        self.msg_tokens = (self.msg_tokens + dt * 1.5).min(10.0);
        if self.msg_tokens < 1.0 {
            return Err((ErrorCode::RateLimited, "You're sending messages too fast.".into()));
        }
        self.msg_tokens -= 1.0;
        Ok(())
    }

    fn handle(&mut self, req: Request) -> Reply {
        let hub = self.hub;
        let db = &hub.db;
        match req {
            Request::Login { .. } => bad("already logged in"),
            Request::Ping { t } => Ok(Event::Pong { t }),

            Request::SendMessage { channel_id, content, reply_to, nonce } => {
                self.take_message_token()?;
                let Some(content) = clean_content(&content) else { return bad("message is empty or too long") };
                if db.channel(channel_id).is_none() {
                    return Err((ErrorCode::NotFound, "No such channel.".into()));
                }
                let reply_to = reply_to.filter(|id| db.get_message(*id).map(|m| m.channel_id == channel_id).unwrap_or(false));
                let message = db
                    .insert_message(channel_id, &self.uid, &content, reply_to)
                    .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::MessageCreated { message, nonce });
                Ok(Event::Ok)
            }
            Request::EditMessage { message_id, content } => {
                let Some(content) = clean_content(&content) else { return bad("message is empty or too long") };
                let Some(m) = db.get_message(message_id) else { return Err((ErrorCode::NotFound, "gone".into())) };
                if m.author != self.uid {
                    return Err((ErrorCode::Forbidden, "You can only edit your own messages.".into()));
                }
                if let Ok(Some(message)) = db.edit_message(message_id, &content) {
                    hub.broadcast(Event::MessageUpdated { message });
                }
                Ok(Event::Ok)
            }
            Request::DeleteMessage { message_id } => {
                let Some(m) = db.get_message(message_id) else { return Ok(Event::Ok) };
                if m.author != self.uid && !self.role().is_admin() {
                    return Err((ErrorCode::Forbidden, "You can't delete that message.".into()));
                }
                db.delete_message(message_id).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::MessageDeleted { channel_id: m.channel_id, message_id });
                Ok(Event::Ok)
            }
            Request::React { message_id, emoji, add } => {
                let emoji = emoji.trim().to_string();
                if emoji.is_empty() || emoji.len() > 32 || emoji.chars().any(|c| c.is_ascii_alphanumeric() || c.is_whitespace())
                {
                    return bad("not an emoji");
                }
                if db.get_message(message_id).is_none() {
                    return Err((ErrorCode::NotFound, "gone".into()));
                }
                if add && db.distinct_reactions(message_id) >= 20 {
                    return bad("too many different reactions");
                }
                db.set_reaction(message_id, &emoji, &self.uid, add).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(message) = db.get_message(message_id) {
                    hub.broadcast(Event::MessageUpdated { message });
                }
                Ok(Event::Ok)
            }
            Request::FetchHistory { channel_id, before, limit } => {
                if db.channel(channel_id).is_none() {
                    return Err((ErrorCode::NotFound, "No such channel.".into()));
                }
                let (messages, complete) = db.history(channel_id, before, limit);
                Ok(Event::History { channel_id, messages, complete })
            }
            Request::Typing { channel_id } => hub.typing(self.conn_id, &self.uid, channel_id),

            Request::CreateChannel { name, kind } => {
                self.require_admin()?;
                let Some(name) = clean_channel_name(&name, kind) else { return bad("channel name is empty") };
                let channel = db.create_channel(&name, kind).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                hub.broadcast(Event::ChannelUpserted { channel });
                Ok(Event::Ok)
            }
            Request::UpdateChannel { channel_id, name, topic } => {
                self.require_admin()?;
                let Some(ch) = db.channel(channel_id) else { return Err((ErrorCode::NotFound, "No such channel.".into())) };
                let name = match name {
                    Some(n) => Some(clean_channel_name(&n, ch.kind).ok_or((ErrorCode::BadRequest, "empty name".into()))?),
                    None => None,
                };
                let topic = topic.map(|t| t.chars().filter(|c| !c.is_control()).take(300).collect::<String>());
                if let Ok(Some(channel)) = db.update_channel(channel_id, name.as_deref(), topic.as_deref()) {
                    hub.broadcast(Event::ChannelUpserted { channel });
                }
                Ok(Event::Ok)
            }
            Request::DeleteChannel { channel_id } => {
                self.require_admin()?;
                hub.evict_channel(channel_id);
                if db.delete_channel(channel_id).map_err(|e| (ErrorCode::Internal, e.to_string()))? {
                    hub.broadcast(Event::ChannelDeleted { channel_id });
                }
                Ok(Event::Ok)
            }
            Request::CreateInvite { max_uses, expires_secs } => {
                let code = new_invite_code(8);
                let expires_at = expires_secs.map(|s| ingle_proto::now_ms() + (s.min(365 * 86400) as i64) * 1000);
                db.create_invite(&code, &self.uid, max_uses, expires_at, Role::Member)
                    .map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                Ok(Event::InviteCreated { link: hub.invite_link(&code), code })
            }
            Request::SetRole { user_id, role } => {
                if self.role() != Role::Owner {
                    return Err((ErrorCode::Forbidden, "Only the owner can change roles.".into()));
                }
                if user_id == self.uid || role == Role::Owner {
                    return bad("ownership can't be changed here");
                }
                match db.get_user(&user_id) {
                    Some(u) if !u.banned && u.role != Role::Owner => {}
                    _ => return Err((ErrorCode::NotFound, "No such member.".into())),
                }
                db.set_role(&user_id, role).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(member) = hub.member(&user_id) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                Ok(Event::Ok)
            }
            Request::Kick { user_id } | Request::Ban { user_id } if user_id == self.uid => bad("you can't remove yourself"),
            Request::Kick { user_id } => self.remove(user_id, false),
            Request::Ban { user_id } => self.remove(user_id, true),
            Request::UpdateProfile { name } => {
                let Some(name) = clean_name(&name, 32) else { return bad("name is empty") };
                db.touch_user(&self.uid, &name).map_err(|e| (ErrorCode::Internal, e.to_string()))?;
                if let Some(member) = hub.member(&self.uid) {
                    hub.broadcast(Event::MemberUpserted { member });
                }
                Ok(Event::Ok)
            }
            Request::UpdateServer { name, description } => {
                self.require_admin()?;
                if let Some(n) = name.and_then(|n| clean_name(&n, 48)) {
                    let _ = db.set_meta("name", &n);
                }
                if let Some(d) = description {
                    let d: String = d.chars().filter(|c| !c.is_control()).take(300).collect();
                    let _ = db.set_meta("description", &d);
                }
                hub.broadcast(Event::ServerUpdated { server: hub.info() });
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
        }
    }

    fn remove(&self, user_id: UserId, ban: bool) -> Reply {
        let my_role = self.role();
        if !my_role.is_admin() {
            return Err((ErrorCode::Forbidden, "Only admins can remove members.".into()));
        }
        let target_role = self.hub.db.get_user(&user_id).map(|u| u.role).unwrap_or(Role::Member);
        if target_role >= my_role {
            return Err((ErrorCode::Forbidden, "You can't remove someone with the same or a higher role.".into()));
        }
        if let Err(e) = self.hub.db.remove_user(&user_id, ban) {
            warn!("remove_user failed: {e}");
            return Err((ErrorCode::Internal, e.to_string()));
        }
        let reason = if ban { "You were banned from this server." } else { "You were kicked from this server." };
        self.hub.eject(&user_id, reason);
        Ok(Event::Ok)
    }
}
