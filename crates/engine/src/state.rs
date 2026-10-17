//! State shared between the engine and the UI. The UI reads it every frame; engine tasks
//! update it and request a repaint.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use ingle_proto::control::*;

use crate::config::Settings;

#[derive(Clone, Debug, PartialEq)]
pub enum ConnStatus {
    Connecting,
    Connected,
    /// Lost the connection; retrying.
    Reconnecting { attempt: u32, error: String },
    /// Gave up (bad invite, banned, version mismatch...).
    Failed(String),
}

impl ConnStatus {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnStatus::Connected)
    }
}

#[derive(Clone, Debug)]
pub struct PendingMessage {
    pub nonce: String,
    pub content: String,
    pub reply_to: Option<MessageId>,
    pub created_at: i64,
    pub failed: Option<String>,
    /// Images being uploaded with it: (file name, bytes) for the preview.
    pub files: Vec<(String, Arc<Vec<u8>>)>,
    /// "Uploading 1 of 2…" while the images go up.
    pub progress: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct ChannelLog {
    pub messages: Vec<Message>,
    pub pending: Vec<PendingMessage>,
    /// Nothing older exists on the server.
    pub complete: bool,
    pub loading: bool,
    /// Initial page fetched.
    pub loaded: bool,
}

impl ChannelLog {
    pub fn upsert(&mut self, m: Message) {
        match self.messages.binary_search_by_key(&m.id, |x| x.id) {
            Ok(i) => self.messages[i] = m,
            Err(i) => self.messages.insert(i, m),
        }
    }

    pub fn remove(&mut self, id: MessageId) {
        if let Ok(i) = self.messages.binary_search_by_key(&id, |x| x.id) {
            self.messages.remove(i);
        }
    }

    pub fn find(&self, id: MessageId) -> Option<&Message> {
        self.messages.binary_search_by_key(&id, |x| x.id).ok().map(|i| &self.messages[i])
    }
}

/// How sure I can be that someone is who they say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Trust {
    /// I made sure of them.
    Verified,
    /// Someone I made sure of vouches for them (their ids).
    Vouched(Vec<UserId>),
    /// Not the person I made sure of under that name (a name like theirs, someone else): the
    /// id of the one I made sure of.
    Impostor(UserId),
    /// Nothing to go on.
    Unknown,
}

/// Making sure of someone, as the app shows it (see `verify`).
#[derive(Clone, Debug)]
pub struct HandshakeView {
    pub sid: String,
    /// The server it goes through.
    pub server: String,
    pub with: UserId,
    pub kind: HandshakeKind,
    pub mine: bool,
    /// What was asked (asking something only they'd know).
    pub question: String,
    pub stage: HandshakeStage,
    pub at: Instant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HandshakeKind {
    /// Comparing words.
    Words,
    /// Asking something only they'd know.
    Ask,
}

#[derive(Clone, Debug, PartialEq)]
pub enum HandshakeStage {
    /// They want to: waiting for me (to say yes to comparing words, or to answer).
    Asked,
    /// Waiting for them.
    Waiting,
    /// The words both should see (picture, word), and what they said about them so far.
    Compare { words: Vec<(String, String)>, they_said: Option<bool> },
    /// Done: I made sure it's them.
    Sure,
    /// I answered their question, and the answers matched: if only they could know that
    /// answer, it's them (my say).
    AnswersMatched,
    /// It didn't work out (and why, to say).
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct ServerView {
    pub key: String,
    pub name: String,
    pub address: String,
    pub status: ConnStatus,
    pub me: Option<Member>,
    pub info: ServerInfo,
    pub channels: Vec<Channel>,
    pub members: BTreeMap<UserId, Member>,
    pub logs: HashMap<ChannelId, ChannelLog>,
    pub voice: BTreeMap<SessionId, VoiceParticipant>,
    pub typing: HashMap<ChannelId, HashMap<UserId, Instant>>,
    pub unread: HashSet<ChannelId>,
    pub mentions: HashMap<ChannelId, u32>,
    /// Where the "NEW" line goes: messages from this id on came in while you weren't looking.
    /// Kept after you've read them, until you leave the conversation (or write in it), so you
    /// can see who wrote what you missed.
    pub new_since: HashMap<ChannelId, MessageId>,
    pub last_invite: Option<String>,
    /// The last bot invite made here (Server Settings → Bots).
    pub last_bot_invite: Option<String>,
    pub rtt_ms: u32,
    /// Custom roles, highest first.
    pub roles: Vec<RoleDef>,
    /// Features the server has beyond protocol v1 (roles, files, avatars).
    pub features: Vec<String>,
    /// My direct message conversations here, newest first.
    pub dms: Vec<Dm>,
    /// My group conversations here, newest first.
    pub groups: Vec<Group>,
    /// Friends, friend requests and blocks recorded on this server.
    pub relations: HashMap<UserId, RelationKind>,
    /// Whether this server was told I'm idle (see `idle`; a new connection starts not idle).
    pub idle_sent: bool,
    /// Whether this server was told Do Not Disturb (see `idle`; a new connection starts without).
    pub dnd_sent: bool,
}

impl ServerView {
    pub fn new(key: String, name: String, address: String) -> ServerView {
        ServerView {
            key,
            name,
            address,
            status: ConnStatus::Connecting,
            me: None,
            info: ServerInfo::default(),
            channels: Vec::new(),
            members: BTreeMap::new(),
            logs: HashMap::new(),
            voice: BTreeMap::new(),
            typing: HashMap::new(),
            unread: HashSet::new(),
            mentions: HashMap::new(),
            new_since: HashMap::new(),
            last_invite: None,
            last_bot_invite: None,
            rtt_ms: 0,
            roles: Vec::new(),
            features: Vec::new(),
            dms: Vec::new(),
            groups: Vec::new(),
            relations: HashMap::new(),
            idle_sent: false,
            dnd_sent: false,
        }
    }

