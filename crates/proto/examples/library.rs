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
//! Programs list `windows` and `linux` files, and `run` (the file to start). The tool reads
//! every file, puts its SHA-256 and size in the list, and signs the list with the release key
//! (with the library's own context), good for `--valid-days` (180 unless told).
//!
//!   cargo run -p ingle-proto --example library -- refresh --key release-key.txt --in index.json
//!
//! signs the same list again with new dates.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use ingle_proto::b64;
use ingle_proto::crypto::Identity;
use ingle_proto::library::{self, Entry, File, Kind, Library, Permission};
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

#[derive(Deserialize)]
struct Described2 {
    plugins: Vec<Described>,
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
            let lib = Library { issued: now(), expires: now() + days * 86_400, plugins };
            let out = get("--out").unwrap_or_else(|| "index.json".into());
            std::fs::write(&out, serde_json::to_string_pretty(&library::sign(&key, &lib))?)?;
            // Check it opens with the key it was signed with, as the app will.
            library::open(&std::fs::read_to_string(&out)?, &key.user_id()).map_err(anyhow::Error::msg)?;
            println!("wrote {out}: {} plugins, signed by {}, good for {days} days", lib.plugins.len(), key.user_id());
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
        _ => bail!("usage: library sign --key FILE --in plugins.json --files DIR [--out index.json] [--valid-days N] | library refresh --key FILE --in index.json"),
    }
    Ok(())
}
