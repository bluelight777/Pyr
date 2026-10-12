//! QUIC endpoints with a plain-UDP fallback.
//!
//! quinn's normal socket turns on ECN, packet-info and don't-fragment options. A few network
//! stacks refuse one of those (Wine does; so might an old Windows build or an unusual
//! driver). Instead of failing to start, we fall back to an ordinary UDP socket: QUIC works
//! the same, just without ECN marks and send/receive batching.

use std::fmt;
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, ready};

use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, Endpoint, EndpointConfig, Runtime, ServerConfig, TokioRuntime, UdpPoller};

/// Bind a QUIC endpoint (a server if `server` is given). Must be called inside a Tokio
/// runtime. The second value is set when the plain-UDP fallback had to be used, with the
/// reason the normal socket was refused.
pub fn bind_endpoint(addr: SocketAddr, server: Option<ServerConfig>) -> io::Result<(Endpoint, Option<io::Error>)> {
    let runtime: Arc<dyn Runtime> = Arc::new(TokioRuntime);
    let socket = std::net::UdpSocket::bind(addr)?;
    match Endpoint::new(EndpointConfig::default(), server.clone(), socket, runtime.clone()) {
        Ok(e) => Ok((e, None)),
        Err(why) if why.kind() != io::ErrorKind::AddrInUse => {
            // The failed attempt has already closed its socket, so the port is free again.
            let socket = std::net::UdpSocket::bind(addr)?;
            socket.set_nonblocking(true)?;
            let io = tokio::net::UdpSocket::from_std(socket)?;
            let e = Endpoint::new_with_abstract_socket(EndpointConfig::default(), server, Arc::new(PlainUdp { io }), runtime)?;
            Ok((e, Some(why)))
        }
        Err(e) => Err(e),
    }
}

struct PlainUdp {
    io: tokio::net::UdpSocket,
}

impl fmt::Debug for PlainUdp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PlainUdp").field("local", &self.io.local_addr().ok()).finish()
    }
}

impl AsyncUdpSocket for PlainUdp {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        Box::pin(PlainPoller(self))
    }

    fn try_send(&self, t: &Transmit) -> io::Result<()> {
        // max_transmit_segments() is 1, so every transmit is exactly one datagram.
        self.io.try_send_to(t.contents, t.destination).map(|_| ())
    }

    fn poll_recv(&self, cx: &mut Context, bufs: &mut [IoSliceMut<'_>], meta: &mut [RecvMeta]) -> Poll<io::Result<usize>> {
        let mut buf = tokio::io::ReadBuf::new(&mut bufs[0]);
        let addr = ready!(self.io.poll_recv_from(cx, &mut buf))?;
        let len = buf.filled().len();
        meta[0] = RecvMeta { addr, len, stride: len, ecn: None, dst_ip: None };
        Poll::Ready(Ok(1))
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.io.local_addr()
    }
}

#[derive(Debug)]
struct PlainPoller(Arc<PlainUdp>);

impl UdpPoller for PlainPoller {
    fn poll_writable(self: Pin<&mut Self>, cx: &mut Context) -> Poll<io::Result<()>> {
        self.0.io.poll_send_ready(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two endpoints on the fallback socket can still talk QUIC.
    #[tokio::test]
    async fn plain_socket_carries_quic() {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        let der = cert.cert.der().clone();
        let key = rustls::pki_types::PrivatePkcs8KeyDer::from(cert.signing_key.serialize_der());
        let server_cfg = ServerConfig::with_single_cert(vec![der.clone()], key.into()).unwrap();
        let runtime: Arc<dyn Runtime> = Arc::new(TokioRuntime);
        let mk = |addr: &str| {
            let s = std::net::UdpSocket::bind(addr).unwrap();
            s.set_nonblocking(true).unwrap();
            Arc::new(PlainUdp { io: tokio::net::UdpSocket::from_std(s).unwrap() })
        };
        let server =
            Endpoint::new_with_abstract_socket(EndpointConfig::default(), Some(server_cfg), mk("127.0.0.1:0"), runtime.clone())
                .unwrap();
        let mut roots = rustls::RootCertStore::empty();
        roots.add(der).unwrap();
        let client_cfg = quinn::ClientConfig::with_root_certificates(Arc::new(roots)).unwrap();
        let mut client = Endpoint::new_with_abstract_socket(EndpointConfig::default(), None, mk("127.0.0.1:0"), runtime).unwrap();
        client.set_default_client_config(client_cfg);
        let addr = server.local_addr().unwrap();
        let accept = tokio::spawn(async move {
            let conn = server.accept().await.unwrap().await.unwrap();
            let mut s = conn.accept_uni().await.unwrap();
            let got = s.read_to_end(1 << 20).await.unwrap();
            let d = conn.read_datagram().await.unwrap();
            (got, d)
        });
        let conn = client.connect(addr, "localhost").unwrap().await.unwrap();
        let mut s = conn.open_uni().await.unwrap();
        let big = vec![7u8; 300_000];
        s.write_all(&big).await.unwrap();
        s.finish().unwrap();
        conn.send_datagram(bytes::Bytes::from_static(b"ping")).unwrap();
        let (got, d) = tokio::time::timeout(std::time::Duration::from_secs(10), accept).await.unwrap().unwrap();
        assert_eq!(got, big);
        assert_eq!(&d[..], b"ping");
    }
}
