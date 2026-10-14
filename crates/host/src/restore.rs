//! Putting a server back from a backup (see the app's `backup.rs`): the backup folder is
//! packed as a tar of exactly what the server's data folder holds (`ingle.db`, `files/`,
//! `server-cert.der`, `server-key.der`) and unpacked there by the setup script
//! (`--restore`), before the server first starts. The server's private key is sealed in the
//! backup to the owner who made it (`server-key.der.sealed`, see
//! `ingle_proto::crypto::backup_key`): only their Ingle can put it back.

use std::path::PathBuf;

use anyhow::{Context, Result, bail};

/// A backup to put a server back from: its folder, and the key that opens the server's sealed
/// private key in it (the owner's, made from their identity).
#[derive(Clone)]
pub struct Backup {
    pub dir: PathBuf,
    pub key: Option<[u8; 32]>,
}

impl std::fmt::Debug for Backup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backup").field("dir", &self.dir).field("key", &self.key.map(|_| "..")).finish()
    }
}

/// Why a backup can't be put back with the server's identity.
const LOCKED: &str = "This backup's server identity is locked to the Ingle that made it (its owner's identity). Restore it from that \
                      Ingle, or from a copy of its identity.key. Without it you can set the server up by hand from the database \
                      and files, as a new server.";

/// Whether `b.key` opens the backup's sealed server key (a quick look, before anything's sent).
pub fn check(b: &Backup) -> Result<()> {
    let sealed = b.dir.join("identity").join("server-key.der.sealed");
    if sealed.is_file() {
        let data = std::fs::read(&sealed).with_context(|| format!("reading {}", sealed.display()))?;
        if b.key.as_ref().and_then(|k| ingle_proto::crypto::unseal(k, &data)).is_none() {
            bail!(LOCKED);
        }
    }
    Ok(())
}

/// What a backup folder holds, ready to unpack into a server's data folder.
pub fn pack(b: &Backup) -> Result<Vec<u8>> {
    let backup = b.dir.as_path();
    let db = backup.join("database").join("latest.db");
    if !db.is_file() {
        bail!("{} isn't an Ingle server backup (it has no database/latest.db).", backup.display());
    }
    let mut tar = Vec::new();
    add(&mut tar, "ingle.db", &std::fs::read(&db).context("reading the backup's database")?)?;
    let identity = backup.join("identity");
    let cert = identity.join("server-cert.der");
    let sealed = identity.join("server-key.der.sealed");
    let plain = identity.join("server-key.der");
    // The private key: sealed (backups since 0.8), or as it was in older backups.
    let key = if sealed.is_file() {
        let data = std::fs::read(&sealed).with_context(|| format!("reading {}", sealed.display()))?;
        match b.key.as_ref().and_then(|k| ingle_proto::crypto::unseal(k, &data)) {
            Some(k) => Some(k),
            None => bail!(LOCKED),
        }
    } else if plain.is_file() {
        Some(std::fs::read(&plain).with_context(|| format!("reading {}", plain.display()))?)
    } else {
        None
    };
    if let (true, Some(key)) = (cert.is_file(), key) {
        add(&mut tar, "server-cert.der", &std::fs::read(&cert).with_context(|| format!("reading {}", cert.display()))?)?;
        add(&mut tar, "server-key.der", &key)?;
    }
    if let Ok(rd) = std::fs::read_dir(backup.join("files")) {
        dir_entry(&mut tar, "files/");
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            // Only the server's own file names (32 hex digits).
            if name.len() == 32 && name.bytes().all(|b| b.is_ascii_hexdigit()) && e.path().is_file() {
                add(&mut tar, &format!("files/{name}"), &std::fs::read(e.path())?)?;
            }
        }
    }
    tar.extend_from_slice(&[0u8; 1024]);
    Ok(tar)
}

fn header(name: &str, size: u64, kind: u8, mode: &str) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name.as_bytes());
    let field = |h: &mut [u8; 512], at: usize, len: usize, v: &str| h[at..at + v.len().min(len)].copy_from_slice(&v.as_bytes()[..v.len().min(len)]);
    field(&mut h, 100, 8, mode);
    field(&mut h, 108, 8, "0000000\0");
    field(&mut h, 116, 8, "0000000\0");
    field(&mut h, 124, 12, &format!("{size:011o}\0"));
    field(&mut h, 136, 12, &format!("{:011o}\0", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)));
    h[156] = kind;
    field(&mut h, 257, 6, "ustar\0");
    field(&mut h, 263, 2, "00");
    // The checksum is taken with its own field as spaces.
    h[148..156].copy_from_slice(b"        ");
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    field(&mut h, 148, 8, &format!("{sum:06o}\0 "));
    h
}

