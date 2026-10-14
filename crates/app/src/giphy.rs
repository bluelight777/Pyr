//! GIF search, like Discord's `/giphy`: type `/giphy cats` in a message box to search
//! GIPHY, pick one (click, or arrows and Enter) and it's posted. GIFs from GIPHY (and from
//! KLIPY, see `klipy.rs`, which shares the picker in a tab of its own) play right in the chat.
//!
//! Searches go straight from this PC to GIPHY (GIPHY doesn't allow passing them through a
//! server) with an API key: built in at release time, or your own pasted into the GIF panel.
//! Only GIPHY's own addresses are ever fetched automatically. Results are cached for a while,
//! since a free key gets 100 calls an hour for everyone using it.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use parking_lot::Mutex;

const API: &str = "https://api.giphy.com/v1/gifs";
/// Biggest GIF downloaded for showing in chat.
const MAX_GIF_BYTES: usize = 12 << 20;
const RESULTS: usize = 24;

/// The key for GIF search on a server: the one its owner set up (`ServerInfo::giphy_key`).
/// (Development builds also take `INGLE_GIPHY_KEY`, for testing against a fake GIPHY.)
pub fn key(server_key: &str) -> Option<String> {
    let k = server_key.trim();
    if !k.is_empty() {
        return Some(k.to_string());
    }
    if cfg!(debug_assertions) {
        return std::env::var("INGLE_GIPHY_KEY").ok().filter(|k| !k.is_empty());
    }
    None
}

/// Where a GIF comes from. Each is searched on its own, in its own tab of the picker (KLIPY's
/// results are never mixed with anyone else's).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Provider {
    #[default]
    Giphy,
    Klipy,
}

