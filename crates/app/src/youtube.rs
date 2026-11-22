//! YouTube links in chat show a preview under their message (0.16.5), like Discord's: the
//! video's picture with a play button, its title and its channel. Clicking it opens the video
//! in the browser.
//!
//! What's shown comes straight from YouTube: the title and channel from its oEmbed address
//! (no key needed), the picture from its picture server. Only YouTube's own addresses are
//! asked, only for links in messages that are on the screen (or about to be), a few at a
//! time, and each video once. Settings → Appearance turns previews off.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use parking_lot::Mutex;

/// The most previews under one message.
const PER_MESSAGE: usize = 3;
/// The most videos asked about at once.
const AT_ONCE: usize = 3;
/// Videos remembered (the oldest asked about goes first).
const KEEP: usize = 300;
/// The biggest answers taken: oEmbed's few lines, and a picture (YouTube's are about 30 kB).
const MAX_INFO: u64 = 64 * 1024;
const MAX_PICTURE: u64 = 2 << 20;

/// A link to a YouTube video in a message: the video, and the link as it was written (it's
/// what's opened, so a time in it is kept).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Video {
    pub id: String,
    pub url: String,
}

/// A video's ID: YouTube's are 11 letters, digits, `-` and `_`.
fn is_id(s: &str) -> bool {
    s.len() == 11 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The video a link is to, if it's a link to a YouTube video: youtube.com's watch, shorts,
/// live and embed pages (www., m. and music. too) and youtu.be's short links, http or https.
pub fn video_id(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let (host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let host = host.to_ascii_lowercase();
    // (No user or port: nothing that only looks like YouTube.)
    if host.contains('@') || host.contains(':') {
        return None;
    }
    let path = path.split('#').next().unwrap_or("");
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let id = if host == "youtu.be" || host == "www.youtu.be" {
        path.split('/').next().unwrap_or("").to_string()
    } else if matches!(host.as_str(), "youtube.com" | "www.youtube.com" | "m.youtube.com" | "music.youtube.com") {
        let mut parts = path.split('/');
        match parts.next().unwrap_or("") {
            "watch" => query.split('&').find_map(|kv| kv.strip_prefix("v="))?.to_string(),
            "shorts" | "live" | "embed" | "v" => parts.next().unwrap_or("").to_string(),
            _ => return None,
        }
    } else {
        return None;
    };
    is_id(&id).then_some(id)
}

/// The YouTube videos linked in a message (each once, three at most), in order. Only what
/// chat shows as a link (markdown.rs's rules): none in `code` or a ```block```, and none
/// written <like this>, which is how Discord's people say "no preview" (chat shows it as
/// it's written).
pub fn links_in(text: &str) -> Vec<Video> {
    use crate::markdown::Block;
    let mut out: Vec<Video> = Vec::new();
    for block in crate::markdown::blocks(text) {
        let (Block::Text(t) | Block::Quote(t)) = block else { continue };
        let chars: Vec<char> = t.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // (\` and the like: the character itself.)
            if chars[i] == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '*' | '_' | '~' | '`' | '@' | '\\' | '>' | '|') {
                i += 2;
                continue;
            }
            // (`code`: a backtick with another after it.)
            if chars[i] == '`' {
                if let Some(end) = chars[i + 1..].iter().position(|c| *c == '`') {
                    i += end + 2;
                    continue;
                }
            }
            let rest: String = chars[i..chars.len().min(i + 8)].iter().collect();
            let starts = rest.starts_with("https://") || rest.starts_with("http://");
            if !starts || !(i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(') {
                i += 1;
                continue;
            }
            // To the next space, without the punctuation after it.
            let mut j = i;
            while j < chars.len() && !chars[j].is_whitespace() {
                j += 1;
            }
            while j > i && matches!(chars[j - 1], '.' | ',' | ')' | '!' | '?' | ':' | ';') {
                j -= 1;
            }
            let link: String = chars[i..j].iter().collect();
            if let Some(id) = video_id(&link) {
                if !out.iter().any(|v| v.id == id) {
                    out.push(Video { id, url: link });
                    if out.len() == PER_MESSAGE {
                        return out;
                    }
                }
            }
            i = j.max(i + 1);
        }
    }
    out
}

/// What YouTube says about a video.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Info {
    pub title: String,
    pub channel: String,
    /// The channel's page (only ever one of YouTube's own).
    pub channel_url: Option<String>,
}

