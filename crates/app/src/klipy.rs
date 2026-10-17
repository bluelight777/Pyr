//! GIF search from KLIPY, beside GIPHY's (see `giphy.rs`): type `/klipy cats`, or pick the
//! KLIPY tab of the GIF picker. Like GIPHY's, it's turned on per server by its owner, with a
//! KLIPY app key everyone's Pyr uses.
//!
//! KLIPY's rules (docs.klipy.com/integration-requirements, /attribution): requests and media
//! go straight from each person's app to KLIPY (no server in between), media is loaded from
//! the addresses KLIPY gives just as given (nothing kept on disk), results show in KLIPY's order
//! in their own tab (never mixed with GIPHY's), the search box says "Search KLIPY", and a share
//! is reported to KLIPY when one is sent.

use std::time::Duration;

use crate::giphy::{Gif, Provider};

const API: &str = "https://api.klipy.com/api/v1";
const RESULTS: usize = 24;

/// The KLIPY app key for a server: the one its owner set up (`ServerInfo::klipy_key`), if it's
/// one that can go in KLIPY's addresses. (Development builds also take `PYR_KLIPY_KEY`.)
pub fn key(server_key: &str) -> Option<String> {
    let k = server_key.trim();
    if path_safe(k) {
        return Some(k.to_string());
    }
    if cfg!(debug_assertions) {
        return pyr_proto::env_var("KLIPY_KEY").filter(|k| path_safe(k));
    }
    None
}

/// Fit to go in a KLIPY address as a part of its path (an app key, a slug).
pub(crate) fn path_safe(s: &str) -> bool {
    !s.is_empty() && s.len() <= 200 && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into()
}

/// Search KLIPY (or what's trending, for an empty query).
pub fn search(key: &str, q: &str) -> Result<Vec<Gif>, String> {
    #[cfg(debug_assertions)]
    if let Some(dir) = fake_dir() {
        std::thread::sleep(Duration::from_millis(300));
        let file = if q.is_empty() { "trending.json" } else { "search.json" };
        return parse(&std::fs::read_to_string(dir.join(file)).map_err(|e| e.to_string())?);
    }
    let url = if q.is_empty() {
        format!("{API}/{key}/gifs/trending?per_page={RESULTS}&content_filter=medium&format_filter=gif")
    } else {
        format!("{API}/{key}/gifs/search?q={}&per_page={RESULTS}&content_filter=medium&format_filter=gif", crate::giphy::enc(q))
    };
    let mut resp = agent().get(&url).call().map_err(|e| match e {
        ureq::Error::StatusCode(401 | 403 | 404) => "KLIPY didn't accept this server's app key.".to_string(),
        ureq::Error::StatusCode(429) => "KLIPY searches are used up for now (a KLIPY test key gets 100 an hour; the server's owner can ask KLIPY for production access). Try again a bit later.".to_string(),
        e => format!("Couldn't reach KLIPY: {e}"),
    })?;
    let text = resp.body_mut().read_to_string().map_err(|e| e.to_string())?;
    parse(&text)
}

/// Reads a KLIPY search or trending response (`data.data[]`, each with `file.<size>.gif`),
/// in KLIPY's order. An item without a GIF we can show is left out.
pub fn parse(text: &str) -> Result<Vec<Gif>, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("KLIPY sent something unexpected: {e}"))?;
    if v.get("result").and_then(|r| r.as_bool()) == Some(false) {
        let why = v.get("message").or_else(|| v.get("errors")).map(|m| m.to_string()).unwrap_or_default();
        return Err(format!("KLIPY couldn't search: {}", why.trim_matches('"')));
    }
    let items = v.get("data").and_then(|d| d.get("data")).and_then(|d| d.as_array()).ok_or("KLIPY sent something unexpected (no results list).")?;
    Ok(items.iter().filter_map(read_item).collect())
}

fn read_item(g: &serde_json::Value) -> Option<Gif> {
    // (Ads only come when asked for, which Pyr doesn't.)
    if g.get("type").and_then(|t| t.as_str()).is_some_and(|t| t == "ad") {
        return None;
    }
    let file = g.get("file")?.as_object()?;
    // A size's GIF: (url, width, height).
    let size = |name: &str| {
        let f = file.get(name)?.get("gif")?;
        let url = f.get("url")?.as_str().filter(|u| is_media(u))?;
        let num = |k: &str| f.get(k).and_then(|x| x.as_f64()).filter(|n| *n > 0.0 && *n <= 4096.0).map(|n| n as f32);
        Some((url.to_string(), num("width"), num("height")))
    };
    // The grid: the smallest that still looks sharp in its column.
    let preview = ["xs", "sm", "md", "hd"]
        .iter()
        .filter_map(|n| size(n))
        .find(|(_, w, _)| w.is_some_and(|w| w >= 120.0))
        .or_else(|| ["sm", "xs", "md"].iter().find_map(|n| size(n)))?;
    // What's posted: the smallest that fills a GIF's place in the chat (about 320 wide).
    let posted = ["sm", "md", "hd"]
        .iter()
        .filter_map(|n| size(n))
        .find(|(_, w, _)| w.is_some_and(|w| w >= 300.0))
        .or_else(|| ["md", "hd", "sm", "xs"].iter().find_map(|n| size(n)))?;
    let text = |k: &str| g.get(k).and_then(|x| x.as_str().map(str::to_string).or_else(|| x.as_u64().map(|n| n.to_string())));
    Some(Gif {
        provider: Provider::Klipy,
        id: text("id").unwrap_or_default(),
        slug: text("slug").unwrap_or_default(),
        title: text("title").unwrap_or_default(),
        preview: preview.0,
        preview_w: preview.1.unwrap_or(200.0),
        preview_h: preview.2.unwrap_or(200.0),
        url: posted.0,
        size: posted.1.zip(posted.2).map(|(w, h)| (w as u32, h as u32)),
    })
}

