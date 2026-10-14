//! The soundboard, like Discord's but better. Every sound you can reach plays in any call (none
//! is locked to its server): your favorites, the ones you played lately, each server's own and
//! Ingle's built-in ones. Hover one to hear it yourself first, right-click for a key that plays
//! it (even in a game), to have it play when you join voice, or to favorite it. Sounds can be
//! twice as long as Discord's, are trimmed and evened out in loudness on the way in, and
//! anyone's can be stopped, or one person's muted.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{self, Align2, Color32, CornerRadius, Id, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2, vec2};
use ingle_engine::config::SoundRef;
use ingle_engine::hotkey::{combo_label, needs_modifier};
use ingle_engine::soundboard as sb;
use ingle_engine::state::{AppState, ServerView};
use ingle_proto::control::{MAX_SERVER_SOUNDS, MAX_SOUND_NAME, SOUNDBOARD_FEATURE, ServerSound, perm};

use crate::app::{Act, Dialog, IngleApp, icon_button};
use crate::dialogs::{field_label, title};
use crate::emoji;
use crate::{icons, theme};

// Compact, like Discord's: three sounds a row, six rows showing.
const CELL: Vec2 = Vec2 { x: 136.0, y: 36.0 };
const COLS: usize = 3;
const GAP: f32 = 6.0;
const RAIL_W: f32 = 36.0;
const GRID_W: f32 = COLS as f32 * CELL.x + (COLS - 1) as f32 * GAP;
const WIDTH: f32 = GRID_W + RAIL_W + 10.0;
/// Six rows of sounds.
const GRID_H: f32 = 6.0 * (CELL.y + GAP);
/// The longest a sound can be, in seconds (a frame under the limit, see `soundboard::prepare`).
const MAX_SECS: f32 = ingle_proto::sound::MAX_MS as f32 / 1000.0 - 0.02;
/// How many recently played sounds show.
const RECENT_SHOWN: usize = 6;
/// The shortest cut of a file, in seconds.
const MIN_CUT: f32 = 0.1;

/// Where the soundboard was opened from: the call's own buttons, or the voice panel under the
/// channel list (it opens over whichever).
pub const FROM_CALL: u8 = 0;
pub const FROM_PANEL: u8 = 1;

#[derive(Default)]
pub struct SoundboardUi {
    pub open: bool,
    /// The button it was opened from (`FROM_CALL` or `FROM_PANEL`).
    from: u8,
    /// The soundboard buttons drawn this frame, to open over.
    anchors: Vec<(u8, Rect)>,
    search: String,
    focus_search: bool,
    jump: Option<usize>,
    volume_open: bool,
    /// The sound last clicked and when (it lights up).
    flash: Option<(String, String, f64)>,
    pub editor: Option<SoundEditor>,
    /// Waiting for the keys that should play this sound.
    recording: Option<SoundRef>,
    /// Why the keys just pressed won't do (a key you type with, alone).
    key_hint: Option<String>,
}

/// The add-a-sound (or edit-a-sound) dialog.
pub struct SoundEditor {
    pub key: String,
    /// Editing this sound of the server (its name, emoji and volume).
    pub edit: Option<String>,
    pcm: Option<Arc<Vec<f32>>>,
    outline: Vec<f32>,
    start: f32,
    end: f32,
    name: String,
    emoji: String,
    volume: f32,
    error: Option<String>,
    loading: bool,
    /// The file being read (see `read_picked`).
    pending: Option<u64>,
    /// The file was longer than what was read of it.
    cut: bool,
    drag: Option<Drag>,
    emoji_open: bool,
    /// The dialog to go back to after (Server Settings).
    back_to: Option<Dialog>,
}

#[derive(Clone, Copy)]
enum Drag {
    Start,
    End,
    /// Moving the whole cut; where it was grabbed (seconds into it).
    Body(f32),
}

impl SoundEditor {
    pub fn add(key: &str, back_to: Option<Dialog>) -> SoundEditor {
        SoundEditor {
            key: key.to_string(),
            edit: None,
            pcm: None,
            outline: Vec::new(),
            start: 0.0,
            end: 0.0,
            name: String::new(),
            emoji: "🔊".into(),
            volume: 1.0,
            error: None,
            loading: false,
            pending: None,
            cut: false,
            drag: None,
            emoji_open: false,
            back_to,
        }
    }

    pub fn edit(key: &str, s: &ServerSound, back_to: Option<Dialog>) -> SoundEditor {
        SoundEditor { edit: Some(s.id.clone()), name: s.name.clone(), emoji: s.emoji.clone(), volume: s.volume, ..SoundEditor::add(key, back_to) }
    }

    /// The file chooser closed: with a file (read it), or with none.
    pub fn picked(&mut self, file: Option<(String, Vec<u8>)>, ctx: &egui::Context) {
        match file {
            Some((name, bytes)) => {
                self.loading = true;
                self.pending = Some(read_picked(name, bytes, ctx));
            }
            None => self.loading = false,
        }
    }
}

/// A picked file, read on its own thread: (which reading, its name, the sound or why not).
type Decoded = (u64, String, Result<sb::Imported, String>);

fn decoded() -> &'static Mutex<Option<Decoded>> {
    static D: OnceLock<Mutex<Option<Decoded>>> = OnceLock::new();
    D.get_or_init(Default::default)
}

/// Reads a sound file someone picked (any length of MP3, WAV, OGG, FLAC, M4A) off the UI
/// thread; returns which reading it is (its result comes with it).
fn read_picked(name: String, bytes: Vec<u8>, ctx: &egui::Context) -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let ext = std::path::Path::new(&name).extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase());
        let got = sb::import(bytes, ext.as_deref()).map_err(|e| format!("{e:#}"));
        *decoded().lock().unwrap() = Some((n, name, got));
        ctx.request_repaint();
    });
    n
}

/// "0:03.2".
fn clock(t: f32) -> String {
    let t = t.max(0.0);
    format!("{}:{:04.1}", (t / 60.0) as u32, t % 60.0)
}