    pub fn has(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }

    pub fn dm(&self, channel_id: ChannelId) -> Option<&Dm> {
        self.dms.iter().find(|d| d.id == channel_id)
    }

    pub fn dm_with(&self, uid: &str) -> Option<&Dm> {
        self.dms.iter().find(|d| d.with == uid)
    }

    pub fn group(&self, channel_id: ChannelId) -> Option<&Group> {
        self.groups.iter().find(|g| g.id == channel_id)
    }

    /// Adds or replaces a group, keeping the newest first.
    pub fn upsert_group(&mut self, group: Group) {
        match self.groups.iter_mut().find(|g| g.id == group.id) {
            Some(g) => {
                let last = g.last_message.max(group.last_message);
                *g = group;
                g.last_message = last;
            }
            None => self.groups.push(group),
        }
        self.groups.sort_by_key(|g| std::cmp::Reverse((g.last_message, g.id)));
    }

    /// A private conversation: a direct message or a group (not a channel everyone sees).
    pub fn is_private(&self, channel_id: ChannelId) -> bool {
        self.dm(channel_id).is_some() || self.group(channel_id).is_some()
    }

    /// The others in a private conversation (a direct message's other person, a group's other
    /// members); `me` is left out.
    pub fn others_in(&self, channel_id: ChannelId, me: &str) -> Vec<UserId> {
        if let Some(d) = self.dm(channel_id) {
            return vec![d.with.clone()];
        }
        self.group(channel_id).map(|g| g.members.iter().filter(|m| *m != me).cloned().collect()).unwrap_or_default()
    }

    /// A group's name: its own, or who's in it (besides `me`), like Discord's.
    pub fn group_name(&self, g: &Group, me: &str) -> String {
        if !g.name.is_empty() {
            return g.name.clone();
        }
        let names: Vec<String> = g.members.iter().filter(|m| *m != me).map(|m| self.member_name(m)).collect();
        if names.is_empty() { "Just you".to_string() } else { names.join(", ") }
    }

    /// Adds or replaces a conversation, keeping the newest first.
    pub fn upsert_dm(&mut self, dm: Dm) {
        match self.dms.iter_mut().find(|d| d.id == dm.id) {
            Some(d) => {
                d.with = dm.with;
                d.last_message = d.last_message.max(dm.last_message);
            }
            None => self.dms.push(dm),
        }
        self.dms.sort_by_key(|d| std::cmp::Reverse((d.last_message, d.id)));
    }

    pub fn role(&self, id: RoleId) -> Option<&RoleDef> {
        self.roles.iter().find(|r| r.id == id)
    }

