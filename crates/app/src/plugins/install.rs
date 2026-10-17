//! Installing plugins from the library: every file downloaded, checked against the SHA-256 the
//! signed list gives, and put into the plugin's own folder (all of it, or none); programs
//! started from there, after checking their files again.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use pyr_proto::library::{Entry, Kind, Permission};
use pyr_proto::release::sha256_hex;
use serde::{Deserialize, Serialize};

const INSTALLED_FILE: &str = "installed.json";

/// This system's name as the library knows it.
pub fn os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "other"
    }
}

/// Where plugins live. Programs can be big, so on Windows they go in the local (not roaming)
/// app data, beside Pyr's settings rather than in them; `PYR_HOME` and test profiles keep
/// them with their own settings.
pub fn plugins_dir(config_dir: &Path) -> PathBuf {
    if pyr_proto::env_set("HOME") || config_dir != pyr_engine::config::config_dir(None) {
        return config_dir.join("plugins");
    }
    directories::ProjectDirs::from("", "", pyr_proto::APP_NAME)
        .map(|d| d.data_local_dir().join("plugins"))
        .unwrap_or_else(|| config_dir.join("plugins"))
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InstalledFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct InstalledPlugin {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub version: String,
    /// A program's file to start; an add-on's module.
    pub run: String,
    pub files: Vec<InstalledFile>,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    pub installed_at: i64,
    /// An add-on that's switched on (its commands work).
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Installed {
    pub plugins: Vec<InstalledPlugin>,
}

impl Installed {
    pub fn load(dir: &Path) -> Installed {
        std::fs::read(dir.join(INSTALLED_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let tmp = dir.join(format!("{INSTALLED_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).unwrap_or_default()).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, dir.join(INSTALLED_FILE)).map_err(|e| e.to_string())
    }

    pub fn get(&self, id: &str) -> Option<&InstalledPlugin> {
        self.plugins.iter().find(|p| p.id == id)
    }

    pub fn put(&mut self, p: InstalledPlugin) {
        self.plugins.retain(|q| q.id != p.id);
        self.plugins.push(p);
        self.plugins.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    }
}

/// An add-on's own data (what it keeps between calls).
pub fn data_path(dir: &Path, id: &str) -> PathBuf {
    dir.join("data").join(format!("{id}.json"))
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Downloads one file, checking its size and SHA-256 (allowing 50 kB/s, and a minute).
pub(crate) fn download(url: &str, size: u64, sha256: &str, mut progress: impl FnMut(u64)) -> Result<Vec<u8>, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(None)
        .timeout_connect(Some(Duration::from_secs(15)))
        .timeout_recv_body(Some(Duration::from_secs(60 + size / 50_000)))
        .build()
        .into();
    let mut resp = agent.get(url).call().map_err(|e| format!("couldn't download it: {e}"))?;
    let mismatch = || "a file doesn't match the library's list (damaged or tampered with), so nothing was installed".to_string();
    // (ureq's own limit, a little over, so a bigger file is seen as the wrong file rather than
    // as a download that broke off.)
    let mut reader = resp.body_mut().with_config().limit(size + 65_536).reader();
    let mut bytes = Vec::with_capacity(size as usize);
    let mut buf = vec![0u8; 128 * 1024];
    loop {
        let n = reader.read(&mut buf).map_err(|e| format!("the download broke off: {e}"))?;
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&buf[..n]);
        if bytes.len() as u64 > size {
            return Err(mismatch());
        }
        progress(n as u64);
    }
    if bytes.len() as u64 != size || sha256_hex(&bytes) != sha256 {
        return Err(mismatch());
    }
    Ok(bytes)
}

/// Installs (or updates) `entry` into `dir/<id>`: downloads every file for this system, checks
/// each, then swaps the whole folder in. `progress(done, total)` in bytes.
pub fn install(dir: &Path, index_url: &str, entry: &Entry, mut progress: impl FnMut(u64, u64)) -> Result<InstalledPlugin, String> {
    entry.check()?;
    let files = entry.files_for(os());
    if files.is_empty() {
        return Err(format!("{} isn't available for this system yet.", entry.name));
    }
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut done = 0u64;
    let staging = dir.join(format!("{}.new", entry.id));
    let _ = std::fs::remove_dir_all(&staging);
    let mut fill = || -> Result<(), String> {
        std::fs::create_dir_all(&staging).map_err(|e| format!("couldn't make its folder: {e}"))?;
        for f in &files {
            let url = super::library::file_url(index_url, &f.url);
            let bytes = download(&url, f.size, &f.sha256, |n| {
                done += n;
                progress(done, total);
            })?;
            let path = staging.join(&f.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            std::fs::write(&path, bytes).map_err(|e| format!("couldn't save {}: {e}", f.path))?;
            #[cfg(unix)]
            if entry.kind == Kind::Program && f.path == entry.run {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
            }
        }
        Ok(())
    };
    if let Err(e) = fill() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(e);
    }
    // Swap it in: the old version aside, the new one in, the old one gone.
    let home = dir.join(&entry.id);
    let old = dir.join(format!("{}.old", entry.id));
    let _ = std::fs::remove_dir_all(&old);
    if home.exists() {
        if let Err(e) = std::fs::rename(&home, &old) {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(format!("couldn't replace the installed version (is {} still open? Close it and try again): {e}", entry.name));
        }
    }
    if let Err(e) = std::fs::rename(&staging, &home) {
        let _ = std::fs::rename(&old, &home);
        let _ = std::fs::remove_dir_all(&staging);
        return Err(format!("couldn't install it: {e}"));
    }
    let _ = std::fs::remove_dir_all(&old);
    let run = match entry.kind {
        Kind::Program => entry.run.clone(),
        Kind::Addon => entry.wasm.as_ref().map(|w| w.path.clone()).unwrap_or_default(),
    };
    Ok(InstalledPlugin {
        id: entry.id.clone(),
        name: entry.name.clone(),
        kind: entry.kind,
        version: entry.version.clone(),
        run,
        files: files.iter().map(|f| InstalledFile { path: f.path.clone(), sha256: f.sha256.clone() }).collect(),
        permissions: entry.permissions.clone(),
        installed_at: now(),
        enabled: true,
    })
}

/// The plugin's files are still the ones that were checked when it was installed.
pub fn verify(dir: &Path, p: &InstalledPlugin) -> Result<(), String> {
    for f in &p.files {
        if !pyr_proto::library::valid_path(&f.path) {
            return Err(format!("{} has a file in an odd place", p.name));
        }
        let bytes = std::fs::read(dir.join(&p.id).join(&f.path)).map_err(|e| unreadable(p, &f.path, &e))?;
        if sha256_hex(&bytes) != f.sha256 {
            return Err(format!("{}'s files changed since it was installed, so it wasn't opened: reinstall it", p.name));
        }
    }
    Ok(())
}

/// Why a plugin's file can't be read, said so it can be put right. On Windows that's nearly
/// always antivirus: Windows Security (or another) took the file away, or is blocking it.
fn unreadable(p: &InstalledPlugin, file: &str, e: &std::io::Error) -> String {
    let gone = e.kind() == std::io::ErrorKind::NotFound;
    if cfg!(windows) {
        let what = if gone { "is gone" } else { "can't be opened" };
        format!(
            "{name}'s {file} {what}. Antivirus may have removed or blocked it: in Windows Security, open Virus & threat protection → Protection history, find {file}, and choose Allow or Restore. Then reinstall {name} if it's still missing.",
            name = p.name
        )
    } else if gone {
        format!("{} is missing a file ({file}): reinstall it", p.name)
    } else {
        format!("{}'s {file} can't be read ({e}): reinstall it", p.name)
    }
}

/// Starts a program (after checking its files). On Windows it's told it opens inside Pyr
/// (`PYR_EMBED`), so it can start hidden for Pyr to show once it's in place.
pub fn spawn(dir: &Path, p: &InstalledPlugin) -> Result<std::process::Child, String> {
    if p.kind != Kind::Program {
        return Err("only programs are opened this way".into());
    }
    verify(dir, p)?;
    let home = dir.join(&p.id);
    let mut cmd = std::process::Command::new(home.join(&p.run));
    cmd.current_dir(&home);
    if cfg!(windows) {
        cmd.env("PYR_EMBED", "1");
    }
    cmd.spawn().map_err(|e| match e.raw_os_error() {
        // ERROR_VIRUS_INFECTED, ERROR_VIRUS_DELETED: antivirus said no.
        Some(225) | Some(226) => unreadable(p, &p.run, &e),
        _ => format!("couldn't open {}: {e}", p.name),
    })
}

/// Starts a program on its own (its files checked first).
#[cfg(test)]
pub fn launch(dir: &Path, p: &InstalledPlugin) -> Result<(), String> {
    spawn(dir, p).map(|_| ())
}

/// Removes a plugin's files and data.
pub fn uninstall(dir: &Path, id: &str) -> Result<(), String> {
    if !pyr_proto::library::valid_id(id) {
        return Err("not a plugin".into());
    }
    let home = dir.join(id);
    if home.exists() {
        std::fs::remove_dir_all(&home).map_err(|e| format!("couldn't remove it (is it still open? Close it and try again): {e}"))?;
    }
    let _ = std::fs::remove_file(data_path(dir, id));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_proto::library::File;

    /// Serves `files` (path → bytes) over HTTP on localhost, for as many requests as come.
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut s) = stream else { continue };
                let mut req = [0u8; 2048];
                let n = std::io::Read::read(&mut s, &mut req).unwrap_or(0);
                let line = String::from_utf8_lossy(&req[..n]).lines().next().unwrap_or("").to_string();
                let path = line.split(' ').nth(1).unwrap_or("/").to_string();
                let body = files.iter().find(|(p, _)| *p == path).map(|(_, b)| b.clone());
                let reply = match body {
                    Some(b) => [format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", b.len()).into_bytes(), b].concat(),
                    None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
                };
                let _ = std::io::Write::write_all(&mut s, &reply);
            }
        });
        format!("http://127.0.0.1:{port}/library/index.json")
    }

    fn file(path: &str, bytes: &[u8]) -> File {
        File { path: path.into(), url: format!("files/tool/{path}"), sha256: sha256_hex(bytes), size: bytes.len() as u64 }
    }

    fn program(files: Vec<File>) -> Entry {
        Entry {
            id: "tool".into(),
            name: "Tool".into(),
            kind: Kind::Program,
            version: "1.0".into(),
            summary: "A tool".into(),
            description: String::new(),
            author: String::new(),
            license: String::new(),
            source: String::new(),
            icon: None,
            permissions: vec![],
            windows: files.clone(),
            linux: files,
            wasm: None,
            run: "tool.exe".into(),
            review: String::new(),
            updated: 0,
        }
    }

    #[test]
    fn installs_checked_files_and_refuses_changed_ones() {
        let dir = super::super::library::temp_dir("install");
        let (exe, dll) = (b"the program".to_vec(), b"a library".to_vec());
        let url = serve(vec![
            ("/library/files/tool/tool.exe".into(), exe.clone()),
            ("/library/files/tool/lib/helper.dll".into(), dll.clone()),
            ("/library/files/tool/evil.exe".into(), b"something else".to_vec()),
        ]);
        let entry = program(vec![file("tool.exe", &exe), file("lib/helper.dll", &dll)]);
        let mut last = (0, 0);
        let p = install(&dir, &url, &entry, |d, t| last = (d, t)).unwrap();
        assert_eq!(last, (20, 20));
        assert_eq!(std::fs::read(dir.join("tool/lib/helper.dll")).unwrap(), dll);
        assert!(verify(&dir, &p).is_ok());
        // A file swapped afterwards is noticed before anything runs.
        std::fs::write(dir.join("tool/tool.exe"), b"tampered").unwrap();
        assert!(launch(&dir, &p).unwrap_err().contains("changed since it was installed"));
        // A download that isn't what the list says installs nothing (and leaves the old one).
        let mut bad = program(vec![file("tool.exe", &exe)]);
        bad.windows[0].url = "files/tool/evil.exe".into();
        bad.linux = bad.windows.clone();
        assert!(install(&dir, &url, &bad, |_, _| {}).unwrap_err().contains("doesn't match"));
        assert!(!dir.join("tool.new").exists());
        assert!(dir.join("tool/lib/helper.dll").exists(), "the installed version stays");
        // An update replaces the whole folder (the old version's extra file goes).
        let p = install(&dir, &url, &program(vec![file("tool.exe", &exe)]), |_, _| {}).unwrap();
        assert!(!dir.join("tool/lib").exists());
        assert!(verify(&dir, &p).is_ok());
        uninstall(&dir, "tool").unwrap();
        assert!(!dir.join("tool").exists());
        assert!(uninstall(&dir, "../x").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn installed_list_round_trips() {
        let dir = super::super::library::temp_dir("list");
        let mut l = Installed::default();
        l.put(InstalledPlugin {
            id: "dice".into(),
            name: "Dice".into(),
            kind: Kind::Addon,
            version: "1.0.0".into(),
            run: "dice.wasm".into(),
            files: vec![],
            permissions: vec![Permission::Commands],
            installed_at: 1,
            enabled: true,
        });
        l.save(&dir).unwrap();
        assert_eq!(Installed::load(&dir).get("dice").unwrap().permissions, vec![Permission::Commands]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
