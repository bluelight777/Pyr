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
    /// Its newest message, in `Ready` (so what came while you were away shows as unread).
    /// Servers before 0.6.4 don't send it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_message: Option<MessageId>,
    /// Who wrote it (your own last message isn't unread).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_from: Option<UserId>,
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
    /// Custom roles (see [`RoleDef`]) this member has.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<RoleId>,
    /// Profile picture: the id of a file on the server (see [`FILES_FEATURE`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
}

pub type RoleId = i64;

/// What a custom role allows (bits of [`RoleDef::perms`]). Admins and the owner can do
/// everything; everyone can chat, react, join voice and create invites.
pub mod perm {
    pub const MANAGE_CHANNELS: u64 = 1 << 0;
    pub const KICK: u64 = 1 << 1;
    pub const BAN: u64 = 1 << 2;
    /// Delete other people's messages.
    pub const MANAGE_MESSAGES: u64 = 1 << 3;
    /// Create, edit and hand out roles below your own highest one.
    pub const MANAGE_ROLES: u64 = 1 << 4;
    /// Rename the server and change its description.
    pub const MANAGE_SERVER: u64 = 1 << 5;
    /// Ping everyone with @everyone or @here.
    pub const MENTION_EVERYONE: u64 = 1 << 6;
    /// Add and remove the server's own emoji.
    pub const MANAGE_EMOJI: u64 = 1 << 7;
    pub const ALL: u64 = MANAGE_CHANNELS | KICK | BAN | MANAGE_MESSAGES | MANAGE_ROLES | MANAGE_SERVER | MENTION_EVERYONE | MANAGE_EMOJI;
    /// (bit, short name, what it means) for settings screens.
    pub const LIST: [(u64, &str, &str); 8] = [
        (MANAGE_CHANNELS, "Manage channels", "Create, rename and delete channels."),
        (MANAGE_MESSAGES, "Manage messages", "Delete other people's messages."),
        (KICK, "Kick members", "Remove people (they can come back with an invite)."),
        (BAN, "Ban members", "Remove people for good."),
        (MANAGE_ROLES, "Manage roles", "Create and edit roles, and give people roles below their own highest role."),
        (MANAGE_SERVER, "Manage server", "Rename the server and change its description."),
        (MENTION_EVERYONE, "Mention @everyone", "Ping the whole server with @everyone or @here."),
        (MANAGE_EMOJI, "Manage emoji", "Upload and remove the server's own emoji."),
    ];
}

/// A custom role, like Discord's: a name and color, what it allows, and its place in the
/// order (higher `position` ranks higher).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct RoleDef {
    pub id: RoleId,
    pub name: String,
    /// 0xRRGGBB; 0 means no color.
    pub color: u32,
    pub position: i32,
    pub perms: u64,
    /// Members with this role are listed in their own group.
    #[serde(default)]
    pub hoist: bool,
}

/// An image or GIF attached to a message. The bytes live on the server; fetch them with a
/// file stream (see [`FILES_FEATURE`]).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub mime: String,
    pub size: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

/// Server features newer than protocol v1, listed in `Ready::features`. A client only uses
/// a feature the server lists, so newer apps work with older servers and the other way
/// round.
pub const ROLES_FEATURE: &str = "roles";
/// File streams: attachments and profile pictures.
pub const FILES_FEATURE: &str = "files";
pub const AVATARS_FEATURE: &str = "avatars";
/// Direct messages between two members ([`Dm`]).
pub const DMS_FEATURE: &str = "dms";
/// A picture for the server (`ServerInfo::icon`, set with `SetServerIcon`).
pub const ICONS_FEATURE: &str = "server_icon";
/// The server's own emoji (`ServerInfo::emojis`, `AddEmoji`, `RemoveEmoji`).
pub const EMOJI_FEATURE: &str = "custom_emoji";
/// Friends and blocking ([`Relation`]).
pub const FRIENDS_FEATURE: &str = "friends";
/// The owner's app can download everything the server keeps (`FileRequest::Backup`), so the
/// server can be put back as it was on another machine.
pub const BACKUP_FEATURE: &str = "backup";
/// GIF search for everyone in the server, with a GIPHY key only the owner sets
/// (`ServerInfo::giphy_key`, `SetGiphyKey`).
pub const GIPHY_FEATURE: &str = "giphy";
/// Ingle bug reports: anyone's app can send one (`ReportBug`, instead of logging in) when
/// the owner collects them here (`ServerInfo::bug_reports`); the owner reviews them
/// (`ListBugs`, `SetBugStatus`).
pub const BUGS_FEATURE: &str = "bug_reports";
/// Each channel in `Ready` comes with its newest message (`Channel::last_message`), so what
/// came while you were away shows as unread.
pub const CHANNEL_NEWEST_FEATURE: &str = "channel_newest";