/// One line of what YouTube sent, as it may be shown: no control characters, `max` characters
/// at most.
fn clean(s: &str, max: usize) -> String {
    let t: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    t.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(max).collect()
}

/// Reads YouTube's oEmbed answer (`title`, `author_name`, `author_url`).
pub fn parse_info(json: &str) -> Option<Info> {
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    let title = clean(v.get("title")?.as_str()?, 200);
    if title.is_empty() {
        return None;
    }
    let channel = clean(v.get("author_name").and_then(|a| a.as_str()).unwrap_or(""), 100);
    let channel_url = v
        .get("author_url")
        .and_then(|a| a.as_str())
        .filter(|u| u.starts_with("https://www.youtube.com/") && u.len() < 300 && !u.chars().any(|c| c.is_whitespace() || c.is_control()))
        .map(str::to_string);
    Some(Info { title, channel, channel_url })
}

/// A video's preview as far as it's got.
#[derive(Clone)]
pub enum Preview {
    Loading,
    /// What could be had: the title and channel, the picture (either may be missing), and
    /// whether the picture is still coming (the title shows as soon as it's in).
    Ready { info: Option<Info>, picture: Option<Arc<Vec<u8>>>, coming: bool },
    /// YouTube doesn't know it (taken down, private), or couldn't be reached at all.
    Failed,
}

struct Entry {
    preview: Preview,
    asked: Instant,
}

/// The videos asked about, and their previews.
#[derive(Clone, Default)]
pub struct Previews {
    map: Arc<Mutex<HashMap<String, Entry>>>,
    running: Arc<std::sync::atomic::AtomicUsize>,
}

impl Previews {
    /// The preview of video `id`, asked for the first time it's wanted (`repaint` when it's in).
    pub fn get(&self, id: &str, repaint: impl Fn() + Send + 'static) -> Preview {
        if !is_id(id) {
            return Preview::Failed;
        }
        let mut m = self.map.lock();
        if let Some(e) = m.get(id) {
            return e.preview.clone();
        }
        if self.running.load(std::sync::atomic::Ordering::Relaxed) >= AT_ONCE {
            return Preview::Loading;
        }
        if m.len() >= KEEP {
            if let Some(oldest) = m.iter().filter(|(_, e)| !matches!(e.preview, Preview::Loading)).min_by_key(|(_, e)| e.asked).map(|(k, _)| k.clone()) {
                m.remove(&oldest);
            }
        }
        m.insert(id.to_string(), Entry { preview: Preview::Loading, asked: Instant::now() });
        drop(m);
        self.running.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let (map, running, id) = (self.map.clone(), self.running.clone(), id.to_string());
        let _ = std::thread::Builder::new().name("pyr-youtube".into()).spawn(move || {
            fetch(&id, &|p| {
                if let Some(e) = map.lock().get_mut(&id) {
                    e.preview = p;
                }
                repaint();
            });
            running.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        });
        Preview::Loading
    }
}

/// Where the asking goes: YouTube, or `PYR_YOUTUBE` (a test one, which answers both
/// `/oembed` and `/vi/…`).
fn bases() -> (String, String) {
    match pyr_proto::env_var("YOUTUBE").map(|d| d.trim().trim_end_matches('/').to_string()).filter(|d| d.starts_with("http://") || d.starts_with("https://")) {
        Some(test) => (test.clone(), test),
        None => ("https://www.youtube.com".into(), "https://i.ytimg.com".into()),
    }
}

fn agent(base: &str) -> ureq::Agent {
    let host = base.split_once("://").map_or(base, |(_, h)| h);
    let mut cfg = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .http_status_as_error(false)
        .user_agent(concat!("Pyr/", env!("CARGO_PKG_VERSION")));
    if host.starts_with("127.0.0.1") || host.starts_with("localhost") {
        cfg = cfg.proxy(None);
    }
    cfg.build().into()
}