    /// A member's roles, highest first.
    pub fn roles_of(&self, uid: &str) -> Vec<&RoleDef> {
        let Some(m) = self.members.get(uid) else { return Vec::new() };
        self.roles.iter().filter(|r| m.roles.contains(&r.id)).collect()
    }

    /// The color a member's name shows in: their highest colored role's.
    pub fn name_color(&self, uid: &str) -> Option<u32> {
        self.roles_of(uid).into_iter().find(|r| r.color != 0).map(|r| r.color)
    }

    /// What I may do here (see `perm`).
    pub fn my_perms(&self) -> u64 {
        let Some(me) = &self.me else { return 0 };
        if me.role.is_admin() {
            return perm::ALL;
        }
        self.roles.iter().filter(|r| me.roles.contains(&r.id)).fold(0, |a, r| a | r.perms)
    }

    /// What a member may do.
    pub fn perms_of(&self, uid: &str) -> u64 {
        let Some(m) = self.members.get(uid) else { return 0 };
        if m.role.is_admin() {
            return perm::ALL;
        }
        self.roles_of(uid).iter().fold(0, |a, r| a | r.perms)
    }

    /// Whether a message pings me, like Discord: @my name, @one of my roles, or
    /// @everyone / @here from someone allowed to use them.
    pub fn pings_me(&self, author: &str, content: &str) -> bool {
        let Some(me) = &self.me else { return false };
        if author == me.user_id {
            return false;
        }
        let lower = content.to_lowercase();
        if has_mention(&lower, &self.member_name(&me.user_id).to_lowercase()) {
            return true;
        }
        if self.roles_of(&me.user_id).iter().any(|r| has_mention(&lower, &r.name.to_lowercase())) {
            return true;
        }
        (has_mention(&lower, "everyone") || has_mention(&lower, "here")) && self.perms_of(author) & perm::MENTION_EVERYONE != 0
    }

    pub fn can(&self, p: u64) -> bool {
        self.my_perms() & p == p
    }

    /// Where someone stands (owner > admins > custom roles by position > no roles).
    pub fn rank(&self, uid: &str) -> i64 {
        match self.members.get(uid).map(|m| m.role) {
            Some(Role::Owner) => i64::MAX,
            Some(Role::Admin) => i64::MAX - 1,
            _ => self.roles_of(uid).first().map(|r| r.position as i64).unwrap_or(i64::MIN),
        }
    }

    /// Whether I can hand out or take away this role (it's below my highest).
    pub fn can_manage_role(&self, r: &RoleDef) -> bool {
        self.can(perm::MANAGE_ROLES) && (self.is_admin() || (r.position as i64) < self.me.as_ref().map(|m| self.rank(&m.user_id)).unwrap_or(i64::MIN))
    }

    pub fn member_name(&self, uid: &str) -> String {
        self.members
            .get(uid)
            .map(|m| m.name.clone())
            .filter(|n| !n.is_empty())
            // (By characters: an id from a server that isn't Ingle's own can be any text.)
            .unwrap_or_else(|| format!("user-{}", uid.chars().take(6).collect::<String>()))
    }

    pub fn is_admin(&self) -> bool {
        self.me.as_ref().map(|m| m.role.is_admin()).unwrap_or(false)
    }

    /// Whether a member is a bot ([`BOTS_FEATURE`]).
    pub fn is_bot(&self, uid: &str) -> bool {
        self.members.get(uid).is_some_and(|m| m.bot)
    }

    pub fn channel(&self, id: ChannelId) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }

    /// Whether a member can see a channel ([`PRIVATE_CHANNELS_FEATURE`]: only some roles see
    /// a private one; admins and the owner see everything).
    pub fn sees(&self, uid: &str, ch: &Channel) -> bool {
        match self.members.get(uid) {
            Some(m) => ch.visible_to(m.role.is_admin(), &m.roles),
            None => !ch.private,
        }
    }

    /// Whether I can send messages in a channel (or a conversation: always).
    pub fn can_post(&self, channel_id: ChannelId) -> bool {
        let Some(ch) = self.channel(channel_id) else { return true };
        let Some(me) = &self.me else { return false };
        ch.postable_by(me.role.is_admin(), &me.roles)
    }

