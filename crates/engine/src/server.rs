//! One task per joined server: connect, log in, keep the shared state in sync, reconnect.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pyr_proto::control::*;
use pyr_proto::framing;
use parking_lot::Mutex;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};

use crate::Shared;
use crate::call::CallEvent;
use crate::config::ServerEntry;
use crate::net::{self, Fatal};
use crate::state::{AppState, ConnStatus, IncomingCall, RingState, ServerView};

enum Outgoing {
    Req { req: Request, reply: Option<oneshot::Sender<Event>> },
    Close,
}

#[derive(Clone)]
pub struct ServerHandle {
    pub key: String,
    out: mpsc::UnboundedSender<Outgoing>,
    conn: Arc<Mutex<Option<quinn::Connection>>>,
}

impl ServerHandle {
    /// Send a request and get its reply (an `Event::Error` if not connected).
    pub fn request(&self, req: Request) -> oneshot::Receiver<Event> {
        let (tx, rx) = oneshot::channel();
        if let Err(mpsc::error::SendError(Outgoing::Req { reply: Some(tx), .. })) =
            self.out.send(Outgoing::Req { req, reply: Some(tx) })
        {
            let _ = tx.send(Event::error(ErrorCode::Internal, "not connected"));
        }
        rx
    }

    /// Fire-and-forget request.
    pub fn notify(&self, req: Request) {
        let _ = self.out.send(Outgoing::Req { req, reply: None });
    }

    pub fn connection(&self) -> Option<quinn::Connection> {
        self.conn.lock().clone()
    }

    pub fn close(&self) {
        let _ = self.out.send(Outgoing::Close);
    }
}

pub fn spawn(shared: Arc<Shared>, entry: ServerEntry) -> ServerHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    let conn = Arc::new(Mutex::new(None));
    let handle = ServerHandle { key: entry.key.clone(), out: tx, conn: conn.clone() };
    shared.rt.spawn(run(shared.clone(), entry, rx, conn));
    handle
}

fn set_status(shared: &Shared, key: &str, status: ConnStatus) {
    if let Some(s) = shared.state.write().server_mut(key) {
        s.status = status;
    }
    (shared.repaint)();
}

async fn run(
    shared: Arc<Shared>,
    entry: ServerEntry,
    mut out_rx: mpsc::UnboundedReceiver<Outgoing>,
    conn_slot: Arc<Mutex<Option<quinn::Connection>>>,
) {
    let key = entry.key.clone();
    let Some(fp) = pyr_proto::b64::decode_array::<32>(&key) else {
        set_status(&shared, &key, ConnStatus::Failed("invalid server fingerprint".into()));
        return;
    };
    let mut attempt: u32 = 0;
    loop {
        let name = shared.config.lock().display_name.clone();
        let invite = shared.config.lock().servers.iter().find(|s| s.key == key).and_then(|s| s.invite.clone());
        let result = net::connect_and_login(
            &shared.endpoints,
            &entry.host,
            entry.port,
            fp,
            &shared.identity,
            &name,
            invite.as_deref(),
        )
        .await;
        match result {
            Ok(li) => {
                attempt = 0;
                info!(server = %entry.host, "connected");
                *conn_slot.lock() = Some(li.conn.clone());
                let end = session(&shared, &key, li, &mut out_rx).await;
                *conn_slot.lock() = None;
                shared.server_disconnected(&key);
                match end {
                    SessionEnd::Closed => return,
                    SessionEnd::Kicked(reason) => {
                        set_status(&shared, &key, ConnStatus::Failed(reason));
                        return;
                    }
                    SessionEnd::Lost(e) => {
                        warn!(server = %entry.host, "connection lost: {e}");
                        set_status(&shared, &key, ConnStatus::Reconnecting { attempt: 1, error: e });
                    }
                }
            }
            Err(e) => {
                if let Some(f) = e.downcast_ref::<Fatal>() {
                    set_status(&shared, &key, ConnStatus::Failed(f.0.clone()));
                    // Wait for a retry/close request, failing anything sent meanwhile.
                    while let Some(o) = out_rx.recv().await {
                        match o {
                            Outgoing::Close => return,
                            Outgoing::Req { reply, .. } => {
                                if let Some(r) = reply {
                                    let _ = r.send(Event::error(ErrorCode::Internal, f.0.clone()));
                                }
                            }
                        }
                    }
                    return;
                }
                attempt += 1;
                debug!("connect attempt {attempt} failed: {e:#}");
                set_status(&shared, &key, ConnStatus::Reconnecting { attempt, error: format!("{e:#}") });
            }
        }
        // Quick retries first (a server restarting for an update is back in a few seconds),
        // then slower ones.
        let delay = match attempt {
            0 => Duration::from_millis(500),
            1..=10 => Duration::from_secs(1),
            n => Duration::from_secs((2 * (n as u64 - 10)).min(30)),
        };
        let wake = tokio::time::Instant::now() + delay;
        loop {
            tokio::select! {
                _ = tokio::time::sleep_until(wake) => break,
                o = out_rx.recv() => match o {
                    None | Some(Outgoing::Close) => return,
                    Some(Outgoing::Req { reply, .. }) => {
                        if let Some(r) = reply {
                            let _ = r.send(Event::error(ErrorCode::Internal, "not connected right now"));
                        }
                    }
                },
            }
        }
    }
}