/// Asks YouTube about video `id` (already checked to be an ID, so it can go in the addresses
/// as it is): its title and channel (told as soon as they're in), then its picture.
fn fetch(id: &str, tell: &dyn Fn(Preview)) {
    use std::io::Read;
    let (info_base, picture_base) = bases();
    let watch = format!("https%3A%2F%2Fwww.youtube.com%2Fwatch%3Fv%3D{id}");
    let info = match agent(&info_base).get(&format!("{info_base}/oembed?format=json&url={watch}")).call() {
        Ok(mut r) if r.status().as_u16() == 200 => r.body_mut().with_config().limit(MAX_INFO).read_to_string().ok().and_then(|s| parse_info(&s)),
        // Taken down or private (YouTube answers 401, 403 or 404): no preview.
        Ok(r) if matches!(r.status().as_u16(), 400 | 401 | 403 | 404) => return tell(Preview::Failed),
        Ok(_) => None,
        Err(e) => {
            tracing::debug!("youtube: couldn't ask about {id}: {e}");
            None
        }
    };
    if info.is_some() {
        tell(Preview::Ready { info: info.clone(), picture: None, coming: true });
    }
    // (hqdefault is there for every video: 480x360, a 16:9 video in a band across it.)
    let picture = agent(&picture_base).get(&format!("{picture_base}/vi/{id}/hqdefault.jpg")).call().ok().filter(|r| r.status().as_u16() == 200).and_then(|mut r| {
        let mut out = Vec::new();
        r.body_mut().as_reader().take(MAX_PICTURE + 1).read_to_end(&mut out).ok()?;
        (out.len() as u64 <= MAX_PICTURE && !out.is_empty()).then(|| Arc::new(out))
    });
    tell(if info.is_none() && picture.is_none() { Preview::Failed } else { Preview::Ready { info, picture, coming: false } });
}

/// YouTube's red, for the card's edge (as Discord's YouTube cards have).
const RED: egui::Color32 = egui::Color32::from_rgb(0xff, 0x26, 0x26);
/// The card's widest (as an invite's).
const CARD_W: f32 = 432.0;

