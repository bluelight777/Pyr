//! How Ingle looks and sounds now: the skin in use (one installed from the Ingle Library, or
//! Campfire, Ingle's own), or one being tried for a while without installing it: a skin Claude
//! is making with you (the Claude connector puts it in the `skin-preview` folder next to
//! Ingle's settings, see [`PREVIEW_DIR`]), one you're making in a folder of your own (worn
//! again whenever its files change), or one sent in that the owner of a server collecting them
//! is looking at. Putting a skin on sets the palette (`theme`), its fonts (each parsed first),
//! the picture behind the chat and its sounds.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Color32, Rect, pos2};
use ingle_engine::Engine;
use ingle_proto::skin::Use;

use crate::theme;

/// Where the Claude connector puts a skin to try (`ingle-mcp`'s `preview_skin`): its files,
/// then the stamp saying which (written last, and removed by `stop_preview`).
pub use ingle_engine::skins::{PREVIEW_DIR, PREVIEW_STAMP};

/// A skin tried for a while instead of the one in use.
#[derive(Clone, Debug, PartialEq)]
pub enum Trial {
    /// From the Claude connector; `stamp` tells one preview from the next.
    Claude { stamp: String },
    /// Sent in to a server I own (`key`), not approved yet.
    SentIn { key: String, id: i64 },
    /// A folder on this PC (a skin someone's making by hand).
    Folder { path: PathBuf },
}

pub struct Trying {
    pub what: Trial,
    pub name: String,
    files: Vec<(String, Vec<u8>)>,
    /// Counts the times its files changed (each is worn again).
    rev: u32,
    /// A folder's files as they were when read (size and time), to see when they change.
    seen: Vec<(String, u64, Option<SystemTime>)>,
    checked: Option<Instant>,
}

impl Trying {
    fn new(what: Trial, name: String, files: Vec<(String, Vec<u8>)>) -> Trying {
        let seen = match &what {
            Trial::Folder { path } => stamps(path, &files),
            _ => Vec::new(),
        };
        Trying { what, name, files, rev: 0, seen, checked: Some(Instant::now()) }
    }
}

/// Each file's size and when it was last changed (skin.json first: what it names can change).
fn stamps(folder: &Path, files: &[(String, Vec<u8>)]) -> Vec<(String, u64, Option<SystemTime>)> {
    let mut paths: Vec<&str> = vec![ingle_proto::skin::MANIFEST];
    paths.extend(files.iter().map(|(p, _)| p.as_str()).filter(|p| *p != ingle_proto::skin::MANIFEST));
    paths
        .into_iter()
        .map(|p| {
            let m = std::fs::metadata(folder.join(p)).ok();
            (p.to_string(), m.as_ref().map(|m| m.len()).unwrap_or(u64::MAX), m.and_then(|m| m.modified().ok()))
        })
        .collect()
}

/// What's on now.
struct Worn {
    /// "" (Campfire), "skin:<id>" or "try".
    what: String,
    picture: Option<egui::TextureHandle>,
    fonts: bool,
}

pub struct Looks {
    config_dir: PathBuf,
    pub trying: Option<Trying>,
    worn: Option<Worn>,
    /// Why the skin wanted couldn't be put on (it's Campfire meanwhile).
    pub problem: Option<String>,
    preview_checked: Option<Instant>,
    preview_stamp: Option<String>,
}

impl Looks {
    pub fn new(config_dir: PathBuf) -> Looks {
        // A preview Claude asked for a while ago (and never stopped) isn't put on again when
        // Ingle starts; one from the last few minutes is.
        let stamp = config_dir.join(PREVIEW_STAMP);
        let recent = std::fs::metadata(&stamp).ok().and_then(|m| m.modified().ok()).and_then(|t| t.elapsed().ok()).is_some_and(|age| age < Duration::from_secs(600));
        let preview_stamp = if recent { None } else { std::fs::read_to_string(&stamp).ok().map(|s| s.trim().to_string()) };
        Looks { config_dir, trying: None, worn: None, problem: None, preview_checked: None, preview_stamp }
    }

    /// Try `files` (a skin sent in) instead of the one in use, until `stop_trying`.
    pub fn try_on(&mut self, what: Trial, name: String, files: Vec<(String, Vec<u8>)>) {
        self.trying = Some(Trying::new(what, name, files));
    }

