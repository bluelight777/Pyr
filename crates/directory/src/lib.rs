//! The invite directory behind Pyr's short links (`https://pyr.gg/<code>`), run by Firelit
//! Games on pyr.gg behind nginx.
//!
//! * `POST /api/v1/announce`: a server lists the invites that open by their code alone,
//!   signed with its TLS key (see [`pyr_proto::directory`]); the directory makes that
//!   server's codes match.
//! * `GET /api/v1/invites/<code>`: what a code opens (the app and the page ask).
//! * `GET /<code>`: pyr.gg's page with the invite in it, and the community's name in the link
//!   preview chat apps show. (`/invite/<code>` and `/invite-<code>` go there too.)
//!
//! It never sees a community's messages or members: only its name, its address, how many
//! members it has and how many are online, and the codes it lists.

pub mod db;
pub mod page;

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper::header::{CACHE_CONTROL, CONTENT_TYPE, HeaderValue, LOCATION};
use hyper::{Method, Request, Response, StatusCode};
use parking_lot::Mutex;
use pyr_proto::directory::{ANNOUNCE_CONTEXT, Announce, AnnounceReply, SCHEME_ECDSA_P256, SCHEME_ED25519, SignedAnnounce, is_code, short_code};
use tokio::task::JoinHandle;
use tracing::{info, warn};

pub struct Config {
    /// Where the directory keeps its database.
    pub data_dir: PathBuf,
    /// Where it listens (localhost: nginx is in front).
    pub bind: SocketAddr,
    /// pyr.gg's files: its index.html is the page an invite opens.
    pub site_dir: Option<PathBuf>,
    /// Take the visitor's address from nginx's X-Real-IP (only when listening on localhost
    /// behind it).
    pub behind_proxy: bool,
}

pub struct Running {
    pub local_addr: SocketAddr,
    pub db: Arc<db::Db>,
    task: JoinHandle<()>,
}

impl Running {
    pub fn stop(self) {
        self.task.abort();
    }
}

/// Biggest announcement (a thousand codes with names is well under this).
/// (An announcement can carry the community's icon, see `pyr_proto::directory::ICON_MAX`.)
const MAX_BODY: usize = 1024 * 1024;
/// Connections at once.
const MAX_CONNECTIONS: usize = 512;

pub async fn start(cfg: Config) -> Result<Running> {
    std::fs::create_dir_all(&cfg.data_dir).with_context(|| format!("creating {}", cfg.data_dir.display()))?;
    let db = Arc::new(db::Db::open(&cfg.data_dir.join("directory.db"))?);
    db.tidy()?;
    let listener = tokio::net::TcpListener::bind(cfg.bind).await.with_context(|| format!("listening on {}", cfg.bind))?;
    let local_addr = listener.local_addr()?;
    let state = Arc::new(State {
        db: db.clone(),
        site: page::Site::new(cfg.site_dir.clone()),
        behind_proxy: cfg.behind_proxy,
        limits: Mutex::new(HashMap::new()),
    });
    let tidy = {
        let state = state.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(3600)).await;
                if let Err(e) = state.db.tidy() {
                    warn!("tidying the directory: {e:#}");
                }
                state.limits.lock().retain(|_, b| b.at.elapsed() < Duration::from_secs(600));
            }
        })
    };
    let slots = Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS));
    let task = tokio::spawn(async move {
        let _tidy = AbortOnDrop(tidy);
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(c) => c,
                Err(e) => {
                    warn!("accepting a connection: {e}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            };
            let Ok(slot) = slots.clone().try_acquire_owned() else { continue };
            let state = state.clone();
            tokio::spawn(async move {
                let _slot = slot;
                let io = hyper_util::rt::TokioIo::new(stream);
                let service = hyper::service::service_fn(move |req| {
                    let state = state.clone();
                    async move { Ok::<_, Infallible>(state.handle(req, peer).await) }
                });
                let conn = hyper::server::conn::http1::Builder::new()
                    .timer(hyper_util::rt::TokioTimer::new())
                    .header_read_timeout(Duration::from_secs(10))
                    .keep_alive(true)
                    .serve_connection(io, service);
                // (A connection can't stay forever, whatever it's doing.)
                let _ = tokio::time::timeout(Duration::from_secs(120), conn).await;
            });
        }
    });
    info!("invite directory listening on {local_addr}");
    Ok(Running { local_addr, db, task })
}

struct AbortOnDrop(JoinHandle<()>);
impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct State {
    db: Arc<db::Db>,
    site: page::Site,
    behind_proxy: bool,
    /// Requests per place (an address, a /64 for IPv6) and kind.
    limits: Mutex<HashMap<(IpAddr, Kind), Bucket>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Read,
    Announce,
}

