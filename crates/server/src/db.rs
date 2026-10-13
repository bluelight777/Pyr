//! SQLite persistence: members, channels, message history, reactions and invites.

use std::path::Path;

use anyhow::{Context, Result};
use ingle_proto::control::{CustomEmoji, Relation, RelationKind, 
    Attachment, Channel, ChannelId, ChannelKind, Dm, FileKind, Member, Message, MessageId, Reaction, Role, RoleDef, RoleId, UserId,
};
use ingle_proto::now_ms;
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
"#;

/// `channels.kind` of a direct message conversation (never listed as a channel).
const DM_KIND: i64 = 2;

fn file_kind_to_i(k: FileKind) -> i64 {
    match k {
        FileKind::Attachment => 0,
        FileKind::Avatar => 1,
    }
}

fn file_kind_from_i(v: i64) -> FileKind {
    if v == 1 { FileKind::Avatar } else { FileKind::Attachment }
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

/// Columns added after the first release.
fn migrate(c: &Connection) -> Result<()> {
    let has_avatar = c
        .prepare("SELECT 1 FROM pragma_table_info('users') WHERE name = 'avatar'")?
        .exists([])?;
    if !has_avatar {
        c.execute_batch("ALTER TABLE users ADD COLUMN avatar TEXT")?;
    }
    Ok(())
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

    // ---------------------------------------------------------------- users

    pub fn get_user(&self, id: &str) -> Option<UserRow> {
        self.conn
            .lock()
            .query_row("SELECT name, role, banned, joined_at, avatar FROM users WHERE id = ?1", [id], |r| {
                Ok(UserRow {
                    name: r.get(0)?,
                    role: Role::from_i64(r.get(1)?),
                    banned: r.get::<_, i64>(2)? != 0,
                    joined_at: r.get(3)?,
                    avatar: r.get(4)?,
                })
            })
            .optional()
            .ok()
            .flatten()
    }

    /// Adds a member. False if they're banned (a ban made meanwhile is never undone here).
    pub fn insert_user(&self, id: &str, name: &str, role: Role) -> Result<bool> {
        let now = now_ms();
        let n = self.conn.lock().execute(
            "INSERT INTO users(id, name, role, banned, joined_at, last_seen) VALUES (?1, ?2, ?3, 0, ?4, ?4)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name, role = excluded.role, joined_at = excluded.joined_at
             WHERE users.banned = 0",
            params![id, name, role.as_i64(), now],
        )?;
        Ok(n > 0)
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

    /// Remove membership. A banned user keeps a row with `banned = 1`.
    pub fn remove_user(&self, id: &str, ban: bool) -> Result<()> {
        let c = self.conn.lock();
        if ban {
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
            .prepare("SELECT id, name, role, joined_at, avatar FROM users WHERE banned = 0 ORDER BY joined_at")
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
            .query_row("SELECT COUNT(*) FROM files WHERE owner = ?1 AND kind = 0 AND message_id IS NULL", [uid], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap_or(0) as usize
    }

    /// Attachments uploaded but not sent within `max_age_ms`, and profile pictures nobody
    /// uses: their rows are removed and their ids returned so the files can be wiped.
    pub fn take_unused_files(&self, max_age_ms: i64) -> Vec<String> {
        let c = self.conn.lock();
        let cutoff = now_ms() - max_age_ms;
        let sql = "SELECT id FROM files WHERE created_at < ?1 AND (
                       (kind = 0 AND message_id IS NULL)
                    OR (kind = 1 AND id NOT IN (SELECT avatar FROM users WHERE avatar IS NOT NULL)
                                 AND id NOT IN (SELECT value FROM meta WHERE key = 'icon')
                                 AND id NOT IN (SELECT id FROM emojis)))";
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

    pub fn channels(&self) -> Vec<Channel> {
        let c = self.conn.lock();
        let mut st = c
            .prepare("SELECT id, kind, name, topic, position FROM channels WHERE kind < 2 ORDER BY kind, position, id")
            .expect("valid SQL");
        st.query_map([], |r| {
            Ok(Channel {
                id: r.get(0)?,
                kind: kind_from_i(r.get(1)?),
                name: r.get(2)?,
                topic: r.get(3)?,
                position: r.get(4)?,
            })
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
    }

    pub fn channel(&self, id: ChannelId) -> Option<Channel> {
        self.conn
            .lock()
            .query_row("SELECT id, kind, name, topic, position FROM channels WHERE id = ?1 AND kind < 2", [id], |r| {
                Ok(Channel {
                    id: r.get(0)?,
                    kind: kind_from_i(r.get(1)?),
                    name: r.get(2)?,
                    topic: r.get(3)?,
                    position: r.get(4)?,
                })
            })
            .optional()
            .ok()
            .flatten()
    }

    pub fn create_channel(&self, name: &str, kind: ChannelKind) -> Result<Channel> {
        let c = self.conn.lock();
        let position: i32 = c
            .query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM channels WHERE kind = ?1", [kind_to_i(kind)], |r| {
                r.get(0)
            })
            .unwrap_or(0);
        c.execute(
            "INSERT INTO channels(kind, name, topic, position, created_at) VALUES (?1, ?2, '', ?3, ?4)",
            params![kind_to_i(kind), name, position, now_ms()],
        )?;
        Ok(Channel { id: c.last_insert_rowid(), kind, name: name.to_string(), topic: String::new(), position })
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
        self.insert_message_with(channel_id, author, content, reply_to, &[])
    }

    /// A message with files the author uploaded (checked by the caller); links them to it.
    pub fn insert_message_with(
        &self,
        channel_id: ChannelId,
        author: &str,
        content: &str,
        reply_to: Option<MessageId>,
        files: &[String],
    ) -> Result<Message> {
        let now = now_ms();
        let c = self.conn.lock();
        c.execute(
            "INSERT INTO messages(channel_id, author, content, created_at, reply_to) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![channel_id, author, content, now, reply_to],
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
            content: content.to_string(),
            created_at: now,
            edited_at: None,
            reply_to,
            reactions: Vec::new(),
            attachments,
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
        Ok(Message {
            id: r.get(0)?,
            channel_id: r.get(1)?,
            author: r.get(2)?,
            content: r.get(3)?,
            created_at: r.get(4)?,
            edited_at: r.get(5)?,
            reply_to: r.get(6)?,
            reactions: Vec::new(),
            attachments: Vec::new(),
        })
    }

    pub fn get_message(&self, id: MessageId) -> Option<Message> {
        let c = self.conn.lock();
        let mut m = c
            .query_row(
                "SELECT id, channel_id, author, content, created_at, edited_at, reply_to FROM messages WHERE id = ?1",
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

    pub fn edit_message(&self, id: MessageId, content: &str) -> Result<Option<Message>> {
        {
            let c = self.conn.lock();
            c.execute("UPDATE messages SET content = ?2, edited_at = ?3 WHERE id = ?1", params![id, content, now_ms()])?;
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
                "SELECT id, channel_id, author, content, created_at, edited_at, reply_to FROM messages
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

/// A random invite code (lower-case letters and digits, no look-alikes).
pub fn new_invite_code(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let bytes = ingle_proto::crypto::random_bytes::<32>();
    bytes.iter().take(len).map(|b| ALPHABET[*b as usize % ALPHABET.len()] as char).collect()
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
}

#[cfg(test)]
mod scrub_tests {
    use super::*;

    /// A deleted or edited-away message must not survive anywhere in the database files.
    #[test]
    fn deleted_and_edited_text_leaves_no_trace_on_disk() {
        let dir = std::env::temp_dir().join(format!("ingle-scrub-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("server.db");
        let db = Db::open(&path).unwrap();
        let ch = db.create_channel("general", ChannelKind::Text).unwrap().id;
        let secret = "the-secret-plan-zebra-7731";
        let edited = "the-old-version-giraffe-4410";
        let m1 = db.insert_message(ch, "u1", secret, None).unwrap();
        let m2 = db.insert_message(ch, "u1", edited, None).unwrap();
        db.edit_message(m2.id, "fine now").unwrap();
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
