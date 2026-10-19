//! The Pyr Library: plugins anyone can install from Pyr, listed in
//! `pyr.gg/library/index2.json` (Pyr 0.8.0 and older read `index.json`, which leaves out skins
//! that need a newer Pyr).
//!
//! Every plugin in it was sent in as Rust source, reviewed, and built from that source by
//! Firelit Games, who signs the list with the same offline key as Pyr's releases (with its
//! own context, so a list can't pass for a release or the other way round). The app only
//! believes a list with that signature, within its dates and no older than one it has already
//! seen; and only installs files whose SHA-256 the list gives. So someone who breaks into the
//! website can't hand out anything else.
//!
//! Two kinds of plugin:
//! - **Programs**: Rust programs of their own (like Chimbrowser, a browser), installed into
//!   Pyr's plugins folder and opened from Pyr. They run like any program you install.
//! - **Add-ons**: Rust built to WebAssembly, run inside Pyr in a sandbox: no files, no
//!   network, no imports at all; just what Pyr passes in (see `pyr-plugin`, the kit).
//!
//! And **skins** ([`crate::skin`]): colors, a picture, fonts and sounds, no code at all. They're
//! listed apart (`skins`), so Pyr from before skins reads the list as it always has.
//!
//! **The Library Keeper** (Pyr 0.9, `index3.json`): new versions of what's already listed are
//! checked, built and reviewed by a service on pyr.gg, which signs the list with a key of its
//! own. That key counts only with a certificate ([`KeeperCert`]) signed by the release key,
//! only until the certificate's date, and only for lists: never for Pyr's releases. And a list
//! the Keeper signs can't bring in native code: each program in it needs an endorsement made
//! with the release key itself ([`endorse`]), so whoever got the Keeper's key could change
//! add-ons (sandboxed) and skins (no code), but not put a program on anyone's PC.

use serde::{Deserialize, Serialize};

use crate::b64;
use crate::crypto::{self, Identity};

/// Signed lists start with this (a release manifest's signature can't pass for a list's).
const CONTEXT: &[u8] = b"ingle plugin library v1\n";
/// A Keeper certificate starts with this.
const CERT_CONTEXT: &[u8] = b"pyr library keeper key v1\n";
/// A program's endorsement starts with this.
const ENDORSE_CONTEXT: &[u8] = b"pyr library program v1\n";

/// How long a signed list counts for, unless told otherwise.
pub const VALID_DAYS: i64 = 180;

/// What's allowed in an id: lowercase letters, digits and dashes, 2–40 long.
pub fn valid_id(id: &str) -> bool {
    (2..=40).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !id.starts_with('-')
        && !id.ends_with('-')
}

/// A file's place inside a plugin's folder: one to three parts of letters, digits, `.`, `_`
/// and `-`, none starting with a dot (so never `..`, an absolute path or a hidden file).
pub fn valid_path(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    (1..=3).contains(&parts.len())
        && path.len() <= 120
        && parts.iter().all(|p| {
            !p.is_empty()
                && !p.starts_with('.')
                && p.len() <= 64
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        })
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A program of its own, opened from Pyr.
    Program,
    /// WebAssembly run inside Pyr, in a sandbox.
    Addon,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Program => "Program",
            Kind::Addon => "Add-on",
        }
    }
}

/// What an add-on may do (shown before it's installed). Anything else it can't.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Permission {
    /// Adds /commands to chat; what it answers is sent as your message when you use one.
    Commands,
    /// Shows a panel of its own.
    Panel,
    /// Keeps a little data of its own on this PC.
    Storage,
}

impl Permission {
    pub fn describe(self) -> &'static str {
        match self {
            Permission::Commands => "Adds /commands to chat. When you use one, what it answers is sent as your message.",
            Permission::Panel => "Shows a panel of its own inside Pyr.",
            Permission::Storage => "Keeps a little data of its own on this PC.",
        }
    }
}