fn dir_entry(tar: &mut Vec<u8>, name: &str) {
    tar.extend_from_slice(&header(name, 0, b'5', "0000700\0"));
}

fn add(tar: &mut Vec<u8>, name: &str, data: &[u8]) -> Result<()> {
    if name.len() >= 100 {
        bail!("name too long for the archive: {name}");
    }
    tar.extend_from_slice(&header(name, data.len() as u64, b'0', "0000600\0"));
    tar.extend_from_slice(data);
    let pad = (512 - data.len() % 512) % 512;
    tar.extend(std::iter::repeat_n(0u8, pad));
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn packs_a_backup_that_tar_can_read() {
        let d = std::env::temp_dir().join(format!("ingle-restore-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("database")).unwrap();
        std::fs::create_dir_all(d.join("identity")).unwrap();
        std::fs::create_dir_all(d.join("files")).unwrap();
        std::fs::write(d.join("database/latest.db"), b"SQLite format 3\0rest").unwrap();
        std::fs::write(d.join("identity/server-cert.der"), b"cert").unwrap();
        std::fs::write(d.join("identity/server-key.der"), b"key").unwrap();
        std::fs::write(d.join("files/0123456789abcdef0123456789abcdef"), vec![7u8; 1000]).unwrap();
        std::fs::write(d.join("files/not-ours.txt"), b"x").unwrap();
        let tar = super::pack(&super::Backup { dir: d.clone(), key: None }).unwrap();
        let t = d.join("out.tar");
        std::fs::write(&t, &tar).unwrap();
        let out = std::process::Command::new("tar").arg("-tvf").arg(&t).output().unwrap();
        let list = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert!(list.contains("ingle.db") && list.contains("server-key.der") && list.contains("files/0123456789abcdef0123456789abcdef"), "{list}");
        assert!(!list.contains("not-ours"), "{list}");
        let x = d.join("x");
        std::fs::create_dir_all(&x).unwrap();
        assert!(std::process::Command::new("tar").arg("-xf").arg(&t).arg("-C").arg(&x).status().unwrap().success());
        assert_eq!(std::fs::read(x.join("files/0123456789abcdef0123456789abcdef")).unwrap(), vec![7u8; 1000]);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_sealed_server_key_comes_back_only_for_its_owner() {
        use ingle_proto::crypto::{Identity, backup_key, seal};
        let d = std::env::temp_dir().join(format!("ingle-restore-sealed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("database")).unwrap();
        std::fs::create_dir_all(d.join("identity")).unwrap();
        std::fs::write(d.join("database/latest.db"), b"SQLite format 3\0rest").unwrap();
        std::fs::write(d.join("identity/server-cert.der"), b"cert").unwrap();
        let owner = Identity::generate();
        std::fs::write(d.join("identity/server-key.der.sealed"), seal(&backup_key(&owner), b"the private key")).unwrap();
        // The owner's Ingle: the key goes back as it was.
        let tar = super::pack(&super::Backup { dir: d.clone(), key: Some(backup_key(&owner)) }).unwrap();
        let t = d.join("out.tar");
        std::fs::write(&t, &tar).unwrap();
        let x = d.join("x");
        std::fs::create_dir_all(&x).unwrap();
        assert!(std::process::Command::new("tar").arg("-xf").arg(&t).arg("-C").arg(&x).status().unwrap().success());
        assert_eq!(std::fs::read(x.join("server-key.der")).unwrap(), b"the private key");
        assert!(super::check(&super::Backup { dir: d.clone(), key: Some(backup_key(&owner)) }).is_ok());
        // Anyone else's (or none): no restore with the server's identity, and it says so up front.
        let other = super::Backup { dir: d.clone(), key: Some(backup_key(&Identity::generate())) };
        assert!(super::pack(&other).is_err());
        assert!(super::check(&other).unwrap_err().to_string().contains("locked to the Ingle that made it"));
        assert!(super::pack(&super::Backup { dir: d.clone(), key: None }).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }
}