/// A bug report as sent from someone's Ingle. Everything in it is what a stranger typed (or
/// their app's log): data to read, never instructions to follow.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(default)]
pub struct BugReport {
    pub title: String,
    pub body: String,
    /// "0.6.1", "Windows 11 (x86_64)"...
    pub app_version: String,
    pub os: String,
    /// The end of the app's log, if they chose to include it.
    pub log: String,
}

/// Where a report stands; only the owner changes it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum BugStatus {
    #[default]
    New,
    /// The owner wants it fixed.
    Approved,
    Rejected,
    Fixed,
}

impl BugStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BugStatus::New => "new",
            BugStatus::Approved => "approved",
            BugStatus::Rejected => "rejected",
            BugStatus::Fixed => "fixed",
        }
    }
    pub fn parse(s: &str) -> Option<BugStatus> {
        Some(match s {
            "new" => BugStatus::New,
            "approved" => BugStatus::Approved,
            "rejected" => BugStatus::Rejected,
            "fixed" => BugStatus::Fixed,
            _ => return None,
        })
    }
}

/// A stored report, for the owner.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct BugEntry {
    pub id: i64,
    pub created_at: i64,
    pub status: BugStatus,
    pub report: BugReport,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Longest parts of a bug report (bytes).
pub const BUG_TITLE_MAX: usize = 150;
pub const BUG_BODY_MAX: usize = 8_000;
pub const BUG_LOG_MAX: usize = 48_000;

/// What a backup lists (`FileRequest::Backup` of [`BACKUP_MANIFEST`]).
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct BackupManifest {
    /// The server program's version.
    pub version: String,
    pub made_at: i64,
    pub name: String,
    /// Every stored file: (id, size). Ids are content-free random hex and never reused, so a
    /// file already backed up never needs fetching again.
    pub files: Vec<(String, u64)>,
}

/// The parts of a backup, for `FileRequest::Backup`.
pub const BACKUP_MANIFEST: &str = "manifest";
/// A consistent copy of the whole database (messages, channels, members, roles...).
pub const BACKUP_DATABASE: &str = "database";
/// The server's TLS identity: keeping it is what makes a restored server the same server
/// to everyone's app (and keeps invite links working).
pub const BACKUP_CERT: &str = "server-cert.der";
pub const BACKUP_KEY: &str = "server-key.der";
/// `file:<id>`: one stored file (attachment, picture, emoji).
pub const BACKUP_FILE_PREFIX: &str = "file:";

/// Where you stand with someone, as you see it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RelationKind {
    Friend,
    /// They asked to be your friend.
    Incoming,
    /// You asked them.
    Outgoing,
    /// You blocked them (they aren't told).
    Blocked,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Relation {
    pub user_id: UserId,
    pub kind: RelationKind,
}

/// A private conversation between me and one other member ([`DMS_FEATURE`]). It works like a
/// text channel (`SendMessage`, `FetchHistory`, `Typing`... with its `id`), but only the two
/// of us can see it, and only its author can delete a message.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Dm {
    pub id: ChannelId,
    /// The other person.
    pub with: UserId,
    /// Newest message, for sorting the list; none while it's empty.
    #[serde(default)]
    pub last_message: Option<MessageId>,
    /// Who wrote it (so my own last message doesn't show as unread).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_from: Option<UserId>,
}