impl crate::app::PyrApp {
    /// A YouTube link's preview under its message: a card with "YouTube", the channel, the
    /// title (click it, or the picture, to open the video in the browser) and the picture with a
    /// play button. It's that size from the start (nothing in the chat moves when it comes in),
    /// unless YouTube says the video's gone: then it's a small card that says so.
    pub(crate) fn youtube_embed(&mut self, ui: &mut egui::Ui, v: &Video) {
        use crate::theme;
        use egui::{Color32, CursorIcon, Rect, Sense, Stroke, pos2, vec2};
        let w = ui.available_width().min(CARD_W);
        let inner = w - 4.0 - 24.0;
        let pic_h = (inner * 9.0 / 16.0).round();
        let top = ui.cursor().min;
        let key = format!("yt:{}", v.id);
        // (Far off the screen it isn't asked for yet: its space is kept.)
        let far = self.ui.load_rect.is_some_and(|r| !r.intersects(Rect::from_min_size(top, vec2(w, 93.0 + pic_h))));
        let preview = if far && !self.images.has(&key) {
            Preview::Loading
        } else {
            let ctx = ui.ctx().clone();
            self.ui.youtube.get(&v.id, move || ctx.request_repaint())
        };
        let gone = matches!(preview, Preview::Failed);
        // Its height: 12 + "YouTube" 16 + 2 + channel 18 + 4 + title 21 + 8 + picture + 12; gone,
        // 12 + "YouTube" 16 + 4 + the line 21 + 12.
        let h = if gone { 65.0 } else { 93.0 + pic_h };
        let (card, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
        let p = ui.painter_at(card.expand(1.0));
        p.rect_filled(card, theme::r(8), theme::ELEV());
        p.rect_filled(Rect::from_min_size(card.min, vec2(4.0, h)), egui::CornerRadius { nw: 8, sw: 8, ne: 0, se: 0 }, RED);
        let x = card.left() + 4.0 + 12.0;
        let mut y = card.top() + 12.0;
        p.text(pos2(x, y), egui::Align2::LEFT_TOP, "YouTube", theme::font(12.0), theme::DIM());
        if gone {
            p.text(pos2(x, y + 20.0), egui::Align2::LEFT_TOP, "This video isn't available", theme::bold_font(15.0), theme::MUTED());
            ui.add_space(4.0);
            return;
        }
        y += 18.0;
        let (info, picture, coming) = match &preview {
            Preview::Ready { info, picture, coming } => (info.clone(), picture.clone(), *coming),
            _ => (None, None, true),
        };
        let ready = matches!(preview, Preview::Ready { .. });
        // The channel (its page opens when it's clicked).
        if let Some(i) = &info {
            let mut job = egui::text::LayoutJob::simple_singleline(i.channel.clone(), theme::font(13.0), theme::TEXT());
            job.wrap = egui::text::TextWrapping::truncate_at_width(inner);
            let g = p.layout_job(job);
            let r = Rect::from_min_size(pos2(x, y), vec2(g.size().x.min(inner), 18.0));
            p.galley(pos2(x, y), g, theme::TEXT());
            if let Some(url) = &i.channel_url {
                let resp = ui.interact(r, ui.id().with(("yt-channel", &v.id, top.y as i32)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                if resp.hovered() {
                    p.line_segment([pos2(r.left(), r.bottom() - 1.0), pos2(r.right(), r.bottom() - 1.0)], Stroke::new(1.0f32, theme::TEXT()));
                }
                if resp.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                }
            }
        }
        y += 22.0;
        // The title, on one line (all of it when pointed at); the link itself if YouTube didn't
        // say.
        let title = match &info {
            Some(i) => i.title.clone(),
            None if ready => v.url.clone(),
            None => "Loading video…".to_string(),
        };
        let color = if ready { theme::LINK() } else { theme::MUTED() };
        let mut job = egui::text::LayoutJob::simple_singleline(title.clone(), theme::bold_font(15.0), color);
        job.wrap = egui::text::TextWrapping::truncate_at_width(inner);
        let g = p.layout_job(job);
        let title_r = Rect::from_min_size(pos2(x, y), vec2(g.size().x.min(inner), 21.0));
        p.galley(pos2(x, y), g, color);
        if ready {
            let resp = ui.interact(title_r, ui.id().with(("yt-title", &v.id, top.y as i32)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(&title);
            if resp.hovered() {
                p.line_segment([pos2(title_r.left(), title_r.bottom() - 1.0), pos2(title_r.right(), title_r.bottom() - 1.0)], Stroke::new(1.0f32, color));
            }
            if resp.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&v.url));
            }
        }
        y += 29.0;
        // The picture (the 16:9 band out of YouTube's 4:3 one), and a play button over it.
        let pic = Rect::from_min_size(pos2(x, y), vec2(inner, pic_h));
        let tex = picture.as_ref().and_then(|bytes| self.images.get(ui.ctx(), &key, Some(bytes.clone()), 480).frame(0.0).map(|(t, _, _)| t));
        match tex {
            Some(t) => {
                let uv = Rect::from_min_max(pos2(0.0, 0.125), pos2(1.0, 0.875));
                p.add(egui::epaint::RectShape::filled(pic, theme::r(6), Color32::WHITE).with_texture(t, uv));
            }
            None => {
                p.rect_filled(pic, theme::r(6), theme::SIDE());
                if coming {
                    p.text(pic.center() + vec2(0.0, 44.0), egui::Align2::CENTER_CENTER, "Loading…", theme::font(12.5), theme::DIM());
                }
            }
        }
        let resp = ui.interact(pic, ui.id().with(("yt-picture", &v.id, top.y as i32)), Sense::click());
        let hot = ready && resp.hovered();
        let c = pic.center();
        p.circle_filled(c, 28.0, if hot { theme::EMBER() } else { Color32::from_rgba_unmultiplied(12, 10, 10, 190) });
        p.circle_stroke(c, 28.0, Stroke::new(2.0f32, Color32::from_white_alpha(if hot { 230 } else { 150 })));
        let s = 11.0;
        p.add(egui::Shape::convex_polygon(vec![c + vec2(-s * 0.6, -s), c + vec2(-s * 0.6, s), c + vec2(s * 1.1, 0.0)], Color32::WHITE, Stroke::NONE));
        if ready {
            let resp = resp.on_hover_cursor(CursorIcon::PointingHand).on_hover_text("Watch on YouTube");
            if resp.clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(&v.url));
            }
        }
        ui.add_space(4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_video_in_youtubes_links() {
        let id = "dkvFcYBznPI";
        for url in [
            "https://www.youtube.com/watch?v=dkvFcYBznPI",
            "https://youtube.com/watch?v=dkvFcYBznPI&t=42s",
            "http://m.youtube.com/watch?feature=share&v=dkvFcYBznPI",
            "https://music.youtube.com/watch?v=dkvFcYBznPI&list=RDAMVM",
            "https://youtu.be/dkvFcYBznPI",
            "https://youtu.be/dkvFcYBznPI?si=abc&t=10",
            "https://www.youtube.com/shorts/dkvFcYBznPI",
            "https://www.youtube.com/live/dkvFcYBznPI?feature=shared",
            "https://www.youtube.com/embed/dkvFcYBznPI",
            "https://WWW.YouTube.com/watch?v=dkvFcYBznPI#comments",
        ] {
            assert_eq!(video_id(url).as_deref(), Some(id), "{url}");
        }
        for url in [
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?v=dkvFcYBznPI1",
            "https://www.youtube.com/playlist?list=PL123",
            "https://www.youtube.com/@somechannel",
            "https://youtube.com.evil.example/watch?v=dkvFcYBznPI",
            "https://evil.example/youtu.be/dkvFcYBznPI",
            "https://user@www.youtube.com/watch?v=dkvFcYBznPI",
            "https://www.youtube.com:8080/watch?v=dkvFcYBznPI",
            "https://www.youtube.com/watch?v=dkvFcYB%nPI",
            "www.youtube.com/watch?v=dkvFcYBznPI",
            "ftp://youtu.be/dkvFcYBznPI",
        ] {
            assert_eq!(video_id(url), None, "{url}");
        }
    }

    #[test]
    fn a_message_s_videos_each_once_three_at_most() {
        let text = "look https://youtu.be/aaaaaaaaaaa, and (https://www.youtube.com/watch?v=bbbbbbbbbbb). Again: https://youtu.be/aaaaaaaaaaa?t=3 \
                    https://youtu.be/ccccccccccc https://youtu.be/ddddddddddd";
        let v = links_in(text);
        assert_eq!(v.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(), ["aaaaaaaaaaa", "bbbbbbbbbbb", "ccccccccccc"]);
        assert_eq!(v[0].url, "https://youtu.be/aaaaaaaaaaa");
        assert_eq!(v[1].url, "https://www.youtube.com/watch?v=bbbbbbbbbbb");
        assert!(links_in("no videos here: https://example.com/watch?v=aaaaaaaaaaa").is_empty());
    }

    #[test]
    fn only_what_chat_shows_as_a_link() {
        let ids = |t: &str| links_in(t).into_iter().map(|v| v.id).collect::<Vec<_>>();
        // <Like this> (Discord's "no preview"), in `code` or a ```block```, or run into a word:
        // not links in chat, so no preview.
        assert!(ids("no preview please <https://youtu.be/aaaaaaaaaaa>").is_empty());
        assert!(ids("`https://youtu.be/aaaaaaaaaaa` and ```\nhttps://youtu.be/bbbbbbbbbbb\n```").is_empty());
        assert!(ids("**https://youtu.be/aaaaaaaaaaa** x:https://youtu.be/bbbbbbbbbbb").is_empty());
        // A quote's links and a link after code are.
        assert_eq!(ids("> https://youtu.be/aaaaaaaaaaa\n`x` https://youtu.be/bbbbbbbbbbb!"), ["aaaaaaaaaaa", "bbbbbbbbbbb"]);
        // A backtick with none after it is only a backtick, and so is \`.
        assert_eq!(ids("it's ` https://youtu.be/ccccccccccc"), ["ccccccccccc"]);
        assert_eq!(ids(r"\` https://youtu.be/ddddddddddd \`"), ["ddddddddddd"]);
    }

    #[test]
    fn reads_what_youtube_says() {
        let json = r#"{"title":"Some  video\nwith a line","author_name":"A Channel","author_url":"https://www.youtube.com/@achannel","type":"video","thumbnail_url":"https://i.ytimg.com/vi/x/hqdefault.jpg"}"#;
        let i = parse_info(json).unwrap();
        assert_eq!(i.title, "Some video with a line");
        assert_eq!(i.channel, "A Channel");
        assert_eq!(i.channel_url.as_deref(), Some("https://www.youtube.com/@achannel"));
        // A channel page that isn't YouTube's isn't kept; no title, nothing.
        let i = parse_info(r#"{"title":"T","author_name":"C","author_url":"https://evil.example/"}"#).unwrap();
        assert_eq!(i.channel_url, None);
        assert!(parse_info(r#"{"author_name":"C"}"#).is_none());
        assert!(parse_info("Not Found").is_none());
        let long = format!(r#"{{"title":"{}"}}"#, "x".repeat(500));
        assert_eq!(parse_info(&long).unwrap().title.chars().count(), 200);
    }

    #[test]
    fn only_videos_are_asked_about() {
        let p = Previews::default();
        assert!(matches!(p.get("../../etc/pa", || {}), Preview::Failed));
        assert!(matches!(p.get("a b c d e f", || {}), Preview::Failed));
    }
}
