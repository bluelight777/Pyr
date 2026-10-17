//! Telling the invite directory at pyr.gg which of this server's invites open by their code
//! alone, so they work as short links, `https://pyr.gg/<code>` (see [`pyr_proto::directory`]).
//!
//! The whole list goes each time, signed with the server's TLS key (the key its invite links
//! pin): when an invite is made (and its maker waits for pyr.gg's answer before getting the
//! short link), taken back or used up, and every ten minutes while something changed (how
//! many are online). A server with nothing listed tells pyr.gg nothing at all.

use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use pyr_proto::directory::{ANNOUNCE_CONTEXT, Announce, AnnounceReply, ListedCode, SCHEME_ECDSA_P256, SCHEME_ED25519, SignedAnnounce};
use rustls::SignatureScheme;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::sign::Signer;
use tokio::sync::{mpsc, oneshot};
use tracing::{info, warn};

use crate::db::Db;
use crate::hub::Hub;

/// How often the list goes again while something changed (and how long a failed try waits,
/// at most, before the next).
const PERIOD: Duration = Duration::from_secs(600);
/// The same list again at least this often (pyr.gg forgets communities it hasn't heard from
/// in a month).
const REFRESH: Duration = Duration::from_secs(6 * 3600);

type Answer = Result<AnnounceReply, String>;

/// The server's line to the directory (while it runs; see [`start`]).
pub struct Directory {
    /// Where it is, `https://pyr.gg`.
    pub base: String,
    tx: mpsc::UnboundedSender<Option<oneshot::Sender<Answer>>>,
}

impl Directory {
    /// Sends the list now and waits for pyr.gg's answer.
    pub async fn sync(&self) -> Answer {
        let (tx, rx) = oneshot::channel();
        self.tx.send(Some(tx)).map_err(|_| "the server is stopping".to_string())?;
        match tokio::time::timeout(Duration::from_secs(25), rx).await {
            Ok(Ok(answer)) => answer,
            Ok(Err(_)) => Err("the server is stopping".into()),
            Err(_) => Err(format!("{} took too long to answer", host_of(&self.base))),
        }
    }

    /// Something changed: the list goes again in a moment.
    pub fn nudge(&self) {
        let _ = self.tx.send(None);
    }
}

fn host_of(base: &str) -> &str {
    base.split_once("://").map_or(base, |(_, h)| h).trim_end_matches('/')
}

/// The server's key, for signing what it tells the directory.
pub struct Signing {
    cert: String,
    signer: Box<dyn Signer>,
    scheme: &'static str,
}

impl Signing {
    pub fn new(cert_der: &[u8], key_der: &[u8]) -> Result<Signing> {
        let key = rustls::crypto::ring::sign::any_supported_type(&PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_der.to_vec())))
            .context("reading the server's key")?;
        let signer = key
            .choose_scheme(&[SignatureScheme::ECDSA_NISTP256_SHA256, SignatureScheme::ED25519])
            .ok_or_else(|| anyhow!("the server's key isn't one pyr.gg can check"))?;
        let scheme = if signer.scheme() == SignatureScheme::ED25519 { SCHEME_ED25519 } else { SCHEME_ECDSA_P256 };
        Ok(Signing { cert: pyr_proto::b64::encode(cert_der), signer, scheme })
    }

    pub fn sign(&self, a: &Announce) -> Result<SignedAnnounce> {
        let payload = serde_json::to_string(a)?;
        let mut msg = ANNOUNCE_CONTEXT.to_vec();
        msg.extend_from_slice(payload.as_bytes());
        let sig = self.signer.sign(&msg).map_err(|e| anyhow!("signing: {e}"))?;
        Ok(SignedAnnounce { cert: self.cert.clone(), scheme: self.scheme.to_string(), payload, sig: pyr_proto::b64::encode(&sig) })
    }
}

/// Whether short links can work for a server at `host`: not one only its own PC can reach.
pub fn reachable_host(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    if h.is_empty() || h.eq_ignore_ascii_case("localhost") {
        return false;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(ip) => !ip.is_loopback() && !ip.is_unspecified(),
        Err(_) => true,
    }
}