enum SessionEnd {
    Closed,
    Kicked(String),
    Lost(String),
}

async fn session(
    shared: &Arc<Shared>,
    key: &str,
    li: net::LoggedIn,
    out_rx: &mut mpsc::UnboundedReceiver<Outgoing>,
) -> SessionEnd {
    let net::LoggedIn { conn, mut send, mut recv, ready } = li;
    apply_ready(shared, key, ready);
    // (A new connection starts out not idle: if I am, it hears so now.)
    crate::idle::wake(shared);
    crate::verify::publish_vouches(shared, key);
    crate::verify::fetch_vouches(shared, key);
    crate::verify::fetch_vouches_for_me(shared, key);
    crate::sync_avatar(shared.clone(), key.to_string());
    crate::resume_voice(shared.clone(), key.to_string());
    crate::backup::spawn_schedule(shared.clone(), key.to_string(), conn.clone());
    crate::sync_blocks(shared, key);
    {
        // Membership is established; the invite is no longer needed (and isn't sent again, so
        // it isn't used up by reconnecting).
        let mut cfg = shared.config.lock();
        if let Some(e) = cfg.servers.iter_mut().find(|s| s.key == key) {
            e.invite = None;
            if let Some(name) = shared.state.read_recursive().server(key).map(|s| s.info.name.clone()) {
                e.name = name;
            }
        }
        shared.save_config_locked(&cfg);
    }
    // Reader task: never cancel a read mid-frame.
    let (frames_tx, mut frames_rx) = mpsc::unbounded_channel::<ServerFrame>();
    let reader = tokio::spawn(async move {
        loop {
            match net::read_frame::<ServerFrame>(&mut recv).await {
                Ok(Some(f)) => {
                    if frames_tx.send(f).is_err() {
                        break;
                    }
                }
                _ => break,
            }
        }
    });
    let mut pending: HashMap<u64, oneshot::Sender<Event>> = HashMap::new();
    let mut next_id: u64 = 2;
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    let end = loop {
        tokio::select! {
            f = frames_rx.recv() => {
                let Some(f) = f else {
                    let reason = conn.close_reason().map(|r| r.to_string()).unwrap_or_else(|| "connection closed".into());
                    break SessionEnd::Lost(reason);
                };
                if let Event::Kicked { reason } = &f.ev {
                    let reason = reason.clone();
                    shared.state.write().toast(reason.clone(), true);
                    break SessionEnd::Kicked(reason);
                }
                if f.re != 0 {
                    if let Some(tx) = pending.remove(&f.re) {
                        let _ = tx.send(f.ev);
                        continue;
                    }
                }
                apply_event(shared, key, f.ev);
            }
            o = out_rx.recv() => match o {
                None | Some(Outgoing::Close) => {
                    conn.close(quinn::VarInt::from_u32(0), b"bye");
                    break SessionEnd::Closed;
                }
                Some(Outgoing::Req { req, reply }) => {
                    let id = if reply.is_some() { next_id += 1; next_id } else { 0 };
                    let bytes = framing::encode(&ClientFrame { id, req });
                    if let Err(e) = send.write_all(&bytes).await {
                        if let Some(r) = reply { let _ = r.send(Event::error(ErrorCode::Internal, "connection lost")); }
                        break SessionEnd::Lost(e.to_string());
                    }
                    if let Some(r) = reply { pending.insert(id, r); }
                }
            },
            _ = tick.tick() => {
                let rtt = conn.rtt().as_millis() as u32;
                let mut st = shared.state.write();
                if let Some(s) = st.server_mut(key) {
                    s.rtt_ms = rtt;
                    // expire typing indicators
                    let now = Instant::now();
                    for m in s.typing.values_mut() {
                        m.retain(|_, t| now.duration_since(*t) < Duration::from_secs(8));
                    }
                }
                drop(st);
                (shared.repaint)();
            }
        }
    };
    reader.abort();
    for (_, tx) in pending.drain() {
        let _ = tx.send(Event::error(ErrorCode::Internal, "connection lost"));
    }
    end
}

