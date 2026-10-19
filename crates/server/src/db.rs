//! SQLite persistence: members, channels, message history, reactions and invites.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use pyr_proto::control::{CustomEmoji, Relation, RelationKind, ServerGif, ServerSound,
    Attachment, Channel, ChannelId, ChannelKind, Dm, FileKind, Group, Member, Message, MessageId, Reaction, Role, RoleDef, RoleId, UserId,
};
use pyr_proto::now_ms;
use pyr_proto::sealed::{KeyRef, KeyShare, MessageKey, NewShare, PLACEHOLDER, Sealed};
use parking_lot::Mutex;
use rusqlite::{Connection, OptionalExtension, params};

pub struct Db {
    conn: Mutex<Connection>,
}

pub struct UserRow {
    pub name: String,
    pub role: Role,
    pub banned: bool,
    pub joined_at: i64,
    pub avatar: Option<String>,
    /// Joined with a bot invite (bots, like Discord's).
    pub bot: bool,
}

/// A stored file (attachment or profile picture).
#[derive(Clone, Debug)]
pub struct FileRow {
    pub id: String,
    pub owner: String,
    pub kind: FileKind,
    pub name: String,
    pub mime: String,
    pub size: u64,
    pub width: u32,
    pub height: u32,
    pub message_id: Option<MessageId>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum InviteError {
    NotFound,
    Expired,
    UsedUp,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS poker_records (
    uid TEXT PRIMARY KEY,
    record TEXT NOT NULL,
    held INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    role INTEGER NOT NULL DEFAULT 0,
    banned INTEGER NOT NULL DEFAULT 0,
    joined_at INTEGER NOT NULL,
    last_seen INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS channels (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind INTEGER NOT NULL,
    name TEXT NOT NULL,
    topic TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id INTEGER NOT NULL,
    author TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    edited_at INTEGER,
    reply_to INTEGER
);
CREATE INDEX IF NOT EXISTS messages_by_channel ON messages(channel_id, id);
CREATE TABLE IF NOT EXISTS reactions (
    message_id INTEGER NOT NULL,
    emoji TEXT NOT NULL,
    user_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (message_id, emoji, user_id)
);
CREATE TABLE IF NOT EXISTS invites (
    code TEXT PRIMARY KEY,
    created_by TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    max_uses INTEGER,
    uses INTEGER NOT NULL DEFAULT 0,
    expires_at INTEGER,
    grants_role INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS roles (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    color INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL DEFAULT 0,
    perms INTEGER NOT NULL DEFAULT 0,
    hoist INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS member_roles (
    user_id TEXT NOT NULL,
    role_id INTEGER NOT NULL,
    PRIMARY KEY (user_id, role_id)
);
CREATE TABLE IF NOT EXISTS files (
    id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    kind INTEGER NOT NULL,
    name TEXT NOT NULL,
    mime TEXT NOT NULL,
    size INTEGER NOT NULL,
    width INTEGER NOT NULL DEFAULT 0,
    height INTEGER NOT NULL DEFAULT 0,
    message_id INTEGER,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS files_by_message ON files(message_id);
-- Direct messages: each is a hidden row in `channels` (kind 2) between two members, a < b.
CREATE TABLE IF NOT EXISTS dms (
    channel_id INTEGER PRIMARY KEY,
    a TEXT NOT NULL,
    b TEXT NOT NULL,
    UNIQUE (a, b)
);
CREATE INDEX IF NOT EXISTS dms_by_b ON dms(b);
-- Group conversations: each a hidden row in `channels` (kind 3) with its members.
CREATE TABLE IF NOT EXISTS groups (
    channel_id INTEGER PRIMARY KEY,
    owner TEXT NOT NULL,
    name TEXT NOT NULL DEFAULT ''
);
CREATE TABLE IF NOT EXISTS group_members (
    channel_id INTEGER NOT NULL,
    user_id TEXT NOT NULL,
    PRIMARY KEY (channel_id, user_id)
);
CREATE INDEX IF NOT EXISTS group_members_by_user ON group_members(user_id);
-- Friends and blocks, directed: kind 0 = a asked b to be friends, 1 = friends (a row each
-- way), 2 = a blocked b.
CREATE TABLE IF NOT EXISTS relations (
    a TEXT NOT NULL,
    b TEXT NOT NULL,
    kind INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (a, b, kind)
);
CREATE INDEX IF NOT EXISTS relations_by_b ON relations(b);
-- The server's own emoji (the picture is a kind-1 file with the same id).
CREATE TABLE IF NOT EXISTS emojis (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    animated INTEGER NOT NULL DEFAULT 0,
    by TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
-- Server GIFs (0.8.0), like Discord's stickers: pictures everyone here can send. Their files
-- are attachments (kind 0) that were never sent, kept for the server.
CREATE TABLE IF NOT EXISTS gifs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    width INTEGER NOT NULL DEFAULT 0,
    height INTEGER NOT NULL DEFAULT 0,
    by TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
-- Soundboard sounds: their files are in `files` (kind 2).
CREATE TABLE IF NOT EXISTS sounds (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    emoji TEXT NOT NULL DEFAULT '',
    volume REAL NOT NULL DEFAULT 1,
    by TEXT NOT NULL,
    length_ms INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
-- Pyr bug reports sent to this server (when its owner collects them). Whoever sent one
-- isn't recorded: no account, no address.
CREATE TABLE IF NOT EXISTS bug_reports (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'new',
    status_at INTEGER NOT NULL DEFAULT 0,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    app_version TEXT NOT NULL DEFAULT '',
    os TEXT NOT NULL DEFAULT '',
    log TEXT NOT NULL DEFAULT ''
);
-- Plugins and skins sent in for the Pyr Library (when the owner collects them): Rust source
-- or a skin's files, waiting for review. Whoever sent one isn't recorded, beyond what they
-- wrote in it and, when they signed it (0.7.5), their Pyr id (`author_id`, added by migrate):
-- that's what lets them send new versions of it and see where theirs stand.
CREATE TABLE IF NOT EXISTS plugin_submissions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'new',
    status_at INTEGER NOT NULL DEFAULT 0,
    name TEXT NOT NULL,
    kind TEXT NOT NULL DEFAULT '',
    version TEXT NOT NULL DEFAULT '',
    summary TEXT NOT NULL DEFAULT '',
    author TEXT NOT NULL DEFAULT '',
    contact TEXT NOT NULL DEFAULT '',
    app_version TEXT NOT NULL DEFAULT '',
    file_count INTEGER NOT NULL DEFAULT 0,
    size INTEGER NOT NULL DEFAULT 0,
    files TEXT NOT NULL
);
-- Vouches (0.7.6): whom a member says they made sure of (`Vouch`), signed by them, so the
-- server can keep and pass them on but never make one up. Only for members here.
CREATE TABLE IF NOT EXISTS vouches (
    by TEXT NOT NULL,
    for_uid TEXT NOT NULL,
    sig TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (by, for_uid)
);
-- Author names in the Pyr Library (0.8): the first signed sender to send a plugin or skin in
-- under a name has it, by `author_key` (so names that only look different are the same one).
CREATE TABLE IF NOT EXISTS author_names (
    key TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    author_id TEXT NOT NULL,
    at INTEGER NOT NULL
);
-- End-to-end encrypted messages (0.9, see `pyr_proto::sealed`): members' message keys (signed
-- by them), the conversation keys they give each other (each wrapped for its receiver: the
-- server can't open them), and the keys someone asked for that nobody's given them yet.
CREATE TABLE IF NOT EXISTS message_keys (
    user_id TEXT PRIMARY KEY,
    key TEXT NOT NULL,
    sig TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS key_shares (
    channel_id INTEGER NOT NULL,
    owner TEXT NOT NULL,
    key_id TEXT NOT NULL,
    recipient TEXT NOT NULL,
    giver TEXT NOT NULL,
    nonce TEXT NOT NULL,
    ct TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (channel_id, owner, key_id, recipient, giver)
);
CREATE INDEX IF NOT EXISTS key_shares_by_recipient ON key_shares(recipient, channel_id);
CREATE TABLE IF NOT EXISTS key_wants (
    channel_id INTEGER NOT NULL,
    owner TEXT NOT NULL,
    key_id TEXT NOT NULL,
    user_id TEXT NOT NULL,
    at INTEGER NOT NULL,
    PRIMARY KEY (channel_id, owner, key_id, user_id)
);
CREATE INDEX IF NOT EXISTS key_wants_by_user ON key_wants(user_id);
"#;

/// What `Db::submission_row` reads, in its order.
const SUBMISSION_COLUMNS: &str =
    "id, created_at, status, name, kind, version, summary, author, contact, app_version, file_count, size, author_id, update_of, changes, note";

/// `channels.kind` of a direct message conversation (never listed as a channel).
const DM_KIND: i64 = 2;
/// `channels.kind` of a group conversation (never listed as a channel either).
const GROUP_KIND: i64 = 3;

fn file_kind_to_i(k: FileKind) -> i64 {
    match k {
        FileKind::Attachment => 0,
        FileKind::Avatar => 1,
        FileKind::Sound => 2,
        FileKind::Sealed => 3,
    }
}

fn file_kind_from_i(v: i64) -> FileKind {
    match v {
        1 => FileKind::Avatar,
        2 => FileKind::Sound,
        3 => FileKind::Sealed,
        _ => FileKind::Attachment,
    }
}

fn kind_to_i(k: ChannelKind) -> i64 {
    match k {
        ChannelKind::Text => 0,
        ChannelKind::Voice => 1,
    }
}

fn kind_from_i(v: i64) -> ChannelKind {
    if v == 1 { ChannelKind::Voice } else { ChannelKind::Text }
}

/// Role ids as kept in a channel's row ("3,7").
fn role_list(ids: &[RoleId]) -> String {
    ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",")
}

fn parse_roles(s: &str) -> Vec<RoleId> {
    s.split(',').filter_map(|x| x.trim().parse().ok()).collect()
}

/// A channel from a row of `CHANNEL_COLUMNS` (then its newest message and author, if asked for).
fn channel_row(r: &rusqlite::Row, newest: bool) -> rusqlite::Result<Channel> {
    Ok(Channel {
        id: r.get(0)?,
        kind: kind_from_i(r.get(1)?),
        name: r.get(2)?,
        topic: r.get(3)?,
        position: r.get(4)?,
        private: r.get::<_, i64>(5)? != 0,
        view_roles: parse_roles(&r.get::<_, String>(6)?),
        read_only: r.get::<_, i64>(7)? != 0,
        post_roles: parse_roles(&r.get::<_, String>(8)?),
        last_message: if newest { r.get(9)? } else { None },
        last_from: if newest { r.get(10)? } else { None },
    })
}

const CHANNEL_COLUMNS: &str = "c.id, c.kind, c.name, c.topic, c.position, c.private, c.view_roles, c.read_only, c.post_roles";

/// Columns added after the first release.
fn migrate(c: &Connection) -> Result<()> {
    let has_avatar = c
        .prepare("SELECT 1 FROM pragma_table_info('users') WHERE name = 'avatar'")?
        .exists([])?;
    if !has_avatar {
        c.execute_batch("ALTER TABLE users ADD COLUMN avatar TEXT")?;
    }
    // Which invite each member joined with (0.7.1): banning someone turns it off.
    let has_joined_with = c.prepare("SELECT 1 FROM pragma_table_info('users') WHERE name = 'joined_with'")?.exists([])?;
    if !has_joined_with {
        c.execute_batch("ALTER TABLE users ADD COLUMN joined_with TEXT")?;
    }
    // Private and read-only channels (0.8): the roles are comma-separated ids.
    let has_access = c.prepare("SELECT 1 FROM pragma_table_info('channels') WHERE name = 'private'")?.exists([])?;
    if !has_access {
        c.execute_batch(
            "ALTER TABLE channels ADD COLUMN private INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE channels ADD COLUMN view_roles TEXT NOT NULL DEFAULT '';
             ALTER TABLE channels ADD COLUMN read_only INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE channels ADD COLUMN post_roles TEXT NOT NULL DEFAULT '';",
        )?;
    }
    // Signed submissions and new versions of them, and the owner's note to the sender (0.7.5).
    for (column, kind) in [("author_id", "TEXT NOT NULL DEFAULT ''"), ("update_of", "INTEGER NOT NULL DEFAULT 0"), ("changes", "TEXT NOT NULL DEFAULT ''"), ("note", "TEXT NOT NULL DEFAULT ''")] {
        let has = c.prepare(&format!("SELECT 1 FROM pragma_table_info('plugin_submissions') WHERE name = '{column}'"))?.exists([])?;
        if !has {
            c.execute_batch(&format!("ALTER TABLE plugin_submissions ADD COLUMN {column} {kind}"))?;
        }
    }
    c.execute_batch("CREATE INDEX IF NOT EXISTS plugin_submissions_author ON plugin_submissions(author_id)")?;
    // Author names (0.8): the names on what was sent in signed before, first come first served.
    let filled = c.query_row("SELECT value FROM meta WHERE key = 'author_names_filled'", [], |r| r.get::<_, String>(0)).is_ok();
    if !filled {
        let sent: Vec<(String, String, String, i64)> = c
            .prepare("SELECT author, author_id, files, created_at FROM plugin_submissions WHERE author_id != '' ORDER BY id")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .flatten()
            .collect();
        for (author, author_id, files, at) in sent {
            let files: Vec<pyr_proto::control::SourceFile> = serde_json::from_str(&files).unwrap_or_default();
            for name in [author, skin_author(&files)] {
                let key = pyr_proto::control::author_key(&name);
                if !key.is_empty() {
                    c.execute("INSERT OR IGNORE INTO author_names(key, name, author_id, at) VALUES (?1, ?2, ?3, ?4)", params![key, name.trim(), author_id, at])?;
                }
            }
        }
        c.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('author_names_filled', '1')", [])?;
    }
    // Bots (0.8): members who joined with a bot invite, and the invites that make them.
    let has_bots = c.prepare("SELECT 1 FROM pragma_table_info('users') WHERE name = 'bot'")?.exists([])?;
    if !has_bots {
        c.execute_batch(
            "ALTER TABLE users ADD COLUMN bot INTEGER NOT NULL DEFAULT 0;
             ALTER TABLE invites ADD COLUMN bot INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    // Short links (0.8): invites the server lists at pyr.gg (see `crate::directory`).
    let has_listed = c.prepare("SELECT 1 FROM pragma_table_info('invites') WHERE name = 'listed'")?.exists([])?;
    if !has_listed {
        c.execute_batch("ALTER TABLE invites ADD COLUMN listed INTEGER NOT NULL DEFAULT 0")?;
    }
    // End-to-end encrypted messages (0.9): the ciphertext (`Sealed`, as JSON), `content` then empty.
    let has_sealed = c.prepare("SELECT 1 FROM pragma_table_info('messages') WHERE name = 'sealed'")?.exists([])?;
    if !has_sealed {
        c.execute_batch("ALTER TABLE messages ADD COLUMN sealed TEXT")?;
    }
    Ok(())
}

/// The author in a skin's own `skin.json` (empty when it isn't a skin, or says none).
pub fn skin_author(files: &[pyr_proto::control::SourceFile]) -> String {
    files
        .iter()
        .find(|f| f.path == pyr_proto::skin::MANIFEST)
        .and_then(|f| pyr_proto::b64::decode(&f.data).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v.get("author").and_then(|a| a.as_str()).map(str::to_string))
        .unwrap_or_default()
}

/// After deleting or editing: copy the zeroed pages into the database file and empty the
/// write-ahead log, so the old text isn't left in the log either.
fn scrub(c: &Connection) {
    if let Err(e) = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
        tracing::warn!("couldn't flush the database log after a delete: {e}");
    }
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path).with_context(|| format!("opening database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        // Deleted (and edited-away) text is overwritten with zeros, not left in free space
        // where a recovery tool could find it.
        conn.pragma_update(None, "secure_delete", "ON")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        conn.execute("INSERT OR IGNORE INTO meta(key, value) VALUES ('schema', '1')", [])?;
        // Databases from before secure delete may still hold old deleted text in free pages:
        // rebuild once to drop it.
        let scrubbed: Option<String> =
            conn.query_row("SELECT value FROM meta WHERE key = 'scrubbed'", [], |r| r.get(0)).ok();
        if scrubbed.is_none() {
            conn.execute_batch("VACUUM")?;
            conn.execute("INSERT OR REPLACE INTO meta(key, value) VALUES ('scrubbed', '1')", [])?;
            scrub(&conn);
        }
        Ok(Db { conn: Mutex::new(conn) })
    }

    pub fn open_in_memory() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        Ok(Db { conn: Mutex::new(conn) })
    }

    pub fn get_meta(&self, key: &str) -> Option<String> {
        self.conn
            .lock()
            .query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_meta(&self, key: &str, value: &str) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---------------------------------------------------------------- poker

    /// A player's poker record (a new one the first time), and the chips they have at tables.
    pub fn poker_record(&self, uid: &str) -> (pyr_poker::progress::Record, u64) {
        let row: Option<(String, i64)> = self.conn.lock().query_row("SELECT record, held FROM poker_records WHERE uid = ?1", [uid], |r| Ok((r.get(0)?, r.get(1)?))).optional().ok().flatten();
        match row {
            Some((json, held)) => (serde_json::from_str(&json).unwrap_or_else(|_| pyr_poker::progress::Record::new()), held.max(0) as u64),
            None => (pyr_poker::progress::Record::new(), 0),
        }
    }

    pub fn set_poker_record(&self, uid: &str, rec: &pyr_poker::progress::Record, held: u64) -> Result<()> {
        let json = serde_json::to_string(rec)?;
        self.conn.lock().execute(
            "INSERT INTO poker_records(uid, record, held) VALUES (?1, ?2, ?3) ON CONFLICT(uid) DO UPDATE SET record = excluded.record, held = excluded.held",
            params![uid, json, held as i64],
        )?;
        Ok(())
    }

    /// Every poker record (for the leaderboard).
    pub fn poker_records(&self) -> Vec<(String, pyr_poker::progress::Record, u64)> {
        let conn = self.conn.lock();
        let Ok(mut st) = conn.prepare("SELECT uid, record, held FROM poker_records") else { return Vec::new() };
        st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
            .map(|rows| rows.flatten().filter_map(|(u, j, h)| Some((u, serde_json::from_str(&j).ok()?, h.max(0) as u64))).collect())
            .unwrap_or_default()
    }

    /// When the server starts: chips that were at tables when it stopped go back to their owners.
    pub fn poker_return_held(&self) -> usize {
        let mut n = 0;
        for (uid, mut rec, held) in self.poker_records() {
            if held > 0 {
                rec.chips += held;
                if self.set_poker_record(&uid, &rec, 0).is_ok() {
                    n += 1;
                }
            }
        }
        n
    }

    // ---------------------------------------------------------------- users

    pub fn get_user(&self, id: &str) -> Option<UserRow> {
        self.conn
            .lock()
            .query_row("SELECT name, role, banned, joined_at, avatar, bot FROM users WHERE id = ?1", [id], |r| {
                Ok(UserRow {
                    name: r.get(0)?,
                    role: Role::from_i64(r.get(1)?),
                    banned: r.get::<_, i64>(2)? != 0,
                    joined_at: r.get(3)?,
                    avatar: r.get(4)?,
                    bot: r.get::<_, i64>(5)? != 0,
                })
            })
            .optional()
            .ok()
            .flatten()
    }

    /// Adds a member. False if they're banned (a ban made meanwhile is never undone here).
    pub fn insert_user(&self, id: &str, name: &str, role: Role) -> Result<bool> {
        self.insert_member(id, name, role, false)
    }

    /// Adds a member, or a bot. False if they're banned.
    pub fn insert_member(&self, id: &str, name: &str, role: Role, bot: bool) -> Result<bool> {
        let now = now_ms();
        let n = self.conn.lock().execute(
            "INSERT INTO users(id, name, role, banned, joined_at, last_seen, bot) VALUES (?1, ?2, ?3, 0, ?4, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, role = excluded.role, joined_at = excluded.joined_at,
             bot = excluded.bot WHERE users.banned = 0",
            params![id, name, role.as_i64(), now, bot as i64],
        )?;
        Ok(n > 0)
    }

    #[cfg(test)]
    pub fn set_joined_at(&self, id: &str, at: i64) -> Result<()> {
        self.conn.lock().execute("UPDATE users SET joined_at = ?2 WHERE id = ?1", params![id, at])?;
        Ok(())
    }

    pub fn touch_user(&self, id: &str, name: &str) -> Result<()> {
        self.conn
            .lock()
            .execute("UPDATE users SET name = ?2, last_seen = ?3 WHERE id = ?1", params![id, name, now_ms()])?;
        Ok(())
    }

    pub fn set_role(&self, id: &str, role: Role) -> Result<bool> {
        Ok(self.conn.lock().execute("UPDATE users SET role = ?2 WHERE id = ?1", params![id, role.as_i64()])? > 0)
    }

    /// The invite someone joined with (noted when they first came in).
    pub fn set_joined_with(&self, id: &str, code: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE users SET joined_with = ?2 WHERE id = ?1", params![id, code])?;
        Ok(())
    }

    /// Remove membership. A banned user keeps a row with `banned = 1`, and the invite they
    /// joined with stops working (or they'd be back with a new identity through it).
    pub fn remove_user(&self, id: &str, ban: bool) -> Result<()> {
        let c = self.conn.lock();
        if ban {
            let joined_with: Option<String> =
                c.query_row("SELECT joined_with FROM users WHERE id = ?1", [id], |r| r.get(0)).optional().ok().flatten().flatten();
            if let Some(code) = joined_with {
                c.execute("DELETE FROM invites WHERE code = ?1", [code])?;
            }
            c.execute("UPDATE users SET banned = 1, role = 0 WHERE id = ?1", [id])?;
            c.execute(
                "INSERT OR IGNORE INTO users(id, name, role, banned, joined_at) VALUES (?1, '', 0, 1, ?2)",
                params![id, now_ms()],
            )?;
        } else {
            c.execute("DELETE FROM users WHERE id = ?1", [id])?;
        }
        c.execute("DELETE FROM member_roles WHERE user_id = ?1", [id])?;
        // Invites they made stop working: a removed troll can't come back with them.
        c.execute("DELETE FROM invites WHERE created_by = ?1", [id])?;
        scrub(&c);
        Ok(())
    }

    pub fn members(&self) -> Vec<Member> {
        let c = self.conn.lock();
        let mut st = c
            .prepare("SELECT id, name, role, joined_at, avatar, bot FROM users WHERE banned = 0 ORDER BY joined_at")
            .expect("valid SQL");
        let mut members: Vec<Member> = st
            .query_map([], |r| {
                Ok(Member {
                    user_id: r.get(0)?,
                    name: r.get(1)?,
                    role: Role::from_i64(r.get(2)?),
                    online: false,
                    joined_at: r.get(3)?,
                    roles: Vec::new(),
                    avatar: r.get(4)?,
                    idle: false,
                    bot: r.get::<_, i64>(5)? != 0,
                    dnd: false,
                })
            })
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        let mut by_user: std::collections::HashMap<String, Vec<RoleId>> = Default::default();
        if let Ok(mut st) = c.prepare("SELECT user_id, role_id FROM member_roles ORDER BY role_id") {
            if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))) {
                for (u, r) in rows.flatten() {
                    by_user.entry(u).or_default().push(r);
                }
            }
        }
        for m in &mut members {
            m.roles = by_user.remove(&m.user_id).unwrap_or_default();
        }
        members
    }

    // ---------------------------------------------------------------- roles

    pub fn roles(&self) -> Vec<RoleDef> {
        let c = self.conn.lock();
        let mut st = c
            .prepare("SELECT id, name, color, position, perms, hoist FROM roles ORDER BY position DESC, id")
            .expect("valid SQL");
        st.query_map([], |r| {
            Ok(RoleDef {
                id: r.get(0)?,
                name: r.get(1)?,
                color: r.get::<_, i64>(2)? as u32,
                position: r.get(3)?,
                perms: r.get::<_, i64>(4)? as u64,
                hoist: r.get::<_, i64>(5)? != 0,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    pub fn role(&self, id: RoleId) -> Option<RoleDef> {
        self.roles().into_iter().find(|r| r.id == id)
    }

    /// A new role goes to the bottom of the order, like Discord.
    pub fn create_role(&self, name: &str, color: u32, perms: u64, hoist: bool) -> Result<RoleDef> {
        let c = self.conn.lock();
        let bottom: i64 = c.query_row("SELECT COALESCE(MIN(position), 1) FROM roles", [], |r| r.get(0))?;
        let position = (bottom - 1) as i32;
        c.execute(
            "INSERT INTO roles(name, color, position, perms, hoist) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name, color as i64, position, perms as i64, hoist as i64],
        )?;
        Ok(RoleDef { id: c.last_insert_rowid(), name: name.to_string(), color, position, perms, hoist })
    }

    pub fn save_role(&self, r: &RoleDef) -> Result<()> {
        self.conn.lock().execute(
            "UPDATE roles SET name = ?2, color = ?3, position = ?4, perms = ?5, hoist = ?6 WHERE id = ?1",
            params![r.id, r.name, r.color as i64, r.position, r.perms as i64, r.hoist as i64],
        )?;
        Ok(())
    }

    /// Deletes a role; returns the members who had it.
    pub fn delete_role(&self, id: RoleId) -> Result<Vec<String>> {
        let c = self.conn.lock();
        let mut st = c.prepare("SELECT user_id FROM member_roles WHERE role_id = ?1")?;
        let had: Vec<String> = st.query_map([id], |r| r.get(0))?.flatten().collect();
        drop(st);
        c.execute("DELETE FROM member_roles WHERE role_id = ?1", [id])?;
        c.execute("DELETE FROM roles WHERE id = ?1", [id])?;
        Ok(had)
    }

    pub fn member_roles(&self, uid: &str) -> Vec<RoleId> {
        let c = self.conn.lock();
        let Ok(mut st) = c.prepare("SELECT role_id FROM member_roles WHERE user_id = ?1 ORDER BY role_id") else {
            return Vec::new();
        };
        st.query_map([uid], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    pub fn set_member_roles(&self, uid: &str, roles: &[RoleId]) -> Result<()> {
        let c = self.conn.lock();
        c.execute("DELETE FROM member_roles WHERE user_id = ?1", [uid])?;
        for r in roles {
            c.execute("INSERT OR IGNORE INTO member_roles(user_id, role_id) VALUES (?1, ?2)", params![uid, r])?;
        }
        Ok(())
    }

    // ---------------------------------------------------------------- files

    pub fn insert_file(&self, f: &FileRow) -> Result<()> {
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO files(id, owner, kind, name, mime, size, width, height, message_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
            params![f.id, f.owner, file_kind_to_i(f.kind), f.name, f.mime, f.size as i64, f.width, f.height, now_ms()],
        )?;
        Ok(())
    }

    fn row_to_file(r: &rusqlite::Row<'_>) -> rusqlite::Result<FileRow> {
        Ok(FileRow {
            id: r.get(0)?,
            owner: r.get(1)?,
            kind: file_kind_from_i(r.get(2)?),
            name: r.get(3)?,
            mime: r.get(4)?,
            size: r.get::<_, i64>(5)? as u64,
            width: r.get(6)?,
            height: r.get(7)?,
            message_id: r.get(8)?,
        })
    }

    pub fn file(&self, id: &str) -> Option<FileRow> {
        self.conn
            .lock()
            .query_row(
                "SELECT id, owner, kind, name, mime, size, width, height, message_id FROM files WHERE id = ?1",
                [id],
                Self::row_to_file,
            )
            .optional()
            .ok()
            .flatten()
    }

    pub fn add_bug(&self, r: &pyr_proto::control::BugReport) -> Result<i64> {
        let c = self.conn.lock();
        c.execute(
            "INSERT INTO bug_reports(created_at, title, body, app_version, os, log) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now_ms(), r.title, r.body, r.app_version, r.os, r.log],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// Reports waiting for the owner to look at them.
    pub fn new_bug_count(&self) -> u64 {
        self.conn.lock().query_row("SELECT COUNT(*) FROM bug_reports WHERE status = 'new'", [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64
    }

    /// The ones waiting to be looked at first, then the rest; newest first in each.
    pub fn bugs(&self, limit: usize) -> Vec<pyr_proto::control::BugEntry> {
        use pyr_proto::control::{BugEntry, BugReport, BugStatus};
        let c = self.conn.lock();
        let Ok(mut q) = c.prepare(
            "SELECT id, created_at, status, title, body, app_version, os, log FROM bug_reports \
             ORDER BY (status = 'new') DESC, id DESC LIMIT ?1",
        )
        else {
            return Vec::new();
        };
        q.query_map([limit as i64], |r| {
            Ok(BugEntry {
                id: r.get(0)?,
                created_at: r.get(1)?,
                status: BugStatus::parse(&r.get::<_, String>(2)?).unwrap_or_default(),
                report: BugReport { title: r.get(3)?, body: r.get(4)?, app_version: r.get(5)?, os: r.get(6)?, log: r.get(7)? },
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    /// Report `id`'s status (None: there's no such report).
    pub fn bug_status(&self, id: i64) -> Option<pyr_proto::control::BugStatus> {
        let s: String = self.conn.lock().query_row("SELECT status FROM bug_reports WHERE id = ?1", [id], |r| r.get(0)).ok()?;
        pyr_proto::control::BugStatus::parse(&s)
    }

    /// Reports a release fixed, marked fixed (as the owner's Mark fixed does): only new and
    /// approved ones (one the owner rejected stays rejected). What became of each id.
    pub fn mark_bugs_fixed(&self, ids: &[i64]) -> Result<Vec<(i64, Option<pyr_proto::control::BugStatus>)>> {
        use pyr_proto::control::BugStatus;
        let mut out = Vec::new();
        for &id in ids {
            let was = self.bug_status(id);
            if matches!(was, Some(BugStatus::New | BugStatus::Approved)) {
                self.set_bug_status(id, BugStatus::Fixed)?;
            }
            out.push((id, was));
        }
        Ok(out)
    }

    pub fn set_bug_status(&self, id: i64, status: pyr_proto::control::BugStatus) -> Result<bool> {
        Ok(self
            .conn
            .lock()
            .execute("UPDATE bug_reports SET status = ?2, status_at = ?3 WHERE id = ?1", params![id, status.as_str(), now_ms()])?
            > 0)
    }

    /// Who has an author's name (by its `author_key`), if anyone: (the name as they first
    /// wrote it, their id).
    pub fn author_name(&self, key: &str) -> Option<(String, String)> {
        self.conn
            .lock()
            .query_row("SELECT name, author_id FROM author_names WHERE key = ?1", [key], |r| Ok((r.get(0)?, r.get(1)?)))
            .ok()
    }

    /// Gives `author_id` the name `name` (by `key`) unless someone has it: whether it's theirs.
    pub fn claim_author_name(&self, key: &str, name: &str, author_id: &str) -> Result<bool> {
        let c = self.conn.lock();
        c.execute("INSERT OR IGNORE INTO author_names(key, name, author_id, at) VALUES (?1, ?2, ?3, ?4)", params![key, name, author_id, now_ms()])?;
        let who: String = c.query_row("SELECT author_id FROM author_names WHERE key = ?1", [key], |r| r.get(0))?;
        Ok(who == author_id)
    }

    /// Tests: the author names forgotten, and filled in again from what was sent in.
    #[cfg(test)]
    pub fn refill_author_names(&self) {
        let c = self.conn.lock();
        c.execute_batch("DELETE FROM author_names; DELETE FROM meta WHERE key = 'author_names_filled';").unwrap();
        migrate(&c).unwrap();
    }

    /// The server's owners (their ids).
    pub fn owner_ids(&self) -> Vec<String> {
        let c = self.conn.lock();
        c.prepare("SELECT id FROM users WHERE role = 2 AND banned = 0")
            .and_then(|mut st| st.query_map([], |r| r.get(0)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default()
    }

    /// A plugin or skin sent in; `files` is its files as JSON, `size` their bytes in all.
    /// `sub.author_id` is stored only once its signature has been checked (the caller clears
    /// it otherwise); `changes` says how a new version differs from the one it updates.
    pub fn add_submission(&self, sub: &pyr_proto::control::PluginSubmission, files: &str, size: u64, changes: &str) -> Result<i64> {
        let c = self.conn.lock();
        c.execute(
            "INSERT INTO plugin_submissions(created_at, name, kind, version, summary, author, contact, app_version, file_count, size, files, author_id, update_of, changes) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                now_ms(),
                sub.name,
                sub.kind,
                sub.version,
                sub.summary,
                sub.author,
                sub.contact,
                sub.app_version,
                sub.files.len() as i64,
                size as i64,
                files,
                sub.author_id,
                sub.update_of,
                changes
            ],
        )?;
        Ok(c.last_insert_rowid())
    }

    /// `by`'s vouches, in place of the ones they had.
    pub fn set_vouches(&self, by: &str, vouches: &[pyr_proto::control::Vouch]) -> Result<()> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        tx.execute("DELETE FROM vouches WHERE by = ?1", [by])?;
        for v in vouches {
            tx.execute("INSERT OR REPLACE INTO vouches(by, for_uid, sig, at) VALUES (?1, ?2, ?3, ?4)", params![by, v.for_uid, pyr_proto::b64::encode(&v.sig), now_ms()])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The vouches these people made (at most `max`).
    pub fn vouches_by(&self, by: &[String], max: usize) -> Vec<pyr_proto::control::Vouch> {
        let c = self.conn.lock();
        let Ok(mut st) = c.prepare("SELECT for_uid, sig FROM vouches WHERE by = ?1 ORDER BY at DESC LIMIT ?2") else { return Vec::new() };
        let mut out = Vec::new();
        for b in by {
            let left = max.saturating_sub(out.len());
            if left == 0 {
                break;
            }
            let rows = st.query_map(params![b, left as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
            for (for_uid, sig) in rows.map(|r| r.flatten().collect::<Vec<_>>()).unwrap_or_default() {
                if let Ok(sig) = pyr_proto::b64::decode(&sig) {
                    out.push(pyr_proto::control::Vouch { by: b.clone(), for_uid, sig });
                }
            }
        }
        out
    }

    /// The vouches for `uid`, newest first (at most `max`).
    pub fn vouches_for(&self, uid: &str, max: usize) -> Vec<pyr_proto::control::Vouch> {
        let c = self.conn.lock();
        let Ok(mut st) = c.prepare("SELECT by, sig FROM vouches WHERE for_uid = ?1 ORDER BY at DESC LIMIT ?2") else { return Vec::new() };
        let rows = st.query_map(params![uid, max as i64], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)));
        rows.map(|r| r.flatten().collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(by, sig)| pyr_proto::b64::decode(&sig).ok().map(|sig| pyr_proto::control::Vouch { by, for_uid: uid.to_string(), sig }))
            .collect()
    }

    /// Submissions waiting for the owner to look at them: skins, or plugins.
    pub fn new_submission_count(&self, skins: bool) -> u64 {
        let op = if skins { "=" } else { "!=" };
        self.conn
            .lock()
            .query_row(&format!("SELECT COUNT(*) FROM plugin_submissions WHERE status = 'new' AND kind {op} 'skin'"), [], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64
    }

    fn submission_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<pyr_proto::control::SubmissionEntry> {
        use pyr_proto::control::{SubmissionEntry, SubmissionStatus};
        Ok(SubmissionEntry {
            id: r.get(0)?,
            created_at: r.get(1)?,
            status: SubmissionStatus::parse(&r.get::<_, String>(2)?).unwrap_or_default(),
            name: r.get(3)?,
            kind: r.get(4)?,
            version: r.get(5)?,
            summary: r.get(6)?,
            author: r.get(7)?,
            contact: r.get(8)?,
            app_version: r.get(9)?,
            files: r.get::<_, i64>(10)? as u32,
            size: r.get::<_, i64>(11)? as u64,
            author_id: r.get(12)?,
            update_of: r.get(13)?,
            changes: r.get(14)?,
            note: r.get(15)?,
        })
    }

    /// The ones waiting to be looked at first, then the rest; newest first in each (no files).
    pub fn submissions(&self, limit: usize) -> Vec<pyr_proto::control::SubmissionEntry> {
        let c = self.conn.lock();
        let Ok(mut q) = c.prepare(&format!("SELECT {SUBMISSION_COLUMNS} FROM plugin_submissions ORDER BY (status = 'new') DESC, id DESC LIMIT ?1")) else {
            return Vec::new();
        };
        q.query_map([limit as i64], Self::submission_row).map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }

    /// What one sender (by their Pyr id) sent in, newest first (no files).
    pub fn submissions_by(&self, author_id: &str, limit: usize) -> Vec<pyr_proto::control::SubmissionEntry> {
        if author_id.is_empty() {
            return Vec::new();
        }
        let c = self.conn.lock();
        let Ok(mut q) = c.prepare(&format!("SELECT {SUBMISSION_COLUMNS} FROM plugin_submissions WHERE author_id = ?1 ORDER BY id DESC LIMIT ?2")) else {
            return Vec::new();
        };
        q.query_map(params![author_id, limit as i64], Self::submission_row).map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }

    /// One submission with its files (as stored: JSON).
    pub fn submission(&self, id: i64) -> Option<(pyr_proto::control::SubmissionEntry, String)> {
        let c = self.conn.lock();
        c.query_row(&format!("SELECT {SUBMISSION_COLUMNS}, files FROM plugin_submissions WHERE id = ?1"), [id], |r| {
            Ok((Self::submission_row(r)?, r.get::<_, String>(16)?))
        })
        .ok()
    }

    /// `note` (for the sender) replaces the one before, if any.
    pub fn set_submission_status(&self, id: i64, status: pyr_proto::control::SubmissionStatus, note: &str) -> Result<bool> {
        Ok(self
            .conn
            .lock()
            .execute(
                "UPDATE plugin_submissions SET status = ?2, status_at = ?3, note = ?4 WHERE id = ?1",
                params![id, status.as_str(), now_ms(), note],
            )?
            > 0)
    }

    /// Every stored file: (id, size), for backups.
    pub fn all_files(&self) -> Vec<(String, u64)> {
        let c = self.conn.lock();
        let Ok(mut q) = c.prepare("SELECT id, size FROM files ORDER BY created_at") else { return Vec::new() };
        q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64)))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    /// A consistent copy of the whole database at `path` (for backups), taken in one go
    /// while nothing else writes.
    pub fn snapshot(&self, path: &Path) -> Result<()> {
        let _ = std::fs::remove_file(path);
        let target = path.to_str().context("backup path isn't text")?;
        self.conn.lock().execute("VACUUM INTO ?1", [target]).context("copying the database")?;
        Ok(())
    }

    /// Total bytes stored (for the storage limit).
    pub fn files_total(&self) -> u64 {
        self.conn.lock().query_row("SELECT COALESCE(SUM(size), 0) FROM files", [], |r| r.get::<_, i64>(0)).unwrap_or(0) as u64
    }

    /// Bytes of files one member has uploaded that are still kept.
    pub fn files_total_of(&self, uid: &str) -> u64 {
        self.conn
            .lock()
            .query_row("SELECT COALESCE(SUM(size), 0) FROM files WHERE owner = ?1", [uid], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as u64
    }

    /// Attachments a member uploaded but hasn't sent yet.
    pub fn unsent_files_of(&self, uid: &str) -> usize {
        self.conn
            .lock()
            .query_row("SELECT COUNT(*) FROM files WHERE owner = ?1 AND kind IN (0, 3) AND message_id IS NULL AND id NOT IN (SELECT id FROM gifs)", [uid], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize
    }

    /// Attachments uploaded but not sent within `max_age_ms`, profile pictures nobody uses,
    /// and sounds played here that aren't among the server's own (a sound from another server,
    /// say): their rows are removed and their ids returned so the files can be wiped.
    pub fn take_unused_files(&self, max_age_ms: i64) -> Vec<String> {
        let c = self.conn.lock();
        let cutoff = now_ms() - max_age_ms;
        let sql = "SELECT id FROM files WHERE created_at < ?1 AND (
                       (kind = 0 AND message_id IS NULL AND id NOT IN (SELECT id FROM gifs))
                    OR (kind = 3 AND message_id IS NULL)
                    OR (kind = 1 AND id NOT IN (SELECT avatar FROM users WHERE avatar IS NOT NULL)
                                 AND id NOT IN (SELECT value FROM meta WHERE key = 'icon')
                                 AND id NOT IN (SELECT id FROM emojis))
                    OR (kind = 2 AND id NOT IN (SELECT id FROM sounds)))";
        let ids: Vec<String> = c
            .prepare(sql)
            .and_then(|mut st| st.query_map([cutoff], |r| r.get(0)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        for id in &ids {
            let _ = c.execute("DELETE FROM files WHERE id = ?1", [id]);
        }
        if !ids.is_empty() {
            scrub(&c);
        }
        ids
    }

    fn rel_kinds(c: &Connection, a: &str, b: &str) -> Vec<i64> {
        let Ok(mut st) = c.prepare("SELECT kind FROM relations WHERE a = ?1 AND b = ?2") else { return Vec::new() };
        st.query_map(params![a, b], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    /// Where `me` stands with `other`, as `me` sees it.
    pub fn relation(&self, me: &str, other: &str) -> Option<RelationKind> {
        let c = self.conn.lock();
        let mine = Self::rel_kinds(&c, me, other);
        if mine.contains(&2) {
            return Some(RelationKind::Blocked);
        }
        if mine.contains(&1) {
            return Some(RelationKind::Friend);
        }
        if mine.contains(&0) {
            return Some(RelationKind::Outgoing);
        }
        if Self::rel_kinds(&c, other, me).contains(&0) {
            return Some(RelationKind::Incoming);
        }
        None
    }

    pub fn relations_of(&self, uid: &str) -> Vec<Relation> {
        let others: Vec<String> = {
            let c = self.conn.lock();
            let Ok(mut st) = c.prepare("SELECT b FROM relations WHERE a = ?1 UNION SELECT a FROM relations WHERE b = ?1 AND kind = 0")
            else {
                return Vec::new();
            };
            st.query_map([uid], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
        };
        others.into_iter().filter_map(|o| self.relation(uid, &o).map(|kind| Relation { user_id: o, kind })).collect()
    }

    /// Friend requests someone has sent that are still waiting.
    pub fn outgoing_requests(&self, uid: &str) -> usize {
        self.conn
            .lock()
            .query_row("SELECT COUNT(*) FROM relations WHERE a = ?1 AND kind = 0", [uid], |r| r.get::<_, i64>(0))
            .unwrap_or(0) as usize
    }

    /// Either has blocked the other.
    pub fn blocked_between(&self, x: &str, y: &str) -> bool {
        let c = self.conn.lock();
        Self::rel_kinds(&c, x, y).contains(&2) || Self::rel_kinds(&c, y, x).contains(&2)
    }

    pub fn set_relation(&self, a: &str, b: &str, kind: i64) -> Result<()> {
        self.conn.lock().execute(
            "INSERT OR IGNORE INTO relations (a, b, kind, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![a, b, kind, now_ms()],
        )?;
        Ok(())
    }

    /// Removes rows of these kinds between the two, both ways.
    pub fn clear_relation(&self, x: &str, y: &str, kinds: &[i64]) -> Result<()> {
        let c = self.conn.lock();
        for k in kinds {
            c.execute("DELETE FROM relations WHERE ((a = ?1 AND b = ?2) OR (a = ?2 AND b = ?1)) AND kind = ?3", params![x, y, k])?;
        }
        Ok(())
    }

    pub fn unblock(&self, me: &str, other: &str) -> Result<()> {
        self.conn.lock().execute("DELETE FROM relations WHERE a = ?1 AND b = ?2 AND kind = 2", params![me, other])?;
        Ok(())
    }

    pub fn emojis(&self) -> Vec<CustomEmoji> {
        let c = self.conn.lock();
        let Ok(mut st) = c.prepare("SELECT id, name, animated, by FROM emojis ORDER BY created_at, rowid") else { return Vec::new() };
        st.query_map([], |r| Ok(CustomEmoji { id: r.get(0)?, name: r.get(1)?, animated: r.get::<_, i64>(2)? != 0, by: r.get(3)? }))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// Adds an emoji; false if the name or picture is already used.
    pub fn add_emoji(&self, e: &CustomEmoji) -> Result<bool> {
        let n = self.conn.lock().execute(
            "INSERT OR IGNORE INTO emojis (id, name, animated, by, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![e.id, e.name, e.animated as i64, e.by, now_ms()],
        )?;
        Ok(n > 0)
    }

    pub fn remove_emoji(&self, id: &str) -> Result<bool> {
        Ok(self.conn.lock().execute("DELETE FROM emojis WHERE id = ?1", [id])? > 0)
    }

    /// False if another emoji has that name (or there's no such emoji).
    pub fn rename_emoji(&self, id: &str, name: &str) -> Result<bool> {
        Ok(self.conn.lock().execute("UPDATE OR IGNORE emojis SET name = ?2 WHERE id = ?1", params![id, name])? > 0)
    }

    // ------------------------------------------------------------- server GIFs

    pub fn gifs(&self) -> Vec<ServerGif> {
        let c = self.conn.lock();
        let Ok(mut st) = c.prepare("SELECT id, name, width, height, by FROM gifs ORDER BY created_at, rowid") else { return Vec::new() };
        st.query_map([], |r| Ok(ServerGif { id: r.get(0)?, name: r.get(1)?, width: r.get::<_, i64>(2)? as u32, height: r.get::<_, i64>(3)? as u32, by: r.get(4)? }))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    pub fn is_gif(&self, id: &str) -> bool {
        self.conn.lock().query_row("SELECT 1 FROM gifs WHERE id = ?1", [id], |_| Ok(())).is_ok()
    }

    /// Adds a GIF; false if the name or picture is already one.
    pub fn add_gif(&self, g: &ServerGif) -> Result<bool> {
        let n = self.conn.lock().execute(
            "INSERT OR IGNORE INTO gifs (id, name, width, height, by, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![g.id, g.name, g.width as i64, g.height as i64, g.by, now_ms()],
        )?;
        Ok(n > 0)
    }

    /// Takes a GIF out, and its file's row with it; true if there was one (its file can go).
    pub fn remove_gif(&self, id: &str) -> Result<bool> {
        let c = self.conn.lock();
        let n = c.execute("DELETE FROM gifs WHERE id = ?1", [id])?;
        if n > 0 {
            c.execute("DELETE FROM files WHERE id = ?1 AND kind = 0 AND message_id IS NULL", [id])?;
            scrub(&c);
        }
        Ok(n > 0)
    }

    /// False if another GIF has that name (or there's no such GIF).
    pub fn rename_gif(&self, id: &str, name: &str) -> Result<bool> {
        Ok(self.conn.lock().execute("UPDATE OR IGNORE gifs SET name = ?2 WHERE id = ?1", params![id, name])? > 0)
    }

    // ------------------------------------------------------------- soundboard

    pub fn sounds(&self) -> Vec<ServerSound> {
        let c = self.conn.lock();
        let Ok(mut st) = c.prepare("SELECT id, name, emoji, volume, by, length_ms FROM sounds ORDER BY created_at, rowid") else {
            return Vec::new();
        };
        st.query_map([], |r| {
            Ok(ServerSound {
                id: r.get(0)?,
                name: r.get(1)?,
                emoji: r.get(2)?,
                volume: r.get::<_, f64>(3)? as f32,
                by: r.get(4)?,
                length_ms: r.get::<_, i64>(5)? as u32,
            })
        })
        .map(|rows| rows.flatten().collect())
        .unwrap_or_default()
    }

    /// Adds a sound; false if that file is already one of the server's sounds.
    pub fn add_sound(&self, s: &ServerSound) -> Result<bool> {
        let n = self.conn.lock().execute(
            "INSERT OR IGNORE INTO sounds (id, name, emoji, volume, by, length_ms, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![s.id, s.name, s.emoji, s.volume as f64, s.by, s.length_ms as i64, now_ms()],
        )?;
        Ok(n > 0)
    }

    /// Changes what's given; false if there's no such sound.
    pub fn edit_sound(&self, id: &str, name: Option<&str>, emoji: Option<&str>, volume: Option<f32>) -> Result<bool> {
        let c = self.conn.lock();
        let mut found = c.query_row("SELECT 1 FROM sounds WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some();
        if let Some(n) = name {
            found &= c.execute("UPDATE sounds SET name = ?2 WHERE id = ?1", params![id, n])? > 0;
        }
        if let Some(e) = emoji {
            c.execute("UPDATE sounds SET emoji = ?2 WHERE id = ?1", params![id, e])?;
        }
        if let Some(v) = volume {
            c.execute("UPDATE sounds SET volume = ?2 WHERE id = ?1", params![id, v as f64])?;
        }
        Ok(found)
    }

    /// Removes a sound and its file; true if it was one of the server's sounds.
    pub fn remove_sound(&self, id: &str) -> Result<bool> {
        let c = self.conn.lock();
        let n = c.execute("DELETE FROM sounds WHERE id = ?1", [id])?;
        if n > 0 {
            c.execute("DELETE FROM files WHERE id = ?1 AND kind = 2", [id])?;
            scrub(&c);
        }
        Ok(n > 0)
    }

    pub fn set_avatar(&self, uid: &str, file_id: Option<&str>) -> Result<Option<String>> {
        let c = self.conn.lock();
        let old: Option<String> =
            c.query_row("SELECT avatar FROM users WHERE id = ?1", [uid], |r| r.get(0)).optional()?.flatten();
        c.execute("UPDATE users SET avatar = ?2 WHERE id = ?1", params![uid, file_id])?;
        Ok(old)
    }

    /// Removes a profile picture's row if nobody uses it any more; true if it should be wiped.
    pub fn release_avatar(&self, id: &str) -> bool {
        let c = self.conn.lock();
        let used: i64 = c
            .query_row(
                "SELECT (SELECT COUNT(*) FROM users WHERE avatar = ?1) + (SELECT COUNT(*) FROM meta WHERE key = 'icon' AND value = ?1) \
                 + (SELECT COUNT(*) FROM emojis WHERE id = ?1)",
                [id],
                |r| r.get(0),
            )
            .unwrap_or(1);
        if used > 0 {
            return false;
        }
        let n = c.execute("DELETE FROM files WHERE id = ?1 AND kind = 1", [id]).unwrap_or(0);
        scrub(&c);
        n > 0
    }

    fn attachments_for(c: &Connection, ids: &[MessageId]) -> std::collections::HashMap<MessageId, Vec<Attachment>> {
        let mut out: std::collections::HashMap<MessageId, Vec<Attachment>> = Default::default();
        if ids.is_empty() {
            return out;
        }
        let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT message_id, id, name, mime, size, width, height FROM files WHERE message_id IN ({list}) ORDER BY created_at, rowid"
        );
        let Ok(mut st) = c.prepare(&sql) else { return out };
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                Attachment {
                    id: r.get(1)?,
                    name: r.get(2)?,
                    mime: r.get(3)?,
                    size: r.get::<_, i64>(4)? as u64,
                    width: r.get(5)?,
                    height: r.get(6)?,
                },
            ))
        });
        if let Ok(rows) = rows {
            for (mid, a) in rows.flatten() {
                out.entry(mid).or_default().push(a);
            }
        }
        out
    }

    /// Files attached to messages in a channel, or to one message (to wipe them).
    fn file_ids_where(c: &Connection, sql_where: &str, arg: i64) -> Vec<String> {
        c.prepare(&format!("SELECT id FROM files WHERE {sql_where}"))
            .and_then(|mut st| st.query_map([arg], |r| r.get(0)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default()
    }

    pub fn owner_exists(&self) -> bool {
        self.conn
            .lock()
            .query_row("SELECT COUNT(*) FROM users WHERE role = 2 AND banned = 0", [], |r| r.get::<_, i64>(0))
            .map(|n| n > 0)
            .unwrap_or(false)
    }

    // ------------------------------------------------------------- channels

    /// The channels, each with its newest message (for unread markers in `Ready`).
    pub fn channels(&self) -> Vec<Channel> {
        let c = self.conn.lock();
        let mut st = c
            .prepare(&format!(
                "SELECT {CHANNEL_COLUMNS}, m.id, m.author \
                 FROM channels c LEFT JOIN messages m ON m.id = (SELECT MAX(id) FROM messages WHERE channel_id = c.id) \
                 WHERE c.kind < 2 ORDER BY c.kind, c.position, c.id"
            ))
            .expect("valid SQL");
        st.query_map([], |r| channel_row(r, true)).map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }

    pub fn channel(&self, id: ChannelId) -> Option<Channel> {
        self.conn
            .lock()
            .query_row(&format!("SELECT {CHANNEL_COLUMNS} FROM channels c WHERE c.id = ?1 AND c.kind < 2"), [id], |r| channel_row(r, false))
            .optional()
            .ok()
            .flatten()
    }

    pub fn create_channel(&self, name: &str, kind: ChannelKind) -> Result<Channel> {
        self.create_channel_with(name, kind, false, &[])
    }

    /// A new channel, private from the start (seen only by `view_roles`) or not.
    pub fn create_channel_with(&self, name: &str, kind: ChannelKind, private: bool, view_roles: &[RoleId]) -> Result<Channel> {
        let c = self.conn.lock();
        let position: i32 = c
            .query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM channels WHERE kind = ?1", [kind_to_i(kind)], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        let view_roles = if private { view_roles.to_vec() } else { Vec::new() };
        c.execute(
            "INSERT INTO channels(kind, name, topic, position, created_at, private, view_roles) VALUES (?1, ?2, '', ?3, ?4, ?5, ?6)",
            params![kind_to_i(kind), name, position, now_ms(), private as i64, role_list(&view_roles)],
        )?;
        Ok(Channel { id: c.last_insert_rowid(), kind, name: name.to_string(), position, private, view_roles, ..Default::default() })
    }

    /// Who can see a channel, and who can post in it.
    pub fn set_channel_access(&self, id: ChannelId, private: bool, view_roles: &[RoleId], read_only: bool, post_roles: &[RoleId]) -> Result<Option<Channel>> {
        let view_roles = if private { view_roles } else { &[] };
        let post_roles = if read_only { post_roles } else { &[] };
        self.conn.lock().execute(
            "UPDATE channels SET private = ?2, view_roles = ?3, read_only = ?4, post_roles = ?5 WHERE id = ?1 AND kind < 2",
            params![id, private as i64, role_list(view_roles), read_only as i64, role_list(post_roles)],
        )?;
        Ok(self.channel(id))
    }

    /// Everyone here who can see a channel (admins and the owner always can).
    pub fn viewers(&self, ch: &Channel) -> Vec<UserId> {
        let c = self.conn.lock();
        let mut st = c.prepare_cached("SELECT id, role FROM users WHERE banned = 0").expect("valid SQL");
        let users: Vec<(String, i64)> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?))).map(|rows| rows.filter_map(|r| r.ok()).collect()).unwrap_or_default();
        let mut st = c.prepare_cached("SELECT user_id, role_id FROM member_roles").expect("valid SQL");
        let mut roles: HashMap<String, Vec<RoleId>> = HashMap::new();
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, RoleId>(1)?))) {
            for (u, r) in rows.filter_map(|r| r.ok()) {
                roles.entry(u).or_default().push(r);
            }
        }
        users
            .into_iter()
            .filter(|(u, role)| ch.visible_to(Role::from_i64(*role).is_admin(), roles.get(u).map(|v| v.as_slice()).unwrap_or(&[])))
            .map(|(u, _)| u)
            .collect()
    }

    pub fn update_channel(&self, id: ChannelId, name: Option<&str>, topic: Option<&str>) -> Result<Option<Channel>> {
        {
            let c = self.conn.lock();
            if let Some(n) = name {
                c.execute("UPDATE channels SET name = ?2 WHERE id = ?1", params![id, n])?;
            }
            if let Some(t) = topic {
                c.execute("UPDATE channels SET topic = ?2 WHERE id = ?1", params![id, t])?;
            }
            scrub(&c);
        }
        Ok(self.channel(id))
    }

    /// Deletes a channel and its messages; returns whether it existed and the ids of the
    /// files that were attached (to wipe).
    pub fn delete_channel(&self, id: ChannelId) -> Result<(bool, Vec<String>)> {
        let c = self.conn.lock();
        let n = c.execute("DELETE FROM channels WHERE id = ?1 AND kind < 2", [id])?;
        if n == 0 {
            return Ok((false, Vec::new()));
        }
        c.execute("DELETE FROM reactions WHERE message_id IN (SELECT id FROM messages WHERE channel_id = ?1)", [id])?;
        let files = Self::file_ids_where(&c, "message_id IN (SELECT id FROM messages WHERE channel_id = ?1)", id);
        c.execute("DELETE FROM files WHERE message_id IN (SELECT id FROM messages WHERE channel_id = ?1)", [id])?;
        c.execute("DELETE FROM messages WHERE channel_id = ?1", [id])?;
        Self::forget_keys(&c, id)?;
        scrub(&c);
        Ok((n > 0, files))
    }

    // ------------------------------------------------------------- direct messages

    /// The two people in a direct message conversation.
    pub fn dm_members(&self, channel_id: ChannelId) -> Option<(String, String)> {
        self.conn
            .lock()
            .query_row("SELECT a, b FROM dms WHERE channel_id = ?1", [channel_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .ok()
            .flatten()
    }

    /// Finds or starts the conversation between two members; true when it's new.
    pub fn open_dm(&self, x: &str, y: &str) -> Result<(ChannelId, bool)> {
        let (a, b) = if x <= y { (x, y) } else { (y, x) };
        let c = self.conn.lock();
        if let Some(id) =
            c.query_row("SELECT channel_id FROM dms WHERE a = ?1 AND b = ?2", params![a, b], |r| r.get(0)).optional()?
        {
            return Ok((id, false));
        }
        c.execute(
            "INSERT INTO channels(kind, name, topic, position, created_at) VALUES (?1, '', '', 0, ?2)",
            params![DM_KIND, now_ms()],
        )?;
        let id = c.last_insert_rowid();
        c.execute("INSERT INTO dms(channel_id, a, b) VALUES (?1, ?2, ?3)", params![id, a, b])?;
        Ok((id, true))
    }

    /// Who's in a private conversation: a direct message's two people, or a group's members.
    pub fn private_members(&self, channel_id: ChannelId) -> Option<Vec<String>> {
        if let Some((a, b)) = self.dm_members(channel_id) {
            return Some(vec![a, b]);
        }
        self.group(channel_id).map(|g| g.members)
    }

    /// A group conversation (as anyone in it sees it).
    pub fn group(&self, channel_id: ChannelId) -> Option<Group> {
        let c = self.conn.lock();
        let (owner, name): (String, String) = c
            .query_row("SELECT owner, name FROM groups WHERE channel_id = ?1", [channel_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .ok()
            .flatten()?;
        let members: Vec<String> = c
            .prepare("SELECT user_id FROM group_members WHERE channel_id = ?1 ORDER BY rowid")
            .and_then(|mut st| st.query_map([channel_id], |r| r.get(0)).map(|rows| rows.flatten().collect()))
            .unwrap_or_default();
        let last: Option<(MessageId, String)> = c
            .query_row("SELECT id, author FROM messages WHERE channel_id = ?1 ORDER BY id DESC LIMIT 1", [channel_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .ok()
            .flatten();
        Some(Group { id: channel_id, members, name, owner, last_message: last.as_ref().map(|l| l.0), last_from: last.map(|l| l.1) })
    }

    /// The group conversations someone's in.
    pub fn groups_of(&self, uid: &str) -> Vec<Group> {
        let ids: Vec<ChannelId> = {
            let c = self.conn.lock();
            c.prepare("SELECT channel_id FROM group_members WHERE user_id = ?1")
                .and_then(|mut st| st.query_map([uid], |r| r.get(0)).map(|rows| rows.flatten().collect()))
                .unwrap_or_default()
        };
        ids.into_iter().filter_map(|id| self.group(id)).collect()
    }

    /// How many groups someone started that are still going.
    /// How many groups someone started and is still in.
    pub fn groups_owned(&self, uid: &str) -> usize {
        self.conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM groups g JOIN group_members m ON m.channel_id = g.channel_id AND m.user_id = g.owner WHERE g.owner = ?1",
                [uid],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
    }

    /// Starts a group conversation: `owner` and `others`.
    pub fn create_group(&self, owner: &str, others: &[String]) -> Result<ChannelId> {
        let c = self.conn.lock();
        c.execute(
            "INSERT INTO channels(kind, name, topic, position, created_at) VALUES (?1, '', '', 0, ?2)",
            params![GROUP_KIND, now_ms()],
        )?;
        let id = c.last_insert_rowid();
        c.execute("INSERT INTO groups(channel_id, owner, name) VALUES (?1, ?2, '')", params![id, owner])?;
        for m in std::iter::once(owner).chain(others.iter().map(|s| s.as_str())) {
            c.execute("INSERT OR IGNORE INTO group_members(channel_id, user_id) VALUES (?1, ?2)", params![id, m])?;
        }
        Ok(id)
    }

    pub fn add_group_members(&self, channel_id: ChannelId, uids: &[String]) -> Result<()> {
        let c = self.conn.lock();
        for u in uids {
            c.execute("INSERT OR IGNORE INTO group_members(channel_id, user_id) VALUES (?1, ?2)", params![channel_id, u])?;
        }
        Ok(())
    }

    /// Takes someone out of a group. When nobody's left, the group and everything said in it
    /// go: the ids of its files are returned (to wipe).
    pub fn leave_group(&self, channel_id: ChannelId, uid: &str) -> Result<Vec<String>> {
        let c = self.conn.lock();
        c.execute("DELETE FROM group_members WHERE channel_id = ?1 AND user_id = ?2", params![channel_id, uid])?;
        let left: i64 = c.query_row("SELECT COUNT(*) FROM group_members WHERE channel_id = ?1", [channel_id], |r| r.get(0))?;
        if left > 0 {
            // The one who started it left: it passes to whoever's been in it longest.
            c.execute(
                "UPDATE groups SET owner = (SELECT user_id FROM group_members WHERE channel_id = ?1 ORDER BY rowid LIMIT 1)
                 WHERE channel_id = ?1 AND owner = ?2",
                params![channel_id, uid],
            )?;
            return Ok(Vec::new());
        }
        c.execute("DELETE FROM groups WHERE channel_id = ?1", [channel_id])?;
        c.execute("DELETE FROM channels WHERE id = ?1 AND kind = ?2", params![channel_id, GROUP_KIND])?;
        c.execute("DELETE FROM reactions WHERE message_id IN (SELECT id FROM messages WHERE channel_id = ?1)", [channel_id])?;
        let files = Self::file_ids_where(&c, "message_id IN (SELECT id FROM messages WHERE channel_id = ?1)", channel_id);
        c.execute("DELETE FROM files WHERE message_id IN (SELECT id FROM messages WHERE channel_id = ?1)", [channel_id])?;
        c.execute("DELETE FROM messages WHERE channel_id = ?1", [channel_id])?;
        Self::forget_keys(&c, channel_id)?;
        scrub(&c);
        Ok(files)
    }

    pub fn rename_group(&self, channel_id: ChannelId, name: &str) -> Result<()> {
        self.conn.lock().execute("UPDATE groups SET name = ?2 WHERE channel_id = ?1", params![channel_id, name])?;
        Ok(())
    }

    /// Someone's conversations, as they see them.
    pub fn dms_of(&self, uid: &str) -> Vec<Dm> {
        let c = self.conn.lock();
        let mut st = c
            .prepare(
                "SELECT d.channel_id, CASE WHEN d.a = ?1 THEN d.b ELSE d.a END, m.id, m.author \
                 FROM dms d LEFT JOIN messages m ON m.id = (SELECT MAX(id) FROM messages WHERE channel_id = d.channel_id) \
                 WHERE d.a = ?1 OR d.b = ?1",
            )
            .expect("valid SQL");
        st.query_map([uid], |r| Ok(Dm { id: r.get(0)?, with: r.get(1)?, last_message: r.get(2)?, last_from: r.get(3)? }))
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default()
    }

    // ------------------------------------------------------------- messages

    pub fn insert_message(
        &self,
        channel_id: ChannelId,
        author: &str,
        content: &str,
        reply_to: Option<MessageId>,
    ) -> Result<Message> {
        self.insert_message_with(channel_id, author, content, reply_to, &[], None)
    }

    /// A message with files the author uploaded (checked by the caller); links them to it.
    pub fn insert_message_with(
        &self,
        channel_id: ChannelId,
        author: &str,
        content: &str,
        reply_to: Option<MessageId>,
        files: &[String],
        sealed: Option<&Sealed>,
    ) -> Result<Message> {
        let now = now_ms();
        let c = self.conn.lock();
        let sealed_json = sealed.map(|s| serde_json::to_string(s).expect("serializes"));
        let content = if sealed.is_some() { "" } else { content };
        c.execute(
            "INSERT INTO messages(channel_id, author, content, created_at, reply_to, sealed) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![channel_id, author, content, now, reply_to, sealed_json],
        )?;
        let id = c.last_insert_rowid();
        for f in files {
            c.execute("UPDATE files SET message_id = ?2 WHERE id = ?1 AND message_id IS NULL", params![f, id])?;
        }
        let attachments = Self::attachments_for(&c, &[id]).remove(&id).unwrap_or_default();
        Ok(Message {
            id,
            channel_id,
            author: author.to_string(),
            content: if sealed.is_some() { PLACEHOLDER.to_string() } else { content.to_string() },
            created_at: now,
            edited_at: None,
            reply_to,
            reactions: Vec::new(),
            attachments,
            sealed: sealed.cloned(),
            lock: Default::default(),
        })
    }

    fn reactions_for(c: &Connection, ids: &[MessageId]) -> std::collections::HashMap<MessageId, Vec<Reaction>> {
        let mut out: std::collections::HashMap<MessageId, Vec<Reaction>> = Default::default();
        if ids.is_empty() {
            return out;
        }
        let list = ids.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(",");
        let sql = format!(
            "SELECT message_id, emoji, user_id FROM reactions WHERE message_id IN ({list}) ORDER BY created_at, rowid"
        );
        let Ok(mut st) = c.prepare(&sql) else { return out };
        let rows = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)));
        if let Ok(rows) = rows {
            for (mid, emoji, uid) in rows.flatten() {
                let list = out.entry(mid).or_default();
                match list.iter_mut().find(|r| r.emoji == emoji) {
                    Some(r) => r.users.push(uid),
                    None => list.push(Reaction { emoji, users: vec![uid] }),
                }
            }
        }
        out
    }

    fn row_to_message(r: &rusqlite::Row<'_>) -> rusqlite::Result<Message> {
        // (Encrypted: older Pyrs show the note in `content`.)
        let sealed: Option<Sealed> = r.get::<_, Option<String>>(7)?.and_then(|j| serde_json::from_str(&j).ok());
        Ok(Message {
            id: r.get(0)?,
            channel_id: r.get(1)?,
            author: r.get(2)?,
            content: if sealed.is_some() { PLACEHOLDER.to_string() } else { r.get(3)? },
            created_at: r.get(4)?,
            edited_at: r.get(5)?,
            reply_to: r.get(6)?,
            reactions: Vec::new(),
            attachments: Vec::new(),
            sealed,
            lock: Default::default(),
        })
    }

    pub fn get_message(&self, id: MessageId) -> Option<Message> {
        let c = self.conn.lock();
        let mut m = c
            .query_row(
                "SELECT id, channel_id, author, content, created_at, edited_at, reply_to, sealed FROM messages WHERE id = ?1",
                [id],
                Self::row_to_message,
            )
            .optional()
            .ok()
            .flatten()?;
        m.reactions = Self::reactions_for(&c, &[id]).remove(&id).unwrap_or_default();
        m.attachments = Self::attachments_for(&c, &[id]).remove(&id).unwrap_or_default();
        Some(m)
    }

    pub fn edit_message(&self, id: MessageId, content: &str, sealed: Option<&Sealed>) -> Result<Option<Message>> {
        {
            let c = self.conn.lock();
            let sealed_json = sealed.map(|s| serde_json::to_string(s).expect("serializes"));
            let content = if sealed.is_some() { "" } else { content };
            c.execute("UPDATE messages SET content = ?2, edited_at = ?3, sealed = ?4 WHERE id = ?1", params![id, content, now_ms(), sealed_json])?;
            scrub(&c);
        }
        Ok(self.get_message(id))
    }

    /// Deletes a message; returns the ids of its files (to wipe).
    pub fn delete_message(&self, id: MessageId) -> Result<Vec<String>> {
        let c = self.conn.lock();
        c.execute("DELETE FROM messages WHERE id = ?1", [id])?;
        c.execute("DELETE FROM reactions WHERE message_id = ?1", [id])?;
        let files = Self::file_ids_where(&c, "message_id = ?1", id);
        c.execute("DELETE FROM files WHERE message_id = ?1", [id])?;
        scrub(&c);
        Ok(files)
    }

    /// Newest-last page of history older than `before`.
    pub fn history(&self, channel_id: ChannelId, before: Option<MessageId>, limit: u32) -> (Vec<Message>, bool) {
        let limit = limit.clamp(1, 100) as i64;
        let c = self.conn.lock();
        let before = before.unwrap_or(i64::MAX);
        let mut st = c
            .prepare(
                "SELECT id, channel_id, author, content, created_at, edited_at, reply_to, sealed FROM messages
                 WHERE channel_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
            )
            .expect("valid SQL");
        let mut msgs: Vec<Message> = st
            .query_map(params![channel_id, before, limit + 1], Self::row_to_message)
            .map(|rows| rows.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();
        let complete = msgs.len() as i64 <= limit;
        msgs.truncate(limit as usize);
        msgs.reverse();
        let ids: Vec<i64> = msgs.iter().map(|m| m.id).collect();
        let mut reactions = Self::reactions_for(&c, &ids);
        let mut attachments = Self::attachments_for(&c, &ids);
        for m in &mut msgs {
            m.reactions = reactions.remove(&m.id).unwrap_or_default();
            m.attachments = attachments.remove(&m.id).unwrap_or_default();
        }
        (msgs, complete)
    }

    pub fn set_reaction(&self, message_id: MessageId, emoji: &str, user: &str, add: bool) -> Result<()> {
        let c = self.conn.lock();
        if add {
            c.execute(
                "INSERT OR IGNORE INTO reactions(message_id, emoji, user_id, created_at) VALUES (?1, ?2, ?3, ?4)",
                params![message_id, emoji, user, now_ms()],
            )?;
        } else {
            c.execute(
                "DELETE FROM reactions WHERE message_id = ?1 AND emoji = ?2 AND user_id = ?3",
                params![message_id, emoji, user],
            )?;
        }
        Ok(())
    }

    pub fn distinct_reactions(&self, message_id: MessageId) -> usize {
        self.conn
            .lock()
            .query_row("SELECT COUNT(DISTINCT emoji) FROM reactions WHERE message_id = ?1", [message_id], |r| {
                r.get::<_, i64>(0)
            })
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    // -------------------------------------------------------------- invites

    pub fn create_invite(
        &self,
        code: &str,
        created_by: &str,
        max_uses: Option<u32>,
        expires_at: Option<i64>,
        grants: Role,
    ) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO invites(code, created_by, created_at, max_uses, uses, expires_at, grants_role)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)",
            params![code, created_by, now_ms(), max_uses, expires_at, grants.as_i64()],
        )?;
        Ok(())
    }

    /// An invite for one bot: used once, by a bot.
    pub fn create_bot_invite(&self, code: &str, created_by: &str, expires_at: i64) -> Result<()> {
        self.conn.lock().execute(
            "INSERT INTO invites(code, created_by, created_at, max_uses, uses, expires_at, grants_role, bot)
             VALUES (?1, ?2, ?3, 1, 0, ?4, 0, 1)",
            params![code, created_by, now_ms(), expires_at],
        )?;
        Ok(())
    }

    /// Whether whoever joins with this invite is a bot.
    pub fn invite_is_bot(&self, code: &str) -> bool {
        self.conn
            .lock()
            .query_row("SELECT bot FROM invites WHERE code = ?1", [code], |r| r.get::<_, i64>(0))
            .optional()
            .ok()
            .flatten()
            .is_some_and(|b| b != 0)
    }

    /// The role an invite would grant, if it can still be used (without using it).
    pub fn invite_role(&self, code: &str) -> Option<Role> {
        self.conn
            .lock()
            .query_row(
                "SELECT grants_role FROM invites WHERE code = ?1 AND (max_uses IS NULL OR uses < max_uses)
                 AND (expires_at IS NULL OR expires_at > ?2)",
                params![code, now_ms()],
                |r| r.get::<_, i64>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(Role::from_i64)
    }

    /// How many usable invites someone has made.
    pub fn invites_by(&self, uid: &str) -> usize {
        self.conn
            .lock()
            .query_row(
                "SELECT COUNT(*) FROM invites WHERE created_by = ?1 AND (max_uses IS NULL OR uses < max_uses)
                 AND (expires_at IS NULL OR expires_at > ?2)",
                params![uid, now_ms()],
                |r| r.get::<_, i64>(0),
            )
            .unwrap_or(0) as usize
    }

    /// Once the server has an owner, old owner links that never expire are removed (they
    /// would make anyone who found one an owner too), as are invites past their date.
    pub fn prune_invites(&self) -> Result<()> {
        let c = self.conn.lock();
        c.execute("DELETE FROM invites WHERE expires_at IS NOT NULL AND expires_at < ?1", [now_ms()])?;
        let owner: i64 = c.query_row("SELECT COUNT(*) FROM users WHERE role = 2 AND banned = 0", [], |r| r.get(0)).unwrap_or(0);
        if owner > 0 {
            c.execute("DELETE FROM invites WHERE grants_role = 2 AND expires_at IS NULL", [])?;
        }
        Ok(())
    }

    /// Check an invite and count one use. Returns the role it grants.
    pub fn redeem_invite(&self, code: &str) -> std::result::Result<Role, InviteError> {
        let c = self.conn.lock();
        let row: Option<(Option<i64>, i64, Option<i64>, i64)> = c
            .query_row(
                "SELECT max_uses, uses, expires_at, grants_role FROM invites WHERE code = ?1",
                [code],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
            .ok()
            .flatten();
        let Some((max_uses, uses, expires_at, grants)) = row else { return Err(InviteError::NotFound) };
        if let Some(exp) = expires_at {
            if now_ms() > exp {
                return Err(InviteError::Expired);
            }
        }
        if let Some(max) = max_uses {
            if uses >= max {
                return Err(InviteError::UsedUp);
            }
        }
        let _ = c.execute("UPDATE invites SET uses = uses + 1 WHERE code = ?1", [code]);
        Ok(Role::from_i64(grants))
    }

    /// An invite that can still be used, if any (used to print links at startup).
    pub fn usable_invite(&self, grants: Role) -> Option<String> {
        self.conn
            .lock()
            .query_row(
                "SELECT code FROM invites WHERE grants_role = ?1 AND (max_uses IS NULL OR uses < max_uses)
                 AND (expires_at IS NULL OR expires_at > ?2) ORDER BY created_at LIMIT 1",
                params![grants.as_i64(), now_ms()],
                |r| r.get(0),
            )
            .optional()
            .ok()
            .flatten()
    }
}

/// A random invite code (lower-case letters and digits, no look-alikes; see
/// [`pyr_proto::directory::random_code`]).
pub fn new_invite_code(len: usize) -> String {
    pyr_proto::directory::random_code(len)
}

/// An invite as `ListInvites` shows it (and what the directory is told).
#[derive(Clone, Debug, PartialEq)]
pub struct InviteRow {
    pub code: String,
    pub created_by: String,
    pub created_at: i64,
    pub uses: u32,
    pub max_uses: Option<u32>,
    pub expires_at: Option<i64>,
    pub grants: Role,
    pub bot: bool,
    /// The server lists it at pyr.gg.
    pub listed: bool,
}

impl InviteRow {
    fn from_row(r: &rusqlite::Row) -> rusqlite::Result<InviteRow> {
        Ok(InviteRow {
            code: r.get(0)?,
            created_by: r.get(1)?,
            created_at: r.get(2)?,
            uses: r.get::<_, i64>(3)?.max(0) as u32,
            max_uses: r.get::<_, Option<i64>>(4)?.map(|n| n.max(0) as u32),
            expires_at: r.get(5)?,
            grants: Role::from_i64(r.get(6)?),
            bot: r.get::<_, i64>(7)? != 0,
            listed: r.get::<_, i64>(8)? != 0,
        })
    }
}

const INVITE_COLUMNS: &str = "code, created_by, created_at, uses, max_uses, expires_at, grants_role, bot, listed";
/// Invites that still work.
const USABLE: &str = "(max_uses IS NULL OR uses < max_uses) AND (expires_at IS NULL OR expires_at > ?1)";

impl Db {
    pub fn invite(&self, code: &str) -> Option<InviteRow> {
        self.conn
            .lock()
            .query_row(&format!("SELECT {INVITE_COLUMNS} FROM invites WHERE code = ?1"), [code], InviteRow::from_row)
            .optional()
            .ok()
            .flatten()
    }

    /// The invites that still work, newest first: someone's (`by`), or everyone's. Owner
    /// links (which make whoever uses one an owner) are left out.
    pub fn usable_invites(&self, by: Option<&str>) -> Vec<InviteRow> {
        let c = self.conn.lock();
        let sql = format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE {USABLE} AND grants_role != {} AND (?2 IS NULL OR created_by = ?2)
             ORDER BY created_at DESC LIMIT 2000",
            Role::Owner.as_i64()
        );
        c.prepare(&sql)
            .and_then(|mut s| s.query_map(params![now_ms(), by], InviteRow::from_row)?.collect())
            .unwrap_or_default()
    }

    /// The invites the server lists at pyr.gg: everyday ones that still work (the community's
    /// own code, `vanity`, goes separately).
    pub fn listed_invites(&self) -> Vec<InviteRow> {
        let c = self.conn.lock();
        let sql = format!(
            "SELECT {INVITE_COLUMNS} FROM invites WHERE {USABLE} AND listed = 1 AND grants_role = {} AND bot = 0
               AND code != COALESCE((SELECT value FROM meta WHERE key = 'vanity'), '')
             ORDER BY created_at DESC LIMIT {}",
            Role::Member.as_i64(),
            pyr_proto::directory::MAX_CODES
        );
        c.prepare(&sql).and_then(|mut s| s.query_map([now_ms()], InviteRow::from_row)?.collect()).unwrap_or_default()
    }

    /// An everyday invite of `uid`'s that still works, made with the same settings, with at
    /// least half its time left (what `CreateInvite { reuse }` hands back).
    pub fn reusable_invite(&self, uid: &str, max_uses: Option<u32>, expires_secs: Option<u64>, listed: bool) -> Option<InviteRow> {
        let now = now_ms();
        self.usable_invites(Some(uid)).into_iter().find(|i| {
            let same_time = match (expires_secs, i.expires_at) {
                (None, None) => true,
                (Some(s), Some(exp)) => {
                    let wanted = (s.min(365 * 86400) as i64) * 1000;
                    (exp - i.created_at - wanted).abs() < 2000 && exp - now >= wanted / 2
                }
                _ => false,
            };
            i.grants == Role::Member && !i.bot && i.max_uses == max_uses && i.listed == listed && same_time
                && self.get_meta("vanity").as_deref() != Some(i.code.as_str())
        })
    }

    pub fn set_listed(&self, code: &str, listed: bool) -> Result<()> {
        self.conn.lock().execute("UPDATE invites SET listed = ?2 WHERE code = ?1", params![code, listed as i64])?;
        Ok(())
    }

    /// Takes an invite back. Whether there was one.
    pub fn delete_invite(&self, code: &str) -> Result<bool> {
        Ok(self.conn.lock().execute("DELETE FROM invites WHERE code = ?1", [code])? > 0)
    }

    /// Members (not counting bots or the banned).
    pub fn member_count(&self) -> u32 {
        self.conn
            .lock()
            .query_row("SELECT COUNT(*) FROM users WHERE banned = 0 AND bot = 0", [], |r| r.get::<_, i64>(0))
            .map(|n| n.max(0) as u32)
            .unwrap_or(0)
    }
}

/// The most givers besides a key's owner whose copies of one key are kept for one receiver
/// (so nobody can crowd out the real one with junk).
const MAX_OTHER_GIVERS: i64 = 4;
/// The most keys one member may have asked for and not been given yet.
const MAX_WANTS: i64 = 5000;

// ------------------------------------------------------------- end-to-end encryption
impl Db {
    /// A member's message key (checked by the caller).
    pub fn set_message_key(&self, k: &MessageKey) -> Result<()> {
        self.conn.lock().execute(
            "INSERT OR REPLACE INTO message_keys(user_id, key, sig) VALUES (?1, ?2, ?3)",
            params![k.user, k.key, k.sig],
        )?;
        Ok(())
    }

    /// Message keys of these members (those who have one).
    pub fn message_keys(&self, uids: &[UserId]) -> HashMap<UserId, MessageKey> {
        let c = self.conn.lock();
        let mut st = c.prepare_cached("SELECT key, sig FROM message_keys WHERE user_id = ?1").expect("valid SQL");
        let mut out = HashMap::new();
        for u in uids {
            if let Ok(Some((key, sig))) = st.query_row([u], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))).optional() {
                out.insert(u.clone(), MessageKey { user: u.clone(), key, sig });
            }
        }
        out
    }

    /// Keeps keys `giver` gives in a conversation (each receiver checked by the caller); the
    /// ones kept, and what was asked for is answered.
    pub fn add_shares(&self, channel_id: ChannelId, giver: &str, shares: &[NewShare]) -> Result<Vec<NewShare>> {
        let mut c = self.conn.lock();
        let tx = c.transaction()?;
        let mut kept = Vec::new();
        for s in shares {
            if s.key.owner != giver {
                let others: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM key_shares WHERE channel_id = ?1 AND owner = ?2 AND key_id = ?3 AND recipient = ?4 AND giver != owner AND giver != ?5",
                    params![channel_id, s.key.owner, s.key.id, s.to, giver],
                    |r| r.get(0),
                )?;
                if others >= MAX_OTHER_GIVERS {
                    continue;
                }
            }
            tx.execute(
                "INSERT OR REPLACE INTO key_shares(channel_id, owner, key_id, recipient, giver, nonce, ct, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![channel_id, s.key.owner, s.key.id, s.to, giver, s.nonce, s.ct, now_ms()],
            )?;
            tx.execute(
                "DELETE FROM key_wants WHERE channel_id = ?1 AND owner = ?2 AND key_id = ?3 AND user_id = ?4",
                params![channel_id, s.key.owner, s.key.id, s.to],
            )?;
            kept.push(s.clone());
        }
        tx.commit()?;
        Ok(kept)
    }

    /// The keys given to `uid` in a conversation, of those asked for (with whoever gave them);
    /// the key's owner's copy first.
    pub fn shares_for(&self, channel_id: ChannelId, uid: &str, keys: &[KeyRef]) -> Vec<KeyShare> {
        let c = self.conn.lock();
        let mut st = c
            .prepare_cached(
                "SELECT s.giver, s.nonce, s.ct, m.key, m.sig FROM key_shares s JOIN message_keys m ON m.user_id = s.giver
                 WHERE s.channel_id = ?1 AND s.owner = ?2 AND s.key_id = ?3 AND s.recipient = ?4 ORDER BY s.giver != s.owner, s.at",
            )
            .expect("valid SQL");
        let mut out = Vec::new();
        for k in keys {
            let rows = st.query_map(params![channel_id, k.owner, k.id, uid], |r| {
                Ok(KeyShare {
                    key: k.clone(),
                    from: MessageKey { user: r.get(0)?, key: r.get(3)?, sig: r.get(4)? },
                    nonce: r.get(1)?,
                    ct: r.get(2)?,
                })
            });
            if let Ok(rows) = rows {
                out.extend(rows.flatten());
            }
        }
        out
    }

    /// Whether `uid` was given a key (or it's theirs).
    pub fn has_key(&self, channel_id: ChannelId, key: &KeyRef, uid: &str) -> bool {
        key.owner == uid
            || self
                .conn
                .lock()
                .prepare_cached("SELECT 1 FROM key_shares WHERE channel_id = ?1 AND owner = ?2 AND key_id = ?3 AND recipient = ?4")
                .and_then(|mut st| st.exists(params![channel_id, key.owner, key.id, uid]))
                .unwrap_or(false)
    }

    /// `uid` asked for these keys and nobody's given them yet.
    pub fn add_wants(&self, channel_id: ChannelId, uid: &str, keys: &[KeyRef]) -> Result<()> {
        let c = self.conn.lock();
        let have: i64 = c.query_row("SELECT COUNT(*) FROM key_wants WHERE user_id = ?1", [uid], |r| r.get(0))?;
        if have >= MAX_WANTS {
            return Ok(());
        }
        for k in keys {
            c.execute(
                "INSERT OR IGNORE INTO key_wants(channel_id, owner, key_id, user_id, at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![channel_id, k.owner, k.id, uid, now_ms()],
            )?;
        }
        Ok(())
    }

    /// Who has a key (was given it, its owner first): who to ask for it.
    pub fn holders(&self, channel_id: ChannelId, key: &KeyRef) -> Vec<UserId> {
        let c = self.conn.lock();
        c.prepare_cached(
            "SELECT recipient FROM key_shares WHERE channel_id = ?1 AND owner = ?2 AND key_id = ?3
             GROUP BY recipient ORDER BY recipient != ?2, MIN(at)",
        )
        .and_then(|mut st| st.query_map(params![channel_id, key.owner, key.id], |r| r.get(0)).map(|rows| rows.flatten().collect()))
        .unwrap_or_default()
    }

    /// What others asked for that `uid` has: (conversation, who asked, key), oldest first.
    pub fn wants_for_holder(&self, uid: &str, limit: usize) -> Vec<(ChannelId, UserId, KeyRef)> {
        let c = self.conn.lock();
        c.prepare_cached(
            "SELECT w.channel_id, w.user_id, w.owner, w.key_id FROM key_wants w
             WHERE w.user_id != ?1 AND EXISTS (SELECT 1 FROM key_shares s WHERE s.channel_id = w.channel_id
                AND s.owner = w.owner AND s.key_id = w.key_id AND s.recipient = ?1)
             ORDER BY w.at LIMIT ?2",
        )
        .and_then(|mut st| {
            st.query_map(params![uid, limit as i64], |r| Ok((r.get(0)?, r.get(1)?, KeyRef { owner: r.get(2)?, id: r.get(3)? })))
                .map(|rows| rows.flatten().collect())
        })
        .unwrap_or_default()
    }

    /// Asked-for keys that whoever asked can't read anymore aren't theirs to have.
    pub fn drop_wants(&self, channel_id: ChannelId, uid: &str) -> Result<()> {
        self.conn.lock().execute("DELETE FROM key_wants WHERE channel_id = ?1 AND user_id = ?2", params![channel_id, uid])?;
        Ok(())
    }

    /// A conversation's keys, when it's gone.
    fn forget_keys(c: &Connection, channel_id: ChannelId) -> Result<()> {
        c.execute("DELETE FROM key_shares WHERE channel_id = ?1", [channel_id])?;
        c.execute("DELETE FROM key_wants WHERE channel_id = ?1", [channel_id])?;
        Ok(())
    }
}

pub fn user_is_member(db: &Db, id: &UserId) -> bool {
    db.get_user(id).map(|u| !u.banned).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_paging_and_reactions() {
        let db = Db::open_in_memory().unwrap();
        let ch = db.create_channel("general", ChannelKind::Text).unwrap();
        for i in 0..25 {
            db.insert_message(ch.id, "u", &format!("m{i}"), None).unwrap();
        }
        let (page, complete) = db.history(ch.id, None, 10);
        assert_eq!(page.len(), 10);
        assert!(!complete);
        assert_eq!(page.last().unwrap().content, "m24");
        let (older, _) = db.history(ch.id, Some(page[0].id), 10);
        assert_eq!(older.last().unwrap().content, "m14");
        let (rest, complete) = db.history(ch.id, Some(older[0].id), 10);
        assert_eq!(rest.len(), 5);
        assert!(complete);

        let m = &page[0];
        db.set_reaction(m.id, "🔥", "a", true).unwrap();
        db.set_reaction(m.id, "🔥", "b", true).unwrap();
        db.set_reaction(m.id, "👍", "a", true).unwrap();
        let got = db.get_message(m.id).unwrap();
        assert_eq!(got.reactions.len(), 2);
        assert_eq!(got.reactions[0].users, vec!["a", "b"]);
        db.set_reaction(m.id, "🔥", "a", false).unwrap();
        assert_eq!(db.get_message(m.id).unwrap().reactions[0].users, vec!["b"]);
    }

    #[test]
    fn a_release_marks_the_reports_it_fixed() {
        use pyr_proto::control::{BugReport, BugStatus};
        let db = Db::open_in_memory().unwrap();
        let r = BugReport { title: "t".into(), ..Default::default() };
        let (a, b, c, d) = (db.add_bug(&r).unwrap(), db.add_bug(&r).unwrap(), db.add_bug(&r).unwrap(), db.add_bug(&r).unwrap());
        db.set_bug_status(a, BugStatus::Approved).unwrap();
        db.set_bug_status(c, BugStatus::Rejected).unwrap();
        db.set_bug_status(d, BugStatus::Fixed).unwrap();
        let out = db.mark_bugs_fixed(&[a, b, c, d, 999]).unwrap();
        let was: Vec<Option<BugStatus>> = out.iter().map(|x| x.1).collect();
        assert_eq!(was, vec![Some(BugStatus::Approved), Some(BugStatus::New), Some(BugStatus::Rejected), Some(BugStatus::Fixed), None]);
        assert_eq!(db.bug_status(a), Some(BugStatus::Fixed));
        assert_eq!(db.bug_status(b), Some(BugStatus::Fixed));
        assert_eq!(db.bug_status(c), Some(BugStatus::Rejected), "a rejected report stays rejected");
        assert_eq!(db.bug_status(999), None);
    }

    #[test]
    fn invites() {
        let db = Db::open_in_memory().unwrap();
        db.create_invite("once", "x", Some(1), None, Role::Owner).unwrap();
        assert_eq!(db.redeem_invite("once"), Ok(Role::Owner));
        assert_eq!(db.redeem_invite("once"), Err(InviteError::UsedUp));
        assert_eq!(db.redeem_invite("nope"), Err(InviteError::NotFound));
        db.create_invite("old", "x", None, Some(1), Role::Member).unwrap();
        assert_eq!(db.redeem_invite("old"), Err(InviteError::Expired));
        assert!(new_invite_code(8).len() == 8);
    }

    /// Each channel comes with its newest message and who wrote it (what the app marks unread
    /// after being away), and a deleted newest message hands that over to the one before.
    #[test]
    fn channels_know_their_newest_message() {
        let db = Db::open_in_memory().unwrap();
        let general = db.create_channel("general", ChannelKind::Text).unwrap().id;
        let quiet = db.create_channel("quiet", ChannelKind::Text).unwrap().id;
        let tavern = db.create_channel("tavern", ChannelKind::Voice).unwrap().id;
        db.insert_message(general, "a", "first", None).unwrap();
        let second = db.insert_message(general, "b", "second", None).unwrap();
        let in_voice = db.insert_message(tavern, "c", "in the voice chat", None).unwrap();
        let newest = |id| db.channels().into_iter().find(|c| c.id == id).map(|c| (c.last_message, c.last_from));
        assert_eq!(newest(general), Some((Some(second.id), Some("b".to_string()))));
        assert_eq!(newest(quiet), Some((None, None)));
        assert_eq!(newest(tavern), Some((Some(in_voice.id), Some("c".to_string()))));
        let third = db.insert_message(general, "a", "third", None).unwrap();
        assert_eq!(newest(general), Some((Some(third.id), Some("a".to_string()))));
        db.delete_message(third.id).unwrap();
        assert_eq!(newest(general), Some((Some(second.id), Some("b".to_string()))));
    }
}

#[cfg(test)]
mod scrub_tests {
    use super::*;

    /// A deleted or edited-away message must not survive anywhere in the database files.
    #[test]
    fn deleted_and_edited_text_leaves_no_trace_on_disk() {
        let dir = std::env::temp_dir().join(format!("pyr-scrub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.db");
        let db = Db::open(&path).unwrap();
        let ch = db.create_channel("general", ChannelKind::Text).unwrap().id;
        let secret = "the-secret-plan-zebra-7731";
        let edited = "the-old-version-giraffe-4410";
        let m1 = db.insert_message(ch, "u1", secret, None).unwrap();
        let m2 = db.insert_message(ch, "u1", edited, None).unwrap();
        db.edit_message(m2.id, "fine now", None).unwrap();
        db.delete_message(m1.id).unwrap();
        let mut found = Vec::new();
        for entry in std::fs::read_dir(&dir).unwrap() {
            let p = entry.unwrap().path();
            let bytes = std::fs::read(&p).unwrap();
            for needle in [secret, edited] {
                if bytes.windows(needle.len()).any(|w| w == needle.as_bytes()) {
                    found.push(format!("{needle} in {}", p.display()));
                }
            }
        }
        drop(db);
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found.is_empty(), "left behind: {found:?}");
    }
}