/// What the directory is told: where the server is, its name and counts, and (while short
/// links are on) its listed invites and own code.
pub fn announcement(db: &Db, host: &str, port: u16, online: Option<u32>) -> Announce {
    let on = db.get_meta("short_links").as_deref() != Some("0");
    let (codes, vanity) = if on {
        let names = |uid: &str| db.get_user(uid).map(|u| u.name).filter(|n| !n.trim().is_empty());
        let codes = db
            .listed_invites()
            .into_iter()
            .map(|i| ListedCode { inviter: names(&i.created_by), code: i.code, expires_at: i.expires_at })
            .collect();
        (codes, db.get_meta("vanity").filter(|v| !v.is_empty()))
    } else {
        (Vec::new(), None)
    };
    Announce {
        ts: pyr_proto::now_ms(),
        host: host.to_string(),
        port,
        name: db.get_meta("name").unwrap_or_default(),
        members: Some(db.member_count()),
        online,
        codes,
        vanity,
    }
}

fn agent(base: &str) -> ureq::Agent {
    let mut cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .http_status_as_error(false)
        .user_agent(concat!("pyr-server/", env!("CARGO_PKG_VERSION")));
    // (A directory on this machine, as in tests, isn't reached through a proxy.)
    let local = host_of(base).split(':').next().is_some_and(|h| h == "127.0.0.1" || h == "localhost") || host_of(base).starts_with("[::1]");
    if local {
        cfg = cfg.proxy(None);
    }
    cfg.build().into()
}

/// Sends an announcement and reads the answer (blocking).
pub fn post(agent: &ureq::Agent, base: &str, signed: &SignedAnnounce) -> Answer {
    let site = host_of(base);
    let url = format!("{}/api/v1/announce", base.trim_end_matches('/'));
    let mut resp = agent.post(&url).send_json(signed).map_err(|e| format!("couldn't reach {site}: {e}"))?;
    let status = resp.status().as_u16();
    let text = resp.body_mut().with_config().limit(256 * 1024).read_to_string().map_err(|e| format!("{site} answered oddly: {e}"))?;
    let reply: Option<AnnounceReply> = serde_json::from_str(&text).ok();
    match reply {
        Some(r) if (200..300).contains(&status) && r.error.is_none() => Ok(r),
        Some(AnnounceReply { error: Some(e), .. }) => Err(format!("{site} said: {e}")),
        _ => Err(format!("{site} answered HTTP {status}")),
    }
}

/// Tells the directory once, now, and waits (for `pyr-server invite`, while the server itself
/// may be running too: each list is the whole list, and the newest wins).
pub fn announce_once(base: &str, signing: &Signing, a: &Announce) -> Answer {
    let signed = signing.sign(a).map_err(|e| format!("{e:#}"))?;
    post(&agent(base), base, &signed)
}

