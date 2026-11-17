//! Reading the props feed (see [`pyr_proto::props_feed`]) while the casino's own props are on
//! here: every couple of minutes the file at pyr.gg is fetched (only when it changed: its
//! ETag), checked against the release key's certificate, and followed
//! (`Poker::props_follow`): what's new there opens here, what came out is settled, with nobody
//! deciding. A server whose owner never turned them on reads nothing.
//!
//! The same feed is followed again each time whether it changed or not: what's wanted here
//! may have (a category turned on), and following is done once per prop either way.

use std::sync::{Arc, Weak};
use std::time::Duration;

use pyr_proto::props_feed::{self, Feed};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::hub::Hub;
use crate::poker::props::FeedSeen;

/// How often the feed is read.
const PERIOD: Duration = Duration::from_secs(120);
/// The biggest feed taken (its props are a few hundred bytes each).
const MAX_BYTES: u64 = 8 << 20;

/// The server's line to the feed's reader (while it runs; see [`start`]).
pub struct Follower {
    tx: mpsc::UnboundedSender<()>,
}

impl Follower {
    /// Something changed here (the casino's own props turned on, a category): the feed is
    /// read now rather than in a couple of minutes.
    pub fn nudge(&self) {
        let _ = self.tx.send(());
    }
}

/// Where the feed is: pyr.gg's, unless `PYR_PROPS_FEED` says another (for testing), or `off`.
pub fn feed_url() -> Option<String> {
    match std::env::var("PYR_PROPS_FEED") {
        Ok(v) if v.trim() == "off" => None,
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => Some(props_feed::FEED_URL.to_string()),
    }
}

/// The key the feed's certificate has to be signed with: the release key, unless
/// `PYR_PROPS_FEED_KEY` says another (for testing: a feed made here).
fn release_key() -> String {
    std::env::var("PYR_PROPS_FEED_KEY").ok().map(|k| k.trim().to_string()).filter(|k| pyr_proto::crypto::is_valid_user_id(k)).unwrap_or_else(|| pyr_proto::release::RELEASE_KEY.to_string())
}

/// Fetches the feed (blocking): its text and its ETag, or None when it's the one with `etag`
/// still.
fn fetch(agent: &ureq::Agent, url: &str, etag: Option<&str>) -> Result<Option<(String, Option<String>)>, String> {
    let mut req = agent.get(url);
    if let Some(e) = etag {
        req = req.header("If-None-Match", e);
    }
    let mut resp = req.call().map_err(|e| format!("couldn't reach the props feed: {e}"))?;
    match resp.status().as_u16() {
        304 => Ok(None),
        200 => {
            let tag = resp.headers().get("etag").and_then(|v| v.to_str().ok()).map(|s| s.to_string());
            let text = resp.body_mut().with_config().limit(MAX_BYTES).read_to_string().map_err(|e| format!("the props feed came oddly: {e}"))?;
            Ok(Some((text, tag)))
        }
        other => Err(format!("the props feed answered HTTP {other}")),
    }
}

/// Starts reading the feed at `url` for `hub` (only while the casino's own props are on
/// there, or some from before still wait for their results).
pub fn start(hub: &Arc<Hub>, url: String) -> Follower {
    let (tx, mut rx) = mpsc::unbounded_channel::<()>();
    let weak: Weak<Hub> = Arc::downgrade(hub);
    tokio::spawn(async move {
        let agent = crate::directory::agent(&url);
        let key = release_key();
        // The feed as last read, and its ETag.
        let mut last: Option<Feed> = None;
        let mut etag: Option<String> = None;
        // The last trouble told of (the same again goes to the debug log only).
        let mut said: Option<String> = None;
        let mut wait = Duration::from_secs(8);
        loop {
            tokio::select! {
                m = rx.recv() => {
                    if m.is_none() {
                        return;
                    }
                    while rx.try_recv().is_ok() {}
                }
                _ = tokio::time::sleep(wait) => {}
            }
            wait = PERIOD;
            let Some(hub) = weak.upgrade() else { return };
            if !hub.poker.props_wants_feed(&hub) {
                continue;
            }
            let (a, u, e) = (agent.clone(), url.clone(), etag.clone());
            let got = tokio::task::spawn_blocking(move || fetch(&a, &u, e.as_deref())).await.unwrap_or_else(|e| Err(e.to_string()));
            let trouble = match got {
                Ok(None) => None,
                Ok(Some((text, tag))) => match props_feed::open(&text, &key) {
                    // (Never back to an older one: someone holding the website couldn't hide
                    // results by serving a feed from before.)
                    Ok(feed) if last.as_ref().is_some_and(|l| feed.made < l.made) => Some("the props feed went back in time".to_string()),
                    Ok(feed) => {
                        etag = tag;
                        last = Some(feed);
                        None
                    }
                    Err(e) => Some(e),
                },
                Err(e) => Some(e),
            };
            match &trouble {
                Some(e) if said.as_ref() != Some(e) => warn!("{e}"),
                Some(e) => debug!("{e}"),
                None if said.is_some() => info!("the props feed reads again"),
                None => {}
            }
            said = trouble.clone();
            if let Some(feed) = &last {
                // (The casino's lock and the database: not on the runtime's own threads.)
                let (h, f) = (hub.clone(), feed.clone());
                let _ = tokio::task::spawn_blocking(move || h.poker.props_follow(&h, &f)).await;
            }
            hub.poker.props_feed_seen(FeedSeen { at: last.as_ref().map_or(0, |f| f.made), props: last.as_ref().map_or(0, |f| f.props.len() as u32), error: trouble.unwrap_or_default() });
        }
    });
    Follower { tx }
}
