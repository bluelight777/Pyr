//! The server's submissions, read (and their status set) straight in its database, as the
//! server's own user on the same machine. Only `plugin_submissions` is touched.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use pyr_proto::control::SourceFile;
use rusqlite::{Connection, OptionalExtension, params};

/// A submission as the Keeper needs it.
#[derive(Clone, Debug)]
pub struct Sub {
    pub id: i64,
    pub status: String,
    pub name: String,
    pub kind: String,
    pub version: String,
    pub summary: String,
    pub author: String,
    pub author_id: String,
    pub update_of: i64,
    pub note: String,
    pub files: Vec<SourceFile>,
}

impl Sub {
    /// Its files, decoded (path, bytes); none if any doesn't decode.
    pub fn decoded(&self) -> Option<Vec<(String, Vec<u8>)>> {
        self.files.iter().map(|f| pyr_proto::b64::decode(&f.data).ok().map(|b| (f.path.clone(), b))).collect()
    }
}

pub struct Store {
    conn: Connection,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let conn = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        conn.busy_timeout(Duration::from_secs(15))?;
        Ok(Store { conn })
    }

    /// New versions waiting (signed, of an earlier submission), oldest first.
    pub fn pending(&self) -> Result<Vec<i64>> {
        let mut q = self.conn.prepare("SELECT id FROM plugin_submissions WHERE status = 'new' AND update_of != 0 AND author_id != '' ORDER BY id")?;
        let ids = q.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<Vec<i64>>>()?;
        Ok(ids)
    }

    pub fn get(&self, id: i64) -> Result<Option<Sub>> {
        let row = self
            .conn
            .query_row(
                "SELECT id, status, name, kind, version, summary, author, author_id, update_of, note, files FROM plugin_submissions WHERE id = ?1",
                [id],
                |r| {
                    Ok((
                        Sub {
                            id: r.get(0)?,
                            status: r.get(1)?,
                            name: r.get(2)?,
                            kind: r.get(3)?,
                            version: r.get(4)?,
                            summary: r.get(5)?,
                            author: r.get(6)?,
                            author_id: r.get::<_, Option<String>>(7)?.unwrap_or_default(),
                            update_of: r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                            note: r.get::<_, Option<String>>(9)?.unwrap_or_default(),
                            files: Vec::new(),
                        },
                        r.get::<_, String>(10)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(|(mut s, files)| {
            s.files = serde_json::from_str(&files).unwrap_or_default();
            s
        }))
    }

    /// The submission live in the Library that `id` is a new version of: following
    /// `update_of` back to the first one listed (by the same sender all the way: the server
    /// only takes a new version from whoever sent the one before).
    pub fn listed_ancestor(&self, id: i64) -> Result<Option<Sub>> {
        let Some(mut s) = self.get(id)? else { return Ok(None) };
        let author = s.author_id.clone();
        for _ in 0..200 {
            if s.update_of == 0 {
                return Ok(None);
            }
            let Some(before) = self.get(s.update_of)? else { return Ok(None) };
            if before.author_id != author {
                return Ok(None);
            }
            if before.status == "listed" {
                return Ok(Some(before));
            }
            s = before;
        }
        Ok(None)
    }

    pub fn set_status(&self, id: i64, status: &str, note: &str) -> Result<()> {
        self.conn.execute("UPDATE plugin_submissions SET status = ?2, status_at = ?3, note = ?4 WHERE id = ?1", params![id, status, now_ms(), note])?;
        Ok(())
    }

    /// A report in the server's bug reports, for its owner (see `alert`).
    pub fn add_report(&self, title: &str, body: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO bug_reports(created_at, title, body, app_version, os) VALUES (?1, ?2, ?3, ?4, 'pyr-keeper')",
            params![now_ms(), title, body, env!("CARGO_PKG_VERSION")],
        )?;
        Ok(())
    }

    /// A note for the sender, its status left as it is (waiting for a person).
    pub fn set_note(&self, id: i64, note: &str) -> Result<()> {
        self.conn.execute("UPDATE plugin_submissions SET note = ?2 WHERE id = ?1", params![id, note])?;
        Ok(())
    }
}
