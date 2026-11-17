//! Games' invites, `https://pyr.gg/<game>/<ROOM>` (Pyr 0.13.18): a program from the Pyr Library,
//! opened in a room (`pyr://play/<game>/<ROOM>`). The page says which game, with its name,
//! summary and icon from the Library's list on the site (`library/index3.json`, or `index2.json`
//! on a site from before there was one), read again when it changes. The list's signature isn't
//! checked here: it's the site's own file, and only words and a picture of the site's go from it
//! into the page (the app checks everything it runs). With no list to read at all, every game is
//! shown by a name made from its id.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use parking_lot::Mutex;
use pyr_proto::directory::DIRECTORY;

/// What a game's invite opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Game {
    /// Its name in the Library (made from its id when the Library has none, or there's no list).
    pub name: String,
    /// The Library's line about it ("" when it has none).
    pub summary: String,
    /// Its icon on the site (`https://pyr.gg/library/files/…`), when it has one there.
    pub icon: Option<String>,
}

/// The longest name and summary a page shows (characters).
const NAME_MAX: usize = 60;
const SUMMARY_MAX: usize = 300;

/// What's never a game: pyr.gg's own folders and pages, server invites' words, and names that
/// would pass for Pyr's own or its staff's (0.13.28's, found by asking its directory some 1,000
/// likely words).
const RESERVED: &[&str] = &[
    "join", "play", "api", "assets", "library", "download", "downloads", "skins", "skin", "pyr", "ingle", "latest", "index",
    "static", "img", "images", "fonts", "css", "js", "favicon", "robots", "sitemap", "help", "about", "privacy", "terms",
    "support", "docs", "doc", "blog", "news", "admin", "login", "signup", "register", "account", "settings", "server",
    "servers", "update", "updates", "release", "releases", "plugin", "plugins", "firelit", "discord", "home", "www", "mail",
    "app", "apps", "dev", "test", "beta", "alpha", "status", "get", "icons", "og", "legal", "security", "faq", "team",
    "windows", "linux", "mac", "macos", "android", "ios", "setup", "install", "signin", "null", "undefined", "root", "owner",
    "mod", "mods", "moderator", "staff", "official", "bot", "bots", "claude", "anthropic",
];

fn reserved(program: &str) -> bool {
    program.starts_with("invite") || RESERVED.contains(&program)
}

/// The game's id (2 to 40 letters, digits and hyphens, not starting or ending with a hyphen)
/// and the room (4 to 8 letters and digits), as written, when `path` is a game's invite
/// (`/<game>/<ROOM>`, a slash after it allowed: see [`canonical`]).
pub fn parse(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix('/')?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let (program, room) = rest.split_once('/')?;
    let p = program.as_bytes();
    let id_ok = (2..=40).contains(&p.len())
        && p.iter().all(|b| b.is_ascii_alphanumeric() || *b == b'-')
        && p[0] != b'-'
        && p[p.len() - 1] != b'-';
    let room_ok = (4..=8).contains(&room.len()) && room.bytes().all(|b| b.is_ascii_alphanumeric());
    (id_ok && room_ok && !reserved(&program.to_ascii_lowercase())).then_some((program, room))
}

/// Where a game's invite is: its id in small letters, its room in capitals (anything else
/// redirects here).
pub fn canonical(program: &str, room: &str) -> String {
    format!("/{}/{}", program.to_ascii_lowercase(), room.to_ascii_uppercase())
}

/// Its address, to share (`https://pyr.gg/pyropoly/K7QX`).
pub fn url(program: &str, room: &str) -> String {
    format!("{DIRECTORY}{}", canonical(program, room))
}

/// What opens it in Pyr (`pyr://play/pyropoly/K7QX`).
pub fn link(program: &str, room: &str) -> String {
    format!("pyr://play{}", canonical(program, room))
}

/// A name from an id, for a game the page knows nothing more about: `torch-and-conquer` is
/// "Torch And Conquer".
pub fn name_from_id(id: &str) -> String {
    id.split('-')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Words from the list as a page shows them: no control characters, at most `max` characters,
/// then no spaces at either end.
fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_string()
}

/// An icon's place in the list (`files/<plugin>/icon.png`, under the site's `library/`) as its
/// address on the site, when it's a plain path there.
fn icon_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("files/")?;
    let plain = !rest.is_empty() && !url.contains("..") && url.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'));
    plain.then(|| format!("{DIRECTORY}/library/{url}"))
}

/// A program in the list.
#[derive(Clone, Debug)]
struct Listed {
    id: String,
    name: String,
    summary: String,
    icon: Option<String>,
}