/// A file to download: where it goes in the plugin's folder, where it is (relative to the
/// list's own address), and what it must be.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct File {
    pub path: String,
    pub url: String,
    /// Lowercase hex SHA-256.
    pub sha256: String,
    pub size: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub version: String,
    /// One line.
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    /// Where its source code is (a web address), if it's published.
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub icon: Option<File>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    /// A program's files for each system (empty: not available there).
    #[serde(default)]
    pub windows: Vec<File>,
    #[serde(default)]
    pub linux: Vec<File>,
    /// An add-on's WebAssembly module.
    #[serde(default)]
    pub wasm: Option<File>,
    /// A program's file to start (one of its files' paths).
    #[serde(default)]
    pub run: String,
    /// What reviewing it found worth knowing (shown with it).
    #[serde(default)]
    pub review: String,
    /// When this version was listed (Unix seconds).
    #[serde(default)]
    pub updated: i64,
    /// A program's endorsement: the release key's signature over what it is ([`endorse`]).
    /// Needed in a list the Keeper signs.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub endorsement: String,
}

impl Entry {
    /// The files to install on this system (a program's for this OS, or an add-on's module).
    pub fn files_for(&self, os: &str) -> Vec<&File> {
        match self.kind {
            Kind::Addon => self.wasm.iter().collect(),
            Kind::Program => match os {
                "windows" => self.windows.iter().collect(),
                "linux" => self.linux.iter().collect(),
                _ => Vec::new(),
            },
        }
    }

    pub fn size_for(&self, os: &str) -> u64 {
        self.files_for(os).iter().map(|f| f.size).sum()
    }

    /// Everything a list entry must get right before the app will touch it.
    pub fn check(&self) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err(format!("bad plugin id {:?}", self.id));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 60 {
            return Err(format!("{}: bad name", self.id));
        }
        let files = self.windows.iter().chain(&self.linux).chain(&self.wasm).chain(&self.icon);
        for f in files {
            if !valid_path(&f.path) {
                return Err(format!("{}: bad file path {:?}", self.id, f.path));
            }
            if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err(format!("{}: bad SHA-256 for {}", self.id, f.path));
            }
            if f.url.is_empty() || f.url.contains("..") {
                return Err(format!("{}: bad address for {}", self.id, f.path));
            }
        }
        match self.kind {
            Kind::Addon => {
                if self.wasm.is_none() {
                    return Err(format!("{}: an add-on needs its module", self.id));
                }
            }
            Kind::Program => {
                for list in [&self.windows, &self.linux] {
                    if !list.is_empty() && !list.iter().any(|f| f.path == self.run) {
                        return Err(format!("{}: what to start ({:?}) isn't one of its files", self.id, self.run));
                    }
                    let mut paths: Vec<&str> = list.iter().map(|f| f.path.as_str()).collect();
                    paths.sort_unstable();
                    if paths.windows(2).any(|w| w[0] == w[1]) {
                        return Err(format!("{}: the same file twice", self.id));
                    }
                }
            }
        }
        Ok(())
    }
}

/// A skin in the Library: its folder's files (`skin.json` and what it uses), each by where it
/// goes in the skin's folder.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SkinEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    /// One line.
    pub summary: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub license: String,
    pub files: Vec<File>,
    /// A picture of Pyr wearing it (one of `files`, or apart).
    #[serde(default)]
    pub preview: Option<File>,
    /// What reviewing it found worth knowing (shown with it).
    #[serde(default)]
    pub review: String,
    /// When this version was listed (Unix seconds).
    #[serde(default)]
    pub updated: i64,
    /// The oldest Pyr that can wear it (empty: any). Older ones don't list it.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub needs: String,
}