    /// Back to the skin in use. (A Claude preview stays stopped until Claude sends another.)
    pub fn stop_trying(&mut self) {
        if matches!(self.trying.as_ref().map(|t| &t.what), Some(Trial::Claude { .. })) {
            let _ = std::fs::remove_file(self.config_dir.join(PREVIEW_STAMP));
        }
        self.trying = None;
    }

    /// Every frame, before the panels: picks up a preview from Claude, puts on the skin that
    /// should be on (`skin`: the id of the one in use, from settings; `files_of` reads an
    /// installed one), and paints its picture.
    pub fn step(&mut self, ctx: &egui::Context, engine: &Engine, skin: &str, files_of: impl Fn(&str) -> Option<Result<Vec<(String, Vec<u8>)>, String>>) -> Option<String> {
        let mut toast = None;
        // A preview from the Claude connector (looked for every second).
        if self.preview_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
            self.preview_checked = Some(Instant::now());
            let stamp = std::fs::read_to_string(self.config_dir.join(PREVIEW_STAMP)).ok().map(|s| s.trim().to_string());
            if stamp != self.preview_stamp {
                self.preview_stamp = stamp.clone();
                match stamp {
                    Some(stamp) => match ingle_engine::skins::check_folder(&self.config_dir.join(PREVIEW_DIR)) {
                        Ok((files, report)) if report.ok() => {
                            let name = report.manifest.name.clone();
                            toast = Some(format!("Trying {name} from Claude."));
                            let rev = self.trying.as_ref().filter(|t| matches!(t.what, Trial::Claude { .. })).map(|t| t.rev + 1).unwrap_or(0);
                            let mut t = Trying::new(Trial::Claude { stamp }, name, files);
                            t.rev = rev;
                            self.trying = Some(t);
                        }
                        Ok((_, report)) => toast = Some(format!("Claude's skin can't be tried yet: {}", report.problems.join(" "))),
                        Err(e) => toast = Some(format!("Claude's skin can't be tried: {e:#}")),
                    },
                    None => {
                        if matches!(self.trying.as_ref().map(|t| &t.what), Some(Trial::Claude { .. })) {
                            self.trying = None;
                        }
                    }
                }
            }
        }
        // A folder being tried: worn again when its files change (looked at every second).
        if let Some(t) = self.trying.as_mut() {
            if let Trial::Folder { path } = &t.what {
                if t.checked.is_none_or(|c| c.elapsed() > Duration::from_secs(1)) {
                    t.checked = Some(Instant::now());
                    let now = stamps(path, &t.files);
                    if now != t.seen {
                        t.seen = now;
                        match ingle_engine::skins::check_folder(path) {
                            Ok((files, report)) if report.ok() => {
                                t.name = report.manifest.name.clone();
                                t.seen = stamps(path, &files);
                                t.files = files;
                                t.rev += 1;
                            }
                            Ok((_, report)) => toast = Some(format!("Your skin's latest change can't be worn yet: {}", report.problems.join(" "))),
                            Err(e) => toast = Some(format!("Your skin's folder can't be read: {e:#}")),
                        }
                    }
                }
            }
        }
        // What should be on.
        let want = match &self.trying {
            Some(t) => format!("try:{:?}:{}", t.what, t.rev),
            None if skin.is_empty() => String::new(),
            None => format!("skin:{skin}"),
        };
        if self.worn.as_ref().is_none_or(|w| w.what != want) {
            let files = match &self.trying {
                Some(t) => Some(Ok(t.files.clone())),
                None if skin.is_empty() => None,
                None => Some(files_of(skin).unwrap_or_else(|| Err("it isn't installed".into()))),
            };
            self.problem = None;
            match files {
                None => self.take_off(ctx, engine),
                Some(Ok(files)) => match self.put_on(ctx, engine, &files) {
                    Ok(()) => {}
                    Err(e) => {
                        self.take_off(ctx, engine);
                        self.problem = Some(e.clone());
                        toast = Some(format!("That skin couldn't be put on: {e}"));
                    }
                },
                Some(Err(e)) => {
                    self.take_off(ctx, engine);
                    self.problem = Some(e.clone());
                    toast = Some(format!("Your skin couldn't be put on: {e}"));
                }
            }
            if let Some(w) = &mut self.worn {
                w.what = want;
            }
        }
        self.paint(ctx);
        toast
    }

    /// Campfire again: Ingle's colors, fonts and sounds, no picture.
    fn take_off(&mut self, ctx: &egui::Context, engine: &Engine) {
        let had_fonts = self.worn.as_ref().is_some_and(|w| w.fonts);
        theme::set_palette(None);
        theme::set_picture(0.0, false);
        theme::apply_style(ctx);
        if had_fonts {
            theme::install_fonts(ctx, None);
        }
        engine.set_skin_sounds(Vec::new());
        self.worn = Some(Worn { what: String::new(), picture: None, fonts: false });
    }

    /// Puts a skin's files on (checked first, its fonts parsed).
    fn put_on(&mut self, ctx: &egui::Context, engine: &Engine, files: &[(String, Vec<u8>)]) -> Result<(), String> {
        let report = ingle_engine::skins::check(files);
        if !report.ok() {
            return Err(report.problems.join(" "));
        }
        let m = &report.manifest;
        let file = |p: &str| files.iter().find(|(q, _)| q == p).map(|(_, b)| b.clone());
        // The picture, at most the size of a big screen.
        let picture = match &m.background {
            Some(b) => {
                let bytes = file(&b.file).ok_or("its picture is missing")?;
                let img = image::load_from_memory(&bytes).map_err(|e| format!("its picture can't be read: {e}"))?;
                let img = if img.width() > 2560 || img.height() > 2560 { img.resize(2560, 2560, image::imageops::FilterType::Triangle) } else { img };
                let rgba = img.to_rgba8();
                let size = [rgba.width() as usize, rgba.height() as usize];
                let color = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                Some(ctx.load_texture("skin-picture", color, egui::TextureOptions::LINEAR))
            }
            None => None,
        };
        // Its sounds, decoded.
        let mut sounds = Vec::new();
        for (name, path) in &m.sounds {
            let (Some(sound), Some(bytes)) = (ingle_engine::sfx::Sound::from_name(name), file(path)) else { continue };
            let pcm = ingle_engine::soundboard::decode(&bytes).map_err(|e| format!("its {name} sound can't be played: {e:#}"))?;
            sounds.push((sound, std::sync::Arc::new(pcm)));
        }
        let had_fonts = self.worn.as_ref().is_some_and(|w| w.fonts);
        let fonts = m.uses().iter().any(|(_, u)| *u == Use::Font);
        theme::set_palette(Some(m.palette()));
        match &m.background {
            Some(b) => theme::set_picture(b.show, b.whole_window()),
            None => theme::set_picture(0.0, false),
        }
        theme::apply_style(ctx);
        if fonts {
            theme::install_fonts(
                ctx,
                Some(theme::SkinFonts { body: m.fonts.body.as_deref().and_then(file), bold: m.fonts.bold.as_deref().and_then(file), scale: m.fonts.scale }),
            );
        } else if had_fonts {
            theme::install_fonts(ctx, None);
        }
        engine.set_skin_sounds(sounds);
        self.worn = Some(Worn { what: String::new(), picture, fonts });
        Ok(())
    }

    /// The skin's picture over the whole window, under everything (the panels' colors let it
    /// show through where it should: see `theme::panel`).
    fn paint(&self, ctx: &egui::Context) {
        let Some(tex) = self.worn.as_ref().and_then(|w| w.picture.as_ref()) else { return };
        let screen = ctx.content_rect();
        let [w, h] = tex.size();
        if w == 0 || h == 0 || screen.width() <= 0.0 {
            return;
        }
        // Cover the window, cut evenly from both sides of the longer way.
        let (sw, sh) = (screen.width(), screen.height());
        let scale = (sw / w as f32).max(sh / h as f32);
        let (cw, ch) = (sw / scale / w as f32, sh / scale / h as f32);
        let uv = Rect::from_min_max(pos2((1.0 - cw) / 2.0, (1.0 - ch) / 2.0), pos2((1.0 + cw) / 2.0, (1.0 + ch) / 2.0));
        ctx.layer_painter(egui::LayerId::background()).image(tex.id(), screen, uv, Color32::WHITE);
    }

    /// Something's being tried (for the banner that says so and takes it off).
    pub fn trying_name(&self) -> Option<(&str, &Trial)> {
        self.trying.as_ref().map(|t| (t.name.as_str(), &t.what))
    }
}
