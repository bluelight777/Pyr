//! Plugins: programs and add-ons from the Ingle Library (see `ingle_proto::library`).
//!
//! - `library`: the signed list, fetched from firelitgames.com.
//! - `install`: downloading, checking, installing, opening and removing.
//! - `programs`: programs running inside Ingle's window.
//! - `addon`: running add-ons (WebAssembly, sandboxed) and talking to them.
//! - `source`: gathering a plugin's source to send in, and starting a new add-on.
//! - `skins`: skins from the library (installing, reading back; wearing one is `crate::looks`).
//! - `ui`: the Plugins page, add-on panels and the owner's review lists.

pub mod addon;
pub mod install;
pub mod library;
pub mod programs;
pub mod skins;
pub mod source;
pub mod ui;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use ingle_proto::control::{ChannelId, MessageId};
use ingle_proto::library::{Entry, Kind, Library, Permission, SkinEntry};
use parking_lot::Mutex;

use addon::{AddOn, Panel, Place, Request, Response};
use install::{Installed, InstalledPlugin};

/// Which tab of the Plugins page is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Library,
    Installed,
    Make,
}

/// What the Library shows (plugins and skins together, or one kind, or the favorites).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Show {
    #[default]
    All,
    Programs,
    Addons,
    Skins,
    Favorites,
}

/// What background work hands back to the window.
#[derive(Default)]
struct Work {
    fetching: bool,
    fetched: Option<Result<Library, String>>,
    /// Installs in progress: (bytes done, total), by plugin id (`skin:<id>` for a skin).
    installing: HashMap<String, (u64, u64)>,
    installed: Vec<(String, Result<InstalledPlugin, String>)>,
    installed_skins: Vec<Result<skins::InstalledSkin, String>>,
    /// Add-ons started in the background: (id, what came of it, tried rather than installed).
    started: Vec<(String, Result<AddOn, String>, bool)>,
    icons: Vec<(String, Vec<u8>)>,
}

/// An open add-on panel.
pub struct OpenPanel {
    pub id: String,
    pub panel: Option<Panel>,
    pub error: Option<String>,
    /// The text boxes' contents, by widget id.
    pub inputs: HashMap<String, String>,
    pub open: bool,
}

/// Where an answer goes when it comes.
enum Waiting {
    /// A /command typed in a channel: its answer is sent there.
    Command { key: String, channel: ChannelId, reply_to: Option<MessageId>, place: Place },
    /// A panel's drawing (or a click in it, which may also send a message).
    Panel { key: Option<(String, ChannelId)> },
}

struct Pending {
    id: String,
    rx: std::sync::mpsc::Receiver<Result<Response, String>>,
    waiting: Waiting,
    since: Instant,
}

/// What the window should do with an answer.
pub enum Outcome {
    Send { key: String, channel: ChannelId, text: String, reply_to: Option<MessageId> },
    Toast(String, bool),
}

pub struct Plugins {
    pub dir: PathBuf,
    pub index_url: String,
    pub library: Option<Library>,
    pub library_error: Option<String>,
    fetched_at: Option<Instant>,
    pub installed: Installed,
    /// Running add-ons, by id.
    pub addons: HashMap<String, AddOn>,
    /// Add-ons that couldn't start, and why (by id).
    pub broken: HashMap<String, String>,
    /// The id of an add-on being tried out from a file (not installed), if any.
    pub tried: Option<String>,
    pub icons: HashMap<String, Arc<Vec<u8>>>,
    work: Arc<Mutex<Work>>,
    repaint: Arc<dyn Fn() + Send + Sync>,
    pending: Vec<Pending>,
    pub panels: Vec<OpenPanel>,
    // The page.
    pub tab: Tab,
    pub search: String,
    pub detail: Option<String>,
    /// The submission form: the folder's gathered source and what's typed in.
    pub form: Option<ui::SubmitForm>,
    /// Asked to remove this plugin (waiting for a yes).
    pub confirm_remove: Option<String>,
    /// The name for a new add-on ("Start an add-on").
    pub new_name: String,
    /// Programs running (inside Ingle, on Windows).
    pub programs: programs::Programs,
    /// The program whose page is showing (instead of the Plugins page).
    pub program_page: Option<String>,
    /// What the Library shows, and only one author's (clicked under a plugin or skin).
    pub show: Show,
    pub author: Option<String>,
    /// Skins installed from the Library, and where.
    pub skins: skins::InstalledSkins,
    pub skins_dir: PathBuf,
}

