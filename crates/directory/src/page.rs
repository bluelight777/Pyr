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

/// What's served when the site's page can't be read: still the invite, with the community's
/// name and icon and a button to open it in Pyr (`render` fills in the title and the invite;
/// the script shows them). Never a blank page.
const BARE: &str = r##"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Pyr</title>
<meta name="description" content="Voice, video and chat for your group.">
<meta property="og:title" content="Pyr">
<meta property="og:description" content="Voice, video and chat for your group.">
<meta property="og:url" content="https://pyr.gg/">
<meta property="og:image" content="https://pyr.gg/assets/img/og.jpg">
<style>
:root { color-scheme: dark; }
body { margin: 0; min-height: 100vh; display: grid; place-items: center; background: #14100d; color: #f3ece6;
  font: 16px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif; }
.card { width: min(420px, calc(100vw - 32px)); padding: 36px 28px; border-radius: 22px; background: #1f1813;
  border: 1px solid #3a2c22; text-align: center; box-shadow: 0 30px 80px rgba(0,0,0,.5); }
.badge { width: 86px; height: 86px; margin: 0 auto 16px; border-radius: 26px; display: grid; place-items: center; overflow: hidden;
  font: 800 38px/1 system-ui, sans-serif; color: #1d120b; background: linear-gradient(160deg, #ffd08a, #ff7a3d 70%, #e2492b); }
.badge img { width: 100%; height: 100%; object-fit: cover; }
.lead { color: #b8a99c; margin: 0; }
h1 { margin: 4px 0 8px; font-size: 30px; line-height: 1.15; overflow-wrap: anywhere; }
.counts { color: #b8a99c; font-size: 14px; margin: 0 0 22px; }
a.btn { display: block; margin-top: 12px; padding: 13px 18px; border-radius: 12px; font-weight: 700; text-decoration: none; }
a.fire { background: linear-gradient(180deg, #ff8a4a, #e8552c); color: #fff; }
a.plain { background: #2b211a; color: #f3ece6; border: 1px solid #3a2c22; }
.note { color: #8f8176; font-size: 13px; margin-top: 18px; }
</style>
</head>
<body>
<main class="card">
  <div class="badge" id="badge">P</div>
  <p class="lead" id="lead">Pyr</p>
  <h1 id="name">Voice, video and chat for your group</h1>
  <p class="counts" id="counts"></p>
  <a class="btn fire" id="open" href="https://pyr.gg/">Open in Pyr</a>
  <a class="btn plain" href="https://pyr.gg/">Get Pyr (free, Windows and Linux)</a>
  <p class="note" id="note">Pyr shows you the server and asks before joining.</p>
</main>
<script>
(function () {
  var el = document.getElementById("pyr-invite"), d = null;
  try { d = el ? JSON.parse(el.textContent) : null; } catch (e) {}
  var $ = function (id) { return document.getElementById(id); };
  if (!d || d.none) return;
  var inv = d.invite;
  if (!inv) { $("lead").textContent = "Sorry,"; $("name").textContent = "this invite isn't valid"; $("open").remove(); $("note").textContent = "It may have expired, or been taken back. Ask for a new one."; return; }
  $("lead").textContent = inv.inviter ? inv.inviter + " invited you to join" : "You've been invited to join";
  $("name").textContent = inv.name || "a Pyr server";
  $("badge").textContent = ((inv.name || "P").replace(/^[^A-Za-z0-9À-￿]+/, "").charAt(0) || "P").toUpperCase();
  if (inv.icon && inv.code) { var img = new Image(); img.alt = ""; img.onload = function () { $("badge").textContent = ""; $("badge").appendChild(img); }; img.src = "/api/v1/icons/" + encodeURIComponent(inv.code) + ".png"; }
  var c = [];
  if (typeof inv.online === "number") c.push(inv.online + " online");
  if (typeof inv.members === "number") c.push(inv.members + (inv.members === 1 ? " member" : " members"));
  $("counts").textContent = c.join(" · ");
  if (/^pyr:\/\//i.test(inv.link || "")) $("open").href = inv.link;
})();
</script>
</body>
</html>
"##;

impl Site {
    pub fn new(dir: Option<PathBuf>) -> Site {
        Site { dir, cached: Mutex::new(None) }
    }

    pub fn index(&self) -> Arc<String> {
        let Some(path) = self.dir.as_ref().map(|d| d.join("index.html")) else { return Arc::new(BARE.to_string()) };
        let read = |path: &std::path::Path| {
            std::fs::read_to_string(path).inspect_err(|e| {
                // (Said each time: it's what makes every invite the plain stand-in page.)
                tracing::error!("can't read the site's page {}: {e} (invites show a plain page instead)", path.display());
            })
        };
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        let mut cached = self.cached.lock();
        if let (Some(m), Some((at, text))) = (modified, cached.as_ref()) {
            if *at == m {
                return text.clone();
            }
        }
        match (modified, read(&path)) {
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
    // The community's icon in link previews (Discord, texts, …): a square picture, so the
    // small card.
    if let Some(l) = invite.filter(|l| l.icon) {
        let icon = pyr_proto::directory::icon_url(&l.code);
        set_meta(&mut html, "property", "og:image", &icon);
        set_meta(&mut html, "name", "twitter:image", &icon);
        set_meta(&mut html, "name", "twitter:card", "summary");
        for gone in ["<meta property=\"og:image:width\"", "<meta property=\"og:image:height\""] {
            if let Some(at) = html.find(gone) {
                if let Some(len) = html[at..].find('>') {
                    html.replace_range(at..at + len + 1, "");
                }
            }
        }
        set_meta(&mut html, "property", "og:image:alt", &l.name);
    }
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
            icon: false,
        }
    }

    #[test]
    fn a_community_with_an_icon_shows_it_in_link_previews() {
        let page = PAGE.replace("</head>", "<meta property=\"og:image\" content=\"https://pyr.gg/assets/img/og.jpg\">\n<meta property=\"og:image:width\" content=\"1200\">\n<meta name=\"twitter:card\" content=\"summary_large_image\">\n</head>");
        let html = render(&page, "qrslmnov", Some(&Lookup { icon: true, ..invite("Brethren of Blood") }));
        assert!(html.contains("<meta property=\"og:image\" content=\"https://pyr.gg/api/v1/icons/qrslmnov.png\">"), "{html}");
        assert!(html.contains("<meta name=\"twitter:card\" content=\"summary\">"));
        assert!(!html.contains("og:image:width"));
        // Without one, the site's own picture stays.
        let html = render(&page, "qrslmnov", Some(&invite("Brethren of Blood")));
        assert!(html.contains("assets/img/og.jpg"));
    }

    #[test]
    fn the_stand_in_page_still_shows_the_invite() {
        let html = render(BARE, "qrslmnov", Some(&invite("Brethren of Blood")));
        assert!(html.contains("<title>Join Brethren of Blood on Pyr</title>"));
        assert!(html.contains("id=\"pyr-invite\""));
        assert!(html.contains("Open in Pyr"));
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