/// Where the sound in `pcm` (48 kHz stereo) starts and ends, in seconds: silence at either
/// end left out (a moment of it kept).
fn audible(pcm: &[f32]) -> (f32, f32) {
    const RATE: f32 = 48_000.0;
    let frames = pcm.len() / 2;
    let loud = |i: &usize| pcm[2 * i].abs().max(pcm[2 * i + 1].abs()) > 0.01;
    let (Some(first), Some(last)) = ((0..frames).find(loud), (0..frames).rev().find(loud)) else {
        return (0.0, frames as f32 / RATE);
    };
    ((first as f32 / RATE - 0.05).max(0.0), ((last + 1) as f32 / RATE + 0.15).min(frames as f32 / RATE))
}

/// A sound's emoji (or a speaker, for none).
fn paint_emoji(p: &egui::Painter, r: Rect, text: &str, custom: &mut dyn FnMut(&str) -> Option<egui::TextureId>) {
    let chars: Vec<char> = text.chars().collect();
    match emoji::match_at(&chars, 0) {
        Some((_, e)) => emoji::paint(p, r, &e, custom),
        None => icons::speaker(p, r.shrink(2.0), theme::MUTED),
    }
}

/// Where a section of the picker comes from (its button on the left).
#[derive(Clone)]
enum Rail {
    Favorites,
    Recent,
    Server(String),
    Ingle,
}

struct Section {
    title: String,
    rail: Rail,
    cells: Vec<Cell>,
}

#[derive(Clone)]
struct Cell {
    sound: SoundRef,
    /// You can edit and delete it (its server's "Manage emoji and sounds").
    manage: bool,
}

/// What's left of `sound` now: its current name, emoji and volume (it may have been renamed),
/// or None if it's gone from its server.
fn current(st: &AppState, sound: &SoundRef) -> Option<SoundRef> {
    if sound.is_builtin() {
        return sb::builtin(&sound.id).map(|b| b.sound_ref());
    }
    match st.server(&sound.server) {
        Some(s) if s.status.is_connected() => s.info.sounds.iter().find(|x| x.id == sound.id).map(|x| server_ref(s, x)),
        // A server that's offline: show it as remembered.
        _ => Some(sound.clone()),
    }
}

fn server_ref(s: &ServerView, x: &ServerSound) -> SoundRef {
    SoundRef { server: s.key.clone(), id: x.id.clone(), name: x.name.clone(), emoji: x.emoji.clone(), volume: x.volume }
}

fn can_manage(st: &AppState, sound: &SoundRef) -> bool {
    !sound.is_builtin() && st.server(&sound.server).is_some_and(|s| s.can(perm::MANAGE_EMOJI) && s.has(SOUNDBOARD_FEATURE))
}

impl IngleApp {
    /// A soundboard button was clicked (`from`: which one): opens or closes the soundboard.
    pub(crate) fn toggle_soundboard(&mut self, st: &AppState, from: u8) {
        if self.ui.sb.open && self.ui.sb.from != from {
            // Open over the other button: it moves there.
            self.ui.sb.from = from;
            return;
        }
        if self.ui.sb.open {
            self.close_soundboard();
            return;
        }
        self.ui.sb.open = true;
        self.ui.sb.from = from;
        self.ui.sb.focus_search = true;
        // Have every server's sounds ready, so the first play of each is instant.
        for s in &st.servers {
            if s.has(SOUNDBOARD_FEATURE) && s.status.is_connected() {
                self.engine.prefetch_sounds(&s.key);
            }
        }
    }

    /// Where a soundboard button is this frame (the soundboard opens over it).
    pub(crate) fn soundboard_anchor(&mut self, from: u8, rect: Rect) {
        self.ui.sb.anchors.push((from, rect));
    }

    pub(crate) fn soundboard_open(&self) -> bool {
        self.ui.sb.open
    }

    fn close_soundboard(&mut self) {
        if self.ui.sb.recording.is_some() {
            self.engine.record_sound_key(false);
        }
        let old = std::mem::take(&mut self.ui.sb);
        self.ui.sb = SoundboardUi { from: old.from, anchors: old.anchors, editor: old.editor, ..Default::default() };
    }

    /// The soundboard, over the button it was opened from. Drawn once a frame, after the
    /// panels that have its buttons.
    pub(crate) fn soundboard_popup(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        let anchors = std::mem::take(&mut self.ui.sb.anchors);
        if !self.ui.sb.open {
            return;
        }
        let Some(call) = st.call.as_ref() else {
            self.close_soundboard();
            return;
        };
        // (With none of its buttons on screen just now, in full screen say, it waits.)
        let from = self.ui.sb.from;
        let Some(anchor) = anchors.iter().find(|a| a.0 == from).or(anchors.first()).map(|a| a.1) else { return };
        let call_key = call.server_key.clone();
        let was_recording = self.ui.sb.recording.is_some();
        let resp = egui::Area::new(Id::new("soundboard"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(pos2(anchor.center().x, anchor.top() - 12.0))
            .constrain(true)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(theme::SIDE)
                    .stroke(Stroke::new(1.0f32, theme::LINE))
                    .corner_radius(14)
                    .inner_margin(egui::Margin::same(12))
                    .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
                    .show(ui, |ui| self.soundboard_contents(ui, st, &call_key, acts));
            });
        // Closes on a click outside it (its buttons toggle it), or Escape (which first stops
        // waiting for keys, when it is).
        let outside = ctx.input(|i| {
            i.pointer.any_click()
                && i.pointer.interact_pos().is_some_and(|p| !resp.response.rect.contains(p) && !anchors.iter().any(|a| a.1.contains(p)))
        });
        let escape = !was_recording && ctx.input(|i| i.key_pressed(egui::Key::Escape));
        if self.ui.sb.open && ((outside && !egui::Popup::is_any_open(ctx)) || escape) {
            self.close_soundboard();
        }
    }

