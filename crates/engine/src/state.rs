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
        }
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
pub struct Toast {
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
    pub toasts: Vec<Toast>,
    pub devices: DeviceLists,
    pub settings: Settings,
    pub user_volumes: HashMap<UserId, f32>,
    pub stream_volumes: HashMap<UserId, f32>,
    /// Microphone level in dBFS (for the settings meter), updated while audio runs.
    pub mic_level_db: f32,
    pub mic_test: bool,
}

impl AppState {
    pub fn server(&self, key: &str) -> Option<&ServerView> {
        self.servers.iter().find(|s| s.key == key)
    }
    pub fn server_mut(&mut self, key: &str) -> Option<&mut ServerView> {
        self.servers.iter_mut().find(|s| s.key == key)
    }
    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast { text: text.into(), error, at: Instant::now() });
        if self.toasts.len() > 5 {
            self.toasts.remove(0);
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