impl SkinEntry {
    /// Whether this Pyr (`version`) can wear it.
    pub fn fits(&self, version: &str) -> bool {
        self.needs.is_empty() || !crate::release::newer(&self.needs, version)
    }

    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }

    /// Everything a list entry must get right before the app will touch it.
    pub fn check(&self) -> Result<(), String> {
        if !valid_id(&self.id) {
            return Err(format!("bad skin id {:?}", self.id));
        }
        if self.name.trim().is_empty() || self.name.chars().count() > 60 {
            return Err(format!("{}: bad name", self.id));
        }
        if self.files.is_empty() || self.files.len() > crate::skin::MAX_FILES {
            return Err(format!("{}: between 1 and {} files", self.id, crate::skin::MAX_FILES));
        }
        if !self.files.iter().any(|f| f.path == crate::skin::MANIFEST) {
            return Err(format!("{}: no skin.json", self.id));
        }
        let mut paths = std::collections::HashSet::new();
        let mut total = 0u64;
        for f in self.files.iter().chain(&self.preview) {
            if !valid_path(&f.path) {
                return Err(format!("{}: bad file path {:?}", self.id, f.path));
            }
            if f.sha256.len() != 64 || !f.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err(format!("{}: bad SHA-256 for {}", self.id, f.path));
            }
            if f.url.is_empty() || f.url.contains("..") {
                return Err(format!("{}: bad address for {}", self.id, f.path));
            }
            if f.size as usize > crate::skin::MAX_FILE {
                return Err(format!("{}: {} is too big", self.id, f.path));
            }
        }
        for f in &self.files {
            if !paths.insert(f.path.as_str()) {
                return Err(format!("{}: the same file twice", self.id));
            }
            total += f.size;
        }
        if total as usize > crate::skin::MAX_TOTAL {
            return Err(format!("{}: too big", self.id));
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct Library {
    /// When it was signed, and when it stops counting (Unix seconds).
    pub issued: i64,
    pub expires: i64,
    pub plugins: Vec<Entry>,
    /// Skins (Pyr 0.7.5 and later).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skins: Vec<SkinEntry>,
}

impl Library {
    /// Whether to believe it now, having already seen one signed at `newest_seen` (0: none).
    pub fn fresh(&self, now: i64, newest_seen: i64) -> Result<(), String> {
        if now > self.expires {
            return Err("the plugin list is past its date (it should have been signed again)".into());
        }
        if self.issued < newest_seen {
            return Err("the plugin list is older than one this app has already seen".into());
        }
        Ok(())
    }

    pub fn find(&self, id: &str) -> Option<&Entry> {
        self.plugins.iter().find(|p| p.id == id)
    }

    pub fn find_skin(&self, id: &str) -> Option<&SkinEntry> {
        self.skins.iter().find(|s| s.id == id)
    }
}

/// The file on the website: the list exactly as signed, and its signature (by the release key,
/// or by the Keeper's, with its certificate).
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Signed {
    pub index: String,
    pub sig: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert: Option<KeeperCert>,
}

/// The release key's word that `key` (a public key, as a user id) may sign Library lists until
/// `not_after` (Unix seconds).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct KeeperCert {
    pub key: String,
    pub not_after: i64,
    pub sig: String,
}

fn cert_message(key: &str, not_after: i64) -> Vec<u8> {
    [CERT_CONTEXT, format!("{key}\n{not_after}").as_bytes()].concat()
}

/// Certifies the Keeper's key (signed with the release key).
pub fn certify(release: &Identity, key: &str, not_after: i64) -> KeeperCert {
    KeeperCert { key: key.to_string(), not_after, sig: b64::encode(&release.sign(&cert_message(key, not_after))) }
}

fn cert_ok(cert: &KeeperCert, release_key: &str) -> bool {
    b64::decode(&cert.sig).is_ok_and(|sig| crypto::verify(release_key, &cert_message(&cert.key, cert.not_after), &sig))
}

/// What a program's endorsement covers: its id, version, what it starts, and every file (each
/// system's, by path and SHA-256).
fn endorse_message(e: &Entry) -> Vec<u8> {
    let mut m = ENDORSE_CONTEXT.to_vec();
    m.extend(format!("{}\n{}\n{}\n", e.id, e.version, e.run).as_bytes());
    for (os, files) in [("windows", &e.windows), ("linux", &e.linux)] {
        for f in files {
            m.extend(format!("{os} {} {} {}\n", f.path, f.sha256, f.size).as_bytes());
        }
    }
    m
}

