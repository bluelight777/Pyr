//! Ingle bug reports. Anyone's app sends a report straight to the bug server (no account),
//! and that server's owner reads and approves them in their own Ingle; approved ones get
//! fixed. The bug server is set when Ingle is built (`INGLE_BUG_SERVER=host:port/<server key>`)
//! and can be overridden with the same environment variable when running.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use ingle_proto::control::{BugEntry, BugReport, BugStatus, ClientFrame, Event, Request, ServerFrame};
use ingle_proto::{PROTOCOL_VERSION, framing};

use crate::Shared;

/// Where reports go: (host, port, server key).
pub fn server() -> Option<(String, u16, [u8; 32])> {
    let s = std::env::var("INGLE_BUG_SERVER").ok().or_else(|| option_env!("INGLE_BUG_SERVER").map(String::from))?;
    let (addr, key) = s.trim().split_once('/')?;
    let (host, port) = addr.rsplit_once(':')?;
    let fp = ingle_proto::b64::decode_array::<32>(key.trim())?;
    Some((host.to_string(), port.parse().ok()?, fp))
}

/// Whether this copy of Ingle can send bug reports.
pub fn can_report() -> bool {
    server().is_some()
}

/// The end of this app's log (for a report, if they agree).
pub fn log_tail(shared: &Shared, max: usize) -> String {
    let text = std::fs::read_to_string(shared.config_dir.join("ingle.log")).unwrap_or_default();
    let start = text.len().saturating_sub(max);
    let start = (start..text.len()).find(|i| text.is_char_boundary(*i)).unwrap_or(text.len());
    // From the start of a line.
    let tail = &text[start..];
    tail.find('\n').filter(|_| start > 0).map(|i| &tail[i + 1..]).unwrap_or(tail).to_string()
}

pub fn os_name() -> String {
    format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH)
}

/// Send a report to the bug server.
pub async fn send(shared: Arc<Shared>, report: BugReport) -> Result<()> {
    let (host, port, fp) = server().ok_or_else(|| anyhow!("This copy of Ingle has nowhere to send bug reports."))?;
    let addr = crate::net::resolve(&host, port).await?;
    let endpoint = shared.endpoints.lock().map_err(|_| anyhow!("network unavailable"))?.for_addr(addr)?;
    let connecting = endpoint.connect_with(crate::net::client_config(fp)?, addr, ingle_proto::APP_ID)?;
    let conn = tokio::time::timeout(Duration::from_secs(10), connecting)
        .await
        .map_err(|_| anyhow!("the bug report server didn't answer"))?
        .context("couldn't reach the bug report server")?;
    let r = async {
        let (mut send, mut recv) = conn.open_bi().await?;
        send.write_all(ingle_proto::CONTROL_PREAMBLE).await?;
        let hello: ServerFrame = tokio::time::timeout(Duration::from_secs(10), crate::net::read_frame(&mut recv))
            .await
            .map_err(|_| anyhow!("the bug report server didn't answer"))??
            .ok_or_else(|| anyhow!("the bug report server closed the connection"))?;
        let Event::Hello { proto, .. } = hello.ev else { bail!("unexpected greeting") };
        if proto != PROTOCOL_VERSION {
            bail!("the bug report server needs updating");
        }
        send.write_all(&framing::encode(&ClientFrame { id: 1, req: Request::ReportBug { report } })).await?;
        let reply: ServerFrame = tokio::time::timeout(Duration::from_secs(15), crate::net::read_frame(&mut recv))
            .await
            .map_err(|_| anyhow!("the bug report server didn't answer"))??
            .ok_or_else(|| anyhow!("the bug report server closed the connection"))?;
        match reply.ev {
            Event::Ok => Ok(()),
            Event::Error { message, .. } => bail!("{message}"),
            _ => bail!("unexpected reply"),
        }
    }
    .await;
    conn.close(quinn::VarInt::from_u32(0), b"");
    r
}

/// The owner's view of a server's reports (fetched with `Engine::list_bugs`).
pub fn sort(reports: &mut [BugEntry]) {
    reports.sort_by_key(|r| (r.status != BugStatus::New, std::cmp::Reverse(r.id)));
}
