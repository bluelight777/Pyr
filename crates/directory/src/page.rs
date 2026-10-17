//! The page a short link opens: pyr.gg's own page with the invite in it, and the community's
//! name in its title and link preview (what chat apps show when the link is pasted).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use pyr_proto::directory::{Lookup, short_url};

/// pyr.gg's `index.html`, read again when it changes on disk.
pub struct Site {
    dir: Option<PathBuf>,
    cached: Mutex<Option<(SystemTime, Arc<String>)>>,
}

/// What's served when there's no site folder (or no index.html in it): enough for the invite.
const BARE: &str = "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>Pyr</title>\n\
<meta property=\"og:title\" content=\"Pyr\">\n<meta property=\"og:description\" content=\"Voice, video and chat for your group.\">\n\
<meta property=\"og:url\" content=\"https://pyr.gg/\">\n</head>\n<body><p><a href=\"https://pyr.gg/\">Pyr</a></p></body>\n</html>\n";

impl Site {
    pub fn new(dir: Option<PathBuf>) -> Site {
        Site { dir, cached: Mutex::new(None) }
    }

    pub fn index(&self) -> Arc<String> {
        let Some(path) = self.dir.as_ref().map(|d| d.join("index.html")) else { return Arc::new(BARE.to_string()) };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let mut cached = self.cached.lock();
        if let (Some(m), Some((at, text))) = (modified, cached.as_ref()) {
            if *at == m {
                return text.clone();
            }
        }
        match (modified, std::fs::read_to_string(&path)) {
            (Some(m), Ok(text)) => {
                let text = Arc::new(text);
                *cached = Some((m, text.clone()));
                text
            }
            _ => cached.as_ref().map(|(_, t)| t.clone()).unwrap_or_else(|| Arc::new(BARE.to_string())),
        }
    }
}

/// `&`, `<`, `>` and `"` as HTML entities (for text, and attribute values in double quotes,
/// which is how the page writes them).
pub fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