/// Endorses a program for lists the Keeper signs (with the release key).
pub fn endorse(release: &Identity, e: &Entry) -> String {
    b64::encode(&release.sign(&endorse_message(e)))
}

pub fn endorsed(e: &Entry, release_key: &str) -> bool {
    b64::decode(&e.endorsement).is_ok_and(|sig| crypto::verify(release_key, &endorse_message(e), &sig))
}

fn message(index: &str) -> Vec<u8> {
    [CONTEXT, index.as_bytes()].concat()
}

pub fn sign(key: &Identity, library: &Library) -> Signed {
    let text = serde_json::to_string_pretty(library).expect("library serializes");
    let sig = key.sign(&message(&text));
    Signed { index: text, sig: b64::encode(&sig), cert: None }
}

/// Signed by the Keeper, with its certificate.
pub fn sign_as_keeper(key: &Identity, cert: &KeeperCert, library: &Library) -> Signed {
    Signed { cert: Some(cert.clone()), ..sign(key, library) }
}

/// Checks the signature against `key` (the release key's public half), parses, and checks
/// every entry (ids, file paths, hashes). Entries that don't pass are dropped, not trusted.
pub fn open(signed_json: &str, key: &str) -> Result<Library, String> {
    let signed: Signed = serde_json::from_str(signed_json).map_err(|e| format!("not a plugin list: {e}"))?;
    let sig = b64::decode(&signed.sig).map_err(|_| "bad signature encoding".to_string())?;
    // Signed by the release key, or by a Keeper it certified.
    let signer = match &signed.cert {
        None => key,
        Some(c) if cert_ok(c, key) => c.key.as_str(),
        Some(_) => return Err("the plugin list's signer isn't certified by Firelit Games".into()),
    };
    if !crypto::verify(signer, &message(&signed.index), &sig) {
        return Err("the plugin list isn't signed by Firelit Games".into());
    }
    let mut lib: Library = serde_json::from_str(&signed.index).map_err(|e| format!("bad plugin list: {e}"))?;
    if let Some(c) = &signed.cert {
        // Only within the certificate's dates; and programs only as the release key endorsed them.
        if lib.issued > c.not_after {
            return Err("the plugin list was signed after its signer's certificate ran out".into());
        }
        lib.expires = lib.expires.min(c.not_after);
        lib.plugins.retain(|p| p.kind != Kind::Program || endorsed(p, key));
    }
    lib.plugins.retain(|p| p.check().is_ok());
    let mut seen = std::collections::HashSet::new();
    lib.plugins.retain(|p| seen.insert(p.id.clone()));
    lib.skins.retain(|s| s.check().is_ok() && s.fits(env!("CARGO_PKG_VERSION")));
    let mut seen = std::collections::HashSet::new();
    lib.skins.retain(|s| seen.insert(s.id.clone()));
    Ok(lib)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> File {
        File { path: path.into(), url: format!("files/x/{path}"), sha256: "ab".repeat(32), size: 10 }
    }

    fn entry() -> Entry {
        Entry {
            id: "chimbrowser".into(),
            name: "Chimbrowser".into(),
            kind: Kind::Program,
            version: "0.1.3-1".into(),
            summary: "A browser".into(),
            description: String::new(),
            author: "Firelit Games".into(),
            license: "MIT OR Apache-2.0".into(),
            source: String::new(),
            icon: Some(file("icon.png")),
            permissions: vec![],
            windows: vec![file("chimbrowser.exe"), file("WebView2Loader.dll")],
            linux: vec![],
            wasm: None,
            run: "chimbrowser.exe".into(),
            review: String::new(),
            updated: 0,
            endorsement: String::new(),
        }
    }

    #[test]
    fn paths_and_ids() {
        for ok in ["a.exe", "WebView2Loader.dll", "assets/ui/x.html", "a-b_c.1"] {
            assert!(valid_path(ok), "{ok}");
        }
        for bad in ["", "../x", "a/../b", "/etc/passwd", "C:\\x", ".hidden", "a/.b", "a/b/c/d", "a//b", "a b", "é"] {
            assert!(!valid_path(bad), "{bad}");
        }
        assert!(valid_id("chimbrowser") && valid_id("dice-roller") && !valid_id("Dice") && !valid_id("-x") && !valid_id("x"));
    }

    #[test]
    fn signed_list_round_trips_and_bad_entries_are_dropped() {
        let key = Identity::generate();
        let mut bad = entry();
        bad.id = "evil".into();
        bad.windows = vec![file("../../evil.exe")];
        bad.run = "../../evil.exe".into();
        let mut twice = entry();
        twice.version = "9.9.9".into();
        let lib = Library { issued: 100, expires: 200, plugins: vec![entry(), bad, twice], ..Default::default() };
        let signed = serde_json::to_string(&sign(&key, &lib)).unwrap();
        let back = open(&signed, &key.user_id()).unwrap();
        assert_eq!(back.plugins.len(), 1, "the bad path and the second chimbrowser are dropped");
        assert_eq!(back.plugins[0], entry());
        // Another key, or a changed list, isn't believed.
        assert!(open(&signed, &Identity::generate().user_id()).is_err());
        let tampered = signed.replace("A browser", "A virus");
        assert!(open(&tampered, &key.user_id()).is_err());
        // Nor is a release manifest's signature (another context).
        let m = crate::release::sign(&key, &crate::release::Manifest {
            version: "1".into(),
            notes: String::new(),
            windows: None,
            linux_deb: None,
            released: vec![],
            issued: 1,
            expires: 2,
        });
        let as_list = serde_json::json!({ "index": m.manifest, "sig": m.sig }).to_string();
        assert!(open(&as_list, &key.user_id()).unwrap_err().contains("isn't signed"));
        // Dates.
        assert!(back.fresh(150, 100).is_ok());
        assert!(back.fresh(250, 0).is_err());
        assert!(back.fresh(150, 101).is_err());
    }

    #[test]
    fn the_keeper_signs_only_with_its_certificate_and_never_a_program_of_its_own() {
        let release = Identity::generate();
        let keeper = Identity::generate();
        let cert = certify(&release, &keeper.user_id(), 1000);
        let mut addon = entry();
        addon.id = "dice".into();
        addon.kind = Kind::Addon;
        addon.windows = vec![];
        addon.run = String::new();
        addon.wasm = Some(file("dice.wasm"));
        // A program the release key endorsed, and one it didn't.
        let mut endorsed_prog = entry();
        endorsed_prog.endorsement = endorse(&release, &endorsed_prog);
        let mut sneaky = entry();
        sneaky.id = "sneaky".into();
        sneaky.endorsement = endorse(&keeper, &sneaky);
        let lib = Library { issued: 100, expires: 5000, plugins: vec![addon.clone(), endorsed_prog.clone(), sneaky], ..Default::default() };
        let signed = serde_json::to_string(&sign_as_keeper(&keeper, &cert, &lib)).unwrap();
        let back = open(&signed, &release.user_id()).unwrap();
        let ids: Vec<&str> = back.plugins.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["dice", "chimbrowser"], "the program the Keeper endorsed itself is dropped");
        assert_eq!(back.expires, 1000, "no longer than the certificate");
        // An endorsed program changed (another file) isn't endorsed any more.
        let mut swapped = endorsed_prog.clone();
        swapped.windows[0].sha256 = "cd".repeat(32);
        assert!(!endorsed(&swapped, &release.user_id()));
        // A certificate from another key, or a changed one, counts for nothing.
        let fake = certify(&keeper, &keeper.user_id(), 1000);
        let s2 = serde_json::to_string(&sign_as_keeper(&keeper, &fake, &lib)).unwrap();
        assert!(open(&s2, &release.user_id()).unwrap_err().contains("certified"));
        let mut longer = cert.clone();
        longer.not_after = 9_999_999;
        let s3 = serde_json::to_string(&sign_as_keeper(&keeper, &longer, &lib)).unwrap();
        assert!(open(&s3, &release.user_id()).is_err());
        // Signed after the certificate ran out.
        let late = Library { issued: 1001, ..lib.clone() };
        let s4 = serde_json::to_string(&sign_as_keeper(&keeper, &cert, &late)).unwrap();
        assert!(open(&s4, &release.user_id()).unwrap_err().contains("ran out"));
        // And the Keeper's key alone (no certificate) isn't the release key.
        let s5 = serde_json::to_string(&sign(&keeper, &lib)).unwrap();
        assert!(open(&s5, &release.user_id()).is_err());
    }

    #[test]
    fn programs_must_start_one_of_their_files() {
        let mut e = entry();
        e.run = "other.exe".into();
        assert!(e.check().is_err());
        let mut a = entry();
        a.kind = Kind::Addon;
        a.windows.clear();
        assert!(a.check().is_err(), "an add-on needs its module");
        a.wasm = Some(file("dice.wasm"));
        assert!(a.check().is_ok());
        assert_eq!(a.files_for("windows").len(), 1);
        assert_eq!(entry().files_for("linux").len(), 0);
        assert_eq!(entry().size_for("windows"), 20);
    }

    fn skin() -> SkinEntry {
        SkinEntry {
            id: "blood-moon".into(),
            name: "Blood Moon".into(),
            version: "1.0".into(),
            summary: "Red and black.".into(),
            description: String::new(),
            author: "Blue".into(),
            license: String::new(),
            files: vec![file("skin.json"), file("background.webp"), file("fonts/Body.ttf")],
            preview: Some(file("preview.webp")),
            review: String::new(),
            updated: 0,
            needs: String::new(),
        }
    }

    #[test]
    fn skins_for_newer_pyrs_are_only_listed_for_them() {
        let mut s = skin();
        assert!(s.fits("0.8.0"));
        s.needs = "0.8.1".into();
        assert!(!s.fits("0.8.0"));
        assert!(s.fits("0.8.1"));
        assert!(s.fits("0.9.0"));
        // (Left out of the list when it's empty, so older lists read the same.)
        assert!(!serde_json::to_string(&skin()).unwrap().contains("needs"));
    }

    #[test]
    fn skins_are_listed_apart_and_older_apps_read_the_list_as_before() {
        let key = Identity::generate();
        let mut no_manifest = skin();
        no_manifest.id = "broken".into();
        no_manifest.files.remove(0);
        let mut escaping = skin();
        escaping.id = "escaping".into();
        escaping.files.push(file("../../x.dll"));
        let mut huge = skin();
        huge.id = "huge".into();
        huge.files[1].size = 50_000_000;
        let lib = Library { issued: 100, expires: 200, plugins: vec![entry()], skins: vec![skin(), no_manifest, escaping, huge, skin()] };
        let signed = serde_json::to_string(&sign(&key, &lib)).unwrap();
        let back = open(&signed, &key.user_id()).unwrap();
        assert_eq!(back.skins, vec![skin()], "the bad ones and the second blood-moon are dropped");
        assert_eq!(back.find_skin("blood-moon").unwrap().size(), 30);
        // Pyr from before skins: the list has no `skins` for it, and reads exactly as before.
        #[derive(Deserialize)]
        struct Before {
            plugins: Vec<Entry>,
        }
        let s: Signed = serde_json::from_str(&signed).unwrap();
        let before: Before = serde_json::from_str(&s.index).unwrap();
        assert_eq!(before.plugins, vec![entry()]);
        // A list with no skins says nothing about them.
        let plain = sign(&key, &Library { issued: 1, expires: 2, plugins: vec![entry()], skins: vec![] });
        assert!(!plain.index.contains("skins"));
    }
}