    pub fn voice_in(&self, channel_id: ChannelId) -> Vec<&VoiceParticipant> {
        self.voice.values().filter(|p| p.channel_id == channel_id).collect()
    }
}

/// `@name` in (lowercased) text, at the start or after a space, and not just the start of a
/// longer word ("@bob" isn't in "@bobby").
pub fn has_mention(lower: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let needle = format!("@{name}");
    let mut from = 0;
    while let Some(i) = lower[from..].find(&needle) {
        let start = from + i;
        let end = start + needle.len();
        let before_ok = lower[..start].chars().next_back().map(|c| c.is_whitespace() || "([{".contains(c)).unwrap_or(true);
        let after_ok = lower[end..].chars().next().map(|c| !c.is_alphanumeric() && c != '_').unwrap_or(true);
        if before_ok && after_ok {
            return true;
        }
        from = start + 1;
        while !lower.is_char_boundary(from) {
            from += 1;
        }
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VideoKey {
    pub session: SessionId,
    pub track: u8,
}

#[derive(Clone, Debug, Default)]
pub struct CallStats {
    pub rtt_ms: u32,
    pub audio_sent: u64,
    pub audio_received: u64,
    pub audio_lost: u64,
    pub audio_concealed: u64,
    pub audio_fec_recovered: u64,
    pub undecryptable: u64,
    pub video_frames_sent: u64,
    pub video_frames_received: u64,
    pub video_send_kbps: u32,
    /// What our screen share (or else camera) is sending now, adjusted to the network (see
    /// `video::adapt`).
    pub sending: Option<Sending>,
    pub jitter_ms: f32,
    pub buffer_ms: u32,
}

/// The size, frame rate and bitrate a camera or screen share goes out at now.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Sending {
    pub height: u32,
    pub fps: u32,
    pub kbps: u32,
    /// Less than its preset, to suit the connection (ours or a viewer's).
    pub lowered: bool,
}

#[derive(Clone, Debug)]
pub struct CallView {
    pub server_key: String,
    pub channel_id: ChannelId,
    pub session: SessionId,
    pub muted: bool,
    pub deafened: bool,
    pub camera_on: bool,
    pub screen_on: bool,
    /// Our screen share is sending sound too.
    pub stream_audio_on: bool,
    pub transmitting: bool,
    /// Sessions currently speaking (including our own).
    pub speaking: HashSet<SessionId>,
    pub watching: HashSet<VideoKey>,
    pub call_code: String,
    pub e2ee_ready: bool,
    pub stats: CallStats,
    pub audio_error: Option<String>,
    pub video_error: Option<String>,
    pub joined_at: Instant,
    /// Soundboard sounds played lately, for showing who played what.
    pub sounds: Vec<SoundShown>,
}

/// A soundboard sound someone in the call played.
#[derive(Clone, Debug)]
pub struct SoundShown {
    pub session: SessionId,
    pub name: String,
    pub emoji: String,
    pub at: Instant,
}

#[derive(Clone, Debug)]
pub struct VoiceRejoin {
    pub server_key: String,
    pub channel_id: ChannelId,
    pub camera: bool,
    /// The screen (or window) being shared, to share again.
    pub screen: Option<String>,
    pub since: Instant,
}

/// What was on before a call dropped and came back (see `AppState::resume_offer`).
#[derive(Clone, Debug, PartialEq)]
pub struct ResumeOffer {
    pub server_key: String,
    pub channel_id: ChannelId,
    pub camera: bool,
    pub screen: bool,
}

/// How long to keep trying to get back into a call after the server drops.
pub const VOICE_REJOIN_FOR: std::time::Duration = std::time::Duration::from_secs(180);

/// How long a call in a direct message rings before it counts as not answered (and missed):
/// about as long as Discord's.
pub const RING_FOR: std::time::Duration = std::time::Duration::from_secs(30);

/// Someone calling you in a direct message (see `DM_CALLS_FEATURE`): it rings until you
/// answer or decline, they hang up, or [`RING_FOR`] goes by.
#[derive(Clone, Debug, PartialEq)]
pub struct IncomingCall {
    pub server_key: String,
    /// The direct message.
    pub channel_id: ChannelId,
    pub from: UserId,
    /// They started it as a video call.
    pub video: bool,
    pub since: Instant,
}

/// How ringing the other person in a call we started is going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RingState {
    Ringing,
    /// They said no.
    Declined,
    /// It rang for [`RING_FOR`] and nobody answered.
    NoAnswer,
}

/// A call we started in a direct message (or group), until someone comes in (or we leave).
#[derive(Clone, Debug, PartialEq)]
pub struct OutgoingCall {
    pub server_key: String,
    pub channel_id: ChannelId,
    /// Who's being rung: the other person in a direct message, the rest of a group.
    pub to: Vec<UserId>,
    /// Who of them said no.
    pub declined: Vec<UserId>,
    pub video: bool,
    /// When it (last) started ringing.
    pub since: Instant,
    pub state: RingState,
}

/// Backups of a server I own (see `backup.rs`).
#[derive(Clone, Debug, Default)]
pub struct BackupStatus {
    pub running: bool,
    /// When the last one finished (ms since 1970).
    pub last_ok: Option<i64>,
    pub error: Option<String>,
}

/// How the last bug report I sent went (the Report a bug window shows it).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum BugSend {
    #[default]
    Idle,
    Sending,
    Sent,
    /// It didn't go: why, and whether that's because the bug server isn't collecting reports
    /// (which its owner can switch on).
    Failed { message: String, not_collecting: bool },
}

