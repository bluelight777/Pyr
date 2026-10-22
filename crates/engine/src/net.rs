//! QUIC client: certificate pinning, connecting and logging in.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use pyr_proto::control::*;
use pyr_proto::crypto::{self, Identity};
use pyr_proto::{PROTOCOL_VERSION, framing};
use quinn::VarInt;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// Accepts exactly one certificate: the one whose SHA-256 matches the invite link (all 32
/// bytes, or the first `len` from a short link — then `seen` records the full one).
#[derive(Debug)]
struct PinnedVerifier {
    fingerprint: [u8; 32],
    len: usize,
    seen: Arc<std::sync::Mutex<Option<[u8; 32]>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let full = crypto::fingerprint(end_entity.as_ref());
        if full[..self.len] == self.fingerprint[..self.len] {
            *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(full);
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("server certificate does not match the invite link".into()))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

pub fn transport_config() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    // Video frames from the server, one stream each (see call::uni_loop for how many are read
    // at once).
    t.max_concurrent_uni_streams(VarInt::from_u32(256));
    t.max_concurrent_bidi_streams(VarInt::from_u32(16));
    t.keep_alive_interval(Some(Duration::from_secs(4)));
    t.max_idle_timeout(Some(Duration::from_secs(20).try_into().expect("valid idle timeout")));
    t.datagram_receive_buffer_size(Some(1 << 20));
    // Sound waiting to go out (while the connection is full) is dropped, oldest first, past
    // this: a fraction of a second of voice and a share's sound. Late sound is no use (it's
    // thrown away when it arrives), and a long queue of it only delays what comes after.
    // (`PYR_AB=dgram`: 64 KB, seconds of it.)
    t.datagram_send_buffer_size(if pyr_proto::ab::old("dgram") { 64 * 1024 } else { 4 * 1024 });
    t.stream_receive_window(VarInt::from_u32(8 << 20));
    t.receive_window(VarInt::from_u32(64 << 20));
    t.send_window(16 << 20);
    if pyr_proto::ab::bbr() {
        t.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    }
    t
}

pub fn client_config(fingerprint: [u8; 32]) -> Result<quinn::ClientConfig> {
    pinned_config(fingerprint, 32, Arc::default())
}

fn pinned_config(
    fingerprint: [u8; 32],
    len: usize,
    seen: Arc<std::sync::Mutex<Option<[u8; 32]>>>,
) -> Result<quinn::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedVerifier { fingerprint, len: len.clamp(1, 32), seen, provider }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![pyr_proto::ALPN.to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(crypto));
    cfg.transport_config(Arc::new(transport_config()));
    Ok(cfg)
}

/// One client endpoint per address family, created lazily and shared by all servers.
#[derive(Default)]
pub struct Endpoints {
    v4: Option<(quinn::Endpoint, u64)>,
    v6: Option<(quinn::Endpoint, u64)>,
}

impl Endpoints {
    pub fn for_addr(&mut self, addr: SocketAddr) -> Result<quinn::Endpoint> {
        let slot = if addr.is_ipv4() { &mut self.v4 } else { &mut self.v6 };
        if let Some((e, _)) = slot {
            return Ok(e.clone());
        }
        let bind: SocketAddr = if addr.is_ipv4() { "0.0.0.0:0".parse()? } else { "[::]:0".parse()? };
        let (e, fallback, raw) = pyr_proto::udp::bind_endpoint_raw(bind, None).context("opening UDP socket")?;
        if let Some(why) = fallback {
            tracing::warn!("the network stack refused QUIC socket options ({why}); using a plain UDP socket");
        }
        *slot = Some((e.clone(), raw));
        Ok(e)
    }

    /// Connected to a server at `addr`: its packets get voice priority (see `qos`).
    pub fn connected(&self, addr: SocketAddr) {
        let slot = if addr.is_ipv4() { &self.v4 } else { &self.v6 };
        if let Some((_, raw)) = slot {
            crate::qos::watch(*raw, addr);
        }
    }
}

pub async fn resolve(host: &str, port: u16) -> Result<SocketAddr> {
    let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
        .await
        .with_context(|| format!("couldn't find the server \"{host}\""))?
        .collect();
    addrs
        .iter()
        .find(|a| a.is_ipv4())
        .or_else(|| addrs.first())
        .copied()
        .ok_or_else(|| anyhow!("no address for {host}"))
}