impl Provider {
    pub fn name(self) -> &'static str {
        match self {
            Provider::Giphy => "GIPHY",
            Provider::Klipy => "KLIPY",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gif {
    pub provider: Provider,
    pub id: String,
    /// KLIPY's name for it (to tell KLIPY it was shared).
    pub slug: String,
    pub title: String,
    /// Small moving preview for the grid.
    pub preview: String,
    pub preview_w: f32,
    pub preview_h: f32,
    /// What gets posted.
    pub url: String,
    /// Its size, when GIPHY said (posted with it, see `posted`).
    pub size: Option<(u32, u32)>,
}

impl Gif {
    /// The link posted: with the GIF's size, so everyone's chat keeps its space before it
    /// loads (`?w=…&h=…`; GIPHY ignores it, and older Ingles show the GIF as before). KLIPY's
    /// links are used just as KLIPY gives them: there the size goes after a `#`, which is
    /// never part of what's fetched, with KLIPY's name for it (so whoever sends it again, say
    /// from their favorites, can tell KLIPY it was shared).
    pub fn posted(&self) -> String {
        match self.provider {
            Provider::Giphy => match self.size {
                Some((w, h)) => format!("{}?w={w}&h={h}", self.url),
                None => self.url.clone(),
            },
            Provider::Klipy => {
                let mut ours = Vec::new();
                if let Some((w, h)) = self.size {
                    ours.push(format!("w={w}&h={h}"));
                }
                if crate::klipy::path_safe(&self.slug) {
                    ours.push(format!("slug={}", self.slug));
                }
                if ours.is_empty() { self.url.clone() } else { format!("{}#{}", self.url, ours.join("&")) }
            }
        }
    }

    /// It as a favorite (from the picker, where its small preview and KLIPY name are known).
    pub fn favorite(&self) -> GifFavorite {
        GifFavorite {
            url: self.posted(),
            preview: self.preview.clone(),
            w: self.preview_w.round() as u16,
            h: self.preview_h.round() as u16,
            slug: self.slug.clone(),
            title: self.title.clone(),
        }
    }
}

pub use ingle_engine::config::GifFavorite;

/// Favorite GIFs kept at most (Discord kept 250 for a while; this is roomier).
pub const MAX_FAVORITES: usize = 500;

/// Whether the GIF at `url` is one of `favs` (whatever size its link says).
pub fn is_favorite(favs: &[GifFavorite], url: &str) -> bool {
    favs.iter().any(|f| clean_ref(&f.url) == clean_ref(url))
}

/// `favs` with `fav` put first, or taken out if it's there already (newest first, like
/// Discord's). Says why when there's no room for another.
pub fn toggle_favorite(favs: &[GifFavorite], fav: GifFavorite) -> Result<Vec<GifFavorite>, String> {
    if is_favorite(favs, &fav.url) {
        return Ok(favs.iter().filter(|f| clean_ref(&f.url) != clean_ref(&fav.url)).cloned().collect());
    }
    if favs.len() >= MAX_FAVORITES {
        return Err(format!("You have {MAX_FAVORITES} favorite GIFs, the most Ingle keeps. Take one out to add another."));
    }
    Ok(std::iter::once(fav).chain(favs.iter().cloned()).collect())
}

/// A GIF seen in chat, as a favorite: its link, GIPHY's smaller version of it for the picker
/// (KLIPY's links are only ever used as KLIPY gave them), its size (from its link, else as it
/// was shown) and KLIPY's name for it when its link says.
pub fn favorite_from_link(url: &str, shown: Option<egui::Vec2>) -> GifFavorite {
    let size = link_size(url).or(shown).unwrap_or(egui::vec2(0.0, 0.0));
    GifFavorite {
        url: url.to_string(),
        preview: giphy_preview(url).unwrap_or_default(),
        w: size.x.round().clamp(0.0, 4096.0) as u16,
        h: size.y.round().clamp(0.0, 4096.0) as u16,
        slug: link_slug(url).unwrap_or_default(),
        title: String::new(),
    }
}

/// GIPHY's 200-wide version of a GIF of theirs (every GIF's versions sit side by side in its
/// folder): small and quick for the picker's grid.
fn giphy_preview(url: &str) -> Option<String> {
    if !is_giphy_media(url) {
        return None;
    }
    let link = clean_ref(url);
    let (folder, file) = link.rsplit_once('/')?;
    (folder.contains("/media/") && file != "200w_d.gif").then(|| format!("{folder}/200w_d.gif"))
}

/// KLIPY's name for a GIF, from the `#…&slug=…` of a link posted from the picker.
pub fn link_slug(url: &str) -> Option<String> {
    let (_, ours) = url.split_once('#')?;
    ours.split('&').find_map(|p| p.strip_prefix("slug=")).filter(|s| crate::klipy::path_safe(s)).map(str::to_string)
}

/// The size a posted GIF link says its GIF is (see `Gif::posted`).
pub fn link_size(url: &str) -> Option<egui::Vec2> {
    let (link, fragment) = url.split_once('#').map_or((url, None), |(l, f)| (l, Some(f)));
    let query = link.split_once('?').map(|q| q.1);
    let size = |params: &str| {
        let num = |k: &str| {
            params.split('&').find_map(|p| p.strip_prefix(k)?.strip_prefix('=')?.parse::<u32>().ok()).filter(|n| (1..=4096).contains(n))
        };
        Some(egui::vec2(num("w")? as f32, num("h")? as f32))
    };
    fragment.and_then(size).or_else(|| query.and_then(size))
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
    /// Call every frame while `/giphy …` is in the box; starts searches as needed (on
    /// `provider`; KLIPY's results aren't kept for next time, as KLIPY asks).
    pub fn update(&mut self, provider: Provider, query: &str, api_key: &str, repaint: impl Fn() + Send + 'static) {
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
        let keep = provider == Provider::Giphy;
        if let Some(v) = self.cached(&q).filter(|_| keep) {
            *self.results.lock() = Some((q, Search::Ready(v)));
            return;
        }
        *self.results.lock() = Some((q.clone(), Search::Loading));
        let (slot, cache, key) = (self.results.clone(), self.cache.clone(), api_key.to_string());
        let _ = std::thread::Builder::new().name("ingle-giphy".into()).spawn(move || {
            let r = match provider {
                Provider::Giphy => search(&key, &q),
                Provider::Klipy => crate::klipy::search(&key, &q),
            };
            if let (Ok(v), true) = (&r, keep) {
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

/// Search GIPHY (or what's trending, for an empty query).
pub fn search(key: &str, q: &str) -> Result<Vec<Gif>, String> {
    #[cfg(debug_assertions)]
    if let Some(dir) = fake_dir() {
        std::thread::sleep(Duration::from_millis(300));
        let file = if q.is_empty() { "trending.json" } else { "search.json" };
        return parse(&std::fs::read_to_string(dir.join(file)).map_err(|e| e.to_string())?);
    }
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

/// Reads a GIPHY search or trending response. Lenient on purpose: a GIF with something odd
/// about it is skipped rather than failing the whole search, and sizes may come as text or
/// numbers.
pub fn parse(text: &str) -> Result<Vec<Gif>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("GIPHY sent something unexpected: {e}"))?;
    let data = v.get("data").and_then(|d| d.as_array()).ok_or("GIPHY sent something unexpected (no results list).")?;
    Ok(data.iter().filter_map(read_gif).collect())
}

fn read_gif(g: &serde_json::Value) -> Option<Gif> {
    let images = g.get("images")?.as_object()?;
    // A rendition with a link: (url, width, height).
    let pick = |names: &[&str]| {
        names.iter().find_map(|n| {
            let r = images.get(*n)?;
            let url = r.get("url")?.as_str().filter(|u| !u.is_empty())?;
            let num = |k: &str| {
                r.get(k)
                    .and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|t| t.parse::<f64>().ok())))
                    .filter(|n| *n > 0.0 && *n <= 4096.0)
                    .map(|n| n as f32)
            };
            Some((url.to_string(), num("width"), num("height")))
        })
    };
    let (preview, preview_w, preview_h) = pick(&["fixed_width_downsampled", "fixed_width", "fixed_width_small", "downsized"])?;
    let (full, w, h) = pick(&["downsized", "downsized_medium", "fixed_height", "original"])?;
    Some(Gif {
        provider: Provider::Giphy,
        id: g.get("id").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        slug: String::new(),
        title: g.get("title").and_then(|x| x.as_str()).unwrap_or_default().to_string(),
        preview: clean(&preview),
        preview_w: preview_w.unwrap_or(200.0),
        preview_h: preview_h.unwrap_or(200.0),
        url: clean(&full),
        size: w.zip(h).map(|(w, h)| (w as u32, h as u32)),
    })
}

/// GIPHY links without their tracking query.
fn clean(url: &str) -> String {
    url.split('?').next().unwrap_or(url).to_string()
}

pub(crate) fn enc(s: &str) -> String {
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

/// Where a GIF link (shown in chat by itself) is from: GIPHY's or KLIPY's media hosts only.
pub fn media_provider(url: &str) -> Option<Provider> {
    if is_giphy_media(url) {
        Some(Provider::Giphy)
    } else if crate::klipy::is_media(url) {
        Some(Provider::Klipy)
    } else {
        None
    }
}

/// GIF links (GIPHY's and KLIPY's) in a message.
pub fn gif_links(text: &str) -> Vec<String> {
    text.split_whitespace().filter(|w| media_provider(w).is_some()).map(|w| w.to_string()).take(4).collect()
}

/// Downloads of GIFs shown in chat and in the search grid.
#[derive(Clone, Default)]
pub struct Downloads {
    map: Arc<Mutex<HashMap<String, Dl>>>,
    running: Arc<std::sync::atomic::AtomicUsize>,
}

impl Downloads {
    pub fn get(&self, url: &str, repaint: impl Fn() + Send + 'static) -> Dl {
        if media_provider(url).is_none() {
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

    /// Whether `url` was tried and couldn't be downloaded (without trying it).
    pub fn failed(&self, url: &str) -> bool {
        media_provider(url).is_none() || matches!(self.map.lock().get(url), Some(Dl::Failed))
    }
}

/// The sizes of GIFs shown before, kept between runs: a GIF's space in the chat is set aside
/// before it loads, so nothing moves when it does (GIPHY links don't say how big they are).
#[derive(Default)]
pub struct GifSizes {
    map: HashMap<String, (u16, u16)>,
    /// Where they're kept (none: not kept, in tests).
    path: Option<std::path::PathBuf>,
    dirty: bool,
    saved_at: Option<Instant>,
}

/// Sizes kept at most (old ones make way).
const SIZES_MAX: usize = 3000;

impl GifSizes {
    pub fn load(dir: &std::path::Path) -> GifSizes {
        let path = dir.join("gif-sizes.json");
        let map = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        GifSizes { map, path: Some(path), dirty: false, saved_at: None }
    }

    /// How big the GIF at `url` was last time (as decoded for the chat).
    pub fn get(&self, url: &str) -> Option<egui::Vec2> {
        self.map.get(clean_ref(url)).map(|(w, h)| egui::vec2(*w as f32, *h as f32))
    }

    pub fn set(&mut self, url: &str, size: egui::Vec2) {
        let v = (size.x.round().clamp(1.0, 4096.0) as u16, size.y.round().clamp(1.0, 4096.0) as u16);
        let url = clean_ref(url);
        if self.map.get(url) == Some(&v) {
            return;
        }
        if self.map.len() >= SIZES_MAX {
            let drop: Vec<String> = self.map.keys().take(SIZES_MAX / 10).cloned().collect();
            for k in drop {
                self.map.remove(&k);
            }
        }
        self.map.insert(url.to_string(), v);
        self.dirty = true;
    }

    /// Saves what's new, at most every half minute (`now`: right away).
    pub fn save(&mut self, now: bool) {
        if !self.dirty || (!now && self.saved_at.is_some_and(|t| t.elapsed() < Duration::from_secs(30))) {
            return;
        }
        let Some(path) = &self.path else { return };
        if let Ok(b) = serde_json::to_vec(&self.map) {
            let tmp = path.with_extension("json.tmp");
            if std::fs::write(&tmp, b).is_ok() && std::fs::rename(&tmp, path).is_ok() {
                self.dirty = false;
            }
        }
        self.saved_at = Some(Instant::now());
    }
}

/// A GIF link as remembered: without its query or size.
fn clean_ref(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// Debug builds only, for trying the picker without GIPHY: `INGLE_GIPHY_FAKE=<folder>` answers
/// trending from `trending.json` and searches from `search.json` in that folder, and serves GIF
/// links from it by file name.
#[cfg(debug_assertions)]
fn fake_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("INGLE_GIPHY_FAKE").map(std::path::PathBuf::from)
}

fn fetch(url: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    // (What's after a # never goes out: it's ours.)
    let url = url.split('#').next().unwrap_or(url);
    #[cfg(debug_assertions)]
    {
        let fake = match media_provider(url) {
            Some(Provider::Klipy) => std::env::var_os("INGLE_KLIPY_FAKE").map(std::path::PathBuf::from),
            _ => fake_dir(),
        };
        if let Some(dir) = fake {
            return std::fs::read(dir.join(url.rsplit('/').next()?.split('?').next()?)).ok();
        }
    }
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
        // Today's links carry a token in the path; sizes may be numbers; odd items are skipped.
        let text = r#"{"data":[
            {"id":"tphCApwvdtC1VJabZ1","title":"Cat Meme GIF","images":{
              "fixed_width_downsampled":{"url":"https://media0.giphy.com/media/v1.Y2lkPWU1ZTE5MDI5/tphCApwvdtC1VJabZ1/200w_d.gif","width":200,"height":"113","size":"123"},
              "downsized":{"url":"https://media0.giphy.com/media/v1.Y2lkPWU1ZTE5MDI5/tphCApwvdtC1VJabZ1/giphy-downsized.gif","width":"480","height":"270"},
              "looping":{"mp4":"https://media0.giphy.com/x.mp4"}}},
            {"id":"weird","images":"none"},
            {"id":"noimg"}],"meta":{"status":200,"msg":"OK"}}"#;
        let v = parse(text).unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!((v[0].preview_w, v[0].preview_h), (200.0, 113.0));
        assert!(is_giphy_media(&v[0].url) && is_giphy_media(&v[0].preview));
        assert!(parse(r#"{"meta":{"status":401,"msg":"Unauthorized"}}"#).is_err());
        // The link posted says how big the GIF is (and is still a GIPHY GIF to older Ingles).
        let posted = v[0].posted();
        assert_eq!(posted, "https://media0.giphy.com/media/v1.Y2lkPWU1ZTE5MDI5/tphCApwvdtC1VJabZ1/giphy-downsized.gif?w=480&h=270");
        assert!(is_giphy_media(&posted));
        assert_eq!(gif_links(&posted), vec![posted.clone()]);
        assert_eq!(link_size(&posted), Some(egui::vec2(480.0, 270.0)));
        assert_eq!(link_size(&v[0].url), None);
        assert_eq!(link_size("https://media.giphy.com/media/a/giphy.gif?w=0&h=10"), None);
        assert_eq!(link_size("https://media.giphy.com/media/a/giphy.gif?cid=x&h=10&w=20"), Some(egui::vec2(20.0, 10.0)));
    }

    #[test]
    fn gif_sizes_are_kept_between_runs() {
        let dir = std::env::temp_dir().join(format!("ingle-gifsizes-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = GifSizes::load(&dir);
        assert_eq!(s.get("https://media.giphy.com/media/a/giphy.gif"), None);
        s.set("https://media.giphy.com/media/a/giphy.gif?w=1&h=1", egui::vec2(480.0, 270.0));
        s.save(true);
        let s = GifSizes::load(&dir);
        // (Whatever the link's query.)
        assert_eq!(s.get("https://media.giphy.com/media/a/giphy.gif"), Some(egui::vec2(480.0, 270.0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn gif(id: &str) -> Gif {
        Gif {
            provider: Provider::Giphy,
            id: id.into(),
            slug: String::new(),
            title: String::new(),
            preview: String::new(),
            preview_w: 1.0,
            preview_h: 1.0,
            url: String::new(),
            size: None,
        }
    }

    #[test]
    fn searches_are_reused_and_never_shown_for_other_words() {
        assert_eq!(normalize("  Cute   CATS "), "cute cats");
        let mut g = GiphyUi::default();
        g.cache.lock().insert("dogs".into(), (Instant::now(), vec![gif("d1")]));
        // Still typing: nothing yet.
        g.update(Provider::Giphy, "Dogs", "key", || {});
        assert!(g.current().is_none());
        // After the pause, the cached results come straight back (no call to GIPHY).
        g.typed_at = Some(Instant::now() - PAUSE * 2);
        g.update(Provider::Giphy, "Dogs", "key", || {});
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

    #[test]
    fn favorites_like_discords() {
        // From the picker: the posted link, its small preview and KLIPY's name.
        let g = Gif {
            provider: Provider::Giphy,
            id: "abc".into(),
            slug: String::new(),
            title: "Cat".into(),
            preview: "https://media2.giphy.com/media/abc/200w_d.gif".into(),
            preview_w: 200.0,
            preview_h: 150.0,
            url: "https://media2.giphy.com/media/abc/giphy-downsized.gif".into(),
            size: Some((480, 360)),
        };
        let f = g.favorite();
        assert_eq!(f.url, "https://media2.giphy.com/media/abc/giphy-downsized.gif?w=480&h=360");
        assert_eq!((f.preview.as_str(), f.w, f.h, f.title.as_str()), ("https://media2.giphy.com/media/abc/200w_d.gif", 200, 150, "Cat"));

        // Newest first; the same GIF at another size is the same favorite; starring it again
        // takes it out.
        let mut favs = toggle_favorite(&[], f.clone()).unwrap();
        let other = favorite_from_link("https://media.giphy.com/media/v1.Y2lk/xyz/giphy.gif", Some(egui::vec2(320.0, 240.0)));
        favs = toggle_favorite(&favs, other.clone()).unwrap();
        assert_eq!(favs.iter().map(|f| f.url.as_str()).collect::<Vec<_>>(), [other.url.as_str(), f.url.as_str()]);
        assert!(is_favorite(&favs, "https://media2.giphy.com/media/abc/giphy-downsized.gif?w=240&h=180"));
        assert!(!is_favorite(&favs, "https://media2.giphy.com/media/abd/giphy-downsized.gif"));
        favs = toggle_favorite(&favs, favorite_from_link("https://media2.giphy.com/media/abc/giphy-downsized.gif", None)).unwrap();
        assert_eq!(favs, vec![other.clone()]);

        // From chat: GIPHY's small version from the same folder, the size it was shown at.
        assert_eq!(other.preview, "https://media.giphy.com/media/v1.Y2lk/xyz/200w_d.gif");
        assert_eq!((other.w, other.h), (320, 240));
        assert_eq!(favorite_from_link("https://i.giphy.com/abc.gif", None).preview, "");
        // KLIPY's links are only used as given; the name comes from the link when it says.
        let k = favorite_from_link("https://static.klipy.com/ii/a/md.gif#w=400&h=225&slug=hello-hi-662", None);
        assert_eq!((k.preview.as_str(), k.slug.as_str(), k.w, k.h), ("", "hello-hi-662", 400, 225));
        assert_eq!(link_slug("https://static.klipy.com/ii/a/md.gif#w=4&h=2&slug=../x"), None);

        // There's a limit, and it says so.
        let full: Vec<GifFavorite> = (0..MAX_FAVORITES).map(|i| favorite_from_link(&format!("https://media.giphy.com/media/g{i}/giphy.gif"), None)).collect();
        assert!(toggle_favorite(&full, f.clone()).unwrap_err().contains("500"));
        // (Taking one out still works when full.)
        assert_eq!(toggle_favorite(&full, full[3].clone()).unwrap().len(), MAX_FAVORITES - 1);
    }
}
