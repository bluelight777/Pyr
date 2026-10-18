//! The Pyr community server.
//!
//! One binary hosts one community: text channels with history, voice channels (each with
//! its own text chat), invites, roles, and a selective forwarding unit (SFU) for voice,
//! webcams and screen shares. Media is end-to-end encrypted by the clients; the server
//! only forwards ciphertext.

pub mod conn;
pub mod db;
pub mod directory;
pub mod hub;
pub mod poker;
pub mod tls;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use pyr_proto::control::{ChannelKind, Role};
use tokio::task::JoinHandle;
use tracing::{info, warn};

pub use hub::Hub;

pub struct ServerConfig {
    pub data_dir: PathBuf,
    pub bind: SocketAddr,
    /// Host name or IP that invite links should contain.
    pub public_host: Option<String>,
    /// Community name used on first start.
    pub name: Option<String>,
}

pub struct RunningServer {
    pub local_addr: SocketAddr,
    pub fingerprint: [u8; 32],
    pub hub: Arc<Hub>,
    /// The public address given now or remembered from an earlier start (None: LAN address).
    pub public_host: Option<String>,
    endpoint: quinn::Endpoint,
    task: JoinHandle<()>,
}

impl RunningServer {
    /// A link anyone can use to join (created on first start, never expires).
    pub fn invite_link(&self) -> String {
        let code = self.hub.db.get_meta("default_invite").unwrap_or_default();
        self.hub.invite_link(&code)
    }

    /// The one-time link that makes its first user the owner, while unused.
    pub fn owner_link(&self) -> Option<String> {
        let code = self.hub.db.usable_invite(Role::Owner)?;
        Some(self.hub.invite_link(&code))
    }

    pub async fn shutdown(self) {
        self.endpoint.close(quinn::VarInt::from_u32(0), b"server shutting down");
        self.task.abort();
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), self.endpoint.wait_idle()).await;
    }
}

/// Best-effort LAN address of this machine (no packets are sent).
pub fn detect_lan_ip() -> Option<String> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_unspecified() { None } else { Some(ip.to_string()) }
}

fn first_run_setup(db: &db::Db, name: Option<&str>) -> Result<()> {
    if db.get_meta("initialized").is_some() {
        return Ok(());
    }
    let name = name
        .and_then(|n| pyr_proto::control::clean_name(n, 48))
        .unwrap_or_else(|| "My Pyr Server".to_string());
    db.set_meta("name", &name)?;
    db.set_meta("description", "")?;
    db.create_channel("General", ChannelKind::Text)?;
    db.create_channel("Links and clips", ChannelKind::Text)?;
    db.create_channel("Lounge", ChannelKind::Voice)?;
    db.create_channel("Gaming", ChannelKind::Voice)?;
    let invite = db::new_invite_code(10);
    db.create_invite(&invite, "server", None, None, Role::Member)?;
    db.set_meta("default_invite", &invite)?;
    let owner = format!("owner-{}", db::new_invite_code(16));
    db.create_invite(&owner, "server", Some(1), Some(owner_link_expiry()), Role::Owner)?;
    db.set_meta("initialized", "1")?;
    info!("created a new community: {name}");
    Ok(())
}

/// Owner links work for a week: plenty of time to set up, not a key left lying around.
pub fn owner_link_expiry() -> i64 {
    pyr_proto::now_ms() + 7 * 24 * 3_600_000
}

/// The server's data folder, made if it's new, and readable only by the server's own account
/// either way (it holds every message and the server's private key). Every way in (running the
/// server, or a command like `invite` before it ever ran) goes through here.
pub fn private_data_dir(dir: &std::path::Path) -> Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        b.mode(0o700);
    }
    b.create(dir).with_context(|| format!("creating {}", dir.display()))?;
    #[cfg(unix)]
    {
        // (A folder system that can't do this, some mounted drives say: carry on, but say so.)
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)) {
            tracing::warn!("couldn't make {} readable only by this account: {e}", dir.display());
        }
    }
    Ok(())
}

/// Opens (or creates) the community in `cfg.data_dir`: TLS identity, database, default
/// channels and invites, and the remembered public host.
fn prepare(cfg: &ServerConfig) -> Result<(tls::TlsIdentity, db::Db)> {
    private_data_dir(&cfg.data_dir)?;
    let tls_id = tls::load_or_create(&cfg.data_dir)?;
    let db = db::Db::open(&cfg.data_dir.join("ingle.db"))?;
    first_run_setup(&db, cfg.name.as_deref())?;
    db.prune_invites()?;
    // --public-host is remembered, so starting the server again (say, from a shortcut)
    // keeps handing out links with the right address. `--public-host ""` forgets it.
    if let Some(h) = &cfg.public_host {
        db.set_meta("public_host", h.trim())?;
    }
    Ok((tls_id, db))
}