pub struct LoggedIn {
    pub conn: quinn::Connection,
    pub send: quinn::SendStream,
    pub recv: quinn::RecvStream,
    pub ready: Event,
}

/// Errors the user must fix (no point retrying).
#[derive(Debug)]
pub struct Fatal(pub String);

impl std::fmt::Display for Fatal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Fatal {}

pub async fn read_frame<T: serde::de::DeserializeOwned>(recv: &mut quinn::RecvStream) -> Result<Option<T>> {
    match read_body(recv).await? {
        Some(body) => Ok(Some(framing::decode(&body)?)),
        None => Ok(None),
    }
}

/// One frame's bytes, not yet read as anything (None: the stream ended).
pub async fn read_body(recv: &mut quinn::RecvStream) -> Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = framing::body_len(len)?;
    let mut body = vec![0u8; n];
    recv.read_exact(&mut body).await?;
    Ok(Some(body))
}

/// For a short invite link: connects to the server, checks its certificate starts with the
/// link's fingerprint and returns the whole fingerprint (which is then pinned from now on).
pub async fn learn_fingerprint(
    endpoints: &std::sync::Mutex<Endpoints>,
    link: &pyr_proto::invite::InviteLink,
) -> Result<[u8; 32]> {
    let (host, port) = (link.host.as_str(), link.port);
    let addr = resolve(host, port).await?;
    let endpoint = endpoints.lock().map_err(|_| anyhow!("endpoint lock poisoned"))?.for_addr(addr)?;
    let seen = Arc::new(std::sync::Mutex::new(None));
    let connecting = endpoint.connect_with(pinned_config(link.fingerprint, link.fp_len, seen.clone())?, addr, pyr_proto::APP_ID)?;
    let conn = tokio::time::timeout(Duration::from_secs(8), connecting)
        .await
        .map_err(|_| anyhow!("The server didn't answer (is it running, and is UDP port {port} open?)"))?
        .map_err(|e| {
            let s = e.to_string();
            if s.contains("does not match the invite") || s.contains("certificate") {
                anyhow!("This server's identity doesn't match the invite link.")
            } else {
                anyhow!("Couldn't connect: {s}")
            }
        })?;
    conn.close(VarInt::from_u32(0), b"");
    let full = seen.lock().unwrap_or_else(|e| e.into_inner()).take();
    full.ok_or_else(|| anyhow!("the server didn't show its certificate"))
}

