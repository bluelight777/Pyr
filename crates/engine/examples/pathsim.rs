//! A network path you can make slow, far away and lossy, for trying calls the way someone on
//! the other side of the world has them: a UDP relay between one app and the server that adds,
//! in each direction, delay, jitter, loss and a bottleneck (a speed limit with a buffer, like a
//! home internet connection's upload).
//!
//!   cargo run -p pyr-engine --example pathsim -- --listen 127.0.0.1:7801 --to 127.0.0.1:7800 \
//!       --up delay=100,jitter=4,loss=0.2,rate=5000,buffer=300 \
//!       --down delay=100,jitter=4,loss=0.2,rate=40000,buffer=100 \
//!       --at 40 up.rate=2000 --log path.log
//!
//! `up` is from the app to the server, `down` back. delay and jitter are in ms (jitter is
//! added on top of the delay, 0..jitter, without reordering); loss is in percent; rate is in
//! kbit/s (0: no limit); buffer is how much the bottleneck holds, in ms at its rate (packets
//! that don't fit are dropped, like a router's). `--at SECONDS dir.key=value` changes a setting
//! that far in. `--log FILE` writes a line a second: packets, kbit/s, drops and the deepest
//! queue in each direction.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tokio::time::Instant;

#[derive(Clone, Copy, Debug)]
struct Params {
    delay_ms: f64,
    jitter_ms: f64,
    loss_pct: f64,
    rate_kbps: f64,
    buffer_ms: f64,
}

impl Default for Params {
    fn default() -> Self {
        Params { delay_ms: 0.0, jitter_ms: 0.0, loss_pct: 0.0, rate_kbps: 0.0, buffer_ms: 200.0 }
    }
}

impl Params {
    fn set(&mut self, key: &str, value: f64) -> anyhow::Result<()> {
        match key {
            "delay" => self.delay_ms = value,
            "jitter" => self.jitter_ms = value,
            "loss" => self.loss_pct = value,
            "rate" => self.rate_kbps = value,
            "buffer" => self.buffer_ms = value,
            _ => anyhow::bail!("unknown setting {key} (delay, jitter, loss, rate, buffer)"),
        }
        Ok(())
    }

    fn parse(spec: &str) -> anyhow::Result<Params> {
        let mut p = Params::default();
        for kv in spec.split(',').filter(|s| !s.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| anyhow::anyhow!("expected key=value, got {kv}"))?;
            p.set(k.trim(), v.trim().parse()?)?;
        }
        Ok(p)
    }
}

#[derive(Default, Clone, Copy)]
struct Counts {
    packets: u64,
    bytes: u64,
    dropped_queue: u64,
    dropped_loss: u64,
    max_queue_ms: f64,
}

/// One direction of the path.
struct Link {
    params: Params,
    /// When the bottleneck finishes sending what it already holds.
    busy_until: Instant,
    /// When the last packet comes out (packets stay in order).
    last_delivery: Instant,
    rng: u64,
    counts: Counts,
}

impl Link {
    fn new(params: Params, seed: u64) -> Link {
        let now = Instant::now();
        Link { params, busy_until: now, last_delivery: now, rng: seed | 1, counts: Counts::default() }
    }