/// The programs in a list (`{"index": "<the list, as JSON text>", "sig": …}`), read as loosely as
/// it can be: an entry missing something the Library needs is still a game here.
fn programs(text: &str) -> Option<Vec<Listed>> {
    let signed: serde_json::Value = serde_json::from_str(text).ok()?;
    let index: serde_json::Value = serde_json::from_str(signed.get("index")?.as_str()?).ok()?;
    let s = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let plugins = index.get("plugins")?.as_array()?;
    Some(
        plugins
            .iter()
            .filter(|p| p.get("kind").and_then(|k| k.as_str()) == Some("program"))
            .filter_map(|p| {
                Some(Listed {
                    id: p.get("id")?.as_str()?.to_string(),
                    name: s(p, "name"),
                    summary: s(p, "summary"),
                    icon: p.get("icon").and_then(|i| i.get("url")).and_then(|u| u.as_str()).and_then(icon_url),
                })
            })
            .collect(),
    )
}

/// The lists a site can have, newest first: `index3.json`, or `index2.json` on a site from before
/// there was one.
const LISTS: [&str; 2] = ["index3.json", "index2.json"];

/// The Library's list on the site, read again when it changes on disk.
pub struct Library {
    dir: Option<PathBuf>,
    /// Each list as last read (by [`LISTS`]' order): when it was changed, and its programs (None:
    /// it doesn't read as a list).
    cached: Mutex<[Option<(SystemTime, Arc<Option<Vec<Listed>>>)>; 2]>,
}

impl Library {
    /// `site`: pyr.gg's files (its `library/` folder has the list).
    pub fn new(site: Option<PathBuf>) -> Library {
        Library { dir: site.map(|d| d.join("library")), cached: Mutex::new([None, None]) }
    }

    /// The newest list there is that reads as one, as its programs.
    fn list(&self) -> Option<Arc<Option<Vec<Listed>>>> {
        let dir = self.dir.as_ref()?;
        let mut cached = self.cached.lock();
        for (i, name) in LISTS.iter().enumerate() {
            let path = dir.join(name);
            let Some(modified) = std::fs::metadata(&path).and_then(|m| m.modified()).ok() else { continue };
            let list = match &cached[i] {
                Some((at, list)) if *at == modified => list.clone(),
                _ => {
                    let list = Arc::new(std::fs::read_to_string(&path).ok().and_then(|t| programs(&t)));
                    if list.is_none() {
                        warn_unreadable(&path);
                    }
                    cached[i] = Some((modified, list.clone()));
                    list
                }
            };
            if list.is_some() {
                return Some(list);
            }
        }
        None
    }

    /// What `/<program>/<room>` opens: the game, or None when the Library has no such program.
    /// (`program` in small letters.)
    pub fn game(&self, program: &str) -> Option<Game> {
        let Some(list) = self.list() else {
            return Some(Game { name: name_from_id(program), summary: String::new(), icon: None });
        };
        let listed = list.as_ref().as_ref()?.iter().find(|p| p.id == program)?;
        let name = match clean(&listed.name, NAME_MAX) {
            n if n.is_empty() => name_from_id(program),
            n => n,
        };
        Some(Game { name, summary: clean(&listed.summary, SUMMARY_MAX), icon: listed.icon.clone() })
    }
}

