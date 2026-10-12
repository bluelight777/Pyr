//! Control-plane messages.
//!
//! The client opens one bidirectional QUIC stream right after connecting. Both sides then
//! exchange length-prefixed JSON frames (see [`crate::framing`]):
//!
//! * client → server: [`ClientFrame`] `{ id, req }` — `id` is echoed back in the reply
//!   (`id = 0` means "no reply wanted").
//! * server → client: [`ServerFrame`] `{ re, ev }` — `re` is the request id this event answers,
//!   or `0` for pushed events.

use serde::{Deserialize, Serialize};

/// A user is identified by their Ed25519 public key (base64url, 43 chars).
pub type UserId = String;
pub type ChannelId = i64;
pub type MessageId = i64;
/// Server-assigned id of one voice session (one person in one voice channel).
pub type SessionId = u32;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ChannelKind {
    Text,
    Voice,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Channel {
    pub id: ChannelId,
    pub kind: ChannelKind,
    pub name: String,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub position: i32,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Member,
    Admin,
    Owner,
}

impl Role {
    pub fn is_admin(self) -> bool {
        self >= Role::Admin
    }
    pub fn as_i64(self) -> i64 {
        match self {
            Role::Member => 0,
            Role::Admin => 1,
            Role::Owner => 2,
        }
    }
    pub fn from_i64(v: i64) -> Role {
        match v {
            2 => Role::Owner,
            1 => Role::Admin,
            _ => Role::Member,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Member {
    pub user_id: UserId,
    pub name: String,
    pub role: Role,
    pub online: bool,
    #[serde(default)]
    pub joined_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// base64url SHA-256 of the server's TLS certificate (stable server identity).
    #[serde(default)]
    pub fingerprint: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Reaction {
    pub emoji: String,
    pub users: Vec<UserId>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Message {
    pub id: MessageId,
    pub channel_id: ChannelId,
    pub author: UserId,
    pub content: String,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<MessageId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    Mic,
    Camera,
    Screen,
    ScreenAudio,
}

/// A media track a voice participant is publishing.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct TrackInfo {
    /// Track number, unique per session (see [`crate::media::track`]).
    pub id: u8,
    pub kind: TrackKind,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub fps: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct VoiceParticipant {
    pub session: SessionId,
    pub user_id: UserId,
    pub channel_id: ChannelId,
    pub self_mute: bool,
    pub self_deaf: bool,
    #[serde(default)]
    pub tracks: Vec<TrackInfo>,
    /// Per-session X25519 public key used for E2EE key exchange (base64url).
    pub e2ee_pub: String,
    /// Ed25519 signature by `user_id` binding `e2ee_pub` to the channel (base64url).
    pub e2ee_sig: String,
}

/// One media key, wrapped for one recipient.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WrappedKey {
    pub to: SessionId,
    #[serde(with = "crate::b64::bytes")]
    pub nonce: Vec<u8>,
    #[serde(with = "crate::b64::bytes")]
    pub ct: Vec<u8>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ClientFrame {
    pub id: u64,
    pub req: Request,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Must be the first request. `sig` signs [`crate::crypto::login_transcript`].
    Login {
        proto: u32,
        user_id: UserId,
        #[serde(with = "crate::b64::bytes")]
        sig: Vec<u8>,
        name: String,
        #[serde(default)]
        invite: Option<String>,
        #[serde(default)]
        client: String,
    },
    SendMessage {
        channel_id: ChannelId,
        content: String,
        #[serde(default)]
        reply_to: Option<MessageId>,
        /// Client-chosen id echoed in `MessageCreated` so the sender can match its pending copy.
        #[serde(default)]
        nonce: String,
    },
    EditMessage {
        message_id: MessageId,
        content: String,
    },
    DeleteMessage {
        message_id: MessageId,
    },
    React {
        message_id: MessageId,
        emoji: String,
        add: bool,
    },
    FetchHistory {
        channel_id: ChannelId,
        #[serde(default)]
        before: Option<MessageId>,
        limit: u32,
    },
    Typing {
        channel_id: ChannelId,
    },
    CreateChannel {
        name: String,
        kind: ChannelKind,
    },
    UpdateChannel {
        channel_id: ChannelId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        topic: Option<String>,
    },
    DeleteChannel {
        channel_id: ChannelId,
    },
    CreateInvite {
        #[serde(default)]
        max_uses: Option<u32>,
        #[serde(default)]
        expires_secs: Option<u64>,
    },
    SetRole {
        user_id: UserId,
        role: Role,
    },
    Kick {
        user_id: UserId,
    },
    Ban {
        user_id: UserId,
    },
    UpdateProfile {
        name: String,
    },
    UpdateServer {
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    VoiceJoin {
        channel_id: ChannelId,
        e2ee_pub: String,
        e2ee_sig: String,
        self_mute: bool,
        self_deaf: bool,
    },
    VoiceLeave,
    VoiceState {
        self_mute: bool,
        self_deaf: bool,
    },
    Publish {
        track: TrackInfo,
    },
    Unpublish {
        track: u8,
    },
    Subscribe {
        session: SessionId,
        track: u8,
    },
    Unsubscribe {
        session: SessionId,
        track: u8,
    },
    RequestKeyframe {
        session: SessionId,
        track: u8,
    },
    E2eeKeys {
        epoch: u16,
        keys: Vec<WrappedKey>,
    },
    Ping {
        t: u64,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ServerFrame {
    #[serde(default)]
    pub re: u64,
    pub ev: Event,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    Unauthorized,
    Forbidden,
    NotFound,
    InviteInvalid,
    Banned,
    RateLimited,
    VersionMismatch,
    Internal,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// First frame from the server. `nonce` is signed in the login transcript.
    Hello {
        proto: u32,
        server_name: String,
        #[serde(with = "crate::b64::bytes")]
        nonce: Vec<u8>,
    },
    /// Login succeeded: full snapshot of the community.
    Ready {
        me: Member,
        server: ServerInfo,
        channels: Vec<Channel>,
        members: Vec<Member>,
        voice: Vec<VoiceParticipant>,
    },
    Ok,
    Error {
        code: ErrorCode,
        message: String,
    },
    History {
        channel_id: ChannelId,
        messages: Vec<Message>,
        /// True when there is nothing older than the oldest returned message.
        complete: bool,
    },
    InviteCreated {
        code: String,
        link: String,
    },
    /// Reply to `VoiceJoin`.
    VoiceReady {
        session: SessionId,
        participants: Vec<VoiceParticipant>,
    },
    MessageCreated {
        message: Message,
        #[serde(default)]
        nonce: String,
    },
    MessageUpdated {
        message: Message,
    },
    MessageDeleted {
        channel_id: ChannelId,
        message_id: MessageId,
    },
    Typing {
        channel_id: ChannelId,
        user_id: UserId,
    },
    ChannelUpserted {
        channel: Channel,
    },
    ChannelDeleted {
        channel_id: ChannelId,
    },
    MemberUpserted {
        member: Member,
    },
    MemberRemoved {
        user_id: UserId,
    },
    ServerUpdated {
        server: ServerInfo,
    },
    VoiceUpserted {
        participant: VoiceParticipant,
    },
    VoiceRemoved {
        session: SessionId,
        user_id: UserId,
        channel_id: ChannelId,
    },
    /// Sent to a publisher: a subscriber needs a keyframe on `track`.
    KeyframeRequested {
        track: u8,
    },
    E2eeKey {
        from: SessionId,
        epoch: u16,
        #[serde(with = "crate::b64::bytes")]
        nonce: Vec<u8>,
        #[serde(with = "crate::b64::bytes")]
        ct: Vec<u8>,
    },
    /// You were removed from the server (kick/ban). The connection closes afterwards.
    Kicked {
        reason: String,
    },
    Pong {
        t: u64,
    },
}

impl Event {
    pub fn error(code: ErrorCode, message: impl Into<String>) -> Event {
        Event::Error { code, message: message.into() }
    }
}

/// Rules shared by client and server for names.
pub fn clean_name(raw: &str, max_chars: usize) -> Option<String> {
    let collapsed: String = raw
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed: String = collapsed.chars().take(max_chars).collect();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

/// Channel names are lower-case, dash separated for text channels (Discord style);
/// voice channels keep their spacing and case.
pub fn clean_channel_name(raw: &str, kind: ChannelKind) -> Option<String> {
    let name = clean_name(raw, 64)?;
    Some(match kind {
        ChannelKind::Voice => name,
        ChannelKind::Text => name
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_json_shape() {
        let f = ServerFrame { re: 3, ev: Event::Ok };
        let s = serde_json::to_string(&f).unwrap();
        assert_eq!(s, r#"{"re":3,"ev":{"type":"ok"}}"#);
        // `re` is optional for pushed events.
        let f: ServerFrame =
            serde_json::from_str(r#"{"ev":{"type":"typing","channel_id":4,"user_id":"abc"}}"#).unwrap();
        assert_eq!(f.re, 0);
        assert!(matches!(f.ev, Event::Typing { channel_id: 4, .. }));
    }

    #[test]
    fn request_roundtrip() {
        let r = ClientFrame {
            id: 1,
            req: Request::E2eeKeys {
                epoch: 2,
                keys: vec![WrappedKey { to: 5, nonce: vec![1, 2, 3], ct: vec![9; 48] }],
            },
        };
        let s = serde_json::to_string(&r).unwrap();
        let back: ClientFrame = serde_json::from_str(&s).unwrap();
        match back.req {
            Request::E2eeKeys { epoch, keys } => {
                assert_eq!(epoch, 2);
                assert_eq!(keys[0].nonce, vec![1, 2, 3]);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn names() {
        assert_eq!(clean_channel_name("  Off Topic  ", ChannelKind::Text).unwrap(), "off-topic");
        assert_eq!(clean_channel_name("Game  Night", ChannelKind::Voice).unwrap(), "Game Night");
        assert!(clean_name("\n\t ", 10).is_none());
        assert_eq!(clean_name("abcdefghijkl", 5).unwrap(), "abcde");
    }
}
