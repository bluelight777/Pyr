//! Going through what's waiting, one submission at a time, and publishing what passes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use pyr_proto::crypto::Identity;
use pyr_proto::library::{self, Entry, File, KeeperCert, Kind, Library, Permission, SkinEntry};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::build::{self, Builder};
use crate::diff;
use crate::review::{self, Reviewer, Verdict};
use crate::store::{Store, Sub};

/// How the Keeper is set up (a JSON file).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// The server's database (where submissions are).
    pub guild_db: PathBuf,
    /// The website's library folder: `index3.json` and `files/` go here.
    pub web_library: PathBuf,
    /// The Keeper's own folder: its state and log.
    pub state_dir: PathBuf,
    /// Its key (the seed, base64url) and the release key's certificate for it.
    pub key_file: PathBuf,
    pub cert_file: PathBuf,
    /// The release key's public half (as a user id).
    pub release_key: String,
    /// The Claude API key, and the model (empty: the newest Opus).
    pub api_key_file: PathBuf,
    #[serde(default)]
    pub model: String,
    /// The sandbox's toolchain, the plugin kit, its vendored crates, and where to build.
    pub rustup_home: PathBuf,
    pub cargo_home: PathBuf,
    pub kit: PathBuf,
    pub vendor: PathBuf,
    pub work: PathBuf,
    #[serde(default = "default_bwrap")]
    pub bwrap: PathBuf,
    #[serde(default = "default_poll")]
    pub poll_secs: u64,
    /// How long a list it signs counts for (it signs again well before).
    #[serde(default = "default_valid")]
    pub valid_days: i64,
}

fn default_bwrap() -> PathBuf {
    PathBuf::from("/usr/bin/bwrap")
}
fn default_poll() -> u64 {
    60
}
fn default_valid() -> i64 {
    30
}

/// What it keeps between runs.
#[derive(Serialize, Deserialize, Default, Debug)]
pub struct State {
    /// The list as published.
    pub library: Library,
    /// Which listed submission each Library entry came from (by its id there: plugin ids, and
    /// skins as "skin:<id>").
    pub links: BTreeMap<i64, String>,
    /// Submissions left for a person (not looked at again).
    pub held: BTreeMap<i64, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Approve(String),
    Reject(String),
    Hold(String),
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub struct Keeper<'a> {
    pub cfg: Config,
    pub store: Store,
    pub reviewer: &'a dyn Reviewer,
    pub builder: &'a dyn Builder,
    pub key: Identity,
    pub cert: KeeperCert,
    pub state: State,
}

/// What it said about a submission, for the log.
#[derive(Serialize)]
struct LogLine<'a> {
    at: i64,
    submission: i64,
    name: &'a str,
    version: &'a str,
    decision: &'a str,
    why: &'a str,
}