    fn soundboard_contents(&mut self, ui: &mut egui::Ui, st: &AppState, call_key: &str, acts: &mut Vec<Act>) {
        let mut sbui = std::mem::take(&mut self.ui.sb);
        ui.set_width(WIDTH);
        let now = ui.input(|i| i.time);
        let keys_ok = self.engine.sound_keys_available();

        // A key being set: wait for it here.
        if let Some(sound) = sbui.recording.clone() {
            if let Some(combo) = self.engine.take_recorded_sound_key() {
                if combo == "Escape" {
                    sbui.recording = None;
                    sbui.key_hint = None;
                } else if needs_modifier(&combo) {
                    // Keep waiting: that key would go off while typing.
                    sbui.key_hint = Some(format!(
                        "{} on its own would play it whenever you type it. Hold Ctrl, Alt or Shift with it, or use a function key or the number pad.",
                        combo_label(&combo)
                    ));
                    self.engine.record_sound_key(true);
                } else {
                    sbui.recording = None;
                    sbui.key_hint = None;
                    let mut s = st.settings.clone();
                    // A key plays one sound, and a sound has one key.
                    s.sound_keys.retain(|(c, x)| *c != combo && !x.same(&sound));
                    s.sound_keys.push((combo.clone(), sound.clone()));
                    acts.push(Act::Settings(s));
                    acts.push(Act::Toast(format!("{} now plays {}.", combo_label(&combo), sound.name), false));
                }
            } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                sbui.recording = None;
                sbui.key_hint = None;
                self.engine.record_sound_key(false);
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }

        // Search, volume, stop.
        let mut play_first = false;
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut sbui.search)
                    .hint_text("Find the perfect sound")
                    .desired_width(WIDTH - 84.0)
                    .margin(egui::Margin::symmetric(10, 7))
                    .id(Id::new("sound-search")),
            );
            if sbui.focus_search {
                r.request_focus();
                sbui.focus_search = false;
            }
            // Enter plays the first one found.
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                play_first = true;
                r.request_focus();
            }
            if icon_button(ui, 30.0, "Soundboard volume", sbui.volume_open, icons::speaker).clicked() {
                sbui.volume_open = !sbui.volume_open;
            }
            let playing = self.engine.sounds_playing();
            let tip = if playing { "Stop every sound playing" } else { "Stops sounds playing (nothing is now)" };
            if icon_button(ui, 30.0, tip, false, |p, r, c| icons::stop(p, r, if playing { theme::LIVE } else { c })).clicked() && playing {
                acts.push(Act::StopSounds);
            }
            if playing {
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            }
        });
        if sbui.volume_open {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Soundboard volume").size(12.5).color(theme::MUTED));
                let mut pct = st.settings.soundboard_volume * 100.0;
                if ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%")).changed() {
                    let mut s = st.settings.clone();
                    s.soundboard_volume = pct / 100.0;
                    acts.push(Act::Settings(s));
                }
            });
            ui.label(RichText::new("How loud everyone's sounds play for you. Right-click someone in the call to mute theirs.").size(11.5).color(theme::DIM));
        }
        if let Some(sound) = &sbui.recording {
            egui::Frame::new().fill(Color32::from_rgb(0x3a, 0x2a, 0x20)).corner_radius(8).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(WIDTH - 16.0);
                ui.label(RichText::new(format!("Press the keys that should play {}… (Esc cancels)", sound.name)).color(theme::EMBER2));
                match &sbui.key_hint {
                    Some(h) => ui.label(RichText::new(h).size(11.5).color(theme::LIVE)),
                    None => ui.label(
                        RichText::new("They work anywhere while you're in voice, even in a game, and the game still gets them.").size(11.5).color(theme::DIM),
                    ),
                };
            });
        }
        ui.add_space(4.0);

        // What there is.
        let q = sbui.search.trim().to_lowercase();
        let mut sections: Vec<Section> = Vec::new();
        let cell = |sound: SoundRef| Cell { manage: can_manage(st, &sound), sound };
        let favorites: Vec<Cell> = st.settings.sound_favorites.iter().filter_map(|f| current(st, f)).map(cell).collect();
        let recent: Vec<Cell> = st.settings.sound_recent.iter().filter_map(|f| current(st, f)).take(RECENT_SHOWN).map(cell).collect();
        let mut servers: Vec<&ServerView> =
            st.servers.iter().filter(|s| s.has(SOUNDBOARD_FEATURE) && s.status.is_connected() && !s.info.sounds.is_empty()).collect();
        // The call's server first.
        servers.sort_by_key(|s| s.key != call_key);
        if q.is_empty() {
            if !favorites.is_empty() {
                sections.push(Section { title: "Favorites".into(), rail: Rail::Favorites, cells: favorites });
            }
            if !recent.is_empty() {
                sections.push(Section { title: "Recently played".into(), rail: Rail::Recent, cells: recent });
            }
            for s in &servers {
                let cells = s.info.sounds.iter().map(|x| cell(server_ref(s, x))).collect();
                sections.push(Section { title: s.name.clone(), rail: Rail::Server(s.key.clone()), cells });
            }
            sections.push(Section { title: "Ingle".into(), rail: Rail::Ingle, cells: sb::BUILTINS.iter().map(|b| cell(b.sound_ref())).collect() });
        } else {
            let mut found: Vec<Cell> = Vec::new();
            let all = servers.iter().flat_map(|s| s.info.sounds.iter().map(|x| server_ref(s, x))).chain(sb::BUILTINS.iter().map(|b| b.sound_ref()));
            for sound in all {
                if sound.name.to_lowercase().contains(&q) && !found.iter().any(|c| c.sound.same(&sound)) {
                    found.push(cell(sound));
                }
            }
            // Names that start with the words first.
            found.sort_by_key(|c| !c.sound.name.to_lowercase().starts_with(&q));
            if play_first {
                if let Some(c) = found.first() {
                    acts.push(Act::PlaySound(c.sound.clone()));
                    sbui.flash = Some((c.sound.server.clone(), c.sound.id.clone(), now));
                }
            }
            sections.push(Section { title: format!("Results for \u{201c}{}\u{201d}", sbui.search.trim()), rail: Rail::Ingle, cells: found });
        }

        let ctx = ui.ctx().clone();
        ui.horizontal_top(|ui| {
            // The rail: jump to a section.
            if q.is_empty() {
                ui.vertical(|ui| {
                    ui.set_width(RAIL_W);
                    ui.spacing_mut().item_spacing.y = 4.0;
                    for (i, sec) in sections.iter().enumerate() {
                        let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(8), theme::HOVER);
                        }
                        match &sec.rail {
                            Rail::Favorites => icons::star(ui.painter(), rect.shrink(7.0), theme::GLOW, true),
                            Rail::Recent => icons::clock(ui.painter(), rect.shrink(8.0), theme::MUTED),
                            Rail::Ingle => icons::flame(ui.painter(), rect.shrink(5.0)),
                            Rail::Server(k) => {
                                let s = st.server(k);
                                let tex = s.and_then(|s| self.server_icon(&ctx, s));
                                let r = rect.shrink(3.0);
                                match tex {
                                    Some(t) => {
                                        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                                        ui.painter().add(egui::epaint::RectShape::filled(r, CornerRadius::same(8), Color32::WHITE).with_texture(t, uv));
                                    }
                                    None => {
                                        ui.painter().rect_filled(r, CornerRadius::same(8), theme::ELEV2);
                                        let initials: String = s
                                            .map(|s| s.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase())
                                            .unwrap_or_default();
                                        ui.painter().text(r.center(), Align2::CENTER_CENTER, initials, theme::bold_font(11.0), theme::TEXT);
                                    }
                                }
                            }
                        }
                        if resp.on_hover_text(&sec.title).clicked() {
                            sbui.jump = Some(i);
                        }
                    }
                });
            }
            // (A scroll area lays out like the row it's in: its contents go down a column.)
            egui::ScrollArea::vertical().id_salt("sound-grid").max_height(GRID_H).min_scrolled_height(GRID_H).auto_shrink([false, false]).show(ui, |ui| ui.vertical(|ui| {
                ui.set_width(GRID_W + 4.0);
                for (i, sec) in sections.iter().enumerate() {
                    let h = ui.horizontal(|ui| {
                        if let Rail::Favorites = sec.rail {
                            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                            icons::star(ui.painter(), r, theme::GLOW, true);
                        }
                        ui.label(RichText::new(sec.title.to_uppercase()).font(theme::bold_font(11.5)).color(theme::MUTED));
                    });
                    if sbui.jump == Some(i) {
                        h.response.scroll_to_me(Some(egui::Align::TOP));
                        sbui.jump = None;
                    }
                    if sec.cells.is_empty() {
                        ui.label(RichText::new("No sounds match that.").color(theme::DIM));
                    }
                    for row in sec.cells.chunks(COLS) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = GAP;
                            for c in row {
                                self.sound_cell(ui, st, c, &mut sbui, now, keys_ok, acts);
                            }
                        });
                        ui.add_space(GAP - ui.spacing().item_spacing.y);
                    }
                    ui.add_space(8.0);
                }
            }));
        });

        // Adding sounds to the call's server.
        ui.add_space(2.0);
        ui.separator();
        ui.horizontal(|ui| {
            let s = st.server(call_key).filter(|s| s.has(SOUNDBOARD_FEATURE));
            match s {
                Some(s) if s.can(perm::MANAGE_EMOJI) => {
                    let full = s.info.sounds.len() >= MAX_SERVER_SOUNDS;
                    if ui.add_enabled(!full, theme::primary_button("Add a sound")).clicked() {
                        sbui.editor = Some(SoundEditor::add(&s.key, None));
                        self.ui.dialog = Some(Dialog::SoundEditor);
                        sbui.open = false;
                    }
                    ui.label(RichText::new(format!("{} of {MAX_SERVER_SOUNDS} on {}", s.info.sounds.len(), s.name)).size(12.0).color(theme::DIM));
                }
                Some(_) => {
                    ui.label(RichText::new("Click a sound to play it for everyone here. Right-click it for more.").size(12.0).color(theme::DIM));
                }
                None => {
                    ui.label(RichText::new("This server needs updating for its own sounds; Ingle's play anywhere.").size(12.0).color(theme::DIM));
                }
            }
        });
        let closing = !sbui.open;
        self.ui.sb = sbui;
        if closing {
            self.close_soundboard();
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn sound_cell(&mut self, ui: &mut egui::Ui, st: &AppState, c: &Cell, sbui: &mut SoundboardUi, now: f64, keys_ok: bool, acts: &mut Vec<Act>) {
        let (rect, resp) = ui.allocate_exact_size(CELL, Sense::click());
        if !ui.is_rect_visible(rect) {
            return;
        }
        let sound = &c.sound;
        let inside = ui.rect_contains_pointer(rect);
        let lit = sbui.flash.as_ref().is_some_and(|(s, i, t)| *s == sound.server && *i == sound.id && now - t < 0.4);
        let fill = if lit {
            Color32::from_rgb(0x4a, 0x30, 0x22)
        } else if inside {
            theme::ELEV2
        } else {
            theme::ELEV
        };
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(9), fill);
        if lit {
            p.rect_stroke(rect, CornerRadius::same(9), Stroke::new(1.5f32, theme::EMBER), StrokeKind::Inside);
            ui.ctx().request_repaint();
        }
        let key = sound.server.clone();
        let ctx = ui.ctx().clone();
        let mut custom = |id: &str| -> Option<egui::TextureId> { if key.is_empty() { None } else { self.custom_emoji_tex(&ctx, &key, id) } };
        paint_emoji(ui.painter(), Rect::from_center_size(pos2(rect.left() + 20.0, rect.center().y), vec2(22.0, 22.0)), &sound.emoji, &mut custom);
        // Its name, cut with "…" to fit (leaving room for the hear-it button when pointed at).
        let room = rect.width() - 40.0 - if inside { 28.0 } else { 6.0 };
        let mut job = egui::text::LayoutJob::simple(sound.name.clone(), theme::font(13.0), theme::TEXT, room);
        job.wrap.max_rows = 1;
        job.wrap.break_anywhere = true;
        job.wrap.overflow_character = Some('…');
        let g = ui.fonts_mut(|f| f.layout_job(job));
        let combo = st.settings.sound_keys.iter().find(|(_, s)| s.same(sound)).map(|(k, _)| k.clone());
        // With a key: the key under its name.
        let lift = if combo.is_some() { 7.0 } else { 0.0 };
        ui.painter().galley(pos2(rect.left() + 36.0, rect.center().y - lift - g.size().y / 2.0), g, theme::TEXT);
        if let Some(k) = &combo {
            let mut job = egui::text::LayoutJob::simple(combo_label(k), theme::font(10.5), theme::DIM, room);
            job.wrap.max_rows = 1;
            job.wrap.break_anywhere = true;
            job.wrap.overflow_character = Some('…');
            let g = ui.fonts_mut(|f| f.layout_job(job));
            ui.painter().galley(pos2(rect.left() + 36.0, rect.center().y + 8.0 - g.size().y / 2.0), g, theme::DIM);
        }
        let fav = st.settings.sound_favorites.iter().any(|f| f.same(sound));
        if fav {
            icons::star(ui.painter(), Rect::from_center_size(rect.left_top() + vec2(9.0, 9.0), vec2(9.0, 9.0)), theme::GLOW, true);
        }
        // Hear it yourself first: the speaker on the right (drawn over the cell, so it takes
        // its own clicks).
        let pr = Rect::from_center_size(pos2(rect.right() - 17.0, rect.center().y), vec2(26.0, 26.0));
        let mut previewed = false;
        if inside {
            let pv = ui.interact(pr, ui.id().with(("hear", &sound.server, &sound.id)), Sense::click());
            ui.painter().rect_filled(pr, CornerRadius::same(7), if pv.hovered() { theme::HOVER } else { theme::ELEV });
            icons::speaker(ui.painter(), pr.shrink(6.0), if pv.hovered() { theme::TEXT } else { theme::MUTED });
            if pv.on_hover_text("Hear it yourself (nobody else does)").clicked() {
                acts.push(Act::PreviewSound(sound.clone()));
                previewed = true;
            }
        }
        let mut tip = format!("{} — click to play it for everyone", sound.name);
        if let Some(k) = &combo {
            tip.push_str(&format!("\nKey: {}", combo_label(k)));
        }
        let resp = resp.on_hover_text(tip);
        if resp.clicked() && !previewed {
            acts.push(Act::PlaySound(sound.clone()));
            sbui.flash = Some((sound.server.clone(), sound.id.clone(), now));
        }
        resp.context_menu(|ui| {
            if ui.button("Play for everyone").clicked() {
                acts.push(Act::PlaySound(sound.clone()));
                ui.close();
            }
            if ui.button("Hear it yourself").clicked() {
                acts.push(Act::PreviewSound(sound.clone()));
                ui.close();
            }
            ui.separator();
            if ui.button(if fav { "Remove from favorites" } else { "Add to favorites" }).clicked() {
                let mut s = st.settings.clone();
                if fav {
                    s.sound_favorites.retain(|f| !f.same(sound));
                } else {
                    s.sound_favorites.push(sound.clone());
                }
                acts.push(Act::Settings(s));
                ui.close();
            }
            let joining = st.settings.join_sound.as_ref().is_some_and(|j| j.same(sound));
            if ui.button(if joining { "Don't play it when I join voice" } else { "Play it when I join voice" }).clicked() {
                let mut s = st.settings.clone();
                s.join_sound = (!joining).then(|| sound.clone());
                acts.push(Act::Settings(s));
                ui.close();
            }
            if keys_ok {
                match &combo {
                    Some(k) => {
                        if ui.button(format!("Change its key ({})…", combo_label(k))).clicked() {
                            sbui.recording = Some(sound.clone());
                            sbui.key_hint = None;
                            self.engine.record_sound_key(true);
                            ui.close();
                        }
                        if ui.button("Remove its key").clicked() {
                            let mut s = st.settings.clone();
                            s.sound_keys.retain(|(_, x)| !x.same(sound));
                            acts.push(Act::Settings(s));
                            ui.close();
                        }
                    }
                    None => {
                        if ui.button("Give it a key…").clicked() {
                            sbui.recording = Some(sound.clone());
                            sbui.key_hint = None;
                            self.engine.record_sound_key(true);
                            ui.close();
                        }
                    }
                }
            }
            if c.manage {
                ui.separator();
                if ui.button("Edit sound…").clicked() {
                    if let Some(x) = st.server(&sound.server).and_then(|s| s.info.sounds.iter().find(|x| x.id == sound.id)) {
                        sbui.editor = Some(SoundEditor::edit(&sound.server, x, None));
                        self.ui.dialog = Some(Dialog::SoundEditor);
                        sbui.open = false;
                    }
                    ui.close();
                }
                if ui.button(RichText::new("Delete sound…").color(theme::LIVE)).clicked() {
                    self.ui.dialog = Some(Dialog::RemoveSound { key: sound.server.clone(), id: sound.id.clone(), name: sound.name.clone(), from_settings: false });
                    sbui.open = false;
                    ui.close();
                }
            }
        });
    }

    /// Add a sound (pick a file, cut up to 10 s of it) or edit one (name, emoji, volume).
    pub(crate) fn sound_editor_dialog(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>, close: &mut bool) {
        let Some(mut ed) = self.ui.sb.editor.take() else {
            *close = true;
            return;
        };
        // The file picked for this dialog, read (an earlier dialog's reading is left alone).
        let read = {
            let mut d = decoded().lock().unwrap();
            if d.as_ref().is_some_and(|x| ed.pending == Some(x.0)) { d.take() } else { None }
        };
        if let Some((_, name, got)) = read {
            ed.loading = false;
            ed.pending = None;
            match got {
                Ok(imp) => {
                    ed.cut = imp.cut;
                    ed.outline = sb::outline(&imp.pcm, 168);
                    // The cut starts where the sound does (silence left out), 10 s at most.
                    let (a, b) = audible(&imp.pcm);
                    ed.start = a;
                    ed.end = b.min(a + MAX_SECS);
                    ed.pcm = Some(Arc::new(imp.pcm));
                    ed.error = None;
                    if ed.name.trim().is_empty() {
                        let stem = std::path::Path::new(&name).file_stem().and_then(|s| s.to_str()).unwrap_or("Sound");
                        ed.name = stem.replace(['_', '-'], " ").chars().take(MAX_SOUND_NAME).collect();
                    }
                }
                Err(e) => ed.error = Some(e),
            }
        }
        let server = st.server(&ed.key).map(|s| s.name.clone()).unwrap_or_default();
        ui.set_width(520.0);
        title(ui, if ed.edit.is_some() { "Edit sound" } else { "Add a sound" });
        if ed.edit.is_none() {
            ui.label(
                RichText::new(format!(
                    "For everyone on {server}, and anywhere they play it. Pick an MP3, WAV, OGG, OPUS, FLAC or M4A and up to 10 seconds of it; \
                     it's evened out in loudness, so it's never deafening or too quiet."
                ))
                .color(theme::MUTED),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button(if ed.pcm.is_some() { "Choose another file…" } else { "Choose a sound file…" })).clicked() {
                    crate::images::pick_async(crate::images::PickFor::Sound(ed.key.clone()), false, ui.ctx());
                    ed.loading = true;
                    ed.error = None;
                }
                if ed.loading {
                    ui.spinner();
                    ui.label(RichText::new("Reading it…").color(theme::MUTED));
                }
            });
            if let Some(pcm) = ed.pcm.clone() {
                ui.add_space(10.0);
                self.trimmer(ui, &mut ed, &pcm);
                ui.horizontal(|ui| {
                    let len = ed.end - ed.start;
                    ui.label(RichText::new(format!("{} – {}  ·  {:.1} s", clock(ed.start), clock(ed.end), len)).color(theme::BODY));
                    if len < 0.05 {
                        ui.label(RichText::new("Too short to hear").color(theme::LIVE));
                    }
                    if ui.button("▶  Hear the cut").clicked() {
                        let cut = sb::prepare(&pcm, ed.start, ed.end);
                        self.engine.preview_pcm(Arc::new(cut), ed.volume);
                    }
                });
                let note = if ed.cut { "Only the first 3 minutes of the file are shown. " } else { "" };
                ui.label(RichText::new(format!("{note}Drag the ends to choose, or drag the middle to move it (10 seconds at most).")).size(11.5).color(theme::DIM));
            }
        }
        field_label(ui, "Name");
        ui.add(egui::TextEdit::singleline(&mut ed.name).hint_text("Air horn").char_limit(MAX_SOUND_NAME).desired_width(f32::INFINITY));
        field_label(ui, "Emoji");
        let mut emoji_button = Rect::NOTHING;
        ui.horizontal(|ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
            emoji_button = r;
            let lit = resp.hovered() || ed.emoji_open;
            ui.painter().rect_filled(r, CornerRadius::same(8), if lit { theme::HOVER } else { theme::ELEV });
            let ctx = ui.ctx().clone();
            let key = ed.key.clone();
            paint_emoji(ui.painter(), r.shrink(6.0), &ed.emoji, &mut |id| self.custom_emoji_tex(&ctx, &key, id));
            if resp.on_hover_text("Pick an emoji").clicked() {
                ed.emoji_open = !ed.emoji_open;
            }
            ui.label(RichText::new("Shows on its button and pops up over whoever plays it.").size(12.0).color(theme::DIM));
        });
        if ed.emoji_open {
            // The picker beside the button, over the dialog.
            let key = ed.key.clone();
            let area = egui::Area::new(Id::new("sound-emoji"))
                .order(egui::Order::Tooltip)
                .pivot(Align2::LEFT_CENTER)
                .fixed_pos(pos2(emoji_button.right() + 12.0, emoji_button.center().y))
                .constrain(true)
                .show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).fill(theme::SIDE).inner_margin(10).show(ui, |ui| self.emoji_picker(ui, st, Some(&key), acts)).inner
                });
            if let Some(e) = area.inner {
                ed.emoji = e;
                ed.emoji_open = false;
            }
            let pressed_at = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
            let outside = pressed_at.is_some_and(|p| !area.response.rect.contains(p) && !emoji_button.contains(p));
            // (Escape closes the picker, not the dialog.)
            if outside || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                ed.emoji_open = false;
            }
        }
        field_label(ui, "Volume");
        ui.horizontal(|ui| {
            let mut pct = ed.volume * 100.0;
            if ui.add(egui::Slider::new(&mut pct, 5.0..=100.0).suffix("%")).changed() {
                ed.volume = pct / 100.0;
            }
            ui.label(RichText::new("Every sound is already about as loud as the others.").size(12.0).color(theme::DIM));
        });
        if let Some(e) = &ed.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(theme::LIVE));
        }
        ui.add_space(14.0);
        let name = ed.name.trim().to_string();
        let ready = !name.is_empty() && (ed.edit.is_some() || (ed.pcm.is_some() && ed.end - ed.start >= 0.05)) && !ed.loading;
        let mut done = false;
        ui.horizontal(|ui| {
            let label = if ed.edit.is_some() { "Save" } else { "Add sound" };
            if ui.add_enabled(ready, theme::primary_button(label).min_size(vec2(120.0, 34.0))).clicked() {
                let emoji = if ed.emoji.trim().is_empty() { "🔊".to_string() } else { ed.emoji.trim().to_string() };
                match (&ed.edit, &ed.pcm) {
                    (Some(id), _) => acts.push(Act::EditSound { key: ed.key.clone(), id: id.clone(), name: name.clone(), emoji, volume: ed.volume }),
                    (None, Some(pcm)) => acts.push(Act::AddSound { key: ed.key.clone(), name: name.clone(), emoji, volume: ed.volume, pcm: pcm.clone(), start: ed.start, end: ed.end }),
                    _ => {}
                }
                done = true;
            }
            if ui.button("Cancel").clicked() {
                done = true;
            }
        });
        if done {
            *close = true;
            if let Some(back) = ed.back_to.take() {
                self.ui.next_dialog = Some(back);
            }
        } else {
            self.ui.sb.editor = Some(ed);
        }
    }

    /// The file's loudness outline with the cut on it: drag its ends, or its middle to move it.
    fn trimmer(&mut self, ui: &mut egui::Ui, ed: &mut SoundEditor, pcm: &[f32]) {
        let total = sb::seconds(pcm).max(0.01);
        // The shortest cut (or the whole sound, if it's shorter).
        let min_cut = MIN_CUT.min(total);
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 86.0), Sense::click_and_drag());
        let x_of = |t: f32| rect.left() + t / total * rect.width();
        let t_of = |x: f32| ((x - rect.left()) / rect.width() * total).clamp(0.0, total);
        // Dragging first, so what's drawn is where things are now.
        let pointer = resp.interact_pointer_pos();
        let (a, b) = (x_of(ed.start), x_of(ed.end));
        if resp.drag_started() {
            if let Some(pos) = pointer {
                ed.drag = Some(if (pos.x - a).abs() < 10.0 {
                    Drag::Start
                } else if (pos.x - b).abs() < 10.0 {
                    Drag::End
                } else if pos.x > a && pos.x < b {
                    Drag::Body(t_of(pos.x) - ed.start)
                } else {
                    // A new cut starting here.
                    ed.start = t_of(pos.x).min(total - min_cut);
                    ed.end = ed.start + min_cut;
                    Drag::End
                });
            }
        }
        if resp.dragged() {
            if let (Some(pos), Some(d)) = (pointer, ed.drag) {
                let t = t_of(pos.x);
                // (Bounds applied in order, so they can't cross: no clamp that could panic.)
                match d {
                    Drag::Start => ed.start = t.min(ed.end - min_cut).max((ed.end - MAX_SECS).max(0.0)).max(0.0),
                    Drag::End => ed.end = t.max(ed.start + min_cut).min((ed.start + MAX_SECS).min(total)),
                    Drag::Body(grab) => {
                        let len = ed.end - ed.start;
                        ed.start = (t - grab).min(total - len).max(0.0);
                        ed.end = ed.start + len;
                    }
                }
            }
        }
        if resp.drag_stopped() {
            ed.drag = None;
        }
        let (a, b) = (x_of(ed.start), x_of(ed.end));
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(8), theme::MAIN);
        let sel = Rect::from_min_max(pos2(a, rect.top()), pos2(b, rect.bottom()));
        p.rect_filled(sel, CornerRadius::same(4), Color32::from_rgba_unmultiplied(0xff, 0x8a, 0x3d, 34));
        let n = ed.outline.len().max(1);
        let w = rect.width() / n as f32;
        for (i, v) in ed.outline.iter().enumerate() {
            let x = rect.left() + (i as f32 + 0.5) * w;
            let h = (v.sqrt() * (rect.height() / 2.0 - 8.0)).max(1.0);
            let c = if x >= a && x <= b { theme::EMBER } else { theme::DIM };
            p.line_segment([pos2(x, rect.center().y - h), pos2(x, rect.center().y + h)], Stroke::new((w - 1.0).max(1.0), c));
        }
        for x in [a, b] {
            let handle = Rect::from_center_size(pos2(x, rect.center().y), vec2(6.0, rect.height() - 6.0));
            p.rect_filled(handle, CornerRadius::same(3), theme::EMBER2);
        }
        p.rect_stroke(sel, CornerRadius::same(4), Stroke::new(1.0f32, theme::EMBER), StrokeKind::Inside);
        if resp.hovered() || ed.drag.is_some() {
            let near = pointer.or(ui.input(|i| i.pointer.hover_pos())).is_some_and(|p| (p.x - a).abs() < 10.0 || (p.x - b).abs() < 10.0);
            let icon = match ed.drag {
                Some(Drag::Body(_)) => egui::CursorIcon::Grabbing,
                Some(_) => egui::CursorIcon::ResizeHorizontal,
                None if near => egui::CursorIcon::ResizeHorizontal,
                None => egui::CursorIcon::Grab,
            };
            ui.ctx().set_cursor_icon(icon);
        }
    }

    /// Server Settings → Soundboard: the server's sounds, to add, edit or delete.
    pub(crate) fn server_sounds_tab(&mut self, ui: &mut egui::Ui, s: &ServerView, back_to: Dialog) {
        ui.label(
            RichText::new("Sounds anyone here can play in voice. Ingle doesn't lock them to this server: members can play them in any call.")
                .color(theme::MUTED),
        );
        ui.add_space(8.0);
        if !s.has(SOUNDBOARD_FEATURE) {
            ui.label(RichText::new("This server needs updating (to Ingle 0.7) before it can have its own sounds.").color(theme::LIVE));
            return;
        }
        ui.horizontal(|ui| {
            let full = s.info.sounds.len() >= MAX_SERVER_SOUNDS;
            if ui.add_enabled(!full, theme::primary_button("Add a sound")).clicked() {
                self.ui.sb.editor = Some(SoundEditor::add(&s.key, Some(back_to.clone())));
                self.ui.next_dialog = Some(Dialog::SoundEditor);
            }
            ui.label(RichText::new(format!("{} of {MAX_SERVER_SOUNDS}", s.info.sounds.len())).color(theme::DIM));
        });
        ui.add_space(6.0);
        if s.info.sounds.is_empty() {
            ui.label(RichText::new("No sounds yet. Ingle's own (air horn, ba dum tss, sad trombone…) work everywhere meanwhile.").color(theme::DIM));
            return;
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
            for x in &s.info.sounds {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                    let ctx = ui.ctx().clone();
                    let key = s.key.clone();
                    paint_emoji(ui.painter(), r, &x.emoji, &mut |id| self.custom_emoji_tex(&ctx, &key, id));
                    ui.label(RichText::new(&x.name).color(theme::TEXT));
                    ui.label(RichText::new(format!("{:.1} s · {}% · added by {}", x.length_ms as f32 / 1000.0, (x.volume * 100.0).round(), s.member_name(&x.by))).size(12.0).color(theme::DIM));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("Delete").color(theme::LIVE)).clicked() {
                            self.ui.next_dialog = Some(Dialog::RemoveSound { key: s.key.clone(), id: x.id.clone(), name: x.name.clone(), from_settings: true });
                        }
                        if ui.button("Edit").clicked() {
                            self.ui.sb.editor = Some(SoundEditor::edit(&s.key, x, Some(back_to.clone())));
                            self.ui.next_dialog = Some(Dialog::SoundEditor);
                        }
                        let sound = server_ref(s, x);
                        if ui.button("▶").on_hover_text("Hear it yourself").clicked() {
                            self.engine.preview_sound(sound);
                        }
                    });
                });
                ui.add_space(2.0);
            }
        });
    }

    /// The add-a-sound dialog closed without its buttons (Escape, a click outside): back to
    /// where it was opened from.
    pub(crate) fn sound_editor_closed(&mut self) {
        if let Some(ed) = self.ui.sb.editor.take() {
            self.ui.next_dialog = ed.back_to;
        }
    }

    /// "Delete this sound?" (`from_settings`: Server Settings opened it, and comes back after).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn remove_sound_dialog(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        key: &str,
        id: &str,
        name: &str,
        from_settings: bool,
        acts: &mut Vec<Act>,
        close: &mut bool,
    ) {
        ui.set_width(400.0);
        title(ui, &format!("Delete \u{201c}{name}\u{201d}?"));
        ui.label(
            RichText::new("It goes from the soundboard for everyone on this server. Where it was someone's favorite or had a key, that goes too.")
                .color(theme::MUTED),
        );
        ui.add_space(14.0);
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.horizontal(|ui| {
            if ui.add(theme::danger_button("Delete sound").min_size(vec2(130.0, 34.0))).clicked() || enter {
                acts.push(Act::RemoveSound { key: key.to_string(), id: id.to_string() });
                *close = true;
            }
            if ui.button("Cancel").clicked() {
                *close = true;
            }
        });
        if *close && from_settings {
            let server = st.server(key).map(|s| s.name.clone()).unwrap_or_default();
            self.ui.next_dialog = Some(Dialog::ServerSettings { key: key.to_string(), name: server, tab: 4 });
        }
    }

    /// Settings → Voice & Video → Soundboard.
    pub(crate) fn soundboard_settings(&mut self, ui: &mut egui::Ui, st: &AppState, draft: &mut ingle_engine::config::Settings) {
        field_label(ui, "Soundboard");
        ui.horizontal(|ui| {
            ui.label("Volume");
            let mut pct = draft.soundboard_volume * 100.0;
            if ui.add(egui::Slider::new(&mut pct, 0.0..=100.0).suffix("%")).changed() {
                draft.soundboard_volume = pct / 100.0;
            }
        });
        ui.label(RichText::new("How loud anyone's soundboard sounds play for you (yours too).").size(12.0).color(theme::DIM));
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label("When you join voice:");
            match draft.join_sound.clone() {
                Some(j) => {
                    ui.label(RichText::new(format!("{} {}", j.emoji, j.name)).color(theme::TEXT));
                    if ui.button("Don't play a sound").clicked() {
                        draft.join_sound = None;
                    }
                }
                None => {
                    ui.label(RichText::new("no sound (right-click one in the soundboard to choose it)").color(theme::DIM));
                }
            }
        });
        if !draft.sound_keys.is_empty() {
            ui.add_space(6.0);
            ui.label("Keys that play sounds (in voice, even in a game):");
            let mut remove = None;
            for (i, (combo, sound)) in draft.sound_keys.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(combo_label(combo)).font(theme::bold_font(12.5)).color(theme::EMBER2));
                    ui.label(format!("{} {}", sound.emoji, sound.name));
                    if ui.small_button("Remove").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                draft.sound_keys.remove(i);
            }
        }
        if !draft.soundboard_muted.is_empty() {
            ui.add_space(6.0);
            ui.label("People whose sounds you've muted:");
            let mut unmute = None;
            for uid in &draft.soundboard_muted {
                let name = st.servers.iter().find_map(|s| s.members.get(uid).map(|m| m.name.clone())).unwrap_or_else(|| "someone".into());
                ui.horizontal(|ui| {
                    ui.label(name);
                    if ui.small_button("Unmute").clicked() {
                        unmute = Some(uid.clone());
                    }
                });
            }
            if let Some(u) = unmute {
                draft.soundboard_muted.retain(|x| *x != u);
            }
        }
    }
}

