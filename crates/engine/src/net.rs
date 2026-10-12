//! QUIC client: certificate pinning, connecting and logging in.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use ingle_proto::control::*;
use ingle_proto::crypto::{self, Identity};
use ingle_proto::{PROTOCOL_VERSION, framing};
use quinn::VarInt;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

/// Accepts exactly one certificate: the one whose SHA-256 matches the invite link.
#[derive(Debug)]
struct PinnedVerifier {
    fingerprint: [u8; 32],
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
        if crypto::fingerprint(end_entity.as_ref()) == self.fingerprint {
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
    t.max_concurrent_uni_streams(VarInt::from_u32(2048));
    t.max_concurrent_bidi_streams(VarInt::from_u32(16));
    t.keep_alive_interval(Some(Duration::from_secs(4)));
    t.max_idle_timeout(Some(Duration::from_secs(20).try_into().expect("valid idle timeout")));
    t.datagram_receive_buffer_size(Some(1 << 20));
    t.datagram_send_buffer_size(64 * 1024);
    t.stream_receive_window(VarInt::from_u32(8 << 20));
    t.receive_window(VarInt::from_u32(64 << 20));
    t.send_window(16 << 20);
    t
}

pub fn client_config(fingerprint: [u8; 32]) -> Result<quinn::ClientConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedVerifier { fingerprint, provider }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![ingle_proto::ALPN.to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(crypto));
    cfg.transport_config(Arc::new(transport_config()));
    Ok(cfg)
}

/// One client endpoint per address family, created lazily and shared by all servers.
#[derive(Default)]
pub struct Endpoints {
    v4: Option<quinn::Endpoint>,
    v6: Option<quinn::Endpoint>,
}

impl Endpoints {
    pub fn for_addr(&mut self, addr: SocketAddr) -> Result<quinn::Endpoint> {
        let slot = if addr.is_ipv4() { &mut self.v4 } else { &mut self.v6 };
        if let Some(e) = slot {
            return Ok(e.clone());
        }
        let bind: SocketAddr = if addr.is_ipv4() { "0.0.0.0:0".parse()? } else { "[::]:0".parse()? };
        let (e, fallback) = ingle_proto::udp::bind_endpoint(bind, None).context("opening UDP socket")?;
        if let Some(why) = fallback {
            tracing::warn!("the network stack refused QUIC socket options ({why}); using a plain UDP socket");
        }
        *slot = Some(e.clone());
        Ok(e)
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
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(_)) => return Ok(None),
        Err(e) => return Err(e.into()),
    }
    let n = framing::body_len(len)?;
    let mut body = vec![0u8; n];
    recv.read_exact(&mut body).await?;
    Ok(Some(framing::decode(&body)?))
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
    let connecting = endpoint.connect_with(client_config(fingerprint)?, addr, ingle_proto::APP_ID)?;
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
    let (mut send, mut recv) = conn.open_bi().await?;
    // QUIC only announces a stream once data is sent on it.
    send.write_all(ingle_proto::CONTROL_PREAMBLE).await?;

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
            client: format!("{} {}", ingle_proto::APP_NAME, env!("CARGO_PKG_VERSION")),
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