impl Plugins {
    pub fn new(config_dir: &std::path::Path, repaint: Arc<dyn Fn() + Send + Sync>) -> Plugins {
        let dir = install::plugins_dir(config_dir);
        let installed = Installed::load(&dir);
        let skins_dir = skins::skins_dir(config_dir);
        let skins = skins::InstalledSkins::load(&skins_dir);
        let library = library::cached(&dir);
        let p = Plugins {
            index_url: library::index_url(),
            library,
            library_error: None,
            fetched_at: None,
            installed,
            addons: HashMap::new(),
            broken: HashMap::new(),
            tried: None,
            icons: HashMap::new(),
            work: Arc::new(Mutex::new(Work::default())),
            repaint,
            pending: Vec::new(),
            panels: Vec::new(),
            tab: Tab::default(),
            search: String::new(),
            detail: None,
            form: None,
            confirm_remove: None,
            new_name: String::new(),
            programs: Default::default(),
            program_page: None,
            show: Show::default(),
            author: None,
            skins,
            skins_dir,
            dir,
        };
        p.start_installed_addons();
        p
    }

    /// Starts every installed add-on that's switched on (and still in the library, if the
    /// list is known), each after checking its files, in the background.
    fn start_installed_addons(&self) {
        let todo: Vec<InstalledPlugin> = self
            .installed
            .plugins
            .iter()
            .filter(|p| p.kind == Kind::Addon && p.enabled && self.listed(&p.id) != Some(false))
            .cloned()
            .collect();
        for p in todo {
            self.start_addon_bg(p);
        }
    }

    fn start_addon_bg(&self, p: InstalledPlugin) {
        let (dir, work, repaint) = (self.dir.clone(), self.work.clone(), self.repaint.clone());
        let _ = std::thread::Builder::new().name("ingle-addon-start".into()).spawn(move || {
            let r = install::verify(&dir, &p)
                .and_then(|_| std::fs::read(dir.join(&p.id).join(&p.run)).map_err(|e| e.to_string()))
                .and_then(|bytes| AddOn::load(&p.id, bytes, Some(install::data_path(&dir, &p.id)), repaint.clone()));
            work.lock().started.push((p.id.clone(), r, false));
            repaint();
        });
    }

    /// Whether a plugin is in the library's list (`None`: the list isn't known yet).
    pub fn listed(&self, id: &str) -> Option<bool> {
        self.library.as_ref().map(|l| l.find(id).is_some())
    }

    /// The library's entry for an installed plugin, if it has a newer version.
    pub fn update_for(&self, p: &InstalledPlugin) -> Option<&Entry> {
        let e = self.library.as_ref()?.find(&p.id)?;
        (e.version != p.version && !e.files_for(install::os()).is_empty()).then_some(e)
    }

    /// The library's entry for an installed skin, if it has a newer version.
    pub fn skin_update_for(&self, s: &skins::InstalledSkin) -> Option<&SkinEntry> {
        let e = self.library.as_ref()?.find_skin(&s.id)?;
        (e.version != s.version).then_some(e)
    }

    /// How many installed plugins and skins have an update.
    pub fn updates(&self) -> usize {
        self.installed.plugins.iter().filter(|p| self.update_for(p).is_some()).count() + self.skins.skins.iter().filter(|s| self.skin_update_for(s).is_some()).count()
    }

