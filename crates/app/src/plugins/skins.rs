//! Skins from the Pyr Library (see `pyr_proto::skin`): installing one (every file downloaded,
//! checked against the signed list's SHA-256 and then as a skin, and put into its own folder,
//! all of it or none), what's installed, and reading an installed skin back (checked again).
//! Wearing one is `crate::looks`.

use std::path::{Path, PathBuf};

use pyr_proto::library::SkinEntry;
use pyr_proto::release::sha256_hex;
use serde::{Deserialize, Serialize};

use super::install::{InstalledFile, download, now};

const INSTALLED_FILE: &str = "installed.json";

/// Where skins live: next to the plugins folder.
pub fn skins_dir(config_dir: &Path) -> PathBuf {
    super::install::plugins_dir(config_dir).with_file_name("skins")
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InstalledSkin {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    pub files: Vec<InstalledFile>,
    pub installed_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct InstalledSkins {
    pub skins: Vec<InstalledSkin>,
}

impl InstalledSkins {
    pub fn load(dir: &Path) -> InstalledSkins {
        std::fs::read(dir.join(INSTALLED_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("{INSTALLED_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join(INSTALLED_FILE)).map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Option<&InstalledSkin> {
        self.skins.iter().find(|s| s.id == id)
    }

    pub fn put(&mut self, s: InstalledSkin) {
        self.skins.retain(|q| q.id != s.id);
        self.skins.push(s);
        self.skins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }
}

/// Installs (or updates) `entry` into `dir/<id>`: downloads every file, checks each against the
/// list and the whole as a skin, then swaps the folder in. `progress(done, total)` in bytes.
pub fn install(dir: &Path, index_url: &str, entry: &SkinEntry, mut progress: impl FnMut(u64, u64)) -> Result<InstalledSkin, String> {
    entry.check()?;
    let total = entry.size();
    let mut done = 0u64;
    let mut files = Vec::with_capacity(entry.files.len());
    for f in &entry.files {
        let url = super::library::file_url(index_url, &f.url);
        let bytes = download(&url, f.size, &f.sha256, |n| {
            done += n;
            progress(done, total);
        })?;
        files.push((f.path.clone(), bytes));
    }
    let report = pyr_engine::skins::check(&files);
    if !report.ok() {
        return Err(format!("it doesn't pass Pyr's checks for skins ({}), so it wasn't installed", report.problems.join(" ")));
    }
    let staging = dir.join(format!("{}.new", entry.id));
    let _ = std::fs::remove_dir_all(&staging);
    let fill = || -> Result<(), String> {
        for (path, bytes) in &files {
            let to = staging.join(path);
            if let Some(parent) = to.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&to, bytes).map_err(|e| format!("couldn't save {path}: {e}"))?;
        }
        Ok(())
    };
    if let Err(e) = fill() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    let home = dir.join(&entry.id);
    let old = dir.join(format!("{}.old", entry.id));
    let _ = std::fs::remove_dir_all(&old);
    if home.exists() {
        if let Err(e) = std::fs::rename(&home, &old) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!("couldn't replace the installed version: {e}"));
        }
    }
    if let Err(e) = std::fs::rename(&staging, &home) {
        let _ = std::fs::rename(&old, &home);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("couldn't install it: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(InstalledSkin {
        id: entry.id.clone(),
        name: entry.name.clone(),
        version: entry.version.clone(),
        author: entry.author.clone(),
        files: entry.files.iter().map(|f| InstalledFile { path: f.path.clone(), sha256: f.sha256.clone() }).collect(),
        installed_at: now(),
    })
}

/// An installed skin's files, each still the one checked when it was installed.
pub fn files(dir: &Path, s: &InstalledSkin) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = Vec::with_capacity(s.files.len());
    for f in &s.files {
        if !pyr_proto::library::valid_path(&f.path) {
            return Err(format!("{} has a file in an odd place", s.name));
        }
        let bytes = std::fs::read(dir.join(&s.id).join(&f.path)).map_err(|e| format!("{}'s {} can't be read ({e}): reinstall it", s.name, f.path))?;
        if sha256_hex(&bytes) != f.sha256 {
            return Err(format!("{}'s files changed since it was installed: reinstall it", s.name));
        }
        out.push((f.path.clone(), bytes));
    }
    Ok(out)
}

/// Removes an installed skin's folder.
pub fn uninstall(dir: &Path, id: &str) -> Result<(), String> {
    let home = dir.join(id);
    if home.exists() {
        std::fs::remove_dir_all(&home).map_err(|e| format!("couldn't remove it: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_proto::library::File;

    #[test]
    fn an_installed_skin_reads_back_only_as_it_was_installed() {
        let dir = super::super::library::temp_dir("skins");
        let json = br##"{ "name": "Ash", "version": "1.0", "colors": { "ember": "#c0c0c0" } }"##.to_vec();
        let s = InstalledSkin {
            id: "ash".into(),
            name: "Ash".into(),
            version: "1.0".into(),
            author: "Mara".into(),
            files: vec![InstalledFile { path: "skin.json".into(), sha256: sha256_hex(&json) }],
            installed_at: 0,
        };
        std::fs::create_dir_all(dir.join("ash")).unwrap();
        std::fs::write(dir.join("ash/skin.json"), &json).unwrap();
        assert_eq!(files(&dir, &s).unwrap(), vec![("skin.json".to_string(), json.clone())]);
        std::fs::write(dir.join("ash/skin.json"), b"{}").unwrap();
        assert!(files(&dir, &s).unwrap_err().contains("changed"));
        let mut odd = s.clone();
        odd.files[0].path = "../skin.json".into();
        assert!(files(&dir, &odd).is_err());
        // A list that doesn't add up is refused before anything's downloaded.
        let entry = SkinEntry {
            id: "Bad Id".into(),
            name: "x".into(),
            version: "1".into(),
            summary: String::new(),
            description: String::new(),
            author: String::new(),
            license: String::new(),
            files: vec![File { path: "skin.json".into(), url: "files/x/skin.json".into(), sha256: "ab".repeat(32), size: 2 }],
            preview: None,
            review: String::new(),
            updated: 0,
        };
        assert!(install(&dir, "http://127.0.0.1:9/library/index.json", &entry, |_, _| {}).is_err());
        let mut installed = InstalledSkins::default();
        installed.put(s.clone());
        installed.save(&dir).unwrap();
        assert_eq!(InstalledSkins::load(&dir).get("ash"), Some(&s));
        uninstall(&dir, "ash").unwrap();
        assert!(!dir.join("ash").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