/// How long a played sound's emoji shows over whoever played it.
pub const POP_SECS: f32 = 2.6;

/// The sound `session` played last, while its emoji still shows: (it, seconds since).
pub fn recent_pop(call: &ingle_engine::state::CallView, session: ingle_proto::control::SessionId) -> Option<(&ingle_engine::state::SoundShown, f32)> {
    let now = std::time::Instant::now();
    call.sounds
        .iter()
        .rev()
        .find(|x| x.session == session)
        .map(|x| (x, now.saturating_duration_since(x.at).as_secs_f32()))
        .filter(|(_, age)| *age < POP_SECS)
}

/// How far into its show a sound's emoji is (`age` seconds): (size, from 0 to a little over 1
/// as it pops in; how far it has risen, in sizes; how solid it is, 1 to 0 as it fades).
pub fn pop_motion(age: f32) -> (f32, f32, f32) {
    let k = (age / 0.3).clamp(0.0, 1.0) - 1.0;
    // Eases out past full size and back ("back" easing).
    let back = 1.70158f32;
    let scale = 1.0 + (back + 1.0) * k * k * k + back * k * k;
    let fade = 0.8;
    let alpha = ((POP_SECS - age) / fade).clamp(0.0, 1.0);
    (scale.max(0.0), age * 0.35, alpha)
}