    /// Installs (or updates) a skin from the library, in the background.
    pub fn install_skin(&mut self, entry: &SkinEntry) {
        let key = format!("skin:{}", entry.id);
        if self.work.lock().installing.contains_key(&key) {
            return;
        }
        let (dir, url, work, repaint, entry) = (self.skins_dir.clone(), self.index_url.clone(), self.work.clone(), self.repaint.clone(), entry.clone());
        work.lock().installing.insert(key.clone(), (0, entry.size()));
        let _ = std::thread::Builder::new().name("ingle-skin-install".into()).spawn(move || {
            let _ = std::fs::create_dir_all(&dir);
            let w2 = work.clone();
            let k2 = key.clone();
            let r = skins::install(&dir, &url, &entry, |done, total| {
                w2.lock().installing.insert(k2.clone(), (done, total));
                repaint();
            });
            let mut w = work.lock();
            w.installing.remove(&key);
            w.installed_skins.push(r);
            drop(w);
            repaint();
        });
    }

    /// Removes an installed skin (it's taken off first, if it's on: see the caller).
    pub fn uninstall_skin(&mut self, id: &str) -> Result<(), String> {
        skins::uninstall(&self.skins_dir, id)?;
        self.skins.skins.retain(|s| s.id != id);
        self.skins.save(&self.skins_dir)
    }

    /// A skin's files as installed (checked against what was installed).
    pub fn skin_files(&self, id: &str) -> Option<Result<Vec<(String, Vec<u8>)>, String>> {
        self.skins.get(id).map(|s| skins::files(&self.skins_dir, s))
    }

    /// Fetches the list (if it isn't already being fetched, and wasn't just now).
    pub fn refresh(&mut self, force: bool) {
        if !force && self.fetched_at.is_some_and(|t| t.elapsed() < Duration::from_secs(10 * 60)) {
            return;
        }
        {
            let mut w = self.work.lock();
            if w.fetching {
                return;
            }
            w.fetching = true;
        }
        self.fetched_at = Some(Instant::now());
        let (dir, work, repaint) = (self.dir.clone(), self.work.clone(), self.repaint.clone());
        let _ = std::thread::Builder::new().name("ingle-library".into()).spawn(move || {
            let _ = std::fs::create_dir_all(&dir);
            let r = library::fetch(&dir);
            let mut w = work.lock();
            w.fetching = false;
            w.fetched = Some(r);
            drop(w);
            repaint();
        });
    }

    pub fn fetching(&self) -> bool {
        self.work.lock().fetching
    }

    /// An install (or update) in progress: bytes done and in all.
    pub fn progress(&self, id: &str) -> Option<(u64, u64)> {
        self.work.lock().installing.get(id).copied()
    }

    pub fn install(&mut self, entry: &Entry) {
        if self.work.lock().installing.contains_key(&entry.id) {
            return;
        }
        // An add-on being updated is stopped first (it starts again from the new files).
        self.addons.remove(&entry.id);
        let (dir, url, work, repaint, entry) = (self.dir.clone(), self.index_url.clone(), self.work.clone(), self.repaint.clone(), entry.clone());
        work.lock().installing.insert(entry.id.clone(), (0, entry.size_for(install::os())));
        let _ = std::thread::Builder::new().name("ingle-plugin-install".into()).spawn(move || {
            let w2 = work.clone();
            let id = entry.id.clone();
            let r = install::install(&dir, &url, &entry, |done, total| {
                w2.lock().installing.insert(id.clone(), (done, total));
                repaint();
            });
            let mut w = work.lock();
            w.installing.remove(&entry.id);
            w.installed.push((entry.id.clone(), r));
            drop(w);
            repaint();
        });
    }

    pub fn uninstall(&mut self, id: &str) -> Result<(), String> {
        if let Some(p) = self.programs.get(id) {
            return Err(format!("{} is open: close it first.", p.name));
        }
        self.addons.remove(id);
        self.panels.retain(|p| p.id != id);
        install::uninstall(&self.dir, id)?;
        self.installed.plugins.retain(|p| p.id != id);
        self.installed.save(&self.dir)
    }

