//! Make a signed `latest.json` for the in-app updater.
//!
//!   cargo run -p ingle-proto --example release -- keygen release-key.txt
//!   cargo run -p ingle-proto --example release -- sign --key release-key.txt --version 0.2.0 \
//!       --notes "What's new" --base-url https://firelitgames.com/ingle \
//!       --windows dist/Ingle-Setup-0.2.0.exe --deb dist/ingle_0.2.0_amd64.deb --out latest.json
//!
//! The key file is the release key's secret. Keep it off the website and backed up: without
//! it you can't publish updates the app will accept.

use std::path::Path;

use anyhow::{Context, Result, bail};
use ingle_proto::b64;
use ingle_proto::crypto::Identity;
use ingle_proto::release::{self, Asset, Manifest};

fn asset(path: &str, base_url: &str) -> Result<Asset> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {path}"))?;
    let name = Path::new(path).file_name().context("file name")?.to_string_lossy().to_string();
    Ok(Asset { url: format!("{}/{name}", base_url.trim_end_matches('/')), sha256: release::sha256_hex(&bytes), size: bytes.len() as u64 })
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("keygen") => {
            let path = args.get(1).context("keygen <file>")?;
            let key = Identity::generate();
            // A new file only (never over an existing key), readable only by you.
            let mut opts = std::fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            let mut f = match opts.open(path) {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => bail!("{path} already exists; not overwriting a release key"),
                Err(e) => return Err(e.into()),
            };
            use std::io::Write;
            f.write_all(b64::encode(&key.seed()).as_bytes())?;
            println!("public key (put in the app): {}", key.user_id());
        }
        Some("sign") => {
            let get = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
            let key_text = std::fs::read_to_string(get("--key").context("--key")?)?;
            let seed = b64::decode_array::<32>(key_text.trim()).context("bad key file")?;
            let key = Identity::from_seed(seed);
            let base = get("--base-url").context("--base-url")?;
            let manifest = Manifest {
                version: get("--version").context("--version")?,
                notes: get("--notes").unwrap_or_default(),
                released: get("--released").map(|r| r.split(',').map(|v| v.trim().to_string()).filter(|v| !v.is_empty()).collect()).unwrap_or_default(),
                windows: get("--windows").map(|p| asset(&p, &base)).transpose()?,
                linux_deb: get("--deb").map(|p| asset(&p, &base)).transpose()?,
            };
            let out = get("--out").unwrap_or_else(|| "latest.json".into());
            std::fs::write(&out, serde_json::to_string_pretty(&release::sign(&key, &manifest))?)?;
            println!("wrote {out} for {} (signed by {})", manifest.version, key.user_id());
        }
        _ => bail!("usage: release keygen <file> | release sign --key FILE --version V --base-url URL [--notes T] [--windows EXE] [--deb DEB] [--out FILE]"),
    }
    Ok(())
}