/// A played sound's emoji, centred on `at` at `size` (it pops in, rises and fades).
pub fn paint_pop(p: &egui::Painter, at: egui::Pos2, size: f32, text: &str, age: f32, custom: &mut dyn FnMut(&str) -> Option<egui::TextureId>) {
    let (scale, rise, alpha) = pop_motion(age);
    if scale <= 0.01 || alpha <= 0.0 {
        return;
    }
    let s = size * scale;
    let r = Rect::from_center_size(at - vec2(0.0, rise * size), vec2(s, s));
    let chars: Vec<char> = text.chars().collect();
    match emoji::match_at(&chars, 0) {
        Some((_, e)) => emoji::paint_faded(p, r, &e, custom, alpha),
        None => icons::speaker(p, r.shrink(s * 0.1), theme::TEXT.gamma_multiply(alpha)),
    }
}

/// A custom emoji's file id (`<:name:id>`), to get its picture.
pub fn custom_emoji_id(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    match emoji::match_at(&chars, 0) {
        Some((_, emoji::EmojiRef::Custom { id, .. })) => Some(id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_at_the_ends_is_left_out_of_the_cut() {
        let rate = 48_000usize;
        // 1 s of silence, 2 s of sound, 1.5 s of silence.
        let mut pcm = vec![0f32; rate * 2];
        pcm.extend((0..rate * 2).flat_map(|i| {
            let v = (i as f32 * 0.05).sin() * 0.5;
            [v, v]
        }));
        pcm.extend(vec![0f32; rate * 3]);
        let (a, b) = audible(&pcm);
        assert!((a - 0.95).abs() < 0.01, "starts just before the sound: {a}");
        assert!((b - 3.15).abs() < 0.01, "ends just after it: {b}");
        // All silence: the whole thing.
        assert_eq!(audible(&[0.0; 960]), (0.0, 0.01));
    }

    #[test]
    fn the_emoji_pops_in_rises_and_fades() {
        let (s0, _, a0) = pop_motion(0.0);
        assert!(s0 < 0.01 && a0 == 1.0, "starts small and solid");
        let peak = (1..30).map(|i| pop_motion(i as f32 * 0.01).0).fold(0f32, f32::max);
        assert!(peak > 1.05 && peak < 1.2, "overshoots a little: {peak}");
        let (s1, r1, a1) = pop_motion(1.0);
        assert!((s1 - 1.0).abs() < 1e-4 && r1 > 0.3 && a1 == 1.0);
        let (_, _, a_end) = pop_motion(POP_SECS);
        assert_eq!(a_end, 0.0, "gone at the end");
        assert!(pop_motion(POP_SECS - 0.4).2 > 0.4 && pop_motion(POP_SECS - 0.4).2 < 0.6);
    }
}