#[derive(Clone, Debug)]
pub struct Toast {
    /// Unique, for dismissing it.
    pub id: u64,
    pub text: String,
    pub error: bool,
    pub at: Instant,
    /// An error that's a bug: reported by itself (see `bugs::watch_errors`). Not a refusal
    /// (`refused`: the server said no, not connected, a limit), which is no bug.
    pub report: bool,
    /// A friend request from this person (see `AppState::friend_request`): a notice, never an
    /// error, shown with Accept and Ignore until it's answered (here or anywhere else), taken
    /// back, or closed.
    pub friend: Option<UserId>,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceLists {
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
    pub cameras: Vec<(String, String)>,
    pub screens: Vec<crate::video::ScreenSource>,
}

#[derive(Clone, Debug, Default)]
pub struct AppState {
    pub my_uid: String,
    pub my_name: String,
    /// I'm idle (shown as a moon on my picture; see `idle`).
    pub my_idle: bool,
    pub safety_code: String,
    pub servers: Vec<ServerView>,
    pub call: Option<CallView>,
    /// The connection to the server dropped mid-call (an update restarting it, say): the call
    /// is rejoined as soon as the server is back (the camera and screen share aren't: see
    /// `resume_offer`).
    pub voice_rejoin: Option<VoiceRejoin>,
    /// Back in a call after a drop, with the camera or a screen share off that was on before:
    /// the app offers to turn them back on (never by itself: what's on your screen may have
    /// changed meanwhile).
    pub resume_offer: Option<ResumeOffer>,
    pub toasts: Vec<Toast>,
    /// The bug report being sent, or how the last one went; `bug_sends_done` counts the sends
    /// that have finished, so the app sees each outcome once.
    pub bug_send: BugSend,
    pub bug_sends_done: u64,
    /// Bug reports on servers I own (fetched when looked at), by server key.
    pub bugs: HashMap<String, Vec<ingle_proto::control::BugEntry>>,
    /// The plugin being sent in for the library, or how the last one went (like `bug_send`).
    pub plugin_send: BugSend,
    pub plugin_sends_done: u64,
    /// What I sent in to the Ingle Library and where each stands (`Engine::my_submissions`):
    /// none until asked, then the list or why it couldn't be had.
    pub my_submissions: Option<Result<Vec<ingle_proto::control::SubmissionEntry>, String>>,
    /// Plugins sent in to servers I own (fetched when looked at), by server key; and the files
    /// of the ones opened, by (server key, id).
    pub submissions: HashMap<String, Vec<ingle_proto::control::SubmissionEntry>>,
    pub submission_files: HashMap<(String, i64), Vec<ingle_proto::control::SourceFile>>,
    /// Backups of servers I own, by server key.
    pub backups: HashMap<String, BackupStatus>,
    pub devices: DeviceLists,
    pub settings: Settings,
    pub user_volumes: HashMap<UserId, f32>,
    pub stream_volumes: HashMap<UserId, f32>,
    /// Microphone level in dBFS (for the settings meter), updated while audio runs.
    pub mic_level_db: f32,
    pub mic_test: bool,
    /// Muted servers and (server, channel) pairs.
    pub muted_servers: HashSet<String>,
    pub muted_channels: HashSet<(String, ChannelId)>,
    /// People I've blocked.
    pub blocked: HashSet<UserId>,
    /// People I made sure are who they say (see `verify`), by id.
    pub verified: BTreeMap<UserId, crate::config::Verified>,
    /// People vouched for by someone I made sure of: their id → who vouched.
    pub vouched: HashMap<UserId, Vec<UserId>>,
    /// Who made sure it's me, by server (their signed vouches for me; see
    /// [`AppState::made_sure_of_me`]).
    pub sure_of_me: HashMap<String, Vec<UserId>>,
    /// Making sure of people: under way, or just done (cards in the app).
    pub handshakes: Vec<HandshakeView>,
    /// Calls in direct messages ringing here (see `ring`).
    pub incoming_calls: Vec<IncomingCall>,
    /// The call we started in a direct message, while the other person isn't in it yet.
    pub outgoing_call: Option<OutgoingCall>,
    /// Calls missed so far, and who from last (the tray icon says so while Ingle is hidden).
    pub missed_calls: u64,
    pub last_missed_from: String,
}

impl AppState {
    /// What a call's place is called: its voice channel's name, for a direct message's call
    /// "@" and the other person's name, and a group's name.
    pub fn call_place(&self, key: &str, channel_id: ChannelId) -> String {
        let Some(s) = self.server(key) else { return String::new() };
        if let Some(c) = s.channel(channel_id) {
            return c.name.clone();
        }
        if let Some(g) = s.group(channel_id) {
            return s.group_name(g, &self.my_uid);
        }
        s.dm(channel_id).map(|d| format!("@{}", s.member_name(&d.with))).unwrap_or_default()
    }