fn warn_unreadable(path: &Path) {
    tracing::warn!("can't read the Pyr Library's list {} (games' invites show games by their ids)", path.display());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_game_s_invite_is_a_game_and_a_room() {
        assert_eq!(parse("/pyropoly/K7QX"), Some(("pyropoly", "K7QX")));
        assert_eq!(parse("/Torch-And-Conquer/war1/"), Some(("Torch-And-Conquer", "war1")));
        assert_eq!(canonical("Torch-And-Conquer", "war1"), "/torch-and-conquer/WAR1");
        assert_eq!(parse(&format!("/{}/ABCD", "a".repeat(40))).map(|p| p.0.len()), Some(40));
        for no in [
            "/pyropoly/ABC", "/pyropoly/ABCD12345", "/pyropoly/AB-CD", "/pyropoly/ABCD/x", "/p/ABCD", "/a-/ABCD", "/-a/ABCD",
            "/pyropoly/CHECK%20", "/pyr%6Fpoly/CHECK", "/pyropoly", "/", "/invite/ABCD", "/invites/ABCD", "/inviteX/ABCD",
            "/join/ABCD", "/play/ABCD", "/api/ABCD", "/assets/ABCD", "/library/ABCD", "/download/ABCD", "/downloads/ABCD",
            "/pyropoly/CHECK//", "/skins/ABCD", "/linux/ABCD", "/Staff/ABCD", "/claude/ABCD",
        ] {
            assert_eq!(parse(no), None, "{no}");
        }
        assert_eq!(parse(&format!("/{}/ABCD", "a".repeat(41))), None);
    }

    #[test]
    fn names_from_ids() {
        assert_eq!(name_from_id("torch-and-conquer"), "Torch And Conquer");
        assert_eq!(name_from_id("nosuch"), "Nosuch");
        assert_eq!(name_from_id("a--b"), "A B");
    }

    #[test]
    fn words_from_the_list_are_trimmed_and_cut() {
        assert_eq!(clean("  One  two\nthree\tfour  ", 300), "One  twothreefour");
        // (Cut first: spaces in front count.)
        assert_eq!(clean(&format!("   {}", "n".repeat(70)), 60), "n".repeat(57));
        assert_eq!(clean(&"é".repeat(70), 60).chars().count(), 60);
        // (Cut where a space falls: the space goes too.)
        assert_eq!(clean("abc def", 4), "abc");
    }

    #[test]
    fn icons_only_from_the_library_s_files() {
        assert_eq!(icon_url("files/pyropoly/icon.png").as_deref(), Some("https://pyr.gg/library/files/pyropoly/icon.png"));
        assert_eq!(icon_url("files/X/Icon.PNG").as_deref(), Some("https://pyr.gg/library/files/X/Icon.PNG"));
        for no in ["/files/x/icon.png", "icon.png", "skins/x/icon.png", "files/x y/icon.png", "files/x/ic%20on.png", "files/x/icon.png?v=2", "files/../../etc/x.png", "https://evil.example/x.png", "", "files/", "files/x/ïcon.png"] {
            assert_eq!(icon_url(no), None, "{no}");
        }
    }

    fn site_with(list: Option<&str>) -> (std::path::PathBuf, Library) {
        let dir = std::env::temp_dir().join(format!("pyr-dir-games-{}-{}", std::process::id(), rand_suffix()));
        std::fs::create_dir_all(dir.join("library")).unwrap();
        if let Some(l) = list {
            std::fs::write(dir.join("library/index3.json"), l).unwrap();
        }
        let lib = Library::new(Some(dir.clone()));
        (dir, lib)
    }

    fn rand_suffix() -> u64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos() as u64
    }

    fn list(plugins: serde_json::Value) -> String {
        serde_json::json!({ "index": serde_json::json!({ "issued": 0, "expires": 0, "plugins": plugins, "skins": [] }).to_string(), "sig": "x" }).to_string()
    }

    #[test]
    fn games_come_from_the_list() {
        let (dir, lib) = site_with(Some(&list(serde_json::json!([
            { "id": "pyropoly", "kind": "program", "name": " Pyropoly ", "summary": "Trade.", "icon": { "path": "icon.png", "url": "files/pyropoly/icon.png" } },
            { "id": "dice", "kind": "addon", "name": "Dice", "summary": "Roll." },
            { "id": "nameless", "kind": "program", "name": "", "summary": "" },
        ]))));
        assert_eq!(
            lib.game("pyropoly"),
            Some(Game { name: "Pyropoly".into(), summary: "Trade.".into(), icon: Some("https://pyr.gg/library/files/pyropoly/icon.png".into()) })
        );
        // An add-on isn't a game; nor is what isn't listed.
        assert_eq!(lib.game("dice"), None);
        assert_eq!(lib.game("nosuch"), None);
        assert_eq!(lib.game("nameless").map(|g| g.name), Some("Nameless".into()));
        // A new list is seen without a restart.
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.join("library/index3.json"), list(serde_json::json!([{ "id": "pyropoly", "kind": "program", "name": "Pyropoly 2" }]))).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(2);
        let f = std::fs::File::options().write(true).open(dir.join("library/index3.json")).unwrap();
        f.set_modified(later).unwrap();
        assert_eq!(lib.game("pyropoly").map(|g| g.name), Some("Pyropoly 2".into()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn with_no_list_every_game_is_shown_by_its_id() {
        let (dir, lib) = site_with(Some("not a list"));
        assert_eq!(lib.game("torch-and-conquer"), Some(Game { name: "Torch And Conquer".into(), summary: String::new(), icon: None }));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Library::new(None).game("nosuch").map(|g| g.name), Some("Nosuch".into()));
        // A list with no plugins in it at all isn't one; an empty one is.
        let no_plugins = serde_json::json!({ "index": serde_json::json!({ "issued": 0, "skins": [] }).to_string(), "sig": "x" }).to_string();
        let (dir, lib) = site_with(Some(&no_plugins));
        assert_eq!(lib.game("pyropoly").map(|g| g.name), Some("Pyropoly".into()));
        let _ = std::fs::remove_dir_all(&dir);
        let (dir, lib) = site_with(Some(&list(serde_json::json!([]))));
        assert_eq!(lib.game("pyropoly"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