fn sort_channels(v: &mut [Channel]) {
    v.sort_by_key(|c| (matches!(c.kind, ChannelKind::Voice), c.position, c.id));
}

fn apply_ready(shared: &Shared, key: &str, ready: Event) {
    let Event::Ready { me, server, mut channels, members, voice, mut roles, features, dms, relations, groups } = ready else { return };
    let last_read = shared.config.lock().last_read.get(key).cloned().unwrap_or_default();
    roles.sort_by_key(|r| (std::cmp::Reverse(r.position), r.id));
    sort_channels(&mut channels);
    let mut st = shared.state.write();
    let my_uid = st.my_uid.clone();
    let server_muted = st.muted_servers.contains(key);
    let muted_channels: std::collections::HashSet<ChannelId> = st.muted_channels.iter().filter(|(k, _)| k == key).map(|(_, c)| *c).collect();
    let blocked = st.blocked.clone();
    let s = match st.server_mut(key) {
        Some(s) => s,
        None => {
            st.servers.push(ServerView::new(key.to_string(), server.name.clone(), String::new()));
            st.servers.last_mut().expect("just pushed")
        }
    };
    // Channels: what came while you were away (or since you last read them here) shows as
    // unread, with the "NEW" line after the last message read. A channel this PC has no mark
    // for yet starts out read (marks are kept from the first unread message on, see
    // `mark_read_before`). Muted ones stay quiet, as they do while you're here.
    // (Servers before 0.6.4 don't say what's newest: nothing to go on.)
    let last_read_known = features.iter().any(|f| f == CHANNEL_NEWEST_FEATURE);
    let mut marks: Vec<(ChannelId, MessageId)> = Vec::new();
    for c in channels.iter().filter(|_| last_read_known) {
        let seen = last_read.get(&c.id).copied();
        let Some(last) = c.last_message else {
            // Nothing in it yet: whatever comes is new.
            if seen.is_none() {
                marks.push((c.id, 0));
            }
            continue;
        };
        let mine = c.last_from.as_deref() == Some(my_uid.as_str());
        match seen {
            Some(seen) if seen >= last => {}
            Some(_) if mine => marks.push((c.id, last)),
            None => marks.push((c.id, last)),
            Some(seen) => {
                let quiet = server_muted || muted_channels.contains(&c.id) || c.last_from.as_ref().is_some_and(|f| blocked.contains(f));
                if !quiet {
                    if s.unread.insert(c.id) {
                        s.new_since.insert(c.id, seen + 1);
                    } else {
                        s.new_since.entry(c.id).or_insert(seen + 1);
                    }
                }
            }
        }
    }
    s.status = ConnStatus::Connected;
    s.name = server.name.clone();
    s.info = server;
    s.me = Some(me);
    s.channels = channels;
    s.members = members.into_iter().map(|m| (m.user_id.clone(), m)).collect();
    s.voice = voice.into_iter().map(|p| (p.session, p)).collect();
    s.roles = roles;
    s.features = features;
    s.idle_sent = false;
    s.dnd_sent = false;
    s.relations = relations.into_iter().map(|r| (r.user_id, r.kind)).collect();
    // Direct messages that came in while we were away show as unread, with the "NEW" line
    // after the last one read here (or on the newest, if this PC never opened it).
    s.dms.clear();
    for dm in dms {
        if let Some(last) = dm.last_message.filter(|_| dm.last_from.as_deref() != Some(my_uid.as_str())) {
            let seen = last_read.get(&dm.id).copied();
            if seen.map(|seen| seen < last).unwrap_or(true) {
                let since = seen.map(|r| r + 1).unwrap_or(last);
                if s.unread.insert(dm.id) {
                    s.new_since.insert(dm.id, since);
                } else {
                    s.new_since.entry(dm.id).or_insert(since);
                }
                s.mentions.entry(dm.id).or_insert(1);
            }
        }
        s.upsert_dm(dm);
    }
    // Groups the same way.
    s.groups.clear();
    for g in groups {
        if let Some(last) = g.last_message.filter(|_| g.last_from.as_deref() != Some(my_uid.as_str())) {
            let seen = last_read.get(&g.id).copied();
            if seen.map(|seen| seen < last).unwrap_or(true) {
                let since = seen.map(|r| r + 1).unwrap_or(last);
                if s.unread.insert(g.id) {
                    s.new_since.insert(g.id, since);
                } else {
                    s.new_since.entry(g.id).or_insert(since);
                }
                s.mentions.entry(g.id).or_insert(1);
            }
        }
        s.upsert_group(g);
    }
    // Gone while we were away (a channel deleted, a group left on another device): nothing of
    // it is kept (no unread mark, no badge).
    let known: std::collections::HashSet<ChannelId> =
        s.channels.iter().map(|c| c.id).chain(s.dms.iter().map(|d| d.id)).chain(s.groups.iter().map(|g| g.id)).collect();
    s.unread.retain(|id| known.contains(id));
    s.mentions.retain(|id, _| known.contains(id));
    s.new_since.retain(|id, _| known.contains(id));
    s.logs.retain(|id, _| known.contains(id));
    // History may have changed while we were away; reload lazily.
    for log in s.logs.values_mut() {
        log.loaded = false;
    }
    drop(st);
    for (c, id) in marks {
        shared.mark_read(key, c, id, false);
    }
    (shared.repaint)();
}