/// Largest attachment and profile picture the server accepts.
pub const MAX_ATTACHMENT_BYTES: u64 = 10 << 20;
pub const MAX_AVATAR_BYTES: u64 = 1 << 20;
pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 10;

/// File streams. The client opens a new bidirectional stream, writes [`FILE_PREAMBLE`], one
/// length-prefixed JSON [`FileRequest`] (and for a put, exactly `size` bytes), and finishes
/// its side. The server answers with one length-prefixed JSON [`FileReply`], followed by the
/// file's bytes for a get.
pub const FILE_PREAMBLE: &[u8; 4] = b"INGF";

/// A profile picture's id: the first 16 bytes of SHA-256(user id ‖ image), in hex. Named by
/// content, so an app can tell whether a server already has its current picture.
pub fn avatar_id(uid: &str, bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(uid.as_bytes());
    h.update(bytes);
    h.finalize()[..16].iter().map(|b| format!("{b:02x}")).collect()
}

/// Width and height of a PNG, GIF, JPEG or WebP image, read from its header.
pub fn image_size(b: &[u8]) -> Option<(u32, u32)> {
    let be32 = |i: usize| b.get(i..i + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]));
    let le16 = |i: usize| b.get(i..i + 2).map(|x| u16::from_le_bytes([x[0], x[1]]) as u32);
    let be16 = |i: usize| b.get(i..i + 2).map(|x| u16::from_be_bytes([x[0], x[1]]) as u32);
    let le24 = |i: usize| b.get(i..i + 3).map(|x| x[0] as u32 | (x[1] as u32) << 8 | (x[2] as u32) << 16);
    match sniff_image(b)? {
        "image/png" => Some((be32(16)?, be32(20)?)),
        "image/gif" => Some((le16(6)?, le16(8)?)),
        "image/jpeg" => {
            // Walk the markers to the first start-of-frame.
            let mut i = 2;
            while i + 9 < b.len() {
                if b[i] != 0xff {
                    i += 1;
                    continue;
                }
                let marker = b[i + 1];
                if (0xc0..=0xcf).contains(&marker) && ![0xc4, 0xc8, 0xcc].contains(&marker) {
                    return Some((be16(i + 7)?, be16(i + 5)?));
                }
                if marker == 0xd8 || marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                    i += 2;
                    continue;
                }
                i += 2 + be16(i + 2)? as usize;
            }
            None
        }
        "image/webp" => match b.get(12..16)? {
            b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
            b"VP8L" => {
                let v = u32::from_le_bytes([*b.get(21)?, *b.get(22)?, *b.get(23)?, *b.get(24)?]);
                Some(((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1))
            }
            b"VP8 " => Some((le16(26)? & 0x3fff, le16(28)? & 0x3fff)),
            _ => None,
        },
        _ => None,
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum FileRequest {
    Get { id: String },
    /// Upload an attachment (then send it with `SendMessage::attachments`) or a profile
    /// picture (then `SetAvatar`).
    Put { kind: FileKind, name: String, mime: String, size: u64 },
    /// Owner only ([`BACKUP_FEATURE`]): one part of a backup (see [`BACKUP_MANIFEST`]).
    Backup { item: String },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Attachment,
    Avatar,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FileReply {
    pub ok: bool,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub error: String,
}

/// The kind of image in `bytes`, from its first bytes (PNG, JPEG, GIF and WebP only).
/// Largest picture side Ingle accepts (attachments, avatars, emoji).
pub const MAX_IMAGE_SIDE: u32 = 8192;
/// Animated GIFs: largest canvas side, most frames, and most pixels decoded in all.
pub const MAX_GIF_SIDE: u32 = 4096;
pub const MAX_GIF_FRAMES: usize = 1000;
pub const MAX_GIF_TOTAL_PIXELS: u64 = 600_000_000;

/// Whether a picture's declared size is safe to decode: every image format says how big the
/// picture is up front, and a tiny file can claim to be gigantic (a "decompression bomb").
pub fn image_dimensions_ok(b: &[u8]) -> bool {
    let Some((w, h)) = image_size(b) else { return false };
    if w == 0 || h == 0 || w > MAX_IMAGE_SIDE || h > MAX_IMAGE_SIDE {
        return false;
    }
    if sniff_image(b) == Some("image/gif") {
        return gif_ok(b, w, h);
    }
    true
}

/// Walks a GIF's blocks: every frame must fit the limits above.
fn gif_ok(b: &[u8], screen_w: u32, screen_h: u32) -> bool {
    let mut i = 13usize;
    let flags = *b.get(10).unwrap_or(&0);
    if flags & 0x80 != 0 {
        i += 3 * (1usize << ((flags & 7) + 1));
    }
    let skip_sub_blocks = |mut i: usize| -> Option<usize> {
        loop {
            let n = *b.get(i)? as usize;
            i += 1;
            if n == 0 {
                return Some(i);
            }
            i += n;
        }
    };
    let canvas = screen_w.max(1) as u64 * screen_h.max(1) as u64;
    let mut frames = 0usize;
    let mut pixels = 0u64;
    let mut frame_side = 0u32;
    while let Some(&tag) = b.get(i) {
        match tag {
            0x3b => break,
            0x21 => match skip_sub_blocks(i + 2) {
                Some(next) => i = next,
                None => break,
            },
            0x2c => {
                let Some(d) = b.get(i + 1..i + 10) else { break };
                let (left, top) = (u16::from_le_bytes([d[0], d[1]]) as u32, u16::from_le_bytes([d[2], d[3]]) as u32);
                let (fw, fh) = (u16::from_le_bytes([d[4], d[5]]) as u32, u16::from_le_bytes([d[6], d[7]]) as u32);
                if left + fw > MAX_IMAGE_SIDE || top + fh > MAX_IMAGE_SIDE {
                    return false;
                }
                frames += 1;
                pixels += canvas.max(fw as u64 * fh as u64);
                frame_side = frame_side.max(fw).max(fh);
                let mut j = i + 10;
                if d[8] & 0x80 != 0 {
                    j += 3 * (1usize << ((d[8] & 7) + 1));
                }
                match skip_sub_blocks(j + 1) {
                    Some(next) => i = next,
                    None => break,
                }
            }
            _ => break,
        }
    }
    if frames > 1
        && (screen_w > MAX_GIF_SIDE
            || screen_h > MAX_GIF_SIDE
            || frame_side > MAX_GIF_SIDE
            || frames > MAX_GIF_FRAMES
            || pixels > MAX_GIF_TOTAL_PIXELS)
    {
        return false;
    }
    true
}

pub fn sniff_image(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("image/webp")
    } else {
        None
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// base64url SHA-256 of the server's TLS certificate (stable server identity).
    #[serde(default)]
    pub fingerprint: String,
    /// The server's icon ([`ICONS_FEATURE`]): a file id, fetched like a profile picture.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// The server's own emoji ([`EMOJI_FEATURE`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emojis: Vec<CustomEmoji>,
    /// The owner collects Ingle bug reports on this server ([`BUGS_FEATURE`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bug_reports: bool,
    /// How many reports are waiting to be looked at.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub new_bug_reports: u64,
    /// The GIPHY key the owner set up for everyone's GIF search here ([`GIPHY_FEATURE`]).
    /// Empty: GIF search is off on this server.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub giphy_key: String,
}

/// One of a server's own emoji. Written in messages as `<:name:id>` (`<a:name:id>` when
/// animated); the picture is a file uploaded as [`FileKind::Avatar`], fetched by its id.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct CustomEmoji {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub animated: bool,
    /// Who added it.
    #[serde(default)]
    pub by: UserId,
}

/// How many emoji a server can have.
pub const MAX_CUSTOM_EMOJI: usize = 250;

/// Emoji names: 2–32 letters, digits and underscores.
pub fn valid_emoji_name(name: &str) -> bool {
    (2..=32).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
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
    /// Who's watching (screen shares; set by the server).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub viewers: Vec<SessionId>,
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

/// Your own mute and deafen, as the server last saw them.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(default)]
pub struct SelfVoiceState {
    pub self_mute: bool,
    pub self_deaf: bool,
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
        /// Files uploaded with a file stream first (needs [`FILES_FEATURE`]).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<String>,
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
    // ---- roles ([`ROLES_FEATURE`])
    CreateRole {
        name: String,
        color: u32,
        perms: u64,
        #[serde(default)]
        hoist: bool,
    },
    UpdateRole {
        role_id: RoleId,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        color: Option<u32>,
        #[serde(default)]
        perms: Option<u64>,
        #[serde(default)]
        hoist: Option<bool>,
        /// Move it up (+1) or down (-1) in the order.
        #[serde(default)]
        shift: i32,
    },
    DeleteRole {
        role_id: RoleId,
    },
    SetMemberRoles {
        user_id: UserId,
        roles: Vec<RoleId>,
    },
    /// Profile picture ([`AVATARS_FEATURE`]): a file uploaded as [`FileKind::Avatar`], or none.
    SetAvatar {
        #[serde(default)]
        file_id: Option<String>,
    },
    /// Add one of the server's own emoji ([`EMOJI_FEATURE`]; needs Manage emoji): a picture
    /// uploaded as [`FileKind::Avatar`].
    AddEmoji {
        name: String,
        file_id: String,
        #[serde(default)]
        animated: bool,
    },
    /// Remove one (needs Manage emoji), or rename it when `name` is given.
    RemoveEmoji {
        id: String,
    },
    RenameEmoji {
        id: String,
        name: String,
    },
    /// Ask someone to be friends ([`FRIENDS_FEATURE`]); accepts if they already asked you.
    FriendRequest {
        user_id: UserId,
    },
    FriendAccept {
        user_id: UserId,
    },
    /// Decline, cancel or unfriend.
    FriendRemove {
        user_id: UserId,
    },
    /// Block someone: no direct messages or friend requests between you, and any friendship
    /// ends. They aren't told.
    Block {
        user_id: UserId,
    },
    Unblock {
        user_id: UserId,
    },
    /// Set or remove the server's icon ([`ICONS_FEATURE`]; needs Manage server): a file
    /// uploaded as [`FileKind::Avatar`].
    SetServerIcon {
        #[serde(default)]
        file_id: Option<String>,
    },
    /// Owner only ([`GIPHY_FEATURE`]): the GIPHY key everyone's GIF search here uses (empty
    /// turns GIF search off).
    SetGiphyKey {
        key: String,
    },
    /// Instead of `Login` ([`BUGS_FEATURE`]): send a bug report, no account needed.
    ReportBug {
        report: BugReport,
    },
    /// Owner only: the reports (newest first), answered with `Bugs`.
    ListBugs,
    /// Owner only.
    SetBugStatus {
        id: i64,
        status: BugStatus,
    },
    /// Owner only: collect bug reports here, or stop.
    SetBugReports {
        on: bool,
    },
    /// Start (or find) the direct message conversation with a member ([`DMS_FEATURE`]).
    /// Answered with `DmOpened`.
    OpenDm {
        user_id: UserId,
    },
    /// A request this server doesn't know (from a newer app).
    #[serde(other)]
    Unknown,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        roles: Vec<RoleDef>,
        /// What this server supports beyond protocol v1 (see [`ROLES_FEATURE`] and friends).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        features: Vec<String>,
        /// My direct message conversations ([`DMS_FEATURE`]).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        dms: Vec<Dm>,
        /// My friends, friend requests and blocks ([`FRIENDS_FEATURE`]).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        relations: Vec<Relation>,
    },
    Ok,
    /// Reply to `ListBugs`.
    Bugs {
        reports: Vec<BugEntry>,
    },
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
        /// Muted and deafened as you were when you left voice here, if that was only a few
        /// minutes ago: an app that doesn't know (one just updated from a version that didn't
        /// remember) comes back the way you were.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before: Option<SelfVoiceState>,
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
    /// Where I stand with someone changed (none: nothing between us now).
    RelationUpdated {
        user_id: UserId,
        #[serde(default)]
        kind: Option<RelationKind>,
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
    RoleUpserted {
        role: RoleDef,
    },
    RoleDeleted {
        role_id: RoleId,
    },
    /// A direct message conversation I'm in was started (by me or the other person).
    DmOpened {
        dm: Dm,
    },
    /// An event this app doesn't know (from a newer server); ignored.
    #[serde(other)]
    Unknown,
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
        .filter(|c| !c.is_control() && !is_invisible(*c))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let trimmed: String = collapsed.chars().take(max_chars).collect();
    if trimmed.is_empty() { None } else { Some(trimmed) }
}

/// Characters that show as nothing (zero-width spaces and joiners, text-direction overrides,
/// blank "filler" letters...): kept out of names so nobody is invisible or look-alike, and
/// no name flips the text around it.
pub fn is_invisible(c: char) -> bool {
    matches!(c,
        '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{115F}' | '\u{1160}' | '\u{17B4}' | '\u{17B5}' | '\u{180B}'..='\u{180F}'
        | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{206F}' | '\u{2800}' | '\u{3164}'
        | '\u{FE00}'..='\u{FE0E}' | '\u{FEFF}' | '\u{FFA0}' | '\u{FFF0}'..='\u{FFF8}' | '\u{1D159}' | '\u{1D173}'..='\u{1D17A}'
        | '\u{E0000}'..='\u{E007F}')
}

/// A member's display name: like [`clean_name`] (32 characters), and not one that would read
/// as a mention of everybody.
pub fn clean_member_name(raw: &str) -> Option<String> {
    let name = clean_name(raw, 32)?;
    let bare = name.trim_start_matches('@').to_lowercase();
    if bare == "everyone" || bare == "here" {
        return None;
    }
    Some(name)
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
    fn unknown_events_and_requests_are_tolerated() {
        let f: ServerFrame = serde_json::from_str(r#"{"ev":{"type":"something_new","x":1}}"#).unwrap();
        assert!(matches!(f.ev, Event::Unknown));
        let r: ClientFrame = serde_json::from_str(r#"{"id":4,"req":{"op":"do_new_thing","y":[1]}}"#).unwrap();
        assert!(matches!(r.req, Request::Unknown));
        // Old servers' Ready (no roles, no features) still parses.
        let f: ServerFrame = serde_json::from_str(
            r#"{"ev":{"type":"ready","me":{"user_id":"a","name":"A","role":"owner","online":true},"server":{"name":"S"},"channels":[],"members":[],"voice":[]}}"#,
        )
        .unwrap();
        assert!(matches!(f.ev, Event::Ready { ref features, .. } if features.is_empty()));
    }

    #[test]
    fn voice_ready_with_and_without_how_you_were() {
        // From servers that don't remember (and as old apps see new servers' replies: the
        // extra field is just skipped).
        let f: ServerFrame =
            serde_json::from_str(r#"{"re":2,"ev":{"type":"voice_ready","session":7,"participants":[]}}"#).unwrap();
        assert!(matches!(f.ev, Event::VoiceReady { session: 7, before: None, .. }));
        let ev = Event::VoiceReady {
            session: 7,
            participants: Vec::new(),
            before: Some(SelfVoiceState { self_mute: true, self_deaf: false }),
        };
        let s = serde_json::to_string(&ev).unwrap();
        assert_eq!(s, r#"{"type":"voice_ready","session":7,"participants":[],"before":{"self_mute":true,"self_deaf":false}}"#);
        let back: Event = serde_json::from_str(&s).unwrap();
        assert!(matches!(back, Event::VoiceReady { before: Some(SelfVoiceState { self_mute: true, self_deaf: false }), .. }));
        let none = serde_json::to_string(&Event::VoiceReady { session: 7, participants: Vec::new(), before: None }).unwrap();
        assert!(!none.contains("before"), "{none}");
    }

    #[test]
    fn sniffs_images() {
        assert_eq!(sniff_image(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(sniff_image(b"GIF89a...."), Some("image/gif"));
        assert_eq!(sniff_image(&[0xff, 0xd8, 0xff, 0xe0]), Some("image/jpeg"));
        assert_eq!(sniff_image(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
        assert_eq!(sniff_image(b"MZ\x90\0 an exe"), None);
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((640, 480)));
        assert_eq!(image_size(b"GIF89a\x40\x01\xf0\x00"), Some((320, 240)));
    }

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
