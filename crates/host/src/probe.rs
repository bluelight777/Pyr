//! "Can friends reach it?" — a QUIC handshake with a server, checked against the
//! fingerprint in its invite link, without logging in.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use pyr_proto::invite::InviteLink;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};

#[derive(Debug)]
struct Pinned {
    link: InviteLink,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if self.link.matches(&pyr_proto::crypto::fingerprint(end_entity.as_ref())) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General("the server's certificate doesn't match the invite link".into()))
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

/// Connects to the server in `link` and completes a QUIC handshake. Errors if nothing
/// answers within `timeout` (blocked UDP port, server down) or if a different server answers.
pub async fn reachable(link: &str, timeout: Duration) -> Result<()> {
    let link = InviteLink::parse(link).map_err(|e| anyhow!("{e}"))?;
    let addr: SocketAddr = tokio::net::lookup_host((link.host.as_str(), link.port))
        .await
        .with_context(|| format!("couldn't find {}", link.host))?
        .find(|a| a.is_ipv4())
        .ok_or_else(|| anyhow!("no IPv4 address for {}", link.host))?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned { link: link.clone(), provider }))
        .with_no_client_auth();
    tls.alpn_protocols = vec![pyr_proto::ALPN.to_vec()];
    let crypto = quinn::crypto::rustls::QuicClientConfig::try_from(tls)?;
    let config = quinn::ClientConfig::new(Arc::new(crypto));
    let (endpoint, _) = pyr_proto::udp::bind_endpoint("0.0.0.0:0".parse()?, None).context("opening a UDP socket")?;
    let connecting = endpoint.connect_with(config, addr, pyr_proto::APP_ID)?;
    let result = match tokio::time::timeout(timeout, connecting).await {
        Err(_) => Err(anyhow!("no answer from {} on UDP port {}", link.host, link.port)),
        Ok(Err(e)) => {
            let s = e.to_string();
            if s.contains("doesn't match") {
                Err(anyhow!("a different server answered at {}:{}", link.host, link.port))
            } else {
                Err(anyhow!("couldn't connect to {}:{}: {s}", link.host, link.port))
            }
        }
        Ok(Ok(conn)) => {
            conn.close(0u32.into(), b"just checking");
            Ok(())
        }
    };
    endpoint.close(0u32.into(), b"");
    let _ = tokio::time::timeout(Duration::from_secs(1), endpoint.wait_idle()).await;
    result
}

/// [`reachable`], trying a few times (a server that just started may need a moment).
pub fn reachable_blocking(link: &str, attempts: u32, timeout: Duration) -> Result<()> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let mut last = None;
    for i in 0..attempts.max(1) {
        if i > 0 {
            std::thread::sleep(Duration::from_secs(2));
        }
        match rt.block_on(reachable(link, timeout)) {
            Ok(()) => return Ok(()),
            Err(e) => last = Some(e),
        }
    }
    match last {
        Some(e) => Err(e),
        None => bail!("not checked"),
    }
}