/// The links a new community needs, without starting the server: the one-time owner link
/// (a fresh one if the first was used) and the everyday invite link for friends.
/// Used by setup scripts, which print them for the person installing the server.
pub fn setup_links(cfg: &ServerConfig) -> Result<SetupLinks> {
    let (tls_id, db) = prepare(cfg)?;
    let host = db
        .get_meta("public_host")
        .filter(|h| !h.is_empty())
        .or_else(detect_lan_ip)
        .unwrap_or_else(|| "127.0.0.1".into());
    // A fresh owner link only while nobody owns the community yet (after that, `pyr-server
    // owner-invite` makes one on purpose).
    let owner = match db.usable_invite(Role::Owner) {
        Some(code) => Some(code),
        None if !db.owner_exists() => {
            let code = format!("owner-{}", db::new_invite_code(16));
            db.create_invite(&code, "server", Some(1), Some(owner_link_expiry()), Role::Owner)?;
            Some(code)
        }
        None => None,
    };
    let invite = db.get_meta("default_invite").unwrap_or_default();
    let link = |code: String| {
        pyr_proto::invite::InviteLink::new(host.clone(), cfg.bind.port(), Some(code), tls_id.fingerprint)
            .to_url()
    };
    Ok(SetupLinks { name: db.get_meta("name").unwrap_or_default(), owner: owner.map(link).unwrap_or_default(), invite: link(invite) })
}

pub struct SetupLinks {
    pub name: String,
    /// Empty once the community has an owner.
    pub owner: String,
    pub invite: String,
}

/// Runs the community in `cfg.data_dir`, without short links (see [`start_with`]).
pub async fn start(cfg: ServerConfig) -> Result<RunningServer> {
    start_with(cfg, None).await
}

