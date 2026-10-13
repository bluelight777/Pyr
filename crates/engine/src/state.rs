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
    pub last_invite: Option<String>,
    pub rtt_ms: u32,
    /// Custom roles, highest first.
    pub roles: Vec<RoleDef>,
    /// Features the server has beyond protocol v1 (roles, files, avatars).
    pub features: Vec<String>,
    /// My direct message conversations here, newest first.
    pub dms: Vec<Dm>,
    /// Friends, friend requests and blocks recorded on this server.
    pub relations: HashMap<UserId, RelationKind>,
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
            last_invite: None,
            rtt_ms: 0,
            roles: Vec::new(),
            features: Vec::new(),
            dms: Vec::new(),
            relations: HashMap::new(),
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
            .unwrap_or_else(|| format!("user-{}", &uid[..uid.len().min(6)]))
    }

    pub fn is_admin(&self) -> bool {
        self.me.as_ref().map(|m| m.role.is_admin()).unwrap_or(false)
    }

    pub fn channel(&self, id: ChannelId) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
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
    pub jitter_ms: f32,
    pub buffer_ms: u32,
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

/// How long to keep trying to get back into a call after the server drops.
pub const VOICE_REJOIN_FOR: std::time::Duration = std::time::Duration::from_secs(180);

/// Backups of a server I own (see `backup.rs`).
#[derive(Clone, Debug, Default)]
pub struct BackupStatus {
    pub running: bool,
    /// When the last one finished (ms since 1970).
    pub last_ok: Option<i64>,
    pub error: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Toast {
    /// Unique, for dismissing it.
    pub id: u64,
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceLists {
    pub inputs: Vec<(String, String)>,
    pub outputs: Vec<(String, String)>,
    pub cameras: Vec<(String, String)>,
    pub screens: Vec<crate::video::ScreenSource>,
}

#[derive(Clone, Debug)]
pub struct AppState {
    pub my_uid: String,
    pub my_name: String,
    pub safety_code: String,
    pub servers: Vec<ServerView>,
    pub call: Option<CallView>,
    /// The connection to the server dropped mid-call (an update restarting it, say): the call
    /// is rejoined, camera and screen share included, as soon as the server is back.
    pub voice_rejoin: Option<VoiceRejoin>,
    pub toasts: Vec<Toast>,
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
}

impl AppState {
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
    pub fn name_of(&self, uid: &str) -> String {
        self.servers.iter().find(|s| s.members.contains_key(uid)).map(|s| s.member_name(uid)).unwrap_or_else(|| "Someone".into())
    }

    pub fn is_muted(&self, key: &str, channel: ChannelId) -> bool {
        self.muted_servers.contains(key) || self.muted_channels.contains(&(key.to_string(), channel))
    }

    /// A notice in the corner. Errors stay until dismissed (and can be copied); the same error
    /// again just moves back to the top instead of piling up.
    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let text = text.into();
        if error {
            self.toasts.retain(|t| !(t.error && t.text == text));
        }
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        self.toasts.push(Toast { id, text, error, at: Instant::now() });
        // Keep a few: notices go first, then the oldest errors.
        while self.toasts.len() > 8 {
            let i = self.toasts.iter().position(|t| !t.error).unwrap_or(0);
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
    use super::has_mention;

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
