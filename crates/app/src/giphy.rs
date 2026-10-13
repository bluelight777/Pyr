//! GIF search, like Discord's `/giphy`: type `/giphy cats` in a message box to search
//! GIPHY, pick one (click, or arrows and Enter) and it's posted. GIFs from GIPHY in
//! messages play right in the chat.
//!
//! Searches go straight from this PC to GIPHY (GIPHY doesn't allow passing them through a
//! server) with an API key: built in at release time, or your own pasted into the GIF panel.
//! Only GIPHY's own addresses are ever fetched automatically. Results are cached for a while,
//! since a free key gets 100 calls an hour for everyone using it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Deserialize;

/// Built into release builds (`INGLE_GIPHY_KEY=... scripts/build-*.sh`).
const BUILT_IN_KEY: Option<&str> = option_env!("INGLE_GIPHY_KEY");
const API: &str = "https://api.giphy.com/v1/gifs";
/// Biggest GIF downloaded for showing in chat.
const MAX_GIF_BYTES: usize = 12 << 20;
const RESULTS: usize = 24;

pub fn key(settings_key: &str) -> Option<String> {
    let k = settings_key.trim();
    if !k.is_empty() {
        return Some(k.to_string());
    }
    std::env::var("INGLE_GIPHY_KEY").ok().filter(|k| !k.is_empty()).or_else(|| BUILT_IN_KEY.filter(|k| !k.is_empty()).map(String::from))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gif {
    pub id: String,
    pub title: String,
    /// Small moving preview for the grid.
    pub preview: String,
    pub preview_w: f32,
    pub preview_h: f32,
    /// What gets posted.
    pub url: String,
}

#[derive(Clone, Debug)]
pub enum Search {
    Loading,
    Ready(Vec<Gif>),
    Failed(String),
}

#[derive(Clone)]
pub enum Dl {
    Loading,
    Ready(Arc<Vec<u8>>),
    Failed,
}

/// How long typing must pause before a search starts (each search uses up some of the key's
/// allowance: a free GIPHY key gets 100 calls an hour, shared by everyone using it).
const PAUSE: Duration = Duration::from_millis(600);
/// Results already fetched are reused for this long (trending, and the same words again).
const CACHE_FOR: Duration = Duration::from_secs(30 * 60);
const CACHE_MAX: usize = 200;

#[derive(Default)]
pub struct GiphyUi {
    /// The query typed, and when it last changed (searches wait for a pause in typing).
    typed: String,
    typed_at: Option<Instant>,
    /// The query being shown (the last one searched).
    pub searched: Option<String>,
    /// The latest search: the query it's for, and how it went.
    results: Arc<Mutex<Option<(String, Search)>>>,
    /// Results already fetched, by query ("" is trending).
    cache: Arc<Mutex<HashMap<String, (Instant, Vec<Gif>)>>>,
    pub sel: usize,
}

/// Words as searched: case and spacing don't make a different search.
pub fn normalize(query: &str) -> String {
    query.split_whitespace().collect::<Vec<_>>().join(" ").to_lowercase()
}

impl GiphyUi {
    /// Call every frame while `/giphy …` is in the box; starts searches as needed.
    pub fn update(&mut self, query: &str, api_key: &str, repaint: impl Fn() + Send + 'static) {
        let q = normalize(query);
        if q != self.typed {
            self.typed = q.clone();
            self.typed_at = Some(Instant::now());
            self.sel = 0;
        }
        if self.waiting() || self.searched.as_deref() == Some(q.as_str()) {
            return;
        }
        self.searched = Some(q.clone());
        if let Some(v) = self.cached(&q) {
            *self.results.lock() = Some((q, Search::Ready(v)));
            return;
        }
        *self.results.lock() = Some((q.clone(), Search::Loading));
        let (slot, cache, key) = (self.results.clone(), self.cache.clone(), api_key.to_string());
        let _ = std::thread::Builder::new().name("ingle-giphy".into()).spawn(move || {
            let r = search(&key, &q);
            if let Ok(v) = &r {
                remember(&mut cache.lock(), &q, v.clone());
            }
            let mut s = slot.lock();
            // Only if it's still the latest search.
            if s.as_ref().is_some_and(|(sq, _)| *sq == q) {
                *s = Some((
                    q,
                    match r {
                        Ok(v) => Search::Ready(v),
                        Err(e) => Search::Failed(e),
                    },
                ));
            }
            drop(s);
            repaint();
        });
    }

    fn cached(&self, q: &str) -> Option<Vec<Gif>> {
        self.cache.lock().get(q).filter(|(t, _)| t.elapsed() < CACHE_FOR).map(|(_, v)| v.clone())
    }

    /// How the search for what's typed now is going (None while typing).
    pub fn current(&self) -> Option<Search> {
        if self.waiting() {
            return None;
        }
        match (&*self.results.lock(), &self.searched) {
            (Some((q, r)), Some(sq)) if q == sq => Some(r.clone()),
            _ => None,
        }
    }

    pub fn waiting(&self) -> bool {
        self.typed_at.is_some_and(|t| t.elapsed() < PAUSE)
    }

    /// The box no longer holds `/giphy` (results already fetched are kept for next time).
    pub fn reset(&mut self) {
        self.typed.clear();
        self.typed_at = None;
        self.searched = None;
        *self.results.lock() = None;
        self.sel = 0;
    }
}

/// Keeps `v` as the results for `q`, making room by dropping expired and then the oldest.
fn remember(cache: &mut HashMap<String, (Instant, Vec<Gif>)>, q: &str, v: Vec<Gif>) {
    if cache.len() >= CACHE_MAX {
        cache.retain(|_, (t, _)| t.elapsed() < CACHE_FOR);
    }
    if cache.len() >= CACHE_MAX {
        if let Some(oldest) = cache.iter().min_by_key(|(_, (t, _))| *t).map(|(k, _)| k.clone()) {
            cache.remove(&oldest);
        }
    }
    cache.insert(q.to_string(), (Instant::now(), v));
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into()
}

#[derive(Deserialize)]
struct Resp {
    data: Vec<RawGif>,
}
#[derive(Deserialize)]
struct RawGif {
    id: String,
    #[serde(default)]
    title: String,
    images: HashMap<String, Rendition>,
}
#[derive(Deserialize, Clone)]
struct Rendition {
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    width: Option<String>,
    #[serde(default)]
    height: Option<String>,
}

/// Search GIPHY (or what's trending, for an empty query).
pub fn search(key: &str, q: &str) -> Result<Vec<Gif>, String> {
    let url = if q.is_empty() {
        format!("{API}/trending?api_key={}&limit={RESULTS}&rating=pg-13", enc(key))
    } else {
        format!("{API}/search?api_key={}&q={}&limit={RESULTS}&rating=pg-13&lang=en", enc(key), enc(q))
    };
    let mut resp = agent().get(&url).call().map_err(|e| match e {
        ureq::Error::StatusCode(401 | 403) => "GIPHY didn't accept the GIF search key.".to_string(),
        ureq::Error::StatusCode(429) => {
            "GIF searches are used up for this hour (GIPHY limits how many one key gets). Try again a bit later.".to_string()
        }
        e => format!("Couldn't reach GIPHY: {e}"),
    })?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    parse(&text)
}

pub fn parse(text: &str) -> Result<Vec<Gif>, String> {
    let r: Resp = serde_json::from_str(text).map_err(|e| format!("GIPHY sent something unexpected: {e}"))?;
    Ok(r.data
        .into_iter()
        .filter_map(|g| {
            let pick = |names: &[&str]| names.iter().find_map(|n| g.images.get(*n).filter(|r| r.url.is_some()).cloned());
            let preview = pick(&["fixed_width_downsampled", "fixed_width", "fixed_width_small", "downsized"])?;
            let full = pick(&["downsized", "downsized_medium", "fixed_height", "original"])?;
            let num = |s: &Option<String>| s.as_deref().and_then(|v| v.parse::<f32>().ok()).unwrap_or(200.0);
            Some(Gif {
                id: g.id.clone(),
                title: g.title.clone(),
                preview: clean(preview.url.as_deref()?),
                preview_w: num(&preview.width),
                preview_h: num(&preview.height),
                url: clean(full.url.as_deref()?),
            })
        })
        .collect())
}

/// GIPHY links without their tracking query.
fn clean(url: &str) -> String {
    url.split('?').next().unwrap_or(url).to_string()
}

fn enc(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// A GIF address from GIPHY (the only kind shown in chat by itself).
pub fn is_giphy_media(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split('/').next().unwrap_or("");
    let path = rest[host.len()..].split('?').next().unwrap_or("");
    let giphy_host = host == "i.giphy.com" || (host.ends_with(".giphy.com") && host.starts_with("media"));
    giphy_host && (path.ends_with(".gif") || path.ends_with(".webp"))
}

/// GIPHY GIF links in a message.
pub fn gif_links(text: &str) -> Vec<String> {
    text.split_whitespace().filter(|w| is_giphy_media(w)).map(|w| w.to_string()).take(4).collect()
}

/// Downloads of GIFs shown in chat and in the search grid.
#[derive(Clone, Default)]
pub struct Downloads {
    map: Arc<Mutex<HashMap<String, Dl>>>,
    running: Arc<std::sync::atomic::AtomicUsize>,
}

impl Downloads {
    pub fn get(&self, url: &str, repaint: impl Fn() + Send + 'static) -> Dl {
        if !is_giphy_media(url) {
            return Dl::Failed;
        }
        let mut m = self.map.lock();
        if let Some(d) = m.get(url) {
            return d.clone();
        }
        // A few at a time.
        if self.running.load(std::sync::atomic::Ordering::Relaxed) >= 6 {
            return Dl::Loading;
        }
        m.insert(url.to_string(), Dl::Loading);
        drop(m);
        self.running.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (map, running, url) = (self.map.clone(), self.running.clone(), url.to_string());
        let _ = std::thread::Builder::new().name("ingle-gif".into()).spawn(move || {
            let r = fetch(&url);
            map.lock().insert(url, match r {
                Some(b) => Dl::Ready(Arc::new(b)),
                None => Dl::Failed,
            });
            running.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
            repaint();
        });
        Dl::Loading
    }
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut resp = agent().get(url).call().ok()?;
    let mut out = Vec::new();
    resp.body_mut().as_reader().take(MAX_GIF_BYTES as u64 + 1).read_to_end(&mut out).ok()?;
    (out.len() <= MAX_GIF_BYTES).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_giphy_results() {
        let text = r#"{"data":[{"id":"abc123","title":"Cat GIF","images":{
            "fixed_width_downsampled":{"url":"https://media2.giphy.com/media/abc123/200w_d.gif?cid=x&rid=y","width":"200","height":"150"},
            "downsized":{"url":"https://media2.giphy.com/media/abc123/giphy-downsized.gif?cid=x","width":"480","height":"360"}}},
            {"id":"nope","images":{}}],"pagination":{},"meta":{}}"#;
        let v = parse(text).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].preview, "https://media2.giphy.com/media/abc123/200w_d.gif");
        assert_eq!(v[0].url, "https://media2.giphy.com/media/abc123/giphy-downsized.gif");
        assert_eq!(v[0].preview_h, 150.0);
    }

    fn gif(id: &str) -> Gif {
        Gif { id: id.into(), title: String::new(), preview: String::new(), preview_w: 1.0, preview_h: 1.0, url: String::new() }
    }

    #[test]
    fn searches_are_reused_and_never_shown_for_other_words() {
        assert_eq!(normalize("  Cute   CATS "), "cute cats");
        let mut g = GiphyUi::default();
        g.cache.lock().insert("dogs".into(), (Instant::now(), vec![gif("d1")]));
        // Still typing: nothing yet.
        g.update("Dogs", "key", || {});
        assert!(g.current().is_none());
        // After the pause, the cached results come straight back (no call to GIPHY).
        g.typed_at = Some(Instant::now() - PAUSE * 2);
        g.update("Dogs", "key", || {});
        assert!(matches!(g.current(), Some(Search::Ready(v)) if v[0].id == "d1"));
        // Results for other words (a slow earlier search finishing late) are never shown.
        *g.results.lock() = Some(("cats".into(), Search::Ready(vec![gif("c1")])));
        assert!(g.current().is_none());
        // Old results expire.
        g.cache.lock().insert("old".into(), (Instant::now() - CACHE_FOR * 2, vec![gif("o1")]));
        assert!(g.cached("old").is_none());
    }

    #[test]
    fn the_cache_stays_small() {
        let mut c = HashMap::new();
        for i in 0..CACHE_MAX {
            c.insert(format!("q{i}"), (Instant::now() - Duration::from_secs(1000 - i as u64), vec![gif("x")]));
        }
        remember(&mut c, "new", vec![gif("n")]);
        assert_eq!(c.len(), CACHE_MAX);
        assert!(c.contains_key("new") && !c.contains_key("q0"));
    }

    #[test]
    fn only_giphy_media_is_fetched() {
        assert!(is_giphy_media("https://media2.giphy.com/media/abc/giphy.gif"));
        assert!(is_giphy_media("https://i.giphy.com/abc.webp"));
        assert!(!is_giphy_media("https://evil.com/media.giphy.com/x.gif"));
        assert!(!is_giphy_media("https://media.giphy.com.evil.com/x.gif"));
        assert!(!is_giphy_media("http://media.giphy.com/x.gif"));
        assert!(!is_giphy_media("https://giphy.com/gifs/abc"));
        assert_eq!(gif_links("look https://media.giphy.com/media/a/giphy.gif lol").len(), 1);
        assert_eq!(enc("cat & dog"), "cat+%26+dog");
    }
}
