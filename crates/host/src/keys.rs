//! Ingle's own SSH key, used to log in to servers it rents for you.
//!
//! Kept next to the rest of Ingle's settings as a normal OpenSSH key pair
//! (`ssh/ingle_ed25519` and `.pub`), so you can also log in yourself:
//! `ssh -i <that file> root@<server>`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use russh::keys::PrivateKey;
use russh::keys::ssh_key::LineEnding;
use russh::keys::ssh_key::private::{Ed25519Keypair, KeypairData};

pub struct IngleKey {
    pub key: Arc<PrivateKey>,
    /// `ssh-ed25519 AAAA… ingle`, for authorized_keys and provider APIs.
    pub public_openssh: String,
    pub path: PathBuf,
}

pub fn key_path(config_dir: &Path) -> PathBuf {
    config_dir.join("ssh").join("ingle_ed25519")
}

/// Loads Ingle's SSH key, creating it the first time.
pub fn load_or_create(config_dir: &Path) -> Result<IngleKey> {
    let path = key_path(config_dir);
    let key = match std::fs::read_to_string(&path) {
        Ok(text) => PrivateKey::from_openssh(text.as_bytes()).with_context(|| format!("reading {}", path.display()))?,
        // Only a missing key is made anew: any other trouble reading it must not replace
        // the key your rented servers trust.
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(e).with_context(|| format!("reading {}", path.display()));
        }
        Err(_) => {
            let mut seed = [0u8; 32];
            getrandom::getrandom(&mut seed).map_err(|e| anyhow::anyhow!("getting random bytes for an SSH key: {e}"))?;
            let key = PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)), "ingle")
                .context("creating an SSH key")?;
            let dir = path.parent().expect("key path has a folder");
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
            let text = key.to_openssh(LineEnding::LF).context("encoding the SSH key")?;
            write_private(&path, text.as_bytes())?;
            let public = key.public_key().to_openssh().context("encoding the SSH key")?;
            std::fs::write(path.with_extension("pub"), format!("{public}\n"))?;
            key
        }
    };
    let public_openssh = key.public_key().to_openssh().context("encoding the SSH key")?;
    Ok(IngleKey { key: Arc::new(key), public_openssh, path })
}

/// A fresh host key for a server Ingle rents: given to the server when it's created (so we
/// know its identity before ever connecting). Returns (private key in OpenSSH form, public
/// key line, SHA256 fingerprint).
pub fn new_host_key() -> Result<(String, String, String)> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| anyhow::anyhow!("getting random bytes for a host key: {e}"))?;
    let key = PrivateKey::new(KeypairData::Ed25519(Ed25519Keypair::from_seed(&seed)), "")
        .context("creating a host key")?;
    let private = key.to_openssh(LineEnding::LF).context("encoding the host key")?.to_string();
    let public = key.public_key().to_openssh().context("encoding the host key")?;
    let fingerprint = key.public_key().fingerprint(russh::keys::HashAlg::Sha256).to_string();
    Ok((private, public, fingerprint))
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("writing {}", path.display()))?;
        f.write_all(data)?;
    }
    #[cfg(not(unix))]
    std::fs::write(path, data).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// The first private key found in the usual places (~/.ssh/id_ed25519 and friends), to
/// pre-fill the key box.
pub fn default_user_key() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let ssh = PathBuf::from(home).join(".ssh");
    ["id_ed25519", "id_ecdsa", "id_rsa"].iter().map(|n| ssh.join(n)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_created_once_and_reloaded() {
        let dir = std::env::temp_dir().join(format!("ingle-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let a = load_or_create(&dir).unwrap();
        assert!(a.public_openssh.starts_with("ssh-ed25519 "), "{}", a.public_openssh);
        let b = load_or_create(&dir).unwrap();
        assert_eq!(a.public_openssh, b.public_openssh);
        let text = std::fs::read_to_string(&a.path).unwrap();
        assert!(text.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
        // OpenSSH's own parser must accept it too (users may log in with it).
        russh::keys::decode_secret_key(&text, None).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