    /// A call in a direct message rang out here: say so.
    pub fn missed_call(&mut self, _key: &str, _channel_id: ChannelId, who: &str) {
        self.missed_calls += 1;
        self.last_missed_from = who.to_string();
        self.toast(format!("You missed a call from {who}."), false);
    }

    /// The call is a private conversation's (a direct message's or a group's), not a voice
    /// channel's.
    pub fn is_dm_call(&self, key: &str, channel_id: ChannelId) -> bool {
        self.server(key).is_some_and(|s| s.is_private(channel_id))
    }

    pub fn server(&self, key: &str) -> Option<&ServerView> {
        self.servers.iter().find(|s| s.key == key)
    }
    pub fn server_mut(&mut self, key: &str) -> Option<&mut ServerView> {
        self.servers.iter_mut().find(|s| s.key == key)
    }
    /// Where I stand with someone, across all my servers (a block anywhere wins, then a
    /// friendship, then requests).
    pub fn relation(&self, uid: &str) -> Option<RelationKind> {
        if self.blocked.contains(uid) {
            return Some(RelationKind::Blocked);
        }
        let all: Vec<RelationKind> = self.servers.iter().filter_map(|s| s.relations.get(uid).copied()).collect();
        [RelationKind::Blocked, RelationKind::Friend, RelationKind::Incoming, RelationKind::Outgoing].into_iter().find(|k| all.contains(k))
    }

    /// Everyone I have some relation with: (user id, relation).
    pub fn relations(&self) -> Vec<(UserId, RelationKind)> {
        let mut uids: Vec<UserId> = self.servers.iter().flat_map(|s| s.relations.keys().cloned()).chain(self.blocked.iter().cloned()).collect();
        uids.sort();
        uids.dedup();
        uids.into_iter().filter_map(|u| self.relation(&u).map(|k| (u, k))).collect()
    }

    /// A name for someone from any server we share.
    /// How sure I can be that `uid` (shown as `name`) is who they say: see [`Trust`].
    pub fn trust(&self, uid: &str, name: &str) -> Trust {
        if self.verified.contains_key(uid) {
            return Trust::Verified;
        }
        let folded = crate::verify::fold(name);
        if !folded.is_empty() {
            if let Some((real, _)) = self.verified.iter().find(|(id, v)| id.as_str() != uid && crate::verify::fold(&v.name) == folded) {
                return Trust::Impostor(real.clone());
            }
        }
        match self.vouched.get(uid) {
            Some(by) if !by.is_empty() => Trust::Vouched(by.clone()),
            _ => Trust::Unknown,
        }
    }