    /// Opens a program (inside Ingle on Windows), or finds it open already.
    pub fn open_program(&mut self, id: &str) -> Result<(), String> {
        let p = self.installed.get(id).ok_or("it isn't installed")?.clone();
        if self.listed(id) == Some(false) {
            return Err(format!("{} was taken out of the Ingle Library, so Ingle won't open it. Remove it.", p.name));
        }
        self.programs.start(&self.dir, &p)
    }

    /// Switches an installed add-on on or off.
    pub fn set_enabled(&mut self, id: &str, on: bool) {
        let Some(p) = self.installed.plugins.iter_mut().find(|p| p.id == id) else { return };
        p.enabled = on;
        let p = p.clone();
        let _ = self.installed.save(&self.dir);
        if on {
            self.broken.remove(id);
            self.start_addon_bg(p);
        } else {
            self.addons.remove(id);
            self.panels.retain(|q| q.id != id);
        }
    }

    /// Tries an add-on from a file (not installed, nothing kept): sandboxed like any other.
    pub fn try_addon(&mut self, bytes: Vec<u8>) {
        if let Some(old) = self.tried.take() {
            self.addons.remove(&old);
            self.panels.retain(|p| p.id != old);
        }
        let (work, repaint) = (self.work.clone(), self.repaint.clone());
        let _ = std::thread::Builder::new().name("ingle-addon-try".into()).spawn(move || {
            let id = "trying".to_string();
            let r = AddOn::load(&id, bytes, None, repaint.clone());
            work.lock().started.push((id, r, true));
            repaint();
        });
    }

    /// Whether an add-on may send messages (it said so, and was installed saying so; one
    /// being tried may).
    fn may_send(&self, id: &str) -> bool {
        self.tried.as_deref() == Some(id) || self.installed.get(id).is_some_and(|p| p.permissions.contains(&Permission::Commands))
    }

    /// `/name args` in the message box, when it's one of a running add-on's commands: the
    /// add-on, the command and the rest.
    pub fn command_in(&self, text: &str) -> Option<(String, String, String)> {
        let rest = text.strip_prefix('/')?;
        let (name, args) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
        let name = name.to_lowercase();
        // Ingle's own come first.
        if ["giphy", "gif", "klipy"].contains(&name.as_str()) {
            return None;
        }
        let mut ids: Vec<&String> = self.addons.keys().collect();
        ids.sort();
        let id = ids.into_iter().find(|id| self.addons[*id].has_command(&name) && self.may_send(id))?;
        Some((id.clone(), name, args.trim().to_string()))
    }

    /// Every running add-on's commands that start with `typed` (for the hint above the box).
    pub fn commands_like(&self, typed: &str) -> Vec<(String, String)> {
        let typed = typed.to_lowercase();
        let mut out: Vec<(String, String)> = self
            .addons
            .iter()
            .filter(|(id, _)| self.may_send(id))
            .flat_map(|(_, a)| a.info.commands.iter().map(|c| (c.name.clone(), format!("{}: {}", a.info.name, c.help))))
            .filter(|(n, _)| n.starts_with(&typed))
            .collect();
        out.sort();
        out.truncate(8);
        out
    }

    pub fn run_command(&mut self, id: &str, name: &str, args: &str, place: Place, key: String, channel: ChannelId, reply_to: Option<MessageId>) {
        let Some(a) = self.addons.get(id) else { return };
        let rx = a.ask(Request::Command { name: name.to_string(), args: args.to_string() }, place.clone());
        self.pending.push(Pending { id: id.to_string(), rx, waiting: Waiting::Command { key, channel, reply_to, place }, since: Instant::now() });
    }

    /// Opens (or brings back) an add-on's panel.
    pub fn open_panel(&mut self, id: &str, place: Place, here: Option<(String, ChannelId)>) {
        if !self.panels.iter().any(|p| p.id == id) {
            self.panels.push(OpenPanel { id: id.to_string(), panel: None, error: None, inputs: HashMap::new(), open: true });
        }
        if let Some(a) = self.addons.get(id) {
            let rx = a.ask(Request::Panel, place);
            self.pending.push(Pending { id: id.to_string(), rx, waiting: Waiting::Panel { key: here }, since: Instant::now() });
        }
    }