pub async fn connect_and_login(
    endpoints: &std::sync::Mutex<Endpoints>,
    host: &str,
    port: u16,
    fingerprint: [u8; 32],
    identity: &Identity,
    name: &str,
    invite: Option<&str>,
) -> Result<LoggedIn> {
    let addr = resolve(host, port).await?;
    let endpoint = endpoints.lock().map_err(|_| anyhow!("endpoint lock poisoned"))?.for_addr(addr)?;
    let connecting = endpoint.connect_with(client_config(fingerprint)?, addr, pyr_proto::APP_ID)?;
    let conn = tokio::time::timeout(Duration::from_secs(10), connecting)
        .await
        .map_err(|_| anyhow!("the server didn't answer (is it running, and is UDP port {port} open?)"))?
        .map_err(|e| {
            let s = e.to_string();
            if s.contains("does not match the invite") || s.contains("certificate") {
                anyhow::Error::new(Fatal("This server's identity doesn't match the invite link.".into()))
            } else {
                anyhow!("connection failed: {s}")
            }
        })?;
    if let Ok(e) = endpoints.lock() {
        e.connected(addr);
    }
    let (mut send, mut recv) = conn.open_bi().await?;
    // Messages go ahead of video (and of files), see `CONTROL_PRIORITY`.
    if !pyr_proto::ab::old("fair") {
        let _ = send.set_priority(pyr_proto::media::CONTROL_PRIORITY);
    }
    // QUIC only announces a stream once data is sent on it.
    send.write_all(pyr_proto::CONTROL_PREAMBLE).await?;

    let hello: ServerFrame = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut recv))
        .await
        .map_err(|_| anyhow!("timed out waiting for the server"))??
        .ok_or_else(|| anyhow!("server closed the connection"))?;
    let Event::Hello { proto, nonce, .. } = hello.ev else { bail!("unexpected greeting from server") };
    if proto != PROTOCOL_VERSION {
        return Err(Fatal(format!(
            "This server uses protocol v{proto}, but this app uses v{PROTOCOL_VERSION}. One of you needs to update."
        ))
        .into());
    }
    let mut exporter = [0u8; 32];
    conn.export_keying_material(&mut exporter, crypto::LOGIN_EXPORTER_LABEL, b"")
        .map_err(|_| anyhow!("TLS exporter unavailable"))?;
    let uid = identity.user_id();
    let sig = identity.sign(&crypto::login_transcript(&nonce, &exporter, &uid));
    let login = ClientFrame {
        id: 1,
        req: Request::Login {
            proto: PROTOCOL_VERSION,
            user_id: uid,
            sig: sig.to_vec(),
            name: name.to_string(),
            invite: invite.map(|s| s.to_string()),
            client: format!("{} {}", pyr_proto::APP_NAME, env!("CARGO_PKG_VERSION")),
            // (This app knows group conversations, and adjusts its shares to its viewers.)
            caps: vec![pyr_proto::control::GROUPS_CAP.to_string(), pyr_proto::control::VIDEO_FEEDBACK_CAP.to_string(), pyr_proto::control::MOVE_CAP.to_string(), pyr_proto::control::CASINO_012_CAP.to_string(), pyr_proto::control::CASINO_013_CAP.to_string()],
        },
    };
    send.write_all(&framing::encode(&login)).await?;
    let reply: ServerFrame = tokio::time::timeout(Duration::from_secs(15), read_frame(&mut recv))
        .await
        .map_err(|_| anyhow!("timed out logging in"))??
        .ok_or_else(|| anyhow!("server closed the connection"))?;
    match reply.ev {
        ev @ Event::Ready { .. } => Ok(LoggedIn { conn, send, recv, ready: ev }),
        Event::Error { code, message } => {
            let fatal = matches!(
                code,
                ErrorCode::InviteInvalid | ErrorCode::Banned | ErrorCode::VersionMismatch | ErrorCode::Unauthorized
            );
            if fatal { Err(Fatal(message).into()) } else { Err(anyhow!(message)) }
        }
        _ => bail!("unexpected reply to login"),
    }
}

// ------------------------------------------------------------------ file streams

/// Download a file (an attachment or a profile picture) from the server.
pub async fn file_get(conn: &quinn::Connection, id: &str) -> Result<(String, Vec<u8>)> {
    use pyr_proto::control::{FILE_PREAMBLE, FileReply, FileRequest, MAX_ATTACHMENT_BYTES};
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(FILE_PREAMBLE).await?;
    send.write_all(&framing::encode(&FileRequest::Get { id: id.to_string() })).await?;
    send.finish()?;
    let reply: FileReply = tokio::time::timeout(Duration::from_secs(20), read_frame(&mut recv))
        .await
        .map_err(|_| anyhow!("timed out"))??
        .ok_or_else(|| anyhow!("no reply"))?;
    if !reply.ok {
        bail!("{}", reply.error);
    }
    // (An encrypted picture has its tag on top.)
    if reply.size > MAX_ATTACHMENT_BYTES + 16 {
        bail!("file too large");
    }
    let bytes = tokio::time::timeout(Duration::from_secs(120), recv.read_to_end(reply.size as usize + 1))
        .await
        .map_err(|_| anyhow!("download timed out"))??;
    if bytes.len() as u64 != reply.size {
        bail!("download cut short");
    }
    Ok((reply.mime, bytes))
}

/// Upload an image; returns its id on the server.
pub async fn file_put(
    conn: &quinn::Connection,
    kind: pyr_proto::control::FileKind,
    name: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<String> {
    use pyr_proto::control::{FILE_PREAMBLE, FileReply, FileRequest};
    let (mut send, mut recv) = conn.open_bi().await?;
    send.write_all(FILE_PREAMBLE).await?;
    let req = FileRequest::Put { kind, name: name.to_string(), mime: mime.to_string(), size: bytes.len() as u64 };
    send.write_all(&framing::encode(&req)).await?;
    send.write_all(bytes).await?;
    send.finish()?;
    let reply: FileReply = tokio::time::timeout(Duration::from_secs(300), read_frame(&mut recv))
        .await
        .map_err(|_| anyhow!("upload timed out"))??
        .ok_or_else(|| anyhow!("the server didn't answer the upload"))?;
    if !reply.ok {
        bail!("{}", reply.error);
    }
    Ok(reply.id)
}
