//! What the directory keeps (SQLite): the communities that announced themselves, and their
//! listed codes.

use std::path::Path;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use pyr_proto::directory::{Announce, AnnounceReply, CODE_MAX, INVITER_MAX, Lookup, MAX_CODES, NAME_MAX, is_code, is_reserved, vanity_problem};
use pyr_proto::invite::InviteLink;
use pyr_proto::now_ms;
use rusqlite::{Connection, OptionalExtension, params};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS servers (
    fp BLOB PRIMARY KEY,
    cert BLOB NOT NULL,
    host TEXT NOT NULL,
    port INTEGER NOT NULL,
    name TEXT NOT NULL,
    members INTEGER,
    online INTEGER,
    last_ts INTEGER NOT NULL,
    seen_at INTEGER NOT NULL,
    first_ip TEXT NOT NULL DEFAULT '',
    blocked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS codes (
    code TEXT PRIMARY KEY,
    fp BLOB NOT NULL,
    expires_at INTEGER,
    inviter TEXT,
    vanity INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS codes_fp ON codes(fp);
CREATE TABLE IF NOT EXISTS blocked_codes (code TEXT PRIMARY KEY, at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS new_servers (place TEXT NOT NULL, at INTEGER NOT NULL);
";

/// A community that hasn't announced itself for this long is forgotten (servers announce at
/// least every few hours while they run).
pub const FORGET_AFTER_MS: i64 = 30 * 86_400_000;
/// An announcement can't be from further ahead than this (clocks differ a little).
pub const CLOCK_SLACK_MS: i64 = 10 * 60_000;
/// New communities one place (an address, or a /64 for IPv6) can announce in a day.
pub const NEW_SERVERS_PER_PLACE: i64 = 20;
/// In all, so a flood can't fill the disk.
pub const MAX_SERVERS: i64 = 200_000;
pub const MAX_ALL_CODES: i64 = 2_000_000;

/// Why an announcement wasn't taken (each has its own HTTP status).
#[derive(Debug, PartialEq)]
pub enum Refused {
    /// Older than (or as old as) one already taken: a replay, or a clock gone backwards.
    Old,
    /// From too far in the future.
    Clock,
    /// Firelit Games took this community off the directory.
    Blocked,
    /// Too many new communities from one place today.
    TooManyNew,
    /// The directory is full.
    Full,
    /// Something in it makes no sense.
    Bad(String),
}

impl Refused {
    pub fn status(&self) -> u16 {
        match self {
            Refused::Old => 409,
            Refused::Clock | Refused::Bad(_) => 400,
            Refused::Blocked => 403,
            Refused::TooManyNew => 429,
            Refused::Full => 503,
        }
    }

    pub fn message(&self) -> String {
        match self {
            Refused::Old => "An announcement as new or newer was taken already.".into(),
            Refused::Clock => "This server's clock is ahead: set it right (or turn on time sync) and try again.".into(),
            Refused::Blocked => "pyr.gg doesn't list this server.".into(),
            Refused::TooManyNew => "Too many new servers from this address today. Try again tomorrow.".into(),
            Refused::Full => "pyr.gg's directory is full right now.".into(),
            Refused::Bad(why) => why.clone(),
        }
    }
}

pub struct Db {
    conn: Mutex<Connection>,
}

/// A community name or an inviter's name as shown: no control characters, trimmed, at most
/// `max` characters.
pub fn clean_name(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).collect::<String>().trim().chars().take(max).collect::<String>().trim().to_string()
}

/// A host as it goes into links: a name or an address, nothing that could break out of one.
fn valid_host(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '_'))
        && !h.starts_with(['-', '.'])
}