    /// A click (or Enter in a text box) in an add-on's panel.
    pub fn panel_event(&mut self, id: &str, widget: &str, value: String, place: Place, here: Option<(String, ChannelId)>) {
        let Some(a) = self.addons.get(id) else { return };
        let rx = a.ask(Request::Event { id: widget.to_string(), value }, place);
        self.pending.push(Pending { id: id.to_string(), rx, waiting: Waiting::Panel { key: here }, since: Instant::now() });
    }

    /// Something's being worked on for this add-on (its panel shows it).
    pub fn busy(&self, id: &str) -> bool {
        self.pending.iter().any(|p| p.id == id)
    }

    /// Takes in what background work and add-ons handed back; returns what the window should
    /// do (send messages, show notices).
    pub fn step(&mut self) -> Vec<Outcome> {
        let mut out = Vec::new();
        let (fetched, installed, started, icons, installed_skins) = {
            let mut w = self.work.lock();
            (w.fetched.take(), std::mem::take(&mut w.installed), std::mem::take(&mut w.started), std::mem::take(&mut w.icons), std::mem::take(&mut w.installed_skins))
        };
        for r in installed_skins {
            match r {
                Ok(s) => {
                    let name = s.name.clone();
                    self.skins.put(s);
                    if let Err(e) = self.skins.save(&self.skins_dir) {
                        out.push(Outcome::Toast(format!("Couldn't save the list of installed skins: {e}"), true));
                    }
                    out.push(Outcome::Toast(format!("{name} is installed: Wear it from its card, or in Settings → Appearance."), false));
                }
                Err(e) => out.push(Outcome::Toast(format!("Couldn't install the skin: {e}"), true)),
            }
        }
        if let Some(r) = fetched {
            match r {
                Ok(lib) => {
                    self.library = Some(lib);
                    self.library_error = None;
                    self.fetch_icons();
                    // Anything taken out of the library stops.
                    let gone: Vec<String> = self.addons.keys().filter(|id| self.tried.as_deref() != Some(id.as_str()) && self.listed(id) == Some(false)).cloned().collect();
                    for id in gone {
                        self.addons.remove(&id);
                        self.panels.retain(|p| p.id != id);
                        out.push(Outcome::Toast(format!("{id} was taken out of the Ingle Library, so it was turned off."), true));
                    }
                }
                Err(e) => {
                    tracing::info!("plugin list: {e}");
                    self.library_error = Some(e);
                }
            }
        }
        for (id, r) in installed {
            match r {
                Ok(p) => {
                    let name = p.name.clone();
                    let kind = p.kind;
                    self.installed.put(p.clone());
                    if let Err(e) = self.installed.save(&self.dir) {
                        out.push(Outcome::Toast(format!("Couldn't save the list of installed plugins: {e}"), true));
                    }
                    if kind == Kind::Addon {
                        self.broken.remove(&id);
                        self.start_addon_bg(p);
                    }
                    out.push(Outcome::Toast(format!("{name} is installed."), false));
                }
                Err(e) => out.push(Outcome::Toast(format!("Couldn't install it: {e}"), true)),
            }
        }
        for (id, r, tried) in started {
            match r {
                Ok(a) => {
                    if tried {
                        out.push(Outcome::Toast(format!("Trying {}: its commands work in chat, and its panel is open.", a.info.name), false));
                        self.tried = Some(id.clone());
                        self.addons.insert(id.clone(), a);
                        self.open_panel(&id, Place::default(), None);
                    } else {
                        self.broken.remove(&id);
                        self.addons.insert(id, a);
                    }
                }
                Err(e) => {
                    if tried {
                        out.push(Outcome::Toast(format!("That add-on didn't start: {e}"), true));
                    } else {
                        tracing::warn!("add-on {id} didn't start: {e}");
                        self.broken.insert(id, e);
                    }
                }
            }
        }
        for (id, bytes) in icons {
            self.icons.insert(id, Arc::new(bytes));
        }
        // Answers.
        let mut i = 0;
        while i < self.pending.len() {
            let answer = match self.pending[i].rx.try_recv() {
                Ok(r) => r,
                Err(TryRecvError::Empty) => {
                    i += 1;
                    continue;
                }
                Err(TryRecvError::Disconnected) => Err("the add-on stopped".into()),
            };
            let p = self.pending.remove(i);
            let name = self.addons.get(&p.id).map(|a| a.info.name.clone()).unwrap_or_else(|| p.id.clone());
            let may_send = self.may_send(&p.id);
            match (p.waiting, answer) {
                (Waiting::Command { key, channel, reply_to, place }, Ok(r)) => {
                    // Its panel, when open, may show what the command changed (Dice's last rolls).
                    if r.error.is_none() && self.panels.iter().any(|x| x.id == p.id && x.open) {
                        self.open_panel(&p.id, place, Some((key.clone(), channel)));
                    }
                    match (r.say, r.error) {
                        (_, Some(e)) => out.push(Outcome::Toast(format!("{name}: {e}"), false)),
                        (Some(text), None) if !text.trim().is_empty() => out.push(Outcome::Send { key, channel, text, reply_to }),
                        _ => {}
                    }
                }
                (Waiting::Panel { key }, Ok(r)) => {
                    if let Some(panel) = self.panels.iter_mut().find(|x| x.id == p.id) {
                        if let Some(pn) = r.panel {
                            panel.panel = Some(pn);
                        }
                        panel.error = r.error.clone();
                    }
                    if let Some(text) = r.say.filter(|t| !t.trim().is_empty()) {
                        match key {
                            Some((key, channel)) if may_send => out.push(Outcome::Send { key, channel, text, reply_to: None }),
                            Some(_) => {}
                            None => out.push(Outcome::Toast("Open a channel first: that goes to the channel you have open.".into(), false)),
                        }
                    }
                }
                (waiting, Err(e)) => {
                    out.push(Outcome::Toast(format!("{name} stopped: {e}. Turn it off and on again (Plugins → Installed) to restart it."), true));
                    if let Waiting::Panel { .. } = waiting {
                        if let Some(panel) = self.panels.iter_mut().find(|x| x.id == p.id) {
                            panel.error = Some(e.clone());
                        }
                    }
                    self.broken.insert(p.id.clone(), e);
                    self.addons.remove(&p.id);
                }
            }
            let _ = p.since;
        }
        self.panels.retain(|p| p.open);
        out
    }