    /// Who made sure it's me, on any server I'm on (for the shield by my own name).
    pub fn made_sure_of_me(&self) -> Vec<UserId> {
        let mut v: Vec<UserId> = self.sure_of_me.iter().filter(|(k, _)| self.server(k).is_some()).flat_map(|(_, by)| by.iter().cloned()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// Who made sure it's me on server `key`, as it says now.
    pub fn set_made_sure_of_me(&mut self, key: &str, mut by: Vec<UserId>) {
        by.sort();
        by.dedup();
        if by.is_empty() {
            self.sure_of_me.remove(key);
        } else {
            self.sure_of_me.insert(key.to_string(), by);
        }
    }

    pub fn name_of(&self, uid: &str) -> String {
        self.servers.iter().find(|s| s.members.contains_key(uid)).map(|s| s.member_name(uid)).unwrap_or_else(|| "Someone".into())
    }

    pub fn is_muted(&self, key: &str, channel: ChannelId) -> bool {
        self.muted_servers.contains(key) || self.muted_channels.contains(&(key.to_string(), channel))
    }

    /// A notice in the corner. Errors stay until dismissed (and can be copied); the same error
    /// again just moves back to the top instead of piling up.
    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast_as(text.into(), error, error);
    }

    /// Something asked for that can't be done (the server said no, not connected, over a
    /// limit): shown like an error, but it's no bug, so it isn't reported.
    pub fn refused(&mut self, text: impl Into<String>) {
        self.toast_as(text.into(), true, false);
    }

    fn toast_as(&mut self, text: String, error: bool, report: bool) {
        if error {
            self.toasts.retain(|t| !(t.error && t.text == text));
        }
        self.push_toast(text, error, report, None);
    }

    /// Someone sent me a friend request: news, not an error. It stays (with Accept and Ignore)
    /// until it's answered or closed; one per person.
    pub fn friend_request(&mut self, text: impl Into<String>, from: &str) {
        self.friend_request_done(from);
        self.push_toast(text.into(), false, false, Some(from.to_string()));
    }

    /// Their friend request was answered (here or on another computer) or taken back: its
    /// notice goes.
    pub fn friend_request_done(&mut self, from: &str) {
        self.toasts.retain(|t| t.friend.as_deref() != Some(from));
    }

    fn push_toast(&mut self, text: String, error: bool, report: bool, friend: Option<UserId>) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.toasts.push(Toast { id, text, error, at: Instant::now(), report, friend });
        // Keep a few: passing notices go first, then the oldest of those that stay.
        while self.toasts.len() > 8 {
            let i = self.toasts.iter().position(|t| !t.error && t.friend.is_none()).unwrap_or(0);
            self.toasts.remove(i);
        }
    }
}

/// A decoded video frame ready to upload as a texture (RGBA8, tightly packed).
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    /// Increments for every new frame in this slot.
    pub generation: u64,
}

pub type SharedFrame = Arc<VideoFrame>;

#[cfg(test)]
mod tests {
    use super::{ServerView, has_mention};

    #[test]
    fn any_member_id_has_a_name() {
        // A server that isn't Ingle's own could send ids that aren't base64: multi-byte
        // characters where a name gets cut used to crash the app.
        let s = ServerView::new("k".into(), "S".into(), String::new());
        assert_eq!(s.member_name("abcdéfgh"), "user-abcdéf");
        assert_eq!(s.member_name("ab"), "user-ab");
        assert_eq!(s.member_name("🔥🔥🔥🔥🔥🔥🔥"), "user-🔥🔥🔥🔥🔥🔥");
    }

    #[test]
    fn mentions_need_whole_names() {
        assert!(has_mention("hey @bob!", "bob"));
        assert!(has_mention("@bob", "bob"));
        assert!(has_mention("hi @big mike, you there", "big mike"));
        assert!(!has_mention("hey @bobby", "bob"));
        assert!(!has_mention("mail me at x@bob", "bob"));
        assert!(has_mention("(@mods) help", "mods"));
        assert!(has_mention("@everyone 🎉", "everyone"));
    }
}
