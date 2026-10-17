//! Sign the Ingle Library's list (`library/index.json` on firelitgames.com/ingle).
//!
//!   cargo run -p ingle-proto --example library -- sign --key release-key.txt \
//!       --in installer/site/library/plugins.json --files dist/library/files --out dist/library/index.json
//!
//! `plugins.json` describes the plugins (see `ingle_proto::library::Entry`), naming each file by
//! where it is under `--files` (the same layout as `library/files/` on the website):
//!
//!   { "plugins": [ { "id": "dice", "name": "Dice", "kind": "addon", "version": "1.0.0",
//!       "summary": "...", "permissions": ["commands"], "wasm": "dice/1.0.0/dice.wasm",
//!       "icon": "dice/icon.png" }, ... ] }
//!
//! Programs list `windows` and `linux` files, and `run` (the file to start). Skins are listed
//! apart, each by its folder under `--files` (every file in it is the skin's, checked as Ingle
//! checks skins):
//!
//!   "skins": [ { "id": "blood-moon", "name": "Blood Moon", "version": "1.0", "summary": "...",
//!       "author": "...", "dir": "blood-moon/1.0" } ]
//!
//! The tool reads every file, puts its SHA-256 and size in the list, and signs the list with
//! the release key (with the library's own context), good for `--valid-days` (180 unless told).
//!
//!   cargo run -p ingle-proto --example library -- refresh --key release-key.txt --in index.json
//!
//! signs the same list again with new dates.
//!
//!   cargo run -p ingle-proto --example library -- check-skin skins/aurora skins/blood-moon
//!
//! checks skin folders the way Ingle and the library do, without signing anything.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ingle_proto::b64;
use ingle_proto::crypto::Identity;
use ingle_proto::library::{self, Entry, File, Kind, Library, Permission, SkinEntry};
use serde::Deserialize;

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn load_key(path: &str) -> Result<Identity> {
    let key_text = std::fs::read_to_string(path)?;
    let seed = b64::decode_array::<32>(key_text.trim()).context("bad key file")?;
    Ok(Identity::from_seed(seed))
}

/// A plugin as described for signing: files named by where they are under `--files`.
#[derive(Deserialize)]
struct Described {
    id: String,
    name: String,
    kind: Kind,
    version: String,
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    review: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    permissions: Vec<Permission>,
    #[serde(default)]
    windows: Vec<String>,
    #[serde(default)]
    linux: Vec<String>,
    #[serde(default)]
    wasm: Option<String>,
    #[serde(default)]
    run: String,
    #[serde(default)]
    updated: i64,
}

/// A skin as described for signing: its folder under `--files`.
#[derive(Deserialize)]
struct DescribedSkin {
    id: String,
    name: String,
    version: String,
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: String,
    #[serde(default)]
    license: String,
    #[serde(default)]
    review: String,
    dir: String,
    #[serde(default)]
    updated: i64,
}

#[derive(Deserialize)]
struct Described2 {
    plugins: Vec<Described>,
    #[serde(default)]
    skins: Vec<DescribedSkin>,
}

/// Every file in a skin's folder: (path in the skin, bytes), in a stable order.
fn skin_files(files: &Path, dir: &str) -> Result<Vec<(String, Vec<u8>)>> {
    if dir.contains("..") || dir.starts_with('/') {
        bail!("{dir}: not a plain path");
    }
    let root = files.join(dir);
    let mut out = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).with_context(|| format!("reading {}", d.display()))? {
            let path = e?.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel: Vec<String> = path.strip_prefix(&root)?.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect();
                out.push((rel.join("/"), std::fs::read(&path)?));
            }
        }
    }
    out.sort();
    Ok(out)
}