    /// Fetches the icons of the plugins in the list, and the skins' pictures (once each; a
    /// skin's as `skin:<id>`).
    fn fetch_icons(&self) {
        let Some(lib) = &self.library else { return };
        let mut todo: Vec<(String, ingle_proto::library::File)> =
            lib.plugins.iter().filter(|p| !self.icons.contains_key(&p.id)).filter_map(|p| p.icon.clone().map(|i| (p.id.clone(), i))).collect();
        todo.extend(
            lib.skins
                .iter()
                .filter(|s| !self.icons.contains_key(&format!("skin:{}", s.id)))
                .filter_map(|s| s.preview.clone().map(|f| (format!("skin:{}", s.id), f))),
        );
        if todo.is_empty() {
            return;
        }
        let (url, work, repaint) = (self.index_url.clone(), self.work.clone(), self.repaint.clone());
        let _ = std::thread::Builder::new().name("ingle-plugin-icons".into()).spawn(move || {
            for (id, f) in todo {
                if f.size as usize > ingle_proto::skin::MAX_FILE {
                    continue;
                }
                let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).build().into();
                let Ok(mut resp) = agent.get(&library::file_url(&url, &f.url)).call() else { continue };
                let Ok(bytes) = resp.body_mut().with_config().limit(f.size + 1).read_to_vec() else { continue };
                if ingle_proto::release::sha256_hex(&bytes) == f.sha256 {
                    work.lock().icons.push((id, bytes));
                    repaint();
                }
            }
        });
    }
}