impl Db {
    pub fn open(path: &Path) -> Result<Db> {
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db { conn: Mutex::new(conn) })
    }

    pub fn open_in_memory() -> Result<Db> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db { conn: Mutex::new(conn) })
    }

    /// Takes an announcement whose signature checked out (the community is `fp`, whose
    /// certificate is `cert`), from `place`: makes the community's listed codes match it.
    pub fn apply(&self, fp: &[u8; 32], cert: &[u8], place: &str, a: &Announce) -> Result<AnnounceReply, Refused> {
        let now = now_ms();
        if a.ts > now + CLOCK_SLACK_MS {
            return Err(Refused::Clock);
        }
        let host = a.host.trim().trim_start_matches('[').trim_end_matches(']');
        if !valid_host(host) {
            return Err(Refused::Bad("The server's address isn't one a link can hold.".into()));
        }
        if a.port == 0 {
            return Err(Refused::Bad("The server's port can't be 0.".into()));
        }
        if a.codes.len() > MAX_CODES {
            return Err(Refused::Bad(format!("At most {MAX_CODES} invites can be listed.")));
        }
        let name = match clean_name(&a.name, NAME_MAX) {
            n if n.is_empty() => "A Pyr server".to_string(),
            n => n,
        };
        let count = |n: Option<u32>| n.map(|n| n.min(100_000_000) as i64);
        let mut c = self.conn.lock();
        let tx = c.transaction().map_err(internal)?;
        let known: Option<(i64, bool)> = tx
            .query_row("SELECT last_ts, blocked FROM servers WHERE fp = ?1", [&fp[..]], |r| Ok((r.get(0)?, r.get::<_, i64>(1)? != 0)))
            .optional()
            .map_err(internal)?;
        match known {
            Some((_, true)) => return Err(Refused::Blocked),
            Some((last, false)) if a.ts <= last => return Err(Refused::Old),
            Some(_) => {}
            None => {
                let servers: i64 = tx.query_row("SELECT COUNT(*) FROM servers", [], |r| r.get(0)).map_err(internal)?;
                if servers >= MAX_SERVERS {
                    return Err(Refused::Full);
                }
                let from_here: i64 = tx
                    .query_row("SELECT COUNT(*) FROM new_servers WHERE place = ?1 AND at > ?2", params![place, now - 86_400_000], |r| r.get(0))
                    .map_err(internal)?;
                if from_here >= NEW_SERVERS_PER_PLACE {
                    return Err(Refused::TooManyNew);
                }
                tx.execute("INSERT INTO new_servers(place, at) VALUES (?1, ?2)", params![place, now]).map_err(internal)?;
            }
        }
        tx.execute(
            "INSERT INTO servers(fp, cert, host, port, name, members, online, last_ts, seen_at, first_ip)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(fp) DO UPDATE SET cert = excluded.cert, host = excluded.host, port = excluded.port,
                name = excluded.name, members = COALESCE(excluded.members, members),
                online = COALESCE(excluded.online, online), last_ts = excluded.last_ts, seen_at = excluded.seen_at",
            params![&fp[..], cert, host, a.port, name, count(a.members), count(a.online), a.ts, now, place],
        )
        .map_err(internal)?;

        let mut reply = AnnounceReply::default();
        // What it wants listed: (code, expires, inviter, its own code).
        let mut wanted: Vec<(String, Option<i64>, Option<String>, bool)> = Vec::new();
        for l in &a.codes {
            let code = l.code.trim().to_ascii_lowercase();
            if !is_code(&code) || is_reserved(&code) || code.len() > CODE_MAX {
                reply.taken.push(l.code.clone());
                continue;
            }
            if l.expires_at.is_some_and(|e| e <= now) || wanted.iter().any(|w| w.0 == code) {
                continue;
            }
            let inviter = l.inviter.as_deref().map(|n| clean_name(n, INVITER_MAX)).filter(|n| !n.is_empty());
            wanted.push((code, l.expires_at, inviter, false));
        }
        if let Some(v) = &a.vanity {
            let v = v.trim().to_ascii_lowercase();
            match vanity_problem(&v) {
                Some(why) => reply.vanity_refused = Some(why.to_string()),
                None => {
                    wanted.retain(|w| w.0 != v);
                    wanted.push((v, None, None, true));
                }
            }
        }
        // Codes it no longer lists go.
        let had: Vec<String> = tx
            .prepare("SELECT code FROM codes WHERE fp = ?1")
            .and_then(|mut s| s.query_map([&fp[..]], |r| r.get(0))?.collect())
            .map_err(internal)?;
        for code in &had {
            if !wanted.iter().any(|w| &w.0 == code) {
                tx.execute("DELETE FROM codes WHERE code = ?1", [code]).map_err(internal)?;
            }
        }
        let mut all: i64 = tx.query_row("SELECT COUNT(*) FROM codes", [], |r| r.get(0)).map_err(internal)?;
        for (code, expires_at, inviter, vanity) in wanted {
            let refuse = |reply: &mut AnnounceReply, why: &str| {
                if vanity {
                    reply.vanity_refused = Some(why.to_string());
                } else {
                    reply.taken.push(code.clone());
                }
            };
            let blocked = tx.query_row("SELECT 1 FROM blocked_codes WHERE code = ?1", [&code], |_| Ok(())).optional().map_err(internal)?.is_some();
            if blocked {
                refuse(&mut reply, "That one isn't available.");
                continue;
            }
            let holder: Option<(Vec<u8>, Option<i64>, i64)> = tx
                .query_row(
                    "SELECT c.fp, c.expires_at, COALESCE(s.seen_at, 0) FROM codes c LEFT JOIN servers s ON s.fp = c.fp WHERE c.code = ?1",
                    [&code],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(internal)?;
            match holder {
                Some((other, exp, seen)) if other.as_slice() != &fp[..] => {
                    // Another community's, unless it's run out or that community is gone.
                    let gone = exp.is_some_and(|e| e <= now) || seen < now - FORGET_AFTER_MS;
                    if !gone {
                        refuse(&mut reply, "Another community has that one already.");
                        continue;
                    }
                    tx.execute("DELETE FROM codes WHERE code = ?1", [&code]).map_err(internal)?;
                    all -= 1;
                }
                Some(_) => {
                    tx.execute(
                        "UPDATE codes SET expires_at = ?2, inviter = ?3, vanity = ?4 WHERE code = ?1",
                        params![code, expires_at, inviter, vanity as i64],
                    )
                    .map_err(internal)?;
                    continue;
                }
                None => {}
            }
            if all >= MAX_ALL_CODES {
                refuse(&mut reply, "pyr.gg's directory is full right now.");
                continue;
            }
            tx.execute(
                "INSERT INTO codes(code, fp, expires_at, inviter, vanity, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![code, &fp[..], expires_at, inviter, vanity as i64, now],
            )
            .map_err(internal)?;
            all += 1;
        }
        tx.commit().map_err(internal)?;
        Ok(reply)
    }

    /// What a code opens, while it does.
    pub fn lookup(&self, code: &str) -> Option<Lookup> {
        let now = now_ms();
        let c = self.conn.lock();
        let row: Option<(Vec<u8>, String, u16, String, Option<i64>, Option<i64>, Option<String>, Option<i64>, bool)> = c
            .query_row(
                "SELECT s.fp, s.host, s.port, s.name, s.members, s.online, c.inviter, c.expires_at, c.vanity
                 FROM codes c JOIN servers s ON s.fp = c.fp
                 WHERE c.code = ?1 AND s.blocked = 0 AND s.seen_at > ?2 AND (c.expires_at IS NULL OR c.expires_at > ?3)
                   AND c.code NOT IN (SELECT code FROM blocked_codes)",
                params![code, now - FORGET_AFTER_MS, now],
                |r| {
                    Ok((
                        r.get(0)?,
                        r.get(1)?,
                        r.get(2)?,
                        r.get(3)?,
                        r.get(4)?,
                        r.get(5)?,
                        r.get(6)?,
                        r.get(7)?,
                        r.get::<_, i64>(8)? != 0,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten();
        let (fp, host, port, name, members, online, inviter, expires_at, vanity) = row?;
        let fp: [u8; 32] = fp.try_into().ok()?;
        Some(Lookup {
            code: code.to_string(),
            name,
            link: InviteLink::new(host, port, Some(code.to_string()), fp).to_url(),
            members: members.map(|n| n as u32),
            online: online.map(|n| n as u32),
            inviter,
            expires_at,
            vanity,
        })
    }

    /// Forgets codes that ran out, communities not heard from in a month, and old counts of
    /// new communities.
    pub fn tidy(&self) -> Result<()> {
        let now = now_ms();
        let c = self.conn.lock();
        c.execute("DELETE FROM codes WHERE expires_at IS NOT NULL AND expires_at <= ?1", [now])?;
        c.execute("DELETE FROM codes WHERE fp IN (SELECT fp FROM servers WHERE seen_at < ?1 AND blocked = 0)", [now - FORGET_AFTER_MS])?;
        c.execute("DELETE FROM servers WHERE seen_at < ?1 AND blocked = 0", [now - FORGET_AFTER_MS])?;
        c.execute("DELETE FROM new_servers WHERE at < ?1", [now - 86_400_000])?;
        Ok(())
    }

    /// Takes a code down and keeps it from being listed again (Firelit Games' own doing).
    /// Whether it was listed.
    pub fn block_code(&self, code: &str) -> Result<bool> {
        let c = self.conn.lock();
        c.execute("INSERT OR IGNORE INTO blocked_codes(code, at) VALUES (?1, ?2)", params![code, now_ms()])?;
        Ok(c.execute("DELETE FROM codes WHERE code = ?1", [code])? > 0)
    }

    pub fn unblock_code(&self, code: &str) -> Result<bool> {
        Ok(self.conn.lock().execute("DELETE FROM blocked_codes WHERE code = ?1", [code])? > 0)
    }

    /// Takes a whole community off the directory (all its codes), for good; by any of its
    /// codes. Its name, if there was one.
    pub fn block_server_of(&self, code: &str) -> Result<Option<String>> {
        let c = self.conn.lock();
        let found: Option<(Vec<u8>, String)> = c
            .query_row("SELECT s.fp, s.name FROM codes c JOIN servers s ON s.fp = c.fp WHERE c.code = ?1", [code], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((fp, name)) = found else { return Ok(None) };
        c.execute("UPDATE servers SET blocked = 1 WHERE fp = ?1", [&fp])?;
        c.execute("DELETE FROM codes WHERE fp = ?1", [&fp])?;
        Ok(Some(name))
    }

    /// (communities, listed codes).
    pub fn counts(&self) -> (i64, i64) {
        let c = self.conn.lock();
        let n = |q: &str| c.query_row(q, [], |r| r.get(0)).unwrap_or(0);
        (n("SELECT COUNT(*) FROM servers WHERE blocked = 0"), n("SELECT COUNT(*) FROM codes"))
    }
}

fn internal(e: rusqlite::Error) -> Refused {
    tracing::warn!("directory database: {e}");
    Refused::Bad("Something went wrong on pyr.gg. Try again in a moment.".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_proto::directory::ListedCode;

    fn ann(ts: i64, codes: &[&str]) -> Announce {
        Announce {
            ts,
            host: "2.24.121.231".into(),
            port: 7766,
            name: "Brethren of Blood".into(),
            members: Some(42),
            online: Some(7),
            codes: codes.iter().map(|c| ListedCode { code: c.to_string(), expires_at: None, inviter: Some("Blue".into()) }).collect(),
            vanity: None,
        }
    }

    #[test]
    fn a_community_lists_its_codes_and_takes_them_back() {
        let db = Db::open_in_memory().unwrap();
        let fp = [7u8; 32];
        let t = now_ms() - 1000;
        let r = db.apply(&fp, b"cert", "1.2.3.4", &ann(t, &["qrslmnov", "abcdefgh"])).unwrap();
        assert_eq!(r, AnnounceReply::default());
        let l = db.lookup("qrslmnov").unwrap();
        assert_eq!((l.name.as_str(), l.members, l.online, l.inviter.as_deref(), l.vanity), ("Brethren of Blood", Some(42), Some(7), Some("Blue"), false));
        assert_eq!(l.link, InviteLink::new("2.24.121.231".into(), 7766, Some("qrslmnov".into()), fp).to_url());
        // The same again (or older): a replay, refused.
        assert_eq!(db.apply(&fp, b"cert", "1.2.3.4", &ann(t, &[])), Err(Refused::Old));
        // A newer list without one: that one goes, the other stays.
        db.apply(&fp, b"cert", "1.2.3.4", &ann(t + 1, &["abcdefgh"])).unwrap();
        assert!(db.lookup("qrslmnov").is_none());
        assert!(db.lookup("abcdefgh").is_some());
        // From the future: refused.
        assert_eq!(db.apply(&fp, b"cert", "1.2.3.4", &ann(now_ms() + CLOCK_SLACK_MS + 60_000, &[])), Err(Refused::Clock));
        assert_eq!(db.counts(), (1, 1));
    }

    #[test]
    fn codes_are_first_come_first_served() {
        let db = Db::open_in_memory().unwrap();
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let t = now_ms() - 1000;
        db.apply(&a, b"a", "1.1.1.1", &ann(t, &["qrslmnov"])).unwrap();
        let r = db.apply(&b, b"b", "2.2.2.2", &ann(t, &["qrslmnov", "zzzzzzzz", "Bad Code", "download"])).unwrap();
        let mut taken = r.taken.clone();
        taken.sort();
        assert_eq!(taken, vec!["Bad Code".to_string(), "download".to_string(), "qrslmnov".to_string()]);
        assert!(db.lookup("qrslmnov").unwrap().link.contains(&pyr_proto::b64::encode(&a[..9])));
        assert!(db.lookup("zzzzzzzz").is_some());
        // A code that ran out can be had again.
        let mut first = ann(t + 1, &[]);
        first.codes.push(ListedCode { code: "expiring".into(), expires_at: Some(now_ms() + 50), inviter: None });
        db.apply(&a, b"a", "1.1.1.1", &first).unwrap();
        assert!(db.lookup("expiring").is_some());
        std::thread::sleep(std::time::Duration::from_millis(80));
        assert!(db.lookup("expiring").is_none(), "ran out");
        let r = db.apply(&b, b"b", "2.2.2.2", &ann(t + 2, &["expiring"])).unwrap();
        assert!(r.taken.is_empty(), "{r:?}");
        assert!(db.lookup("expiring").unwrap().link.contains(&pyr_proto::b64::encode(&b[..9])));
    }

    #[test]
    fn a_communitys_own_code() {
        let db = Db::open_in_memory().unwrap();
        let (a, b) = ([1u8; 32], [2u8; 32]);
        let t = now_ms() - 1000;
        let mut x = ann(t, &[]);
        x.vanity = Some("brethren".into());
        assert_eq!(db.apply(&a, b"a", "1.1.1.1", &x).unwrap(), AnnounceReply::default());
        let l = db.lookup("brethren").unwrap();
        assert!(l.vanity && l.inviter.is_none() && l.expires_at.is_none());
        // Someone else's wish for it, and a kept one.
        let mut y = ann(t, &[]);
        y.vanity = Some("brethren".into());
        assert!(db.apply(&b, b"b", "2.2.2.2", &y).unwrap().vanity_refused.is_some());
        y.ts += 1;
        y.vanity = Some("pyr-official".into());
        assert!(db.apply(&b, b"b", "2.2.2.2", &y).unwrap().vanity_refused.is_some());
        // Changing it frees the old one.
        x.ts += 1;
        x.vanity = Some("bob".into());
        db.apply(&a, b"a", "1.1.1.1", &x).unwrap();
        assert!(db.lookup("brethren").is_none() && db.lookup("bob").is_some());
        // Firelit Games can take one down for good, or a whole community.
        assert!(db.block_code("bob").unwrap());
        assert!(db.lookup("bob").is_none());
        x.ts += 1;
        assert_eq!(db.apply(&a, b"a", "1.1.1.1", &x).unwrap().vanity_refused.as_deref(), Some("That one isn't available."));
        x.ts += 1;
        x.codes = vec![ListedCode { code: "abcdefgh".into(), expires_at: None, inviter: None }];
        db.apply(&a, b"a", "1.1.1.1", &x).unwrap();
        assert_eq!(db.block_server_of("abcdefgh").unwrap().as_deref(), Some("Brethren of Blood"));
        x.ts += 1;
        assert_eq!(db.apply(&a, b"a", "1.1.1.1", &x), Err(Refused::Blocked));
        assert!(db.lookup("abcdefgh").is_none());
    }

    #[test]
    fn floods_are_turned_away() {
        let db = Db::open_in_memory().unwrap();
        let t = now_ms() - 1000;
        for i in 0..NEW_SERVERS_PER_PLACE {
            let mut fp = [0u8; 32];
            fp[0] = i as u8;
            db.apply(&fp, b"c", "6.6.6.6", &ann(t, &[])).unwrap();
        }
        assert_eq!(db.apply(&[99u8; 32], b"c", "6.6.6.6", &ann(t, &[])), Err(Refused::TooManyNew));
        assert!(db.apply(&[99u8; 32], b"c", "7.7.7.7", &ann(t, &[])).is_ok(), "another place");
        // Too many codes, a host that could break a link, names cleaned up.
        let many: Vec<String> = (0..MAX_CODES + 1).map(|i| format!("code{i:05}")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(matches!(db.apply(&[98u8; 32], b"c", "8.8.8.8", &ann(t, &refs)), Err(Refused::Bad(_))));
        let mut bad = ann(t, &[]);
        bad.host = "evil.example/x?".into();
        assert!(matches!(db.apply(&[97u8; 32], b"c", "8.8.8.8", &bad), Err(Refused::Bad(_))));
        let mut odd = ann(t, &["qrstuvwx"]);
        odd.name = "  \u{7}Camp\nfire  ".into();
        db.apply(&[96u8; 32], b"c", "8.8.8.8", &odd).unwrap();
        assert_eq!(db.lookup("qrstuvwx").unwrap().name, "Campfire");
        assert_eq!(clean_name(&"x".repeat(500), NAME_MAX).len(), NAME_MAX);
    }
}