/// Starts keeping the directory at `base` up to date about `hub`'s invites.
pub fn start(hub: &Arc<Hub>, base: String, signing: Signing) -> Directory {
    let (tx, mut rx) = mpsc::unbounded_channel::<Option<oneshot::Sender<Answer>>>();
    let weak: Weak<Hub> = Arc::downgrade(hub);
    let at = base.clone();
    tokio::spawn(async move {
        let agent = agent(&at);
        let signing = Arc::new(signing);
        // What pyr.gg was last told (all but the time) and when.
        let mut last: Option<(Announce, Instant)> = None;
        // Whether pyr.gg lists anything of ours: then even an empty list has to go (to take
        // those away).
        let mut listed_there = weak.upgrade().is_some_and(|h| h.db.get_meta("directory_listed").as_deref() == Some("1"));
        let mut wait = Duration::from_secs(5);
        // The last trouble told of (the same again goes to the debug log only).
        let mut said: Option<String> = None;
        loop {
            let mut waiting: Vec<oneshot::Sender<Answer>> = Vec::new();
            let woken = tokio::select! {
                m = rx.recv() => match m {
                    None => return,
                    Some(w) => {
                        waiting.extend(w);
                        true
                    }
                },
                _ = tokio::time::sleep(wait) => false,
            };
            if woken && waiting.is_empty() {
                // A nudge: whatever else is changing gets a moment to arrive too.
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            while let Ok(w) = rx.try_recv() {
                waiting.extend(w);
            }
            let Some(hub) = weak.upgrade() else { return };
            let a = announcement(&hub.db, &hub.public_host, hub.port, Some(hub.online_people() as u32));
            let has_any = !a.codes.is_empty() || a.vanity.is_some();
            let same = last.as_ref().is_some_and(|(l, when)| Announce { ts: a.ts, ..l.clone() } == a && when.elapsed() < REFRESH);
            if !has_any && !listed_there {
                // Nothing here and nothing there: pyr.gg needn't hear of this server at all.
                for w in waiting {
                    let _ = w.send(Ok(AnnounceReply::default()));
                }
                wait = PERIOD;
                continue;
            }
            if same && waiting.is_empty() {
                wait = PERIOD;
                continue;
            }
            let answer = match signing.sign(&a) {
                Ok(signed) => {
                    let (agent, at) = (agent.clone(), at.clone());
                    tokio::task::spawn_blocking(move || post(&agent, &at, &signed)).await.unwrap_or_else(|e| Err(e.to_string()))
                }
                Err(e) => Err(format!("{e:#}")),
            };
            match &answer {
                Ok(r) => {
                    // Codes another community has: they stay full links only.
                    for code in &r.taken {
                        let _ = hub.db.set_listed(code, false);
                    }
                    if let Some(why) = &r.vanity_refused {
                        warn!("pyr.gg wouldn't give this server its own code: {why}");
                    }
                    if has_any != listed_there {
                        info!("{} {}", host_of(&at), if has_any { "lists this server's invites" } else { "no longer lists this server's invites" });
                    }
                    listed_there = has_any;
                    let _ = hub.db.set_meta("directory_listed", if has_any { "1" } else { "0" });
                    last = Some((a, Instant::now()));
                    wait = PERIOD;
                    if said.take().is_some() {
                        info!("{} can be reached again", host_of(&at));
                    }
                }
                Err(e) => {
                    if said.as_ref() != Some(e) {
                        warn!("telling {} about this server's invites: {e}", host_of(&at));
                    } else {
                        tracing::debug!("telling {} about this server's invites: {e}", host_of(&at));
                    }
                    said = Some(e.clone());
                    last = None;
                    // Again soon, then less and less often.
                    wait = (wait * 2).clamp(Duration::from_secs(30), PERIOD);
                }
            }
            for w in waiting {
                let _ = w.send(answer.clone());
            }
        }
    });
    Directory { base, tx }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_addresses_others_can_reach() {
        for h in ["127.0.0.1", "localhost", "::1", "[::1]", "0.0.0.0", ""] {
            assert!(!reachable_host(h), "{h}");
        }
        for h in ["2.24.121.231", "play.example.com", "192.168.1.5", "2001:db8::1"] {
            assert!(reachable_host(h), "{h}");
        }
    }

    #[test]
    fn the_announcement_lists_what_the_server_lists() {
        let db = Db::open_in_memory().unwrap();
        db.set_meta("name", "Camp").unwrap();
        db.create_invite("listedaa", "server", None, None, pyr_proto::control::Role::Member).unwrap();
        db.set_listed("listedaa", true).unwrap();
        db.create_invite("unlisted", "server", None, None, pyr_proto::control::Role::Member).unwrap();
        db.create_invite("owner-abc", "server", Some(1), None, pyr_proto::control::Role::Owner).unwrap();
        db.set_listed("owner-abc", true).unwrap();
        db.create_invite("usedupaa", "server", Some(1), None, pyr_proto::control::Role::Member).unwrap();
        db.set_listed("usedupaa", true).unwrap();
        db.redeem_invite("usedupaa").unwrap();
        db.create_invite("brethren", "server", None, None, pyr_proto::control::Role::Member).unwrap();
        db.set_listed("brethren", true).unwrap();
        db.set_meta("vanity", "brethren").unwrap();
        let a = announcement(&db, "2.24.121.231", 7766, Some(3));
        assert_eq!(a.codes.iter().map(|c| c.code.as_str()).collect::<Vec<_>>(), vec!["listedaa"], "never owner links, used-up ones or the own code twice");
        assert_eq!((a.name.as_str(), a.vanity.as_deref(), a.online, a.members), ("Camp", Some("brethren"), Some(3), Some(0)));
        db.set_meta("short_links", "0").unwrap();
        let a = announcement(&db, "2.24.121.231", 7766, None);
        assert!(a.codes.is_empty() && a.vanity.is_none(), "short links off: nothing listed");
    }

    #[test]
    fn announcements_are_signed_with_the_servers_key() {
        let dir = std::env::temp_dir().join(format!("pyr-dir-sign-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tls = crate::tls::load_or_create(&dir).unwrap();
        let s = Signing::new(&tls.cert_der, &tls.key_der).unwrap();
        let a = Announce { ts: 1, host: "h".into(), port: 1, name: "n".into(), ..Default::default() };
        let signed = s.sign(&a).unwrap();
        assert_eq!(signed.scheme, SCHEME_ECDSA_P256);
        assert_eq!(pyr_proto::b64::decode(&signed.cert).unwrap(), tls.cert_der);
        assert_eq!(serde_json::from_str::<Announce>(&signed.payload).unwrap(), a);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
