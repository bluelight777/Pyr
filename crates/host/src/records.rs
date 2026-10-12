//! A note of the servers set up from this PC (`hosted-servers.json` in Ingle's settings
//! folder): where they are, how to log in, and for rented ones which droplet they are, so
//! they can be found, updated or deleted later.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct HostedServer {
    pub name: String,
    pub host: String,
    pub port: u16,
    /// "digitalocean" or "ssh".
    pub provider: String,
    pub ssh_user: String,
    pub ssh_port: u16,
    pub droplet_id: Option<u64>,
    pub region: Option<String>,
    pub size: Option<String>,
    pub invite_link: Option<String>,
    pub host_key: Option<String>,
    pub created_ms: i64,
    /// False while setup hasn't finished (a rented server may exist without Ingle on it).
    pub installed: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Records {
    pub servers: Vec<HostedServer>,
}

pub fn path(config_dir: &Path) -> PathBuf {
    config_dir.join("hosted-servers.json")
}

pub fn load(config_dir: &Path) -> Records {
    std::fs::read(path(config_dir)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Adds or updates the record for `server` (matched by droplet id, else by host and port).
pub fn save(config_dir: &Path, server: &HostedServer) -> Result<()> {
    let mut all = load(config_dir);
    let same = |s: &HostedServer| match (s.droplet_id, server.droplet_id) {
        (Some(a), Some(b)) => a == b,
        _ => s.host == server.host && s.port == server.port && !s.host.is_empty(),
    };
    match all.servers.iter_mut().find(|s| same(s)) {
        Some(s) => *s = server.clone(),
        None => all.servers.push(server.clone()),
    }
    write(config_dir, &all)
}

/// Forgets a rented server (after deleting it).
pub fn remove_droplet(config_dir: &Path, droplet_id: u64) -> Result<()> {
    let mut all = load(config_dir);
    all.servers.retain(|s| s.droplet_id != Some(droplet_id));
    write(config_dir, &all)
}

fn write(config_dir: &Path, all: &Records) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let p = path(config_dir);
    let tmp = p.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(all)?).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, &p).with_context(|| format!("writing {}", p.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_update_in_place() {
        let dir = std::env::temp_dir().join(format!("ingle-records-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = HostedServer { name: "A".into(), droplet_id: Some(7), provider: "digitalocean".into(), ..Default::default() };
        save(&dir, &s).unwrap();
        s.host = "203.0.113.5".into();
        s.installed = true;
        save(&dir, &s).unwrap();
        let other = HostedServer { name: "B".into(), host: "198.51.100.2".into(), port: 7766, provider: "ssh".into(), ..Default::default() };
        save(&dir, &other).unwrap();
        let all = load(&dir);
        assert_eq!(all.servers.len(), 2);
        assert_eq!(all.servers[0].host, "203.0.113.5");
        assert!(all.servers[0].installed);
        remove_droplet(&dir, 7).unwrap();
        assert_eq!(load(&dir).servers, vec![other]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