/// Tells KLIPY a GIF was sent (its share counts toward what KLIPY shows people), with the
/// words it was found by (none for trending). In the background; nothing waits for it.
pub fn share(key: &str, slug: &str, q: &str) {
    if !path_safe(key) || !path_safe(slug) {
        return;
    }
    #[cfg(debug_assertions)]
    if fake_dir().is_some() {
        return;
    }
    let url = format!("{API}/{key}/gifs/share/{slug}");
    let body = if q.is_empty() { serde_json::json!({}) } else { serde_json::json!({ "q": q }) };
    let _ = std::thread::Builder::new().name("pyr-klipy-share".into()).spawn(move || {
        if let Err(e) = agent().post(&url).header("Content-Type", "application/json").send(body.to_string()) {
            tracing::debug!("KLIPY share report didn't go: {e}");
        }
    });
}

/// A GIF address from KLIPY's media hosts (`static.klipy.com`, `static1.klipy.com`, …).
pub fn is_media(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://") else { return false };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let path = rest[host.len()..].split(['?', '#']).next().unwrap_or("");
    let sub = host.strip_suffix(".klipy.com").or_else(|| host.strip_suffix(".klipy.co"));
    let media_host = sub.is_some_and(|s| s.strip_prefix("static").is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit())));
    let path = path.to_ascii_lowercase();
    media_host && path.len() > 1 && (path.ends_with(".gif") || path.ends_with(".webp"))
}

/// Debug builds only, for trying the picker without KLIPY: `PYR_KLIPY_FAKE=<folder>` answers
/// trending from `trending.json` and searches from `search.json` in that folder.
#[cfg(debug_assertions)]
fn fake_dir() -> Option<std::path::PathBuf> {
    pyr_proto::env_var("KLIPY_FAKE").map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_klipy_results_in_order() {
        let text = r#"{"result":true,"data":{"data":[
            {"id":8041071659142944,"slug":"hello-hi-662","title":"Hello","type":"gif","file":{
              "hd":{"gif":{"url":"https://static.klipy.com/ii/abc/hd.gif","width":640,"height":360,"size":3000000}},
              "md":{"gif":{"url":"https://static.klipy.com/ii/abc/md.gif","width":400,"height":225,"size":900000}},
              "sm":{"gif":{"url":"https://static.klipy.com/ii/abc/sm.gif?v=2","width":220,"height":124,"size":200000}},
              "xs":{"gif":{"url":"https://static1.klipy.com/ii/abc/xs.gif","width":90,"height":51,"size":60000}}}},
            {"id":2,"slug":"ad-1","type":"ad","file":{}},
            {"id":3,"slug":"elsewhere","type":"gif","file":{"md":{"gif":{"url":"https://evil.example/x.gif","width":400,"height":300}}}},
            {"id":4,"slug":"big-only","type":"gif","file":{"sm":{"gif":{"url":"https://static2.klipy.com/b/sm.gif","width":320,"height":240}}}}],
            "current_page":1,"per_page":24,"has_next":true}}"#;
        let v = parse(text).unwrap();
        assert_eq!(v.iter().map(|g| g.slug.as_str()).collect::<Vec<_>>(), ["hello-hi-662", "big-only"], "in KLIPY's order, ads and other hosts left out");
        let g = &v[0];
        assert_eq!(g.provider, Provider::Klipy);
        assert_eq!(g.id, "8041071659142944");
        // xs is too small for the grid: sm (its link exactly as KLIPY gave it). sm is too small
        // for the chat: md.
        assert_eq!(g.preview, "https://static.klipy.com/ii/abc/sm.gif?v=2");
        assert_eq!(g.url, "https://static.klipy.com/ii/abc/md.gif");
        assert_eq!(g.size, Some((400, 225)));
        // Posted with its size (and KLIPY's name for it) after a #, so the address fetched is
        // KLIPY's own.
        let posted = g.posted();
        assert_eq!(posted, "https://static.klipy.com/ii/abc/md.gif#w=400&h=225&slug=hello-hi-662");
        assert_eq!(crate::giphy::link_slug(&posted).as_deref(), Some("hello-hi-662"));
        assert!(is_media(&posted));
        assert_eq!(crate::giphy::link_size(&posted), Some(eframe::egui::vec2(400.0, 225.0)));
        assert_eq!(crate::giphy::gif_links(&format!("look {posted}")), vec![posted.clone()]);
        assert!(parse(r#"{"result":false,"message":"Invalid app key"}"#).is_err());
        assert!(parse("<html>").is_err());
    }

    #[test]
    fn only_klipy_media_and_sane_keys() {
        assert!(is_media("https://static.klipy.com/ii/a/b.gif"));
        assert!(is_media("https://static2.klipy.com/ii/a/b.webp?x=1#w=1&h=2"));
        assert!(is_media("https://static.klipy.co/a.gif"));
        assert!(!is_media("http://static.klipy.com/a.gif"));
        assert!(!is_media("https://api.klipy.com/a.gif"));
        assert!(!is_media("https://static.klipy.com.evil.com/a.gif"));
        assert!(!is_media("https://evil.com/static.klipy.com/a.gif"));
        assert!(!is_media("https://staticx.klipy.com/a.gif"));
        assert!(!is_media("https://static.klipy.com/a.mp4"));
        assert_eq!(key(" abc_DEF-123 ").as_deref(), Some("abc_DEF-123"));
        assert!(!path_safe("a/b") && !path_safe("../etc") && !path_safe("") && !path_safe("a b"));
        assert!(path_safe("hello-hi-662"));
    }
}