    fn random(&mut self) -> f64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 11) as f64 / (1u64 << 53) as f64
    }

    /// When a packet of `len` bytes arriving now comes out the other end, or None if it's lost.
    fn admit(&mut self, len: usize, now: Instant) -> Option<Instant> {
        let p = self.params;
        let mut depart = now;
        if p.rate_kbps > 0.0 {
            let start = self.busy_until.max(now);
            let queued_ms = start.duration_since(now).as_secs_f64() * 1000.0;
            if queued_ms > p.buffer_ms {
                self.counts.dropped_queue += 1;
                return None;
            }
            self.counts.max_queue_ms = self.counts.max_queue_ms.max(queued_ms);
            let tx = Duration::from_secs_f64(len as f64 * 8.0 / (p.rate_kbps * 1000.0));
            self.busy_until = start + tx;
            depart = self.busy_until;
        }
        if p.loss_pct > 0.0 && self.random() * 100.0 < p.loss_pct {
            self.counts.dropped_loss += 1;
            return None;
        }
        let jitter = if p.jitter_ms > 0.0 { self.random() * p.jitter_ms } else { 0.0 };
        let at = (depart + Duration::from_secs_f64((p.delay_ms + jitter) / 1000.0)).max(self.last_delivery);
        self.last_delivery = at;
        self.counts.packets += 1;
        self.counts.bytes += len as u64;
        Some(at)
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let mut listen: SocketAddr = "127.0.0.1:7801".parse()?;
    let mut to: SocketAddr = "127.0.0.1:7800".parse()?;
    let mut up = Params::default();
    let mut down = Params::default();
    let mut changes: Vec<(f64, String)> = Vec::new();
    let mut log: Option<String> = None;
    let mut seed = 0x5EED_1234_u64;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| anyhow::anyhow!("{a} needs a value"));
        match a.as_str() {
            "--listen" => listen = val()?.parse()?,
            "--to" => to = val()?.parse()?,
            "--up" => up = Params::parse(&val()?)?,
            "--down" => down = Params::parse(&val()?)?,
            "--at" => {
                let secs: f64 = val()?.parse()?;
                changes.push((secs, val()?));
            }
            "--log" => log = Some(val()?),
            "--seed" => seed = val()?.parse()?,
            other => anyhow::bail!("unknown option {other}"),
        }
    }
    let client_side = Arc::new(UdpSocket::bind(listen).await?);
    let server_side = Arc::new(UdpSocket::bind(if to.is_ipv4() { "127.0.0.1:0" } else { "[::1]:0" }).await?);
    server_side.connect(to).await?;
    let links = Arc::new(Mutex::new((Link::new(up, seed), Link::new(down, seed.rotate_left(17) ^ 0xABCD))));
    let client: Arc<Mutex<Option<SocketAddr>>> = Arc::default();
    println!("pathsim: {listen} -> {to}  up {up:?}  down {down:?}");

    // Each direction: packets wait in order until it's their time.
    let (up_tx, mut up_rx) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    let (down_tx, mut down_rx) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    {
        let s = server_side.clone();
        tokio::spawn(async move {
            while let Some((at, pkt)) = up_rx.recv().await {
                tokio::time::sleep_until(at).await;
                let _ = s.send(&pkt).await;
            }
        });
    }
    {
        let (s, client) = (client_side.clone(), client.clone());
        tokio::spawn(async move {
            while let Some((at, pkt)) = down_rx.recv().await {
                tokio::time::sleep_until(at).await;
                let to = *client.lock();
                if let Some(to) = to {
                    let _ = s.send_to(&pkt, to).await;
                }
            }
        });
    }
    // From the app.
    {
        let (s, links, client) = (client_side.clone(), links.clone(), client.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            while let Ok((n, from)) = s.recv_from(&mut buf).await {
                *client.lock() = Some(from);
                let at = links.lock().0.admit(n, Instant::now());
                if let Some(at) = at {
                    let _ = up_tx.send((at, buf[..n].to_vec()));
                }
            }
        });
    }
    // From the server.
    {
        let (s, links) = (server_side.clone(), links.clone());
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            while let Ok(n) = s.recv(&mut buf).await {
                let at = links.lock().1.admit(n, Instant::now());
                if let Some(at) = at {
                    let _ = down_tx.send((at, buf[..n].to_vec()));
                }
            }
        });
    }
    // Settings that change during the run.
    let started = Instant::now();
    for (secs, change) in changes {
        let links = links.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(started + Duration::from_secs_f64(secs)).await;
            let apply = || -> anyhow::Result<()> {
                for one in change.split(',') {
                    let (k, v) = one.split_once('=').ok_or_else(|| anyhow::anyhow!("expected dir.key=value"))?;
                    let (dir, key) = k.split_once('.').ok_or_else(|| anyhow::anyhow!("expected up.key or down.key"))?;
                    let mut l = links.lock();
                    let link = if dir == "up" { &mut l.0 } else { &mut l.1 };
                    link.params.set(key, v.parse()?)?;
                }
                Ok(())
            };
            match apply() {
                Ok(()) => println!("pathsim: at {secs} s: {change}"),
                Err(e) => eprintln!("pathsim: bad change {change}: {e}"),
            }
        });
    }
    // A line a second.
    let mut out = match &log {
        Some(p) => Some(std::fs::File::create(p)?),
        None => None,
    };
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    let mut last = (Counts::default(), Counts::default());
    loop {
        tick.tick().await;
        let (u, d) = {
            let mut l = links.lock();
            let c = (l.0.counts, l.1.counts);
            l.0.counts.max_queue_ms = 0.0;
            l.1.counts.max_queue_ms = 0.0;
            c
        };
        let line = format!(
            "t={:.0} up_pkts={} up_kbps={:.0} up_qdrop={} up_lossdrop={} up_maxq_ms={:.0} down_pkts={} down_kbps={:.0} down_qdrop={} down_lossdrop={} down_maxq_ms={:.0}",
            started.elapsed().as_secs_f64(),
            u.packets - last.0.packets,
            (u.bytes - last.0.bytes) as f64 * 8.0 / 1000.0,
            u.dropped_queue - last.0.dropped_queue,
            u.dropped_loss - last.0.dropped_loss,
            u.max_queue_ms,
            d.packets - last.1.packets,
            (d.bytes - last.1.bytes) as f64 * 8.0 / 1000.0,
            d.dropped_queue - last.1.dropped_queue,
            d.dropped_loss - last.1.dropped_loss,
            d.max_queue_ms,
        );
        last = (u, d);
        if let Some(f) = out.as_mut() {
            use std::io::Write;
            let _ = writeln!(f, "{line}");
        }
    }
}