fn file(files: &Path, rel: &str) -> Result<File> {
    if rel.contains("..") || rel.starts_with('/') {
        bail!("{rel}: not a plain path");
    }
    let bytes = std::fs::read(files.join(rel)).with_context(|| format!("reading {rel}"))?;
    let name = Path::new(rel).file_name().context("file name")?.to_string_lossy().to_string();
    Ok(File { path: name, url: format!("files/{rel}"), sha256: ingle_proto::release::sha256_hex(&bytes), size: bytes.len() as u64 })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let get = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    match args.first().map(String::as_str) {
        Some("sign") => {
            let key = load_key(&get("--key").context("--key")?)?;
            let files = std::path::PathBuf::from(get("--files").context("--files")?);
            let described: Described2 = serde_json::from_str(&std::fs::read_to_string(get("--in").context("--in")?)?)?;
            let days: i64 = get("--valid-days").map(|d| d.parse()).transpose()?.unwrap_or(library::VALID_DAYS);
            let mut plugins = Vec::new();
            for d in described.plugins {
                let list = |l: &[String]| l.iter().map(|r| file(&files, r)).collect::<Result<Vec<_>>>();
                let e = Entry {
                    windows: list(&d.windows)?,
                    linux: list(&d.linux)?,
                    wasm: d.wasm.as_deref().map(|r| file(&files, r)).transpose()?,
                    icon: d.icon.as_deref().map(|r| file(&files, r)).transpose()?,
                    id: d.id,
                    name: d.name,
                    kind: d.kind,
                    version: d.version,
                    summary: d.summary,
                    description: d.description,
                    author: d.author,
                    license: d.license,
                    source: d.source,
                    review: d.review,
                    permissions: d.permissions,
                    run: d.run,
                    updated: if d.updated > 0 { d.updated } else { now() },
                };
                e.check().map_err(anyhow::Error::msg)?;
                println!("{} {} ({}): {} bytes", e.id, e.version, e.kind.label(), e.windows.iter().chain(&e.linux).chain(&e.wasm).map(|f| f.size).sum::<u64>());
                plugins.push(e);
            }
            let mut skins = Vec::new();
            for d in described.skins {
                let found = skin_files(&files, &d.dir)?;
                let report = ingle_proto::skin::check(&found);
                if !report.ok() {
                    bail!("{}: {}", d.id, report.problems.join("; "));
                }
                for n in &report.notes {
                    println!("{}: note: {n}", d.id);
                }
                // A skin is listed under its maker's name: the author in its own skin.json (what
                // came with it when it was sent in; the Claude connector fills in the sender's
                // Ingle name), never another. The list may leave it out; if it says, it must agree.
                let m = &report.manifest;
                let maker = m.author.trim().to_string();
                if !d.author.trim().is_empty() && d.author.trim() != maker {
                    bail!("{}: listed as by {:?}, but its skin.json says {:?} (a skin is listed under its maker's name)", d.id, d.author.trim(), maker);
                }
                if d.version.trim() != m.version.trim() || d.name.trim() != m.name.trim() {
                    bail!("{}: listed as {} {}, but its skin.json says {} {}", d.id, d.name, d.version, m.name, m.version);
                }
                let list: Vec<File> = found
                    .iter()
                    .map(|(rel, bytes)| File {
                        path: rel.clone(),
                        url: format!("files/{}/{rel}", d.dir.trim_end_matches('/')),
                        sha256: ingle_proto::release::sha256_hex(bytes),
                        size: bytes.len() as u64,
                    })
                    .collect();
                let preview = list.iter().find(|f| ingle_proto::skin::is_preview(&f.path)).cloned();
                let e = SkinEntry {
                    id: d.id,
                    name: d.name,
                    version: d.version,
                    summary: d.summary,
                    description: d.description,
                    author: maker,
                    license: d.license,
                    files: list,
                    preview,
                    review: d.review,
                    updated: if d.updated > 0 { d.updated } else { now() },
                };
                e.check().map_err(anyhow::Error::msg)?;
                println!("{} {} (Skin): {} bytes", e.id, e.version, e.size());
                skins.push(e);
            }
            let lib = Library { issued: now(), expires: now() + days * 86_400, plugins, skins };
            let out = get("--out").unwrap_or_else(|| "index.json".into());
            std::fs::write(&out, serde_json::to_string_pretty(&library::sign(&key, &lib))?)?;
            // Check it opens with the key it was signed with, as the app will.
            library::open(&std::fs::read_to_string(&out)?, &key.user_id()).map_err(anyhow::Error::msg)?;
            println!("wrote {out}: {} plugins and {} skins, signed by {}, good for {days} days", lib.plugins.len(), lib.skins.len(), key.user_id());
        }
        Some("refresh") => {
            let key = load_key(&get("--key").context("--key")?)?;
            let input = get("--in").context("--in")?;
            let days: i64 = get("--valid-days").map(|d| d.parse()).transpose()?.unwrap_or(library::VALID_DAYS);
            let mut lib = library::open(&std::fs::read_to_string(&input)?, &key.user_id()).map_err(anyhow::Error::msg)?;
            lib.issued = now();
            lib.expires = now() + days * 86_400;
            let out = get("--out").unwrap_or(input);
            std::fs::write(&out, serde_json::to_string_pretty(&library::sign(&key, &lib))?)?;
            println!("wrote {out}: signed again, good for {days} days");
        }
        Some("check-skin") => {
            let mut bad = 0;
            for folder in &args[1..] {
                let found = skin_files(Path::new(folder), "")?;
                let report = ingle_proto::skin::check(&found);
                for n in &report.notes {
                    println!("{folder}: note: {n}");
                }
                if report.ok() {
                    println!("{folder}: ok, {} ({} files, {} KB)", report.manifest.name, found.len(), found.iter().map(|(_, b)| b.len()).sum::<usize>() / 1000);
                } else {
                    bad += 1;
                    for p in &report.problems {
                        println!("{folder}: {p}");
                    }
                }
            }
            if bad > 0 {
                bail!("{bad} of {} skins don't pass Ingle's checks", args.len() - 1);
            }
        }
        _ => bail!("usage: library sign --key FILE --in plugins.json --files DIR [--out index.json] [--valid-days N] | library refresh --key FILE --in index.json | library check-skin FOLDER..."),
    }
    Ok(())
}