fn log_newest(s: &ServerView, cid: ChannelId) -> Option<MessageId> {
    s.logs.get(&cid).and_then(|l| l.messages.last().map(|m| m.id))
}


fn apply_event(shared: &Arc<Shared>, key: &str, ev: Event) {
    // Someone's Pyr making sure of me (or answering me making sure of them): apart from the rest.
    let ev = match ev {
        Event::Handshake { from, data } => return crate::verify::on_handshake(shared, key, &from, &data),
        ev => ev,
    };
    let active = shared.active_view.lock().clone();
    let mut forward: Option<CallEvent> = None;
    let mut toast: Option<String> = None;
    let mut dm_note: Option<String> = None;
    // A friend request (the notice, and from whom), or one that was answered or taken back.
    let mut friend_note: Option<(String, UserId)> = None;
    let mut friend_done: Option<UserId> = None;
    let mut ding = false;
    // Read up to here (channel, message, save now): what came while you were looking at it.
    let mut read_upto: Option<(ChannelId, MessageId, bool)> = None;
    // A channel's first unread message (see `mark_read_before`).
    let mut unread_from: Option<(ChannelId, MessageId)> = None;
    // A soundboard sound someone played: (session, channel, sound, name, emoji, volume).
    let mut sound: Option<(SessionId, ChannelId, String, String, String, f32)> = None;
    // Calls in direct messages: someone ringing (channel, from, video), someone declining
    // (channel, by), and someone coming into or leaving one (channel, who, came in).
    let mut ringing: Option<(ChannelId, UserId, bool, Option<ChannelId>)> = None;
    let mut declined: Option<(ChannelId, UserId)> = None;
    let mut dm_voice: Option<(ChannelId, UserId, bool)> = None;
    let mut ring = false;
    // The call we're in moved to this conversation: join it there.
    let mut follow: Option<ChannelId> = None;
    // A group we're no longer in (left here or on another device), and whether we're in its
    // call here (and so leave it).
    let mut gone_group: Option<ChannelId> = None;
    let mut leave_call = false;
    {
        let mut st = shared.state.write();
        let my_uid = st.my_uid.clone();
        let server_muted = st.muted_servers.contains(key);
        let muted_channels: std::collections::HashSet<ChannelId> =
            st.muted_channels.iter().filter(|(k, _)| k == key).map(|(_, c)| *c).collect();
        let blocked = st.blocked.clone();
        let Some(s) = st.server_mut(key) else { return };
        match ev {
            Event::RelationUpdated { user_id, kind } => {
                let before = s.relations.get(&user_id).copied();
                match kind {
                    Some(k) => s.relations.insert(user_id.clone(), k),
                    None => s.relations.remove(&user_id),
                };
                let who = s.member_name(&user_id);
                // (News, not errors: they show as notices, never with Report on them. A request
                // stays, with Accept and Ignore, until it's answered.)
                if kind == Some(RelationKind::Incoming) && before != Some(RelationKind::Incoming) && !blocked.contains(&user_id) {
                    friend_note = Some((format!("{who} sent you a friend request."), user_id.clone()));
                    ding = true;
                }
                if before == Some(RelationKind::Incoming) && kind != Some(RelationKind::Incoming) {
                    friend_done = Some(user_id.clone());
                }
                if kind == Some(RelationKind::Friend) && before == Some(RelationKind::Outgoing) {
                    dm_note = Some(format!("{who} accepted your friend request."));
                }
            }
            Event::MessageCreated { message, nonce } if blocked.contains(&message.author) => {
                // From someone I blocked: kept (shown folded away), but no ping or unread.
                let cid = message.channel_id;
                let _ = nonce;
                if let Some(log) = s.logs.get_mut(&cid) {
                    if log.loaded {
                        log.upsert(message);
                    }
                }
            }
            Event::MessageCreated { message, nonce } => {
                let cid = message.channel_id;
                let mention = message.author != my_uid && s.pings_me(&message.author, &message.content);
                let log = s.logs.entry(cid).or_default();
                if !nonce.is_empty() {
                    log.pending.retain(|p| p.nonce != nonce);
                }
                let mine = message.author == my_uid;
                let mid = message.id;
                log.upsert(message);
                s.typing.entry(cid).or_default().retain(|u, _| !mine || u != &my_uid);
                let viewing = active.as_ref().map(|(k, c)| k == key && *c == cid).unwrap_or(false);
                // Open in front of you. Open in a window that's behind another one, minimized or
                // hidden to the tray doesn't count: that's unread until you come back.
                let looking = viewing && shared.window_in_front();
                // A direct message, or a group: like a direct message.
                let in_dm = s.is_private(cid);
                if s.dm(cid).is_some() {
                    let (with, newest) = (s.dm(cid).map(|d| d.with.clone()).unwrap_or_default(), log_newest(s, cid));
                    s.upsert_dm(Dm { id: cid, with, last_message: newest, last_from: None });
                } else if let Some(mut g) = s.group(cid).cloned() {
                    g.last_message = log_newest(s, cid);
                    s.upsert_group(g);
                }
                if mine {
                    // You've written here, so you've read it: no "NEW" line above your reply.
                    s.new_since.remove(&cid);
                    read_upto = Some((cid, mid, in_dm));
                } else if looking {
                    read_upto = Some((cid, mid, in_dm));
                }
                // A ping for every new message you can see, unless the channel (or server) is
                // muted or you're looking right at it. Mentions and direct messages ping even
                // in a muted server, like Discord.
                let muted = server_muted && !mention && !in_dm || muted_channels.contains(&cid);
                if !mine && !looking && !muted {
                    ding = true;
                }
                if !mine && !looking {
                    if !muted || mention {
                        // The "NEW" line goes above the first message you haven't seen: this one,
                        // unless older ones are still unread.
                        if s.unread.insert(cid) {
                            s.new_since.insert(cid, mid);
                            if !in_dm {
                                unread_from = Some((cid, mid));
                            }
                        } else {
                            s.new_since.entry(cid).or_insert(mid);
                        }
                    }
                    // Every direct message counts like a mention (the red badge).
                    if mention || in_dm {
                        *s.mentions.entry(cid).or_default() += 1;
                    }
                    if mention && !in_dm {
                        if let Some(m) = s.logs.get(&cid).and_then(|l| l.messages.last()) {
                            let who = s.member_name(&m.author);
                            let channel = s.channel(cid).map(|c| c.name.clone()).unwrap_or_default();
                            let text: String = m.content.chars().take(80).collect();
                            dm_note = Some(format!("{who} mentioned you in #{channel}: {text}"));
                        }
                    }
                    if in_dm {
                        if let Some(m) = s.logs.get(&cid).and_then(|l| l.messages.last()) {
                            let who = s.member_name(&m.author);
                            let text = if m.content.is_empty() { "sent a picture".to_string() } else { m.content.clone() };
                            let text: String = text.chars().take(80).collect();
                            // (In a group: which one.)
                            dm_note = Some(match s.group(cid) {
                                Some(g) => format!("{who} in {}: {text}", s.group_name(g, &my_uid)),
                                None => format!("{who}: {text}"),
                            });
                        }
                    }
                }
            }
            Event::MessageUpdated { message } => {
                if let Some(log) = s.logs.get_mut(&message.channel_id) {
                    if log.find(message.id).is_some() {
                        log.upsert(message);
                    }
                }
            }
            Event::MessageDeleted { channel_id, message_id } => {
                if let Some(log) = s.logs.get_mut(&channel_id) {
                    // Drop its images from memory too.
                    if let Some(m) = log.find(message_id) {
                        let mut files = shared.files.lock();
                        for a in &m.attachments {
                            files.forget(&a.id);
                        }
                    }
                    log.remove(message_id);
                }
            }
            Event::Typing { channel_id, user_id } => {
                if user_id != my_uid {
                    s.typing.entry(channel_id).or_default().insert(user_id, Instant::now());
                }
            }
            Event::ChannelUpserted { channel } => {
                match s.channels.iter_mut().find(|c| c.id == channel.id) {
                    Some(c) => *c = channel,
                    None => s.channels.push(channel),
                }
                sort_channels(&mut s.channels);
            }
            // Deleted, or I can't see it any more (a private channel I lost the role for).
            Event::ChannelDeleted { channel_id } => {
                s.channels.retain(|c| c.id != channel_id);
                s.logs.remove(&channel_id);
                s.unread.remove(&channel_id);
                s.mentions.remove(&channel_id);
                s.new_since.remove(&channel_id);
                s.typing.remove(&channel_id);
            }
            Event::MemberUpserted { member } => {
                if member.user_id == my_uid {
                    s.me = Some(member.clone());
                }
                s.members.insert(member.user_id.clone(), member);
            }
            Event::MemberRemoved { user_id } => {
                s.members.remove(&user_id);
            }
            Event::RoleUpserted { role } => {
                match s.roles.iter_mut().find(|r| r.id == role.id) {
                    Some(r) => *r = role,
                    None => s.roles.push(role),
                }
                s.roles.sort_by_key(|r| (std::cmp::Reverse(r.position), r.id));
            }
            Event::DmOpened { dm } => s.upsert_dm(dm),
            Event::RoleDeleted { role_id } => {
                s.roles.retain(|r| r.id != role_id);
                for m in s.members.values_mut() {
                    m.roles.retain(|r| *r != role_id);
                }
                if let Some(me) = s.me.as_mut() {
                    me.roles.retain(|r| *r != role_id);
                }
            }
            Event::Unknown => {}
            Event::ServerUpdated { server } => {
                s.name = server.name.clone();
                s.info = server;
            }
            Event::VoiceUpserted { participant } => {
                if s.is_private(participant.channel_id) && !s.voice.contains_key(&participant.session) {
                    dm_voice = Some((participant.channel_id, participant.user_id.clone(), true));
                }
                s.voice.insert(participant.session, participant.clone());
                forward = Some(CallEvent::Upserted(participant));
            }
            Event::VoiceRemoved { session, user_id, channel_id } => {
                if s.is_private(channel_id) {
                    dm_voice = Some((channel_id, user_id, false));
                }
                s.voice.remove(&session);
                forward = Some(CallEvent::Removed(session));
            }
            // (Only ever sent to the people in it; not from someone I've blocked.)
            Event::Ringing { channel_id, from, video, moved_from } => {
                if !blocked.contains(&from) && from != my_uid {
                    if !s.is_private(channel_id) {
                        // A direct message started just now (a group comes with GroupUpserted).
                        s.upsert_dm(Dm { id: channel_id, with: from.clone(), last_message: None, last_from: None });
                    }
                    ringing = Some((channel_id, from, video, moved_from));
                }
            }
            Event::GroupUpserted { group } => s.upsert_group(group),
            Event::GroupRemoved { channel_id } => {
                s.groups.retain(|g| g.id != channel_id);
                s.logs.remove(&channel_id);
                s.unread.remove(&channel_id);
                s.mentions.remove(&channel_id);
                // Out of it: its call too (the server took us out), and its ringing.
                s.voice.retain(|_, p| p.channel_id != channel_id);
                gone_group = Some(channel_id);
            }
            Event::CallDeclined { channel_id, by } => declined = Some((channel_id, by)),
            Event::KeyframeRequested { track } => forward = Some(CallEvent::KeyframeRequested(track)),
            Event::VideoFeedback { track, viewers } => forward = Some(CallEvent::VideoFeedback(track, viewers)),
            Event::E2eeKey { from, epoch, nonce, ct } => forward = Some(CallEvent::Key { from, epoch, nonce, ct }),
            Event::SoundPlayed { session, channel_id, sound: s, name, emoji, volume } => {
                sound = Some((session, channel_id, s, name, emoji, volume));
            }
            // (Handshakes are handled before this; vouches only come as replies.)
            Event::Pong { .. } | Event::Ok | Event::Hello { .. } | Event::Ready { .. } | Event::Handshake { .. } | Event::Vouches { .. } => {}
            Event::Error { message, .. } => toast = Some(message),
            Event::History { .. } | Event::InviteCreated { .. } | Event::Invites { .. } | Event::VoiceReady { .. } | Event::Kicked { .. } | Event::Bugs { .. } | Event::PluginSubmissions { .. } | Event::PluginSubmission { .. } | Event::SubmissionFile { .. } | Event::Submitted { .. } | Event::MySubmissions { .. } => {}
        }
        // (The server saying no: no bug.)
        if let Some(t) = toast {
            st.refused(t);
        }
        if let Some(t) = dm_note {
            st.toast(t, false);
        }
        if let Some(uid) = friend_done {
            st.friend_request_done(&uid);
        }
        if let Some((t, uid)) = friend_note {
            st.friend_request(t, &uid);
        }
        let here = |st: &AppState, c: ChannelId| st.call.as_ref().is_some_and(|x| x.server_key == key && x.channel_id == c);
        let this = |c: &IncomingCall, cid: ChannelId| c.server_key == key && c.channel_id == cid;
        if let Some(cid) = gone_group {
            st.incoming_calls.retain(|c| !this(c, cid));
            if st.outgoing_call.as_ref().is_some_and(|o| o.server_key == key && o.channel_id == cid) {
                st.outgoing_call = None;
            }
            leave_call = here(&st, cid);
        }
        if let Some((cid, from, video, moved_from)) = ringing {
            match moved_from.filter(|m| here(&st, *m)) {
                // The call we're in moved here (people were added to it): come along.
                Some(_) => follow = Some(cid),
                // (Not a call we're in already.)
                None if !here(&st, cid) => {
                    // Rung again while it's still ringing (the same call): it keeps its time,
                    // so it still stops when it would have.
                    let since = st.incoming_calls.iter().find(|c| this(c, cid) && c.from == from).map(|c| c.since).unwrap_or_else(Instant::now);
                    st.incoming_calls.retain(|c| !this(c, cid));
                    st.incoming_calls.push(IncomingCall { server_key: key.to_string(), channel_id: cid, from, video, since });
                    ring = true;
                }
                None => {}
            }
        }
        if let Some((cid, by)) = declined {
            if by == my_uid {
                // Declined here or on another of my devices: it stops ringing everywhere.
                st.incoming_calls.retain(|c| !this(c, cid));
            } else if let Some(o) = st.outgoing_call.as_mut().filter(|o| o.server_key == key && o.channel_id == cid) {
                if o.to.contains(&by) && !o.declined.contains(&by) {
                    o.declined.push(by);
                }
                // Everyone said no.
                if o.to.iter().all(|t| o.declined.contains(t)) {
                    o.state = RingState::Declined;
                }
            }
        }
        if let Some((cid, who, came)) = dm_voice {
            if came && who == my_uid {
                // Answered (here, or on another of my devices).
                st.incoming_calls.retain(|c| !this(c, cid));
            }
            // Someone answered: they're in. In a group, the others keep ringing (until they
            // answer, say no, or it rings out); once nobody's left ringing, it's settled.
            let answered = match st.outgoing_call.as_mut().filter(|o| came && o.server_key == key && o.channel_id == cid && o.to.contains(&who)) {
                Some(o) => {
                    o.to.retain(|t| *t != who);
                    o.declined.retain(|t| *t != who);
                    if !o.to.is_empty() && o.to.iter().all(|t| o.declined.contains(t)) {
                        o.state = RingState::Declined;
                    }
                    o.to.is_empty()
                }
                None => false,
            };
            if answered {
                st.outgoing_call = None;
            }
            if !came {
                // They hung up before it was answered: a missed call.
                if let Some(i) = st.incoming_calls.iter().position(|c| this(c, cid) && c.from == who) {
                    st.incoming_calls.remove(i);
                    let name = st.server(key).map(|s| s.member_name(&who)).unwrap_or_default();
                    st.missed_call(key, cid, &name);
                }
            }
        }
    }
    if ring {
        crate::ring::ensure(shared);
    }
    if leave_call {
        crate::leave_voice_on(shared);
    }
    if let Some(cid) = follow {
        crate::join_voice_on(shared, key, cid, crate::AfterJoin::default());
    }
    if let Some((cid, id, save)) = read_upto {
        shared.mark_read(key, cid, id, save);
    }
    if let Some((cid, id)) = unread_from {
        shared.mark_read_before(key, cid, id);
    }
    if let Some((session, channel_id, s, name, emoji, volume)) = sound {
        shared.sound_played(key, session, channel_id, s, name, emoji, volume);
    }
    if ding {
        shared.play(crate::sfx::Sound::Message);
    }
    if let Some(ev) = forward {
        shared.forward_call_event(key, ev);
    }
    (shared.repaint)();
}