impl<'a> Keeper<'a> {
    pub fn load_key(path: &Path) -> Result<Identity> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Identity::from_seed(pyr_proto::b64::decode_array::<32>(text.trim()).context("not a key")?))
    }

    pub fn new(cfg: Config, reviewer: &'a dyn Reviewer, builder: &'a dyn Builder) -> Result<Keeper<'a>> {
        let key = Self::load_key(&cfg.key_file)?;
        let cert: KeeperCert = serde_json::from_str(&std::fs::read_to_string(&cfg.cert_file)?)?;
        anyhow::ensure!(cert.key == key.user_id(), "the certificate is for another key");
        let store = Store::open(&cfg.guild_db)?;
        std::fs::create_dir_all(&cfg.state_dir)?;
        let path = cfg.state_dir.join("state.json");
        let state = if path.exists() {
            serde_json::from_str(&std::fs::read_to_string(&path)?)?
        } else {
            // First run: the list that's live (as the release key signed it).
            let mut st = State::default();
            for name in ["index3.json", "index2.json"] {
                let p = cfg.web_library.join(name);
                if let Ok(text) = std::fs::read_to_string(&p)
                    && let Ok(lib) = library::open(&text, &cfg.release_key)
                {
                    info!("starting from {} ({} plugins, {} skins)", p.display(), lib.plugins.len(), lib.skins.len());
                    st.library = lib;
                    break;
                }
            }
            st
        };
        Ok(Keeper { cfg, store, reviewer, builder, key, cert, state })
    }

    fn save(&self) -> Result<()> {
        let path = self.cfg.state_dir.join("state.json");
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(&self.state)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }

    fn log(&self, sub: &Sub, decision: &str, why: &str) {
        let line = serde_json::to_string(&LogLine { at: now(), submission: sub.id, name: &sub.name, version: &sub.version, decision, why }).unwrap_or_default();
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(self.cfg.state_dir.join("log.jsonl")) {
            let _ = writeln!(f, "{line}");
        }
    }

    /// One pass: everything waiting. How many it decided.
    pub fn pass(&mut self) -> Result<usize> {
        let mut done = 0;
        for id in self.store.pending()? {
            if self.state.held.contains_key(&id) {
                continue;
            }
            let Some(sub) = self.store.get(id)? else { continue };
            match self.decide(&sub) {
                Ok(Decision::Approve(note)) => {
                    self.store.set_status(id, "listed", &note)?;
                    self.log(&sub, "listed", &note);
                    info!(id, name = %sub.name, version = %sub.version, "listed");
                }
                Ok(Decision::Reject(why)) => {
                    self.store.set_status(id, "rejected", &why)?;
                    self.log(&sub, "rejected", &why);
                    info!(id, name = %sub.name, "rejected: {why}");
                }
                Ok(Decision::Hold(why)) => {
                    self.store.set_note(id, &format!("Waiting for Firelit Games: {why}"))?;
                    self.state.held.insert(id, why.clone());
                    self.log(&sub, "held", &why);
                    info!(id, name = %sub.name, "held: {why}");
                }
                Err(e) => {
                    // (The API down, a disk problem: tried again next time.)
                    warn!(id, "couldn't decide yet: {e:#}");
                    continue;
                }
            }
            done += 1;
            self.save()?;
        }
        // Signed again well before it runs out.
        if self.state.library.expires > 0 && self.state.library.expires - now() < 7 * 86_400 && !self.state.library.plugins.is_empty() {
            self.publish()?;
        }
        Ok(done)
    }

    /// The Library entry a listed submission is, by link or (the first time) by name and kind.
    fn entry_for(&mut self, listed: &Sub) -> Option<String> {
        if let Some(id) = self.state.links.get(&listed.id) {
            return Some(id.clone());
        }
        let name = listed.name.trim().to_lowercase();
        let found: Vec<String> = if listed.kind == pyr_proto::control::SKIN_KIND {
            self.state.library.skins.iter().filter(|s| s.name.trim().to_lowercase() == name).map(|s| format!("skin:{}", s.id)).collect()
        } else {
            let kind = if listed.kind == "program" { Kind::Program } else { Kind::Addon };
            self.state.library.plugins.iter().filter(|p| p.kind == kind && p.name.trim().to_lowercase() == name).map(|p| p.id.clone()).collect()
        };
        (found.len() == 1).then(|| {
            self.state.links.insert(listed.id, found[0].clone());
            found[0].clone()
        })
    }

    /// What to do with one submission (and, for an approval, it's published by the time this
    /// returns).
    pub fn decide(&mut self, sub: &Sub) -> Result<Decision> {
        let Some(listed) = self.store.listed_ancestor(sub.id)? else {
            return Ok(Decision::Hold("it isn't a new version of something already in the Library (a first version is always looked at by a person)".into()));
        };
        let Some(link) = self.entry_for(&listed) else {
            return Ok(Decision::Hold(format!("#{} isn't matched to one Library listing", listed.id)));
        };
        if sub.kind != listed.kind {
            return Ok(Decision::Reject(format!("it's a {}, but #{} is a {}", sub.kind, listed.id, listed.kind)));
        }
        let Some(files) = sub.decoded() else { return Ok(Decision::Reject("its files don't decode".into())) };
        let Some(old_files) = listed.decoded() else { return Ok(Decision::Hold("the listed version's files don't decode".into())) };
        if sub.kind == "program" {
            return Ok(Decision::Hold("programs run outside Pyr's sandbox, so a person looks at every version".into()));
        }
        let changes = diff::between(&old_files, &files);
        if changes.trim().is_empty() {
            return Ok(Decision::Reject("it's the same as the version that's listed".into()));
        }
        if changes.len() > 200_000 {
            return Ok(Decision::Hold("the change is too big for an automatic review".into()));
        }
        if let Some(skin_id) = link.strip_prefix("skin:") {
            self.skin(sub, &listed, skin_id.to_string(), &files, &old_files, changes)
        } else {
            self.addon(sub, link, &files, changes)
        }
    }

    fn verdict(&self, req: &review::Request) -> Result<Verdict> {
        self.reviewer.review(req)
    }

    fn skin(&mut self, sub: &Sub, listed: &Sub, id: String, files: &[(String, Vec<u8>)], old: &[(String, Vec<u8>)], changes: String) -> Result<Decision> {
        let report = pyr_proto::skin::check(files);
        if !report.ok() {
            return Ok(Decision::Reject(format!("the skin isn't right: {}", report.problems.join(" "))));
        }
        let Some(current) = self.state.library.find_skin(&id).cloned() else { return Ok(Decision::Hold("its listing is gone".into())) };
        let m = &report.manifest;
        if m.name.trim() != current.name.trim() || m.author.trim() != current.author.trim() {
            return Ok(Decision::Hold("its name or author changed".into()));
        }
        if !pyr_proto::release::newer(m.version.trim(), &current.version) {
            return Ok(Decision::Reject(format!("its version ({}) isn't newer than the listed one ({})", m.version.trim(), current.version)));
        }
        // The pictures that are new or changed, for Claude to look at.
        let before: BTreeMap<&str, &Vec<u8>> = old.iter().map(|(p, b)| (p.as_str(), b)).collect();
        let mut images = Vec::new();
        for (path, bytes) in files {
            let lower = path.to_lowercase();
            let media = if lower.ends_with(".png") {
                "image/png"
            } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
                "image/jpeg"
            } else if lower.ends_with(".webp") {
                "image/webp"
            } else if lower.ends_with(".gif") {
                "image/gif"
            } else {
                continue;
            };
            if before.get(path.as_str()) == Some(&bytes) {
                continue;
            }
            if bytes.len() > 4_500_000 {
                return Ok(Decision::Hold(format!("{path} is too big for an automatic look")));
            }
            images.push((path.clone(), media.to_string(), bytes.clone()));
        }
        if images.len() > 12 {
            return Ok(Decision::Hold("too many new pictures for an automatic look".into()));
        }
        let req = review::Request {
            kind: "skin".into(),
            name: current.name.clone(),
            old_version: current.version.clone(),
            new_version: m.version.trim().to_string(),
            author: current.author.clone(),
            permissions: vec![],
            diff: changes,
            changed_files: vec![],
            images,
        };
        let v = self.verdict(&req)?;
        let decision = self.decision(&v);
        if let Decision::Approve(_) = decision {
            let dir = format!("{id}/{}", m.version.trim());
            let mut list = Vec::new();
            for (rel, bytes) in files {
                list.push(self.put_file(&dir, rel, bytes)?);
            }
            let preview = list.iter().find(|f| pyr_proto::skin::is_preview(&f.path)).cloned();
            let entry = SkinEntry {
                version: m.version.trim().to_string(),
                files: list,
                preview,
                review: whats_new(&v),
                updated: now(),
                needs: if m.look.is_set() { pyr_proto::skin::Look::SINCE.to_string() } else { String::new() },
                ..current
            };
            entry.check().map_err(anyhow::Error::msg)?;
            if let Some(s) = self.state.library.skins.iter_mut().find(|s| s.id == id) {
                *s = entry;
            }
            self.publish()?;
            self.state.links.insert(sub.id, format!("skin:{id}"));
            let _ = listed;
        }
        Ok(decision)
    }

    fn addon(&mut self, sub: &Sub, id: String, files: &[(String, Vec<u8>)], changes: String) -> Result<Decision> {
        let Some(current) = self.state.library.find(&id).cloned() else { return Ok(Decision::Hold("its listing is gone".into())) };
        if current.kind != Kind::Addon {
            return Ok(Decision::Hold("programs are looked at by a person".into()));
        }
        let m = match build::manifest(files) {
            Ok(m) => m,
            Err(why) => return Ok(Decision::Hold(why)),
        };
        let version = if m.version.is_empty() { sub.version.clone() } else { m.version.clone() };
        if !pyr_proto::release::newer(&version, &current.version) {
            return Ok(Decision::Reject(format!("its version ({version}) isn't newer than the listed one ({})", current.version)));
        }
        // No permission it didn't already have.
        let mut perms: Vec<Permission> = Vec::new();
        for p in &m.permissions {
            match serde_json::from_value::<Permission>(serde_json::Value::String(p.clone())) {
                Ok(p) => perms.push(p),
                Err(_) => return Ok(Decision::Hold(format!("it asks for {p:?}, which isn't a permission Pyr knows"))),
            }
        }
        if let Some(extra) = perms.iter().find(|p| !current.permissions.contains(p)) {
            return Ok(Decision::Hold(format!("it asks for a new permission ({}): a person decides that", extra.describe())));
        }
        let wasm = match self.builder.build_addon(files) {
            Ok(w) => w,
            Err(e) => return Ok(Decision::Reject(format!("it didn't build in the Library's sandbox: {e:#}"))),
        };
        let changed: Vec<(String, String)> = files
            .iter()
            .filter(|(p, b)| p.ends_with(".rs") && changes.contains(&format!("+++ b/{p}\n")) && b.len() < 60_000)
            .filter_map(|(p, b)| String::from_utf8(b.clone()).ok().map(|s| (p.clone(), s)))
            .collect();
        let req = review::Request {
            kind: "addon".into(),
            name: current.name.clone(),
            old_version: current.version.clone(),
            new_version: version.clone(),
            author: current.author.clone(),
            permissions: perms.iter().map(|p| p.describe().to_string()).collect(),
            diff: changes,
            changed_files: changed,
            images: vec![],
        };
        let v = self.verdict(&req)?;
        let decision = self.decision(&v);
        if let Decision::Approve(_) = decision {
            let file = self.put_file(&format!("{id}/{version}"), &format!("{}.wasm", m.package.replace('-', "_")), &wasm)?;
            let entry = Entry { version: version.clone(), wasm: Some(file), permissions: perms, review: whats_new(&v), updated: now(), ..current };
            entry.check().map_err(anyhow::Error::msg)?;
            if let Some(e) = self.state.library.plugins.iter_mut().find(|e| e.id == id) {
                *e = entry;
            }
            self.publish()?;
            self.state.links.insert(sub.id, id);
        }
        Ok(decision)
    }

    fn decision(&self, v: &Verdict) -> Decision {
        let why = if v.note_to_author.trim().is_empty() { v.reasons.join(" ") } else { v.note_to_author.trim().to_string() };
        let why: String = why.chars().filter(|c| !c.is_control()).take(600).collect();
        match v.decision.as_str() {
            "approve" => Decision::Approve(format!("Listed by the Pyr Library Keeper. {why}")),
            "reject" => Decision::Reject(why),
            _ => Decision::Hold(why),
        }
    }

    /// A file into the website's library (`files/<dir>/<rel>`), and how the list names it.
    fn put_file(&self, dir: &str, rel: &str, bytes: &[u8]) -> Result<File> {
        let rel_path = build::safe_rel(rel).context("not a plain path")?;
        build::safe_rel(dir).context("not a plain folder")?;
        let to = self.cfg.web_library.join("files").join(dir).join(rel_path);
        std::fs::create_dir_all(to.parent().context("parent")?)?;
        let tmp = to.with_extension("keeper-tmp");
        std::fs::write(&tmp, bytes)?;
        std::fs::rename(&tmp, &to)?;
        Ok(File { path: rel.to_string(), url: format!("files/{dir}/{rel}"), sha256: pyr_proto::release::sha256_hex(bytes), size: bytes.len() as u64 })
    }

    /// Takes a list Firelit Games signed (with the release key itself: a new listing, a
    /// program's new version endorsed) as the one to keep, and puts it up signed by the Keeper.
    pub fn import(&mut self, signed_text: &str) -> Result<()> {
        let signed: library::Signed = serde_json::from_str(signed_text)?;
        anyhow::ensure!(signed.cert.is_none(), "only a list signed with the release key itself can be taken in");
        let lib = library::open(signed_text, &self.cfg.release_key).map_err(anyhow::Error::msg)?;
        // (Made from the one that's up, so the Keeper's own listings since stay: the library
        // tool's `endorse` re-signs the live list, and `sign --base` adds to it.)
        self.state.library = lib;
        self.publish()
    }

    /// Signs the list and puts it up (`index3.json`, replaced in one step).
    pub fn publish(&mut self) -> Result<()> {
        let lib = &mut self.state.library;
        lib.issued = now();
        lib.expires = (now() + self.cfg.valid_days * 86_400).min(self.cert.not_after);
        let signed = library::sign_as_keeper(&self.key, &self.cert, lib);
        let text = serde_json::to_string_pretty(&signed)?;
        // It must open as Pyr opens it.
        let check = library::open(&text, &self.cfg.release_key).map_err(anyhow::Error::msg)?;
        anyhow::ensure!(check.plugins.len() == lib.plugins.len(), "a plugin in the list wouldn't be believed (a program without an endorsement?)");
        let path = self.cfg.web_library.join("index3.json");
        let tmp = path.with_extension("keeper-tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path)?;
        self.save()
    }
}

fn whats_new(v: &Verdict) -> String {
    let w: String = v.whats_new.chars().filter(|c| !c.is_control()).take(300).collect();
    format!("New in this version: {w} (Checked, built and reviewed by the Pyr Library Keeper.)")
}