struct Bucket {
    tokens: f64,
    at: Instant,
}

type Reply = Response<Full<Bytes>>;

/// Where a visitor is, for limits: an IPv6 address by its /64 (one home or server gets a
/// whole one).
fn place(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
        v4 => v4,
    }
}

impl State {
    fn allow(&self, ip: IpAddr, kind: Kind) -> bool {
        // (per second, at most at once)
        let (rate, burst) = match kind {
            Kind::Read => (2.0, 60.0),
            Kind::Announce => (1.0, 30.0),
        };
        let mut limits = self.limits.lock();
        // (A flood from very many addresses can't grow this without end.)
        if limits.len() > 200_000 {
            limits.clear();
        }
        let b = limits.entry((place(ip), kind)).or_insert(Bucket { tokens: burst, at: Instant::now() });
        let now = Instant::now();
        b.tokens = (b.tokens + now.duration_since(b.at).as_secs_f64() * rate).min(burst);
        b.at = now;
        if b.tokens < 1.0 {
            return false;
        }
        b.tokens -= 1.0;
        true
    }

    fn client_ip(&self, req: &Request<Incoming>, peer: SocketAddr) -> IpAddr {
        if self.behind_proxy && peer.ip().is_loopback() {
            if let Some(ip) = req.headers().get("x-real-ip").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok()) {
                return ip;
            }
        }
        peer.ip()
    }

    async fn handle(self: Arc<Self>, req: Request<Incoming>, peer: SocketAddr) -> Reply {
        let ip = self.client_ip(&req, peer);
        let path = req.uri().path().to_string();
        let head = req.method() == Method::HEAD;
        let mut reply = match (req.method().clone(), path.as_str()) {
            (Method::GET | Method::HEAD, "/api/v1/health") => text(StatusCode::OK, "ok"),
            (Method::GET | Method::HEAD, p) if p.starts_with("/api/v1/invites/") => {
                if !self.allow(ip, Kind::Read) {
                    return error(StatusCode::TOO_MANY_REQUESTS, "Too many lookups. Try again in a minute.");
                }
                let code = p["/api/v1/invites/".len()..].trim_end_matches('/').to_ascii_lowercase();
                match is_code(&code).then(|| self.db.lookup(&code)).flatten() {
                    Some(l) => json(StatusCode::OK, &l),
                    None => error(StatusCode::NOT_FOUND, "This invite isn't valid: it may have expired, or been taken back."),
                }
            }
            (Method::GET | Method::HEAD, p) if p.starts_with("/api/v1/icons/") => {
                if !self.allow(ip, Kind::Read) {
                    return error(StatusCode::TOO_MANY_REQUESTS, "Too many lookups. Try again in a minute.");
                }
                let code = p["/api/v1/icons/".len()..].trim_end_matches(".png").to_ascii_lowercase();
                match is_code(&code).then(|| self.db.icon(&code)).flatten() {
                    Some(png) => {
                        let mut r = Response::new(Full::new(Bytes::from(png)));
                        r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
                        r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=300"));
                        r.headers_mut().insert("x-content-type-options", HeaderValue::from_static("nosniff"));
                        r
                    }
                    None => error(StatusCode::NOT_FOUND, "No icon here."),
                }
            }
            (Method::POST, "/api/v1/announce") => {
                if !self.allow(ip, Kind::Announce) {
                    return error(StatusCode::TOO_MANY_REQUESTS, "Too many announcements. Try again in a minute.");
                }
                let body = match tokio::time::timeout(Duration::from_secs(15), Limited::new(req.into_body(), MAX_BODY).collect()).await {
                    Ok(Ok(b)) => b.to_bytes(),
                    Ok(Err(_)) => return error(StatusCode::PAYLOAD_TOO_LARGE, "That announcement is too big."),
                    Err(_) => return error(StatusCode::REQUEST_TIMEOUT, "The announcement took too long to arrive."),
                };
                let state = self.clone();
                let place = place(ip).to_string();
                match tokio::task::spawn_blocking(move || state.announce(&body, &place)).await {
                    Ok(Ok(r)) => json(StatusCode::OK, &r),
                    Ok(Err((status, msg))) => {
                        let r = AnnounceReply { error: Some(msg), ..Default::default() };
                        json(StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST), &r)
                    }
                    Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "Something went wrong on pyr.gg."),
                }
            }
            (Method::GET | Method::HEAD, p) if !p.starts_with("/api/") => {
                if !self.allow(ip, Kind::Read) {
                    return error(StatusCode::TOO_MANY_REQUESTS, "Too many requests. Try again in a minute.");
                }
                self.page(p)
            }
            _ => error(StatusCode::NOT_FOUND, "Nothing here."),
        };
        if head {
            *reply.body_mut() = Full::new(Bytes::new());
        }
        reply
    }

    /// The page for a short link's path (`/qrslmnov`; `/invite/…` and `/invite-…` are sent to
    /// that).
    fn page(&self, path: &str) -> Reply {
        let code = short_code(&format!("{}{path}", pyr_proto::directory::DIRECTORY));
        let Some(code) = code else {
            // No code at all: the plain page.
            let mut r = Response::new(Full::new(Bytes::from(page::render_plain(&self.site.index()))));
            *r.status_mut() = StatusCode::NOT_FOUND;
            r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
            return r;
        };
        if path.trim_end_matches('/') != format!("/{code}") {
            let mut r = Response::new(Full::new(Bytes::new()));
            *r.status_mut() = StatusCode::MOVED_PERMANENTLY;
            r.headers_mut().insert(LOCATION, HeaderValue::from_str(&format!("/{code}")).unwrap_or(HeaderValue::from_static("/")));
            return r;
        }
        let invite = self.db.lookup(&code);
        let html = page::render(&self.site.index(), &code, invite.as_ref());
        let mut r = Response::new(Full::new(Bytes::from(html)));
        if invite.is_none() {
            *r.status_mut() = StatusCode::NOT_FOUND;
        }
        r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
        // (Short: online counts change, and a taken-back invite should stop showing soon.)
        r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("public, max-age=60"));
        r
    }

    /// Checks an announcement's signature and takes it. Errors are (HTTP status, why).
    fn announce(&self, body: &[u8], place: &str) -> Result<AnnounceReply, (u16, String)> {
        let signed: SignedAnnounce = serde_json::from_slice(body).map_err(|e| (400, format!("That isn't an announcement: {e}")))?;
        let (fp, cert, announce) = verify(&signed).map_err(|e| (401, e))?;
        self.db.apply(&fp, &cert, place, &announce).map_err(|r| (r.status(), r.message()))
    }
}

