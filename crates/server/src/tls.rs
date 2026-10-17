//! Self-signed TLS identity for the server. Clients pin its SHA-256 fingerprint
//! (it travels inside invite links), so no certificate authority is involved.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use quinn::VarInt;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

pub struct TlsIdentity {
    pub cert_der: Vec<u8>,
    pub key_der: Vec<u8>,
    pub fingerprint: [u8; 32],
}

pub fn load_or_create(dir: &Path) -> Result<TlsIdentity> {
    let cert_path = dir.join("server-cert.der");
    let key_path = dir.join("server-key.der");
    let (cert_der, key_der) = if cert_path.exists() && key_path.exists() {
        (std::fs::read(&cert_path)?, std::fs::read(&key_path)?)
    } else {
        let ck = rcgen::generate_simple_self_signed(vec![ingle_proto::APP_ID.to_string(), "localhost".to_string()])
            .context("generating TLS certificate")?;
        let cert = ck.cert.der().to_vec();
        let key = ck.signing_key.serialize_der();
        std::fs::write(&cert_path, &cert)?;
        write_private(&key_path, &key)?;
        (cert, key)
    };
    let fingerprint = ingle_proto::crypto::fingerprint(&cert_der);
    Ok(TlsIdentity { cert_der, key_der, fingerprint })
}

fn write_private(path: &Path, data: &[u8]) -> Result<()> {
    std::fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub fn transport_config() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    // Video frames each use a stream; a few dozen in flight is plenty (see conn::uni_loop).
    t.max_concurrent_uni_streams(VarInt::from_u32(64));
    t.max_concurrent_bidi_streams(VarInt::from_u32(16));
    t.keep_alive_interval(Some(Duration::from_secs(4)));
    t.max_idle_timeout(Some(Duration::from_secs(20).try_into().expect("valid idle timeout")));
    t.datagram_receive_buffer_size(Some(1 << 20));
    // Keep the outgoing datagram queue short: stale audio is worse than lost audio. A
    // fraction of a second of a few people talking at once (`INGLE_AB=dgram`: 128 KB, seconds).
    t.datagram_send_buffer_size(if ingle_proto::ab::old("dgram") { 128 * 1024 } else { 16 * 1024 });
    t.stream_receive_window(VarInt::from_u32(8 << 20));
    t.receive_window(VarInt::from_u32(64 << 20));
    t.send_window(32 << 20);
    if ingle_proto::ab::bbr() {
        t.congestion_controller_factory(Arc::new(quinn::congestion::BbrConfig::default()));
    }
    t
}

pub fn server_config(id: &TlsIdentity) -> Result<quinn::ServerConfig> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut tls = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(id.cert_der.clone())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(id.key_der.clone())),
        )?;
    tls.alpn_protocols = vec![ingle_proto::ALPN.to_vec()];
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)?;
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    cfg.transport_config(Arc::new(transport_config()));
    Ok(cfg)
}