/// Runs the community in `cfg.data_dir`. `invite_directory` is the directory that makes its
/// invites short links (`https://pyr.gg`, see [`directory`]), or None for none.
pub async fn start_with(cfg: ServerConfig, invite_directory: Option<String>) -> Result<RunningServer> {
    let (tls_id, db) = prepare(&cfg)?;
    let server_config = tls::server_config(&tls_id)?;
    let (endpoint, fallback) = pyr_proto::udp::bind_endpoint(cfg.bind, Some(server_config))
        .with_context(|| format!("binding UDP {} (is another server already using this port?)", cfg.bind))?;
    if let Some(why) = fallback {
        warn!("the network stack refused QUIC socket options ({why}); using a plain UDP socket");
    }
    let local_addr = endpoint.local_addr()?;
    let configured = db.get_meta("public_host").filter(|h| !h.is_empty());
    let public_host = configured.clone().unwrap_or_else(|| {
        if local_addr.ip().is_loopback() { local_addr.ip().to_string() } else { detect_lan_ip().unwrap_or_else(|| "127.0.0.1".into()) }
    });
    let hub = Hub::new(db, tls_id.fingerprint, public_host, local_addr.port(), cfg.data_dir.join("files"));
    // Short links: invites listed at pyr.gg, when the server's address works beyond this PC.
    match invite_directory {
        Some(base) if directory::reachable_host(&hub.public_host) => match directory::Signing::new(&tls_id.cert_der, &tls_id.key_der) {
            Ok(signing) => {
                let _ = hub.directory.set(directory::start(&hub, base, signing));
            }
            Err(e) => {
                warn!("short links are off: {e:#}");
                let _ = hub.short_links_off_why.set(format!("Short links can't work for this server: {e:#}."));
            }
        },
        Some(_) => {
            let _ = hub.short_links_off_why.set(format!(
                "Short links need an address others can reach, and this server's is {} (start it with --public-host and your public address or domain).",
                hub.public_host
            ));
        }
        None => {
            let _ = hub.short_links_off_why.set("This server was started with short links off (--directory off).".into());
        }
    }

    // How everyone's viewers are getting on with their camera or screen share.
    tokio::spawn(hub.clone().feedback_loop());

    // Poker: chips left at tables when the server last stopped go back to their owners, and
    // the tables' clock (deals, bots' turns, people's time running out).
    let n = hub.db.poker_return_held();
    if n > 0 {
        tracing::info!("gave {n} people back the poker chips they had at tables");
    }
    let h = hub.clone();
    tokio::spawn(async move {
        let mut every = tokio::time::interval(std::time::Duration::from_millis(200));
        loop {
            every.tick().await;
            h.poker.tick(&h);
        }
    });

    // Attachments that were uploaded but never sent, and unused profile pictures.
    let h = hub.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(600)).await;
            let ids = h.db.take_unused_files(60 * 60 * 1000);
            h.wipe_files(&ids);
        }
    });

    let ep = endpoint.clone();
    let h = hub.clone();
    let task = tokio::spawn(async move {
        while let Some(incoming) = ep.accept().await {
            // Caps on connections per address and in all; under a flood of handshakes, new
            // ones must prove their address first.
            let ip = incoming.remote_address().ip();
            match h.admit(ip, incoming.remote_address_validated()) {
                (hub::Admit::Accept, Some(slot)) => {
                    tokio::spawn(conn::handle(h.clone(), incoming, slot));
                }
                (hub::Admit::Retry, _) => {
                    let _ = incoming.retry();
                }
                _ => incoming.refuse(),
            }
        }
    });

    Ok(RunningServer { local_addr, fingerprint: tls_id.fingerprint, hub, public_host: configured, endpoint, task })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn public_host_is_remembered() {
        let dir = std::env::temp_dir().join(format!("pyr-pubhost-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = |host: Option<&str>| ServerConfig {
            data_dir: dir.clone(),
            bind: "127.0.0.1:0".parse().unwrap(),
            public_host: host.map(str::to_string),
            name: None,
        };
        let s = start(cfg(Some("play.example.com"))).await.unwrap();
        assert!(s.invite_link().starts_with("pyr://play.example.com:"));
        s.shutdown().await;
        let s = start(cfg(None)).await.unwrap();
        assert_eq!(s.public_host.as_deref(), Some("play.example.com"), "remembered without the flag");
        assert!(s.invite_link().starts_with("pyr://play.example.com:"));
        s.shutdown().await;
        let s = start(cfg(Some(""))).await.unwrap();
        assert_eq!(s.public_host, None, "an empty --public-host forgets it");
        assert!(s.invite_link().starts_with("pyr://127.0.0.1:"));
        s.shutdown().await;
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn setup_links_match_the_running_server() {
        let dir = std::env::temp_dir().join(format!("pyr-links-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = || ServerConfig {
            data_dir: dir.clone(),
            bind: "127.0.0.1:7766".parse().unwrap(),
            public_host: Some("203.0.113.9".into()),
            name: Some("Campfire".into()),
        };
        let first = setup_links(&cfg()).unwrap();
        assert_eq!(first.name, "Campfire");
        assert!(first.owner.starts_with("pyr://203.0.113.9/owner-"), "{}", first.owner);
        // Asking again (say, the setup script runs twice) gives the same unused owner link.
        let again = setup_links(&cfg()).unwrap();
        assert_eq!(again.owner, first.owner);
        assert_eq!(again.invite, first.invite);
        // The server started later hands out the same links.
        let s = start(ServerConfig { bind: "127.0.0.1:0".parse().unwrap(), public_host: None, ..cfg() }).await.unwrap();
        let port = s.local_addr.port();
        assert_eq!(s.owner_link().unwrap(), first.owner.replace(".9/", &format!(".9:{port}/")));
        assert_eq!(s.invite_link(), first.invite.replace(".9/", &format!(".9:{port}/")));
        // Once the owner link is used up, a new one-time link is made.
        let code = first.owner.split('/').nth(3).unwrap().split('?').next().unwrap().to_string();
        s.hub.db.redeem_invite(&code).unwrap();
        s.shutdown().await;
        let after = setup_links(&cfg()).unwrap();
        assert_ne!(after.owner, first.owner);
        assert!(after.owner.contains("/owner-"));
        // Once someone owns the community, no new owner links are made (and old ones that
        // never expire are removed at start).
        {
            let db = db::Db::open(&dir.join("ingle.db")).unwrap();
            db.insert_user(&pyr_proto::crypto::Identity::generate().user_id(), "Owner", Role::Owner).unwrap();
            db.create_invite("owner-legacy", "server", Some(1), None, Role::Owner).unwrap();
        }
        let owned = setup_links(&cfg()).unwrap();
        assert!(owned.owner.is_empty() || owned.owner == after.owner, "{}", owned.owner);
        let db = db::Db::open(&dir.join("ingle.db")).unwrap();
        assert_eq!(db.invite_role("owner-legacy"), None, "a never-expiring owner link is removed once there's an owner");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn the_data_folder_is_the_servers_alone() {
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let base = std::env::temp_dir().join(format!("pyr-private-data-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        // A new one (made by `pyr-server invite`, say, before the server ever ran)...
        let new = base.join("new").join("pyr-data");
        private_data_dir(&new).unwrap();
        assert_eq!(mode(&new), 0o700);
        // ...and one made earlier that others could read.
        let old = base.join("old");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_data_dir(&old).unwrap();
        assert_eq!(mode(&old), 0o700);
        let _ = std::fs::remove_dir_all(&base);
    }
}