/// JSON that can sit inside a `<script>` element: nothing in it can end the element or start
/// a comment (`<`, `>` and `&` as \u escapes, and the two line separators JavaScript dislikes).
pub fn script_json(v: &serde_json::Value) -> String {
    serde_json::to_string(v)
        .unwrap_or_else(|_| "null".into())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Sets the `content` of the first `<meta {attr}="{key}" content="…">` (left as it is if the
/// page has none).
fn set_meta(html: &mut String, attr: &str, key: &str, content: &str) {
    let start = format!("<meta {attr}=\"{key}\" content=\"");
    let Some(at) = html.find(&start) else { return };
    let from = at + start.len();
    let Some(len) = html[from..].find('"') else { return };
    html.replace_range(from..from + len, &escape_html(content));
}

/// The page at an address that isn't a short link at all (the page's script then doesn't go
/// looking for an invite).
pub fn render_plain(index: &str) -> String {
    let mut html = index.to_string();
    let block = "<script type=\"application/json\" id=\"pyr-invite\">{\"none\":true}</script>\n";
    match html.find("</head>") {
        Some(at) => html.insert_str(at, block),
        None => html.push_str(block),
    }
    html
}

/// The page for `code`: the invite it opens, or (None) that it's no invite (any more).
pub fn render(index: &str, code: &str, invite: Option<&Lookup>) -> String {
    let mut html = index.to_string();
    let url = short_url(code);
    let (title, description) = match invite {
        Some(l) => {
            let mut d = match &l.inviter {
                Some(who) => format!("{who} invited you to join {} on Pyr: voice channels, webcams, screen sharing and chat.", l.name),
                None => format!("You're invited to join {} on Pyr: voice channels, webcams, screen sharing and chat.", l.name),
            };
            match (l.online, l.members) {
                (Some(o), Some(m)) => d.push_str(&format!(" {o} online, {m} {}.", if m == 1 { "member" } else { "members" })),
                (None, Some(m)) => d.push_str(&format!(" {m} {}.", if m == 1 { "member" } else { "members" })),
                _ => {}
            }
            (format!("Join {} on Pyr", l.name), d)
        }
        None => ("Pyr: this invite isn't valid".to_string(), "This invite may have expired, or been taken back. Ask for a new one.".to_string()),
    };
    if let (Some(a), Some(b)) = (html.find("<title>"), html.find("</title>")) {
        if a < b {
            html.replace_range(a + "<title>".len()..b, &escape_html(&title));
        }
    }
    set_meta(&mut html, "name", "description", &description);
    set_meta(&mut html, "property", "og:title", &title);
    set_meta(&mut html, "property", "og:description", &description);
    set_meta(&mut html, "property", "og:url", &url);
    set_meta(&mut html, "name", "twitter:title", &title);
    set_meta(&mut html, "name", "twitter:description", &description);
    // The invite itself, for the page's script (a data block: never run, only read).
    let data = serde_json::json!({ "code": code, "url": url, "invite": invite });
    let block = format!(
        "<meta name=\"robots\" content=\"noindex\">\n<script type=\"application/json\" id=\"pyr-invite\">{}</script>\n",
        script_json(&data)
    );
    match html.find("</head>") {
        Some(at) => html.insert_str(at, &block),
        None => html.push_str(&block),
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "<html><head><title>Pyr: hi</title>\n<meta name=\"description\" content=\"plain\">\n\
<meta property=\"og:title\" content=\"Pyr\">\n<meta property=\"og:description\" content=\"d\">\n\
<meta property=\"og:url\" content=\"https://pyr.gg/\">\n<meta name=\"twitter:title\" content=\"t\">\n</head><body>x</body></html>";

    fn invite(name: &str) -> Lookup {
        Lookup {
            code: "qrslmnov".into(),
            name: name.into(),
            link: "pyr://2.24.121.231/qrslmnov/eIbQ7wcA9qSj".into(),
            members: Some(42),
            online: Some(7),
            inviter: Some("Blue".into()),
            expires_at: None,
            vanity: false,
        }
    }

    #[test]
    fn the_invite_goes_into_the_page_and_its_preview() {
        let html = render(PAGE, "qrslmnov", Some(&invite("Brethren of Blood")));
        assert!(html.contains("<title>Join Brethren of Blood on Pyr</title>"), "{html}");
        assert!(html.contains("<meta property=\"og:title\" content=\"Join Brethren of Blood on Pyr\">"));
        assert!(html.contains("content=\"Blue invited you to join Brethren of Blood on Pyr: voice channels, webcams, screen sharing and chat. 7 online, 42 members.\""));
        assert!(html.contains("<meta property=\"og:url\" content=\"https://pyr.gg/qrslmnov\">"));
        assert!(html.contains("noindex"));
        let at = html.find("id=\"pyr-invite\">").unwrap() + "id=\"pyr-invite\">".len();
        let end = html[at..].find("</script>").unwrap();
        let data: serde_json::Value = serde_json::from_str(&html[at..at + end]).unwrap();
        assert_eq!(data["invite"]["link"], "pyr://2.24.121.231/qrslmnov/eIbQ7wcA9qSj");
        assert_eq!(data["url"], "https://pyr.gg/qrslmnov");
    }

    #[test]
    fn a_name_cant_break_out_of_the_page() {
        let evil = "</script><script>alert(1)</script>\"><img src=x onerror=alert(2)> & '";
        let html = render(PAGE, "qrslmnov", Some(&invite(evil)));
        assert!(!html.contains("<script>alert"), "{html}");
        assert!(!html.contains("<img"), "{html}");
        assert_eq!(html.matches("</script>").count(), 1, "only the data block's own end");
        // It still reads back as the same name.
        let at = html.find("id=\"pyr-invite\">").unwrap() + "id=\"pyr-invite\">".len();
        let end = html[at..].find("</script>").unwrap();
        let data: serde_json::Value = serde_json::from_str(&html[at..at + end]).unwrap();
        assert_eq!(data["invite"]["name"], evil);
    }

    #[test]
    fn no_invite_says_so() {
        let html = render(PAGE, "nosuchxx", None);
        assert!(html.contains("this invite isn't valid"));
        assert!(html.contains("\"invite\":null"));
        // A page with none of the tags still gets the data.
        assert!(render("<p>hi</p>", "abc", None).contains("pyr-invite"));
    }
}