/// Checks a signed announcement: the signature must be the certificate's key's, over
/// [`ANNOUNCE_CONTEXT`] and the announcement. Returns the certificate's fingerprint (what
/// invite links pin), the certificate, and the announcement.
pub fn verify(signed: &SignedAnnounce) -> Result<([u8; 32], Vec<u8>, Announce), String> {
    let cert = pyr_proto::b64::decode(&signed.cert).map_err(|_| "The certificate isn't base64.".to_string())?;
    let sig = pyr_proto::b64::decode(&signed.sig).map_err(|_| "The signature isn't base64.".to_string())?;
    if cert.len() > 8192 || sig.len() > 1024 {
        return Err("The certificate or signature is too big.".into());
    }
    let alg = match signed.scheme.as_str() {
        SCHEME_ECDSA_P256 => webpki::ring::ECDSA_P256_SHA256,
        SCHEME_ED25519 => webpki::ring::ED25519,
        other => return Err(format!("Unknown signature scheme {other:?}.")),
    };
    let der = rustls_pki_types::CertificateDer::from(cert.as_slice());
    let ee = webpki::EndEntityCert::try_from(&der).map_err(|e| format!("The certificate can't be read: {e}"))?;
    let mut msg = ANNOUNCE_CONTEXT.to_vec();
    msg.extend_from_slice(signed.payload.as_bytes());
    ee.verify_signature(alg, &msg, &sig).map_err(|_| "The signature doesn't match the certificate.".to_string())?;
    let announce: Announce = serde_json::from_str(&signed.payload).map_err(|e| format!("The announcement can't be read: {e}"))?;
    Ok((pyr_proto::crypto::fingerprint(&cert), cert, announce))
}

fn json<T: serde::Serialize>(status: StatusCode, v: &T) -> Reply {
    let mut r = Response::new(Full::new(Bytes::from(serde_json::to_vec(v).unwrap_or_default())));
    *r.status_mut() = status;
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    r.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn error(status: StatusCode, message: &str) -> Reply {
    json(status, &serde_json::json!({ "error": message }))
}

fn text(status: StatusCode, body: &'static str) -> Reply {
    let mut r = Response::new(Full::new(Bytes::from_static(body.as_bytes())));
    *r.status_mut() = status;
    r.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("text/plain; charset=utf-8"));
    r
}
