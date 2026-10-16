//! The Plugins page (the library of plugins and skins, what's installed, making your own),
//! add-ons' panels, and the library owner's lists of plugins and skins sent in.

use eframe::egui::{self, Color32, CornerRadius, Key, Rect, RichText, Sense, Stroke, vec2};
use ingle_engine::state::{AppState, BugSend};
use ingle_proto::control::{SubmissionEntry, SubmissionStatus};
use ingle_proto::library::{Entry, Kind, SkinEntry};

use super::addon::{Place, Widget};
use super::{Show, Tab, source};
use crate::app::{Act, Dialog, IngleApp};
use crate::{icons, theme};

/// The form for sending a plugin in: what was found in its folder, and who's sending it.
pub struct SubmitForm {
    pub gathered: source::Gathered,
    pub author: String,
    pub contact: String,
    pub agree: bool,
    /// The plugin sends done when this form was filled in (to show only this one's outcome).
    pub sends_before: u64,
    /// Scroll it into view (it opens below the fold).
    pub scroll: bool,
}

const KIT_URL: &str = "https://github.com/bluelight777/ingle/tree/main/plugins";
const SKINS_URL: &str = "https://github.com/bluelight777/ingle/tree/main/skins";

/// How many pixels a side a picture shown `size` points big is decoded at: enough for this
/// screen, in a few steps (so a card's thumbnail and a page-wide picture of the same skin are
/// each sharp, and kept apart).
pub(crate) fn picture_side(ctx: &egui::Context, size: egui::Vec2) -> u32 {
    let px = size.x.max(size.y) * ctx.pixels_per_point();
    [256u32, 512, 1024, 2048].into_iter().find(|s| *s as f32 >= px).unwrap_or(2048)
}

/// "1 file", "3 files".
fn files_text(n: usize) -> String {
    if n == 1 { "1 file".to_string() } else { format!("{n} files") }
}

fn size_text(bytes: u64) -> String {
    if bytes >= 1_000_000 {
        format!("{:.1} MB", bytes as f64 / 1e6)
    } else {
        format!("{} KB", bytes.div_ceil(1000).max(1))
    }
}

fn pill(ui: &mut egui::Ui, text: &str, fill: Color32, color: Color32) {
    egui::Frame::new().fill(fill).corner_radius(5).inner_margin(egui::Margin::symmetric(6, 1)).show(ui, |ui| {
        ui.label(RichText::new(text).font(theme::bold_font(11.0)).color(color));
    });
}

fn kind_pill(ui: &mut egui::Ui, kind: Kind) {
    match kind {
        Kind::Program => pill(ui, "PROGRAM", Color32::from_rgb(0x2d, 0x33, 0x52), Color32::from_rgb(0xb9, 0xc2, 0xff)),
        Kind::Addon => pill(ui, "ADD-ON", Color32::from_rgb(0x3a, 0x2a, 0x16), theme::EMBER2()),
    }
}

fn note(ui: &mut egui::Ui, text: &str, error: bool) {
    let (fill, stroke) = if error { (Color32::from_rgb(0x3a, 0x1c, 0x1c), theme::LIVE()) } else { (theme::ELEV(), theme::LINE()) };
    egui::Frame::new().fill(fill).stroke(Stroke::new(1.0f32, stroke)).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).color(if error { theme::TEXT() } else { theme::MUTED() })).wrap());
    });
}

/// Widgets on an ELEV card, one step lighter: the theme's buttons, check boxes and radio
/// circles are ELEV themselves, so they'd vanish into the card.
fn card_visuals(ui: &mut egui::Ui) {
    let w = &mut ui.visuals_mut().widgets;
    let hover = Color32::from_rgb(0x46, 0x3b, 0x34);
    w.inactive.bg_fill = theme::ELEV2();
    w.inactive.weak_bg_fill = theme::ELEV2();
    w.hovered.bg_fill = hover;
    w.hovered.weak_bg_fill = hover;
}


/// A plugin or a skin, as the Library lists them together.
#[derive(Clone)]
enum Item {
    Plugin(Entry),
    Skin(SkinEntry),
}

impl Item {
    /// Its key among favorites (and in `Plugins::detail`): a plugin's id, `skin:<id>` for a skin.
    fn key(&self) -> String {
        match self {
            Item::Plugin(e) => e.id.clone(),
            Item::Skin(s) => format!("skin:{}", s.id),
        }
    }
    fn name(&self) -> &str {
        match self {
            Item::Plugin(e) => &e.name,
            Item::Skin(s) => &s.name,
        }
    }
    fn author(&self) -> &str {
        match self {
            Item::Plugin(e) => &e.author,
            Item::Skin(s) => &s.author,
        }
    }
    fn summary(&self) -> &str {
        match self {
            Item::Plugin(e) => &e.summary,
            Item::Skin(s) => &s.summary,
        }
    }
    fn description(&self) -> &str {
        match self {
            Item::Plugin(e) => &e.description,
            Item::Skin(s) => &s.description,
        }
    }
    fn updated(&self) -> i64 {
        match self {
            Item::Plugin(e) => e.updated,
            Item::Skin(s) => s.updated,
        }
    }
}

fn skin_pill(ui: &mut egui::Ui) {
    pill(ui, "SKIN", Color32::from_rgb(0x2b, 0x3a, 0x2e), Color32::from_rgb(0x9f, 0xe0, 0xb0));
}

/// The favorite star in a card's corner.
fn star_button(ui: &mut egui::Ui, on: bool) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::click());
    let c = if on { theme::OWNER() } else if resp.hovered() { theme::TEXT() } else { theme::DIM() };
    icons::star(ui.painter(), r.shrink(3.0), c, on);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if on { "Take it out of your favorites" } else { "Add it to your favorites" })
}

/// The part of a picture of size `pic` that covers a box of size `boxed` (cut evenly).
pub(crate) fn cover_uv(pic: egui::Vec2, boxed: egui::Vec2) -> Rect {
    if pic.x <= 0.0 || pic.y <= 0.0 || boxed.x <= 0.0 || boxed.y <= 0.0 {
        return Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    }
    let scale = (boxed.x / pic.x).max(boxed.y / pic.y);
    let (cw, ch) = (boxed.x / scale / pic.x, boxed.y / scale / pic.y);
    Rect::from_min_max(egui::pos2((1.0 - cw) / 2.0, (1.0 - ch) / 2.0), egui::pos2((1.0 + cw) / 2.0, (1.0 + ch) / 2.0))
}

impl IngleApp {
    /// Where an add-on is being used right now: who I am, and the server and channel open.
    pub(crate) fn addon_place(&self, st: &AppState) -> (Place, Option<(String, ingle_proto::control::ChannelId)>) {
        let here = (!self.ui.home && !self.ui.plugins_open)
            .then(|| self.ui.server.clone())
            .flatten()
            .and_then(|k| self.ui.channel.get(&k).map(|c| (k.clone(), *c)));
        let server = here.as_ref().and_then(|(k, _)| st.server(k));
        let channel = here.as_ref().and_then(|(_, c)| server.and_then(|s| s.channel(*c)).map(|c| c.name.clone())).unwrap_or_default();
        let place = Place { me: st.my_name.clone(), server: server.map(|s| s.info.name.clone()).unwrap_or_default(), channel };
        (place, here)
    }

    /// The Plugins button, at the bottom of the server rail (and the programs open inside
    /// Ingle under it).
    pub(crate) fn plugins_button(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
        let on = self.ui.plugins_open && self.plugins.program_page.is_none();
        let fill = if on {
            Color32::from_rgb(0x4a, 0x30, 0x22)
        } else if resp.hovered() {
            theme::ELEV2()
        } else {
            theme::ELEV()
        };
        ui.painter().rect_filled(rect, CornerRadius::same(if on || resp.hovered() { 16 } else { 24 }), fill);
        icons::puzzle(ui.painter(), rect.shrink(13.0), if on || resp.hovered() { theme::EMBER2() } else { theme::EMBER() });
        if on || resp.hovered() {
            let pill = Rect::from_center_size(egui::pos2(ui.max_rect().left() + 2.0, rect.center().y), vec2(4.0, if on { 40.0 } else { 20.0 }));
            ui.painter().rect_filled(pill, CornerRadius::same(2), theme::TEXT());
        }
        let updates = self.plugins.updates();
        if updates > 0 {
            let c = rect.right_bottom() - vec2(6.0, 6.0);
            ui.painter().circle_filled(c, 9.0, theme::RAIL());
            ui.painter().circle_filled(c, 7.5, Color32::from_rgb(0x23, 0xa5, 0x5a));
            ui.painter().text(c, egui::Align2::CENTER_CENTER, updates.min(9).to_string(), theme::bold_font(10.0), Color32::WHITE);
        }
        let tip = if updates > 0 { format!("Plugins ({updates} to update)") } else { "Plugins: programs, add-ons and skins for Ingle".to_string() };
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
            self.ui.plugins_open = true;
            self.ui.home = false;
            self.plugins.program_page = None;
            self.plugins.refresh(false);
        }
        self.program_buttons(ui);
    }

    /// Programs open inside Ingle, in the rail under Plugins: click one to go to it.
    fn program_buttons(&mut self, ui: &mut egui::Ui) {
        let open: Vec<(String, String, bool)> = self.plugins.programs.open.iter().map(|p| (p.id.clone(), p.name.clone(), p.popped)).collect();
        for (id, name, popped) in open {
            ui.add_space(8.0);
            let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
            let on = self.ui.plugins_open && self.plugins.program_page.as_deref() == Some(id.as_str());
            let hot = on || resp.hovered();
            ui.painter().rect_filled(rect, CornerRadius::same(if hot { 16 } else { 24 }), if hot { theme::ELEV2() } else { theme::ELEV() });
            let mut icon_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink(8.0)));
            self.plugin_icon(&mut icon_ui, &id, Kind::Program, 32.0);
            if hot {
                let pill = Rect::from_center_size(egui::pos2(ui.max_rect().left() + 2.0, rect.center().y), vec2(4.0, if on { 40.0 } else { 20.0 }));
                ui.painter().rect_filled(pill, CornerRadius::same(2), theme::TEXT());
            }
            // A green dot: it's running.
            let c = rect.right_bottom() - vec2(6.0, 6.0);
            ui.painter().circle_filled(c, 7.0, theme::RAIL());
            ui.painter().circle_filled(c, 5.0, theme::ONLINE());
            let tip = if popped { format!("{name} (in its own window)") } else { name.clone() };
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
            if resp.clicked() {
                self.show_program(&id);
            }
            resp.context_menu(|ui| {
                if popped {
                    if cfg!(windows) && ui.button("Bring into Ingle").clicked() {
                        self.plugins.programs.pop_in(&id);
                        self.show_program(&id);
                        ui.close();
                    }
                } else if ui.button("Pop out").clicked() {
                    self.plugins.programs.pop_out(&id);
                    ui.close();
                }
                if ui.button(RichText::new(format!("Close {name}")).color(theme::LIVE())).clicked() {
                    self.plugins.programs.close(&id);
                    ui.close();
                }
            });
        }
    }

    /// Shows a program's page (where it is, inside Ingle).
    pub(crate) fn show_program(&mut self, id: &str) {
        self.ui.plugins_open = true;
        self.ui.home = false;
        self.plugins.program_page = Some(id.to_string());
    }

    /// A program's page: its name and buttons along the top, and the program itself under them
    /// (its own window, kept right there: see `programs`).
    pub(crate) fn program_page(&mut self, ctx: &egui::Context, id: &str) {
        let Some((name, ready, slow, popped)) = self.plugins.programs.get(id).map(|p| (p.name.clone(), p.ready(), p.slow(), p.popped)) else {
            self.plugins.program_page = None;
            return;
        };
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::STAGE())).show(ctx, |ui| {
            egui::TopBottomPanel::top("program-header")
                .exact_height(48.0)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(theme::MAIN()).inner_margin(egui::Margin::symmetric(16, 0)))
                .show_inside(ui, |ui| {
                    ui.horizontal_centered(|ui| {
                        self.plugin_icon(ui, id, Kind::Program, 26.0);
                        ui.add_space(4.0);
                        ui.label(RichText::new(&name).font(theme::bold_font(15.5)).color(theme::TEXT()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").on_hover_text(format!("Close {name}")).clicked() {
                                self.plugins.programs.close(id);
                            }
                            if cfg!(windows) {
                                if popped {
                                    if ui.button("Bring into Ingle").clicked() {
                                        self.plugins.programs.pop_in(id);
                                    }
                                } else if ui.button("Pop out").on_hover_text(format!("{name} in a window of its own")).clicked() {
                                    self.plugins.programs.pop_out(id);
                                }
                            }
                        });
                    });
                    let r = ui.max_rect();
                    ui.painter().line_segment([egui::pos2(r.left() - 16.0, r.bottom()), egui::pos2(r.right() + 16.0, r.bottom())], Stroke::new(1.0f32, theme::SHADOW_LINE()));
                });
            egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::STAGE())).show_inside(ui, |ui| {
                let area = ui.max_rect();
                let c = area.center();
                let say = |ui: &egui::Ui, y: f32, text: &str, col: Color32, size: f32| {
                    ui.painter().text(egui::pos2(c.x, c.y + y), egui::Align2::CENTER_CENTER, text, theme::font(size), col);
                };
                if popped {
                    say(ui, -10.0, &format!("{name} is open in a window of its own."), theme::MUTED(), 15.0);
                    say(ui, 14.0, "Bring it back in with the button at the top.", theme::DIM(), 13.0);
                    return;
                }
                // Where the program goes (it shows over this once its window is found).
                self.ui.program_area = Some(area);
                if !ready {
                    let t = ui.input(|i| i.time) as f32;
                    icons::refresh(ui.painter(), Rect::from_center_size(egui::pos2(c.x, c.y - 30.0), vec2(28.0, 28.0)), theme::MUTED(), t * 5.0);
                    say(ui, 10.0, &format!("Opening {name}…"), theme::MUTED(), 15.0);
                    if slow {
                        say(ui, 34.0, "This is taking a while. If it opened in a window of its own, it'll stay there.", theme::DIM(), 13.0);
                    }
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(33));
                }
            });
        });
    }

    /// The Plugins page.
    pub(crate) fn plugins_page(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::panel(theme::MAIN(), true)).inner_margin(egui::Margin::symmetric(28, 20))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(10), theme::ELEV());
                icons::puzzle(ui.painter(), r.shrink(8.0), theme::EMBER());
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Plugins").font(theme::bold_font(22.0)).color(theme::TEXT()));
                    ui.label(RichText::new("Programs, add-ons and skins from the Ingle Library: every plugin built from its source after someone looked at it, every skin checked.").color(theme::MUTED()));
                });
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let installed = self.plugins.installed.plugins.len() + self.plugins.skins.skins.len();
                for (tab, label) in [(Tab::Library, "Library".to_string()), (Tab::Installed, format!("Installed ({installed})")), (Tab::Make, "Make your own".to_string())] {
                    let selected = self.plugins.tab == tab;
                    if ui.add(egui::Button::selectable(selected, RichText::new(label).font(theme::bold_font(14.0))).min_size(vec2(0.0, 30.0))).clicked() {
                        self.plugins.tab = tab;
                        self.plugins.detail = None;
                    }
                }
                if self.plugins.tab == Tab::Library && self.plugins.detail.is_none() {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.plugins.fetching() {
                            ui.spinner();
                        } else if ui.button("Refresh").on_hover_text("Get the latest list from the Ingle Library").clicked() {
                            self.plugins.refresh(true);
                        }
                        ui.add(egui::TextEdit::singleline(&mut self.plugins.search).hint_text("Search plugins and skins").desired_width(220.0));
                    });
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(8.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.plugins.tab {
                Tab::Library => match self.plugins.detail.clone() {
                    Some(id) => self.plugin_detail(ui, &id, st, acts),
                    None => self.library_grid(ui, st, acts),
                },
                Tab::Installed => self.installed_list(ui, st, acts),
                Tab::Make => self.make_your_own(ui, st, acts),
            });
        });
    }

    fn plugin_icon(&mut self, ui: &mut egui::Ui, id: &str, kind: Kind, side: f32) {
        let (r, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
        let bytes = self.plugins.icons.get(id).cloned();
        let key = format!("plugin-icon:{id}");
        let tex = bytes.and_then(|b| self.images.get(ui.ctx(), &key, Some(b), 128).frame(0.0));
        match tex {
            Some((tex, _, _)) => {
                ui.painter().image(tex, r, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                ui.painter().rect_filled(r, CornerRadius::same((side / 4.5) as u8), theme::ELEV2());
                let c = if kind == Kind::Program { Color32::from_rgb(0xb9, 0xc2, 0xff) } else { theme::EMBER2() };
                icons::puzzle(ui.painter(), r.shrink(side * 0.24), c);
            }
        }
    }

    /// The button for a plugin: Install, Open, Update, or how far along it is.
    fn plugin_action(&mut self, ui: &mut egui::Ui, e: &Entry, acts: &mut Vec<Act>) {
        if let Some((done, total)) = self.plugins.progress(&e.id) {
            ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).desired_width(120.0).show_percentage());
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        let installed = self.plugins.installed.get(&e.id).cloned();
        let available = !e.files_for(super::install::os()).is_empty();
        match installed {
            None if !available => {
                let only = match (e.windows.is_empty(), e.linux.is_empty()) {
                    (false, true) => "Windows only for now",
                    (true, false) => "Linux only for now",
                    _ => "Not for this system yet",
                };
                ui.label(RichText::new(only).color(theme::DIM()));
            }
            None => {
                if ui.add(theme::primary_button("Install").min_size(vec2(92.0, 30.0))).clicked() {
                    self.plugins.install(e);
                }
            }
            Some(p) => {
                if self.plugins.update_for(&p).is_some() {
                    if ui.add(theme::primary_button(&format!("Update to {}", e.version)).min_size(vec2(92.0, 30.0))).clicked() {
                        self.plugins.install(e);
                    }
                }
                match p.kind {
                    Kind::Program => {
                        if ui.add(egui::Button::new("Open").min_size(vec2(72.0, 30.0))).clicked() {
                            acts.push(Act::PluginOpen(e.id.clone()));
                        }
                    }
                    Kind::Addon => {
                        let running = self.plugins.addons.contains_key(&e.id);
                        if running && ui.add(egui::Button::new("Open panel").min_size(vec2(92.0, 30.0))).clicked() {
                            acts.push(Act::AddonPanel(e.id.clone()));
                        }
                        if !running {
                            ui.label(RichText::new(if p.enabled { "Starting…" } else { "Off" }).color(theme::DIM()));
                        }
                    }
                }
            }
        }
    }

    fn library_grid(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        let Some(lib) = self.plugins.library.clone() else {
            if self.plugins.fetching() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Getting the list from the Ingle Library…").color(theme::MUTED()));
                });
            } else {
                let why = self.plugins.library_error.clone().unwrap_or_else(|| "The list hasn't been fetched yet.".into());
                note(ui, &format!("Couldn't get the plugin list: {why}"), true);
                ui.add_space(8.0);
                if ui.button("Try again").clicked() {
                    self.plugins.refresh(true);
                }
            }
            return;
        };
        if let Some(e) = &self.plugins.library_error {
            note(ui, &format!("Showing the list from before: {e}"), false);
            ui.add_space(8.0);
        }
        // What to show, and in what order.
        let favorites = &st.settings.library_favorites;
        let sort = if st.settings.library_sort.is_empty() { "favorites" } else { st.settings.library_sort.as_str() };
        ui.horizontal_wrapped(|ui| {
            for (show, label) in [(Show::All, "All"), (Show::Programs, "Programs"), (Show::Addons, "Add-ons"), (Show::Skins, "Skins"), (Show::Favorites, "★ Favorites")] {
                if ui.add(egui::Button::selectable(self.plugins.show == show, label)).clicked() {
                    self.plugins.show = show;
                }
            }
            if let Some(a) = self.plugins.author.clone() {
                ui.add_space(6.0);
                if ui.add(egui::Button::new(RichText::new(format!("by {a}  ✕")).color(theme::TEXT())).fill(theme::ELEV2())).on_hover_text("Show everyone's").clicked() {
                    self.plugins.author = None;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = |s: &str| match s {
                    "name" => "Name",
                    "author" => "Author",
                    "newest" => "Newest",
                    _ => "Favorites first",
                };
                egui::ComboBox::from_id_salt("library-sort").selected_text(format!("Sort: {}", label(sort))).width(170.0).show_ui(ui, |ui| {
                    for by in ["favorites", "name", "author", "newest"] {
                        if ui.selectable_label(sort == by, label(by)).clicked() && sort != by {
                            acts.push(Act::SortLibrary(by.to_string()));
                        }
                    }
                });
            });
        });
        ui.add_space(10.0);
        let q = self.plugins.search.trim().to_lowercase();
        let mut shown: Vec<Item> = lib.plugins.iter().cloned().map(Item::Plugin).chain(lib.skins.iter().cloned().map(Item::Skin)).collect();
        shown.retain(|it| {
            let kind_ok = match self.plugins.show {
                Show::All => true,
                Show::Programs => matches!(it, Item::Plugin(e) if e.kind == Kind::Program),
                Show::Addons => matches!(it, Item::Plugin(e) if e.kind == Kind::Addon),
                Show::Skins => matches!(it, Item::Skin(_)),
                Show::Favorites => favorites.contains(&it.key()),
            };
            let author_ok = self.plugins.author.as_ref().is_none_or(|a| it.author().eq_ignore_ascii_case(a));
            let text_ok = q.is_empty() || format!("{} {} {} {}", it.name(), it.summary(), it.author(), it.description()).to_lowercase().contains(&q);
            kind_ok && author_ok && text_ok
        });
        let by_name = |a: &Item, b: &Item| a.name().to_lowercase().cmp(&b.name().to_lowercase());
        match sort {
            "name" => shown.sort_by(by_name),
            "author" => shown.sort_by(|a, b| (a.author().is_empty(), a.author().to_lowercase()).cmp(&(b.author().is_empty(), b.author().to_lowercase())).then(by_name(a, b))),
            "newest" => shown.sort_by(|a, b| b.updated().cmp(&a.updated()).then(by_name(a, b))),
            _ => shown.sort_by(|a, b| (!favorites.contains(&a.key())).cmp(&!favorites.contains(&b.key())).then(by_name(a, b))),
        }
        if shown.is_empty() {
            let why = match (self.plugins.show, q.is_empty()) {
                (_, false) => "Nothing matches that.",
                (Show::Favorites, _) => "No favorites yet: star a plugin or skin to keep it here.",
                (Show::Skins, _) => "No skins in the Library yet.",
                _ => "The library is empty for now.",
            };
            ui.label(RichText::new(why).color(theme::MUTED()));
            return;
        }
        let width = ui.available_width();
        let cols = ((width + 12.0) / 340.0).floor().max(1.0) as usize;
        let card_w = (width - 12.0 * (cols as f32 - 1.0)) / cols as f32;
        // Card content height: the summary gets up to three lines, and the buttons sit at the bottom.
        const CARD_H: f32 = 172.0;
        for row in shown.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for it in row {
                    let mut open = false;
                    let key = it.key();
                    // A Ui that senses clicks behind the card's own buttons (it's registered before them),
                    // laid out top to bottom (a Frame takes its parent's layout, and this row's is sideways).
                    let card = ui.scope_builder(
                        egui::UiBuilder::new().id_salt(("plugin-card", &key)).layout(egui::Layout::top_down(egui::Align::Min)).sense(Sense::click()),
                        |ui| {
                            let hovered = ui.response().hovered();
                            let stroke = if hovered { theme::ELEV2().gamma_multiply(1.6) } else { theme::LINE() };
                            egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0f32, stroke)).corner_radius(12).inner_margin(16).show(ui, |ui| {
                                card_visuals(ui);
                                ui.set_width(card_w - 34.0);
                                let top = ui.min_rect().top();
                                ui.horizontal(|ui| {
                                    match it {
                                        Item::Plugin(e) => self.plugin_icon(ui, &e.id, e.kind, 48.0),
                                        Item::Skin(s) => self.skin_picture(ui, &s.id, vec2(48.0, 48.0)),
                                    }
                                    ui.add_space(4.0);
                                    ui.vertical(|ui| {
                                        ui.set_max_width(ui.available_width() - 30.0);
                                        ui.add(egui::Label::new(RichText::new(it.name()).font(theme::bold_font(17.0)).color(theme::TEXT())).truncate());
                                        ui.horizontal(|ui| {
                                            match it {
                                                Item::Plugin(e) => kind_pill(ui, e.kind),
                                                Item::Skin(_) => skin_pill(ui),
                                            }
                                            self.author_link(ui, it.author());
                                        });
                                    });
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                                        if star_button(ui, favorites.contains(&key)).clicked() {
                                            acts.push(Act::ToggleFavorite(key.clone()));
                                        }
                                    });
                                });
                                ui.add_space(10.0);
                                let mut job = egui::text::LayoutJob::simple(it.summary().to_string(), egui::TextStyle::Body.resolve(ui.style()), theme::BODY(), ui.available_width());
                                job.wrap.max_rows = 3;
                                ui.label(job);
                                let used = ui.min_rect().bottom() - top;
                                ui.add_space((CARD_H - 30.0 - used).max(10.0));
                                ui.horizontal(|ui| {
                                    ui.set_min_height(30.0);
                                    match it {
                                        Item::Plugin(e) => {
                                            let size = e.size_for(super::install::os()).max(e.wasm.as_ref().map(|w| w.size).unwrap_or(0));
                                            // (With an update to offer, its button says the version, and
                                            // there's no room for this as well.)
                                            let update = self.plugins.installed.get(&e.id).is_some_and(|p| self.plugins.update_for(p).is_some());
                                            if !update {
                                                ui.label(RichText::new(if size > 0 { format!("{} · {}", e.version, size_text(size)) } else { e.version.clone() }).size(12.5).color(theme::DIM()));
                                            }
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.button("Details").clicked() {
                                                    open = true;
                                                }
                                                self.plugin_action(ui, e, acts);
                                            });
                                        }
                                        Item::Skin(sk) => {
                                            let update = self.plugins.skins.get(&sk.id).is_some_and(|i| self.plugins.skin_update_for(i).is_some());
                                            if !update {
                                                ui.label(RichText::new(format!("{} · {}", sk.version, size_text(sk.size()))).size(12.5).color(theme::DIM()));
                                            }
                                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                                if ui.button("Details").clicked() {
                                                    open = true;
                                                }
                                                self.skin_action(ui, sk, st, acts);
                                            });
                                        }
                                    }
                                });
                            });
                        },
                    );
                    if open || card.response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.plugins.detail = Some(key.clone());
                    }
                }
            });
            ui.add_space(12.0);
        }
    }

    /// "by <author>" under a plugin or skin: click it for everything they made.
    fn author_link(&mut self, ui: &mut egui::Ui, author: &str) {
        if author.trim().is_empty() {
            return;
        }
        let resp = ui.add(egui::Label::new(RichText::new(format!("by {author}")).size(12.5).color(theme::MUTED())).sense(Sense::click()));
        if resp.hovered() {
            ui.painter().hline(resp.rect.x_range(), resp.rect.bottom() - 1.0, Stroke::new(1.0f32, theme::MUTED()));
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("Everything by {author}")).clicked() {
            self.plugins.author = Some(author.trim().to_string());
            self.plugins.detail = None;
        }
    }

    /// A skin's picture from the Library (or a palette while there's none).
    fn skin_picture(&mut self, ui: &mut egui::Ui, id: &str, size: egui::Vec2) {
        let (r, _) = ui.allocate_exact_size(size, Sense::hover());
        let key = format!("skin:{id}");
        let bytes = self.plugins.icons.get(&key).cloned();
        let side = picture_side(ui.ctx(), size);
        let tex = bytes.and_then(|b| self.images.get(ui.ctx(), &format!("skin-preview:{id}:{side}"), Some(b), side).frame(0.0));
        match tex {
            Some((tex, ts, _)) => {
                // Cover the box, cut evenly.
                let uv = cover_uv(ts, r.size());
                let rounding = CornerRadius::same((size.y / 4.5).min(14.0) as u8);
                ui.painter().add(egui::epaint::RectShape::filled(r, rounding, Color32::WHITE).with_texture(tex, uv));
            }
            None => {
                ui.painter().rect_filled(r, CornerRadius::same((size.y / 4.5).min(14.0) as u8), theme::ELEV2());
                icons::palette(ui.painter(), r.shrink(size.y.min(size.x) * 0.2));
            }
        }
    }

    /// The button for a skin: Install, Wear, Wearing, Update, or how far along it is.
    fn skin_action(&mut self, ui: &mut egui::Ui, s: &SkinEntry, st: &AppState, acts: &mut Vec<Act>) {
        if let Some((done, total)) = self.plugins.progress(&format!("skin:{}", s.id)) {
            ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32).desired_width(120.0).show_percentage());
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
            return;
        }
        let Some(installed) = self.plugins.skins.get(&s.id).cloned() else {
            if ui.add(theme::primary_button("Install").min_size(vec2(92.0, 30.0))).clicked() {
                acts.push(Act::InstallSkin(s.clone()));
            }
            return;
        };
        if self.plugins.skin_update_for(&installed).is_some() && ui.add(theme::primary_button(&format!("Update to {}", s.version)).min_size(vec2(92.0, 30.0))).clicked() {
            acts.push(Act::InstallSkin(s.clone()));
        }
        if st.settings.skin == s.id {
            if ui.button("Take off").on_hover_text("Back to Campfire, Ingle's own").clicked() {
                acts.push(Act::WearSkin(String::new()));
            }
            ui.label(RichText::new("✓ Wearing").color(theme::ONLINE()));
        } else if ui.add(egui::Button::new("Wear").min_size(vec2(72.0, 30.0))).clicked() {
            acts.push(Act::WearSkin(s.id.clone()));
        }
    }

    fn skin_detail(&mut self, ui: &mut egui::Ui, id: &str, st: &AppState, acts: &mut Vec<Act>) {
        let Some(s) = self.plugins.library.as_ref().and_then(|l| l.find_skin(id)).cloned() else {
            self.plugins.detail = None;
            return;
        };
        ui.set_max_width(860.0);
        if ui.add(egui::Button::new(RichText::new("← All plugins and skins").color(theme::LINK())).frame(false)).clicked() {
            self.plugins.detail = None;
        }
        ui.add_space(10.0);
        if self.plugins.icons.contains_key(&format!("skin:{id}")) {
            let w = ui.available_width().min(860.0);
            self.skin_picture(ui, id, vec2(w, (w * 0.5).min(380.0)));
            ui.add_space(12.0);
        }
        ui.horizontal(|ui| {
            ui.label(RichText::new(&s.name).font(theme::bold_font(26.0)).color(theme::TEXT()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let key = format!("skin:{id}");
                if star_button(ui, st.settings.library_favorites.contains(&key)).clicked() {
                    acts.push(Act::ToggleFavorite(key));
                }
            });
        });
        ui.horizontal(|ui| {
            skin_pill(ui);
            let mut bits = vec![format!("Version {}", s.version)];
            if !s.license.is_empty() {
                bits.push(s.license.clone());
            }
            ui.label(RichText::new(bits.join(" · ")).color(theme::MUTED()));
            self.author_link(ui, &s.author);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            self.skin_action(ui, &s, st, acts);
            let installed = self.plugins.skins.get(id).is_some();
            if installed && self.plugins.progress(&format!("skin:{id}")).is_none() && ui.button("Remove").on_hover_text("Takes it off too, if it's on").clicked() {
                acts.push(Act::RemoveSkin(id.to_string()));
            }
        });
        ui.add_space(14.0);
        ui.label(RichText::new(&s.summary).font(theme::bold_font(15.0)).color(theme::TEXT()));
        if !s.description.is_empty() {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(&s.description).color(theme::BODY())).wrap());
        }
        ui.add_space(14.0);
        ui.label(RichText::new("WHAT IT CHANGES").font(theme::bold_font(11.5)).color(theme::DIM()));
        ui.add_space(4.0);
        // Installed: what its skin.json says. Otherwise: what its files are.
        let manifest = self
            .plugins
            .skins
            .get(id)
            .and_then(|_| std::fs::read(self.plugins.skins_dir.join(id).join(ingle_proto::skin::MANIFEST)).ok())
            .and_then(|b| serde_json::from_slice::<ingle_proto::skin::Manifest>(&b).ok());
        match manifest {
            Some(m) => skin_summary(ui, &m),
            None => {
                let files: Vec<String> = s.files.iter().filter(|f| f.path != ingle_proto::skin::MANIFEST).map(|f| format!("{} ({})", f.path, size_text(f.size))).collect();
                let text = if files.is_empty() { "Its colors.".to_string() } else { format!("Its colors, and {}.", files.join(", ")) };
                ui.add(egui::Label::new(RichText::new(text).color(theme::BODY())).wrap());
            }
        }
        ui.add_space(8.0);
        note(
            ui,
            "Skins are colors, a picture, fonts and sounds: no code. Ingle checks every file before it uses one, and you can go back to Campfire, Ingle's own, at any time (Settings → Appearance).",
            false,
        );
        if !s.review.is_empty() {
            ui.add_space(14.0);
            ui.label(RichText::new("FROM THE REVIEW").font(theme::bold_font(11.5)).color(theme::DIM()));
            ui.add_space(4.0);
            note(ui, &s.review, false);
        }
        ui.add_space(14.0);
        ui.label(RichText::new(format!("Download: {}", size_text(s.size()))).size(12.5).color(theme::DIM()));
    }

    fn plugin_detail(&mut self, ui: &mut egui::Ui, id: &str, st: &AppState, acts: &mut Vec<Act>) {
        if let Some(skin) = id.strip_prefix("skin:") {
            return self.skin_detail(ui, skin, st, acts);
        }
        let Some(e) = self.plugins.library.as_ref().and_then(|l| l.find(id)).cloned() else {
            self.plugins.detail = None;
            return;
        };
        // Lines that span a wide window are hard to read.
        ui.set_max_width(860.0);
        if ui.add(egui::Button::new(RichText::new("← All plugins and skins").color(theme::LINK())).frame(false)).clicked() {
            self.plugins.detail = None;
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            self.plugin_icon(ui, &e.id, e.kind, 72.0);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(&e.name).font(theme::bold_font(26.0)).color(theme::TEXT()));
                ui.horizontal(|ui| {
                    kind_pill(ui, e.kind);
                    let mut bits = vec![format!("Version {}", e.version)];
                    if !e.license.is_empty() {
                        bits.push(e.license.clone());
                    }
                    ui.label(RichText::new(bits.join(" · ")).color(theme::MUTED()));
                    self.author_link(ui, &e.author);
                    if star_button(ui, st.settings.library_favorites.contains(&e.id)).clicked() {
                        acts.push(Act::ToggleFavorite(e.id.clone()));
                    }
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    self.plugin_action(ui, &e, acts);
                    if self.plugins.installed.get(&e.id).is_some() && self.plugins.progress(&e.id).is_none() && ui.button("Remove").clicked() {
                        self.plugins.confirm_remove = Some(e.id.clone());
                    }
                    if !e.source.is_empty() && ui.button("Source code").on_hover_text(e.source.as_str()).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&e.source));
                    }
                });
            });
        });
        self.confirm_remove_row(ui, &e.id);
        ui.add_space(14.0);
        ui.label(RichText::new(&e.summary).font(theme::bold_font(15.0)).color(theme::TEXT()));
        if !e.description.is_empty() {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(&e.description).color(theme::BODY())).wrap());
        }
        ui.add_space(14.0);
        ui.label(RichText::new("WHAT IT CAN DO").font(theme::bold_font(11.5)).color(theme::DIM()));
        ui.add_space(4.0);
        match e.kind {
            Kind::Program => note(
                ui,
                "It's a program of its own, opened from Ingle: it runs on your PC like any app you install (it isn't sandboxed). \
                 Firelit Games read its source and built it from that source; Ingle checks it's that build before opening it.",
                false,
            ),
            Kind::Addon => {
                if e.permissions.is_empty() {
                    note(ui, "Runs inside Ingle in a sandbox, and asks for nothing.", false);
                }
                for p in &e.permissions {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        icons::tick(ui.painter(), r, theme::ONLINE());
                        ui.add(egui::Label::new(RichText::new(p.describe()).color(theme::BODY())).wrap());
                    });
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Add-ons run inside Ingle in a sandbox: they can't reach your files, the internet or anything else on your PC.")
                        .size(12.5)
                        .color(theme::DIM()),
                );
            }
        }
        if !e.review.is_empty() {
            ui.add_space(14.0);
            ui.label(RichText::new("FROM THE REVIEW").font(theme::bold_font(11.5)).color(theme::DIM()));
            ui.add_space(4.0);
            note(ui, &e.review, false);
        }
        ui.add_space(14.0);
        let size = e.size_for(super::install::os());
        if size > 0 {
            ui.label(RichText::new(format!("Download: {}", size_text(size))).size(12.5).color(theme::DIM()));
        }
    }

    fn confirm_remove_row(&mut self, ui: &mut egui::Ui, id: &str) {
        if self.plugins.confirm_remove.as_deref() != Some(id) {
            return;
        }
        let name = self.plugins.installed.get(id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string());
        ui.add_space(8.0);
        egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0f32, theme::LIVE())).corner_radius(8).inner_margin(10).show(ui, |ui| {
            card_visuals(ui);
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Remove {name} and its data from this PC?")).color(theme::TEXT()));
                if ui.add(theme::danger_button("Remove")).clicked() {
                    match self.plugins.uninstall(id) {
                        Ok(()) => self.engine.shared.state.write().toast(format!("{name} was removed."), false),
                        Err(e) => self.engine.shared.state.write().toast(format!("Couldn't remove {name}: {e}"), true),
                    }
                    self.plugins.confirm_remove = None;
                }
                if ui.button("Keep it").clicked() {
                    self.plugins.confirm_remove = None;
                }
            });
        });
    }

    fn installed_list(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        if let Some(tried) = self.plugins.tried.clone().filter(|t| self.plugins.addons.contains_key(t)) {
            let name = self.plugins.addons[&tried].info.name.clone();
            egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0f32, theme::EMBER2())).corner_radius(10).inner_margin(12).show(ui, |ui| {
                card_visuals(ui);
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width((ui.available_width() - 220.0).max(160.0));
                        ui.label(RichText::new(format!("Trying {name}")).font(theme::bold_font(15.0)).color(theme::TEXT()));
                        ui.add(
                            egui::Label::new(RichText::new("From a file, until Ingle closes. Nobody has reviewed it, but it runs in the same sandbox as any add-on.").color(theme::MUTED()))
                                .wrap(),
                        );
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Stop").clicked() {
                            self.plugins.addons.remove(&tried);
                            self.plugins.panels.retain(|p| p.id != tried);
                            self.plugins.tried = None;
                        }
                        if ui.button("Open panel").clicked() {
                            acts.push(Act::AddonPanel(tried.clone()));
                        }
                    });
                });
            });
            ui.add_space(10.0);
        }
        let list = self.plugins.installed.plugins.clone();
        if list.is_empty() && self.plugins.skins.skins.is_empty() {
            ui.label(RichText::new("Nothing installed yet. Have a look in the Library.").color(theme::MUTED()));
            return;
        }
        self.installed_skins(ui, st, acts);
        for p in list {
            egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0f32, theme::LINE())).corner_radius(10).inner_margin(12).show(ui, |ui| {
                card_visuals(ui);
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    self.plugin_icon(ui, &p.id, p.kind, 40.0);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        // Leave room for its buttons on the right.
                        ui.set_max_width((ui.available_width() - 380.0).max(160.0));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&p.name).font(theme::bold_font(15.0)).color(theme::TEXT()));
                            kind_pill(ui, p.kind);
                            ui.label(RichText::new(format!("Version {}", p.version)).size(12.5).color(theme::DIM()));
                        });
                        let status = if self.plugins.listed(&p.id) == Some(false) {
                            Some(("No longer in the Ingle Library: remove it.".to_string(), true))
                        } else if let Some(why) = self.plugins.broken.get(&p.id) {
                            Some((format!("Stopped: {why}"), true))
                        } else if let Some(e) = self.plugins.update_for(&p) {
                            Some((format!("Version {} is out.", e.version), false))
                        } else {
                            None
                        };
                        if let Some((s, bad)) = status {
                            ui.label(RichText::new(s).size(12.5).color(if bad { theme::LIVE() } else { theme::ONLINE() }));
                        } else if let Some(e) = self.plugins.library.as_ref().and_then(|l| l.find(&p.id)) {
                            ui.add(egui::Label::new(RichText::new(&e.summary).size(12.5).color(theme::MUTED())).truncate());
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Remove").clicked() {
                            self.plugins.confirm_remove = Some(p.id.clone());
                        }
                        if let Some(e) = self.plugins.library.as_ref().and_then(|l| l.find(&p.id)).cloned() {
                            if ui.button("Details").clicked() {
                                self.plugins.tab = Tab::Library;
                                self.plugins.detail = Some(p.id.clone());
                            }
                            self.plugin_action(ui, &e, acts);
                        } else if p.kind == Kind::Program && self.plugins.library.is_none() && ui.button("Open").clicked() {
                            acts.push(Act::PluginOpen(p.id.clone()));
                        }
                        if p.kind == Kind::Addon {
                            let mut on = p.enabled;
                            if ui.checkbox(&mut on, "On").on_hover_text("Off: its commands and panel go away (nothing is removed)").changed() {
                                self.plugins.set_enabled(&p.id, on);
                            }
                        }
                    });
                });
                self.confirm_remove_row(ui, &p.id);
            });
            ui.add_space(8.0);
        }
    }

    /// Installed skins (first in the Installed tab), and Campfire.
    fn installed_skins(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        let skins = self.plugins.skins.skins.clone();
        if skins.is_empty() {
            return;
        }
        ui.label(RichText::new("SKINS").font(theme::bold_font(11.5)).color(theme::DIM()));
        ui.add_space(4.0);
        for s in skins {
            egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0f32, theme::LINE())).corner_radius(10).inner_margin(12).show(ui, |ui| {
                card_visuals(ui);
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    self.skin_picture(ui, &s.id, vec2(64.0, 40.0));
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.set_max_width((ui.available_width() - 300.0).max(160.0));
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&s.name).font(theme::bold_font(15.0)).color(theme::TEXT()));
                            skin_pill(ui);
                            ui.label(RichText::new(format!("Version {}", s.version)).size(12.5).color(theme::DIM()));
                        });
                        let listed = self.plugins.library.as_ref().map(|l| l.find_skin(&s.id).is_some());
                        if listed == Some(false) {
                            ui.label(RichText::new("No longer in the Ingle Library: remove it.").size(12.5).color(theme::LIVE()));
                        } else if let Some(e) = self.plugins.skin_update_for(&s) {
                            ui.label(RichText::new(format!("Version {} is out.", e.version)).size(12.5).color(theme::ONLINE()));
                        } else {
                            self.author_link(ui, &s.author);
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Remove").clicked() {
                            acts.push(Act::RemoveSkin(s.id.clone()));
                        }
                        match self.plugins.library.as_ref().and_then(|l| l.find_skin(&s.id)).cloned() {
                            Some(e) => {
                                if ui.button("Details").clicked() {
                                    self.plugins.tab = Tab::Library;
                                    self.plugins.detail = Some(format!("skin:{}", s.id));
                                }
                                self.skin_action(ui, &e, st, acts);
                            }
                            None if st.settings.skin == s.id => {
                                if ui.button("Take off").clicked() {
                                    acts.push(Act::WearSkin(String::new()));
                                }
                            }
                            None => {}
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
        ui.add_space(8.0);
        if !self.plugins.installed.plugins.is_empty() {
            ui.label(RichText::new("PLUGINS").font(theme::bold_font(11.5)).color(theme::DIM()));
            ui.add_space(4.0);
        }
    }

    /// What I sent in, and where each stands (asked for with Check).
    fn sent_in_list(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("What you sent in").font(theme::bold_font(17.0)).color(theme::TEXT()));
            if ui.button(if st.my_submissions.is_some() { "Check again" } else { "Check" }).clicked() {
                acts.push(Act::MySubmissions);
            }
        });
        ui.add(
            egui::Label::new(
                RichText::new("Plugins and skins sent from this Ingle (or by Claude through its connector), and whether they were approved.").color(theme::MUTED()),
            )
            .wrap(),
        );
        ui.add_space(6.0);
        match &st.my_submissions {
            None => {}
            Some(Err(e)) => note(ui, &format!("Couldn't check: {e}"), true),
            Some(Ok(list)) if list.is_empty() => {
                ui.label(RichText::new("Nothing yet (sent from this Ingle, signed with its identity).").color(theme::DIM()));
            }
            Some(Ok(list)) => {
                for e in list {
                    egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0f32, theme::LINE())).corner_radius(10).inner_margin(10).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("#{} {}", e.id, e.name)).font(theme::bold_font(14.5)).color(theme::TEXT()));
                            if e.kind == ingle_proto::control::SKIN_KIND {
                                skin_pill(ui);
                            } else {
                                kind_pill(ui, if e.kind == "program" { Kind::Program } else { Kind::Addon });
                            }
                            ui.label(RichText::new(format!("version {}", e.version)).color(theme::DIM()));
                            if e.update_of != 0 {
                                ui.label(RichText::new(format!("(a new version of #{})", e.update_of)).color(theme::DIM()));
                            }
                        });
                        let (said, color) = match e.status {
                            SubmissionStatus::New => ("Waiting to be looked at", theme::MUTED()),
                            SubmissionStatus::Approved => ("Approved: it'll be in the Ingle Library soon", theme::ONLINE()),
                            SubmissionStatus::Listed => ("In the Ingle Library", theme::ONLINE()),
                            SubmissionStatus::Rejected => ("Not approved", theme::LIVE()),
                        };
                        ui.label(RichText::new(said).color(color));
                        if !e.note.is_empty() {
                            ui.add(egui::Label::new(RichText::new(format!("From the reviewer: {}", e.note)).color(theme::BODY())).wrap().selectable(true));
                        }
                    });
                    ui.add_space(6.0);
                }
            }
        }
    }

    fn make_your_own(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        let section = |ui: &mut egui::Ui, title: &str, body: &str| {
            ui.label(RichText::new(title).font(theme::bold_font(17.0)).color(theme::TEXT()));
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(body).color(theme::MUTED())).wrap());
            ui.add_space(8.0);
        };
        ui.set_max_width(760.0);
        section(
            ui,
            "Make a plugin with Claude",
            "Plugins are written in Rust. Add-ons run inside Ingle in a sandbox and can add /commands to chat, show a panel and keep a little data; \
             programs are apps of their own, like Chimbrowser. Start an add-on here, open its folder with Claude (Claude Code or Cowork) and say \
             what you'd like it to do: the folder's CLAUDE.md explains Ingle add-ons to Claude. With the Claude connector set up, Claude can \
             also send it in for review, and send new versions of it later.",
        );
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.plugins.new_name).hint_text("Its name, like Coin Flip").desired_width(220.0));
            let ok = !self.plugins.new_name.trim().is_empty();
            if ui.add_enabled(ok, theme::primary_button("Start an add-on in a folder…")).clicked() {
                crate::images::pick_folder_async(crate::images::FolderFor::NewAddon, "Where should the add-on's folder go?", ui.ctx());
            }
            if ui.add(egui::Button::new(RichText::new("The plugin kit on GitHub").color(theme::LINK())).frame(false)).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(KIT_URL));
            }
        });
        ui.add_space(22.0);
        section(
            ui,
            "Try an add-on file",
            "Built an add-on? Try its .wasm file here before sending it in. It runs in the same sandbox as any add-on, until Ingle closes.",
        );
        if ui.button("Try an add-on file…").clicked() {
            crate::images::pick_async(crate::images::PickFor::AddonFile, false, ui.ctx());
        }
        ui.add_space(22.0);
        section(
            ui,
            "Make a skin with Claude",
            "Skins change Ingle's colors, put a picture behind the chat, and bring fonts and sounds of their own. Set up the Claude connector, \
             then ask Claude to make you one: it can show it to you right here as it goes, and send it in for the Ingle Library. Once it's \
             approved, anyone can wear it.",
        );
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button("Set up the Claude connector…")).clicked() {
                self.ui.dialog = Some(Dialog::Settings { tab: 6 });
            }
            if ui.button("Try a skin folder…").on_hover_text("A skin you're making yourself: Ingle wears it until you stop, and again whenever you save a change").clicked() {
                crate::images::pick_folder_async(crate::images::FolderFor::TrySkin, "Choose the skin's folder (with its skin.json)", ui.ctx());
            }
            if ui.add(egui::Button::new(RichText::new("How skins are made").color(theme::LINK())).frame(false)).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(SKINS_URL));
            }
        });
        ui.add_space(22.0);
        self.sent_in_list(ui, st, acts);
        ui.add_space(22.0);
        section(
            ui,
            "Send a plugin for review",
            "Pick the plugin's folder (the one with its Cargo.toml). Ingle sends its source, not a built file: every plugin in the library is built \
             from its source, after someone has looked at it. Rust only, up to 600 KB.",
        );
        if !ingle_engine::plugins::can_submit() {
            note(ui, "This copy of Ingle has nowhere to send plugins.", true);
            return;
        }
        if ui.add(theme::primary_button("Choose the plugin's folder…")).clicked() {
            crate::images::pick_folder_async(crate::images::FolderFor::PluginSource, "Choose the plugin's folder (with its Cargo.toml)", ui.ctx());
        }
        let Some(form) = self.plugins.form.as_mut() else { return };
        ui.add_space(12.0);
        let mut send = false;
        let mut close = false;
        let shown = egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0f32, theme::LINE())).corner_radius(10).inner_margin(14).show(ui, |ui| {
            card_visuals(ui);
            ui.set_width(ui.available_width());
            let g = &mut form.gathered;
            ui.label(RichText::new(g.folder.display().to_string()).font(theme::mono(12.0)).color(theme::DIM()));
            ui.add_space(6.0);
            egui::Grid::new("submit-form").num_columns(2).spacing(vec2(12.0, 8.0)).show(ui, |ui| {
                ui.label("Name");
                ui.add(egui::TextEdit::singleline(&mut g.name).desired_width(300.0));
                ui.end_row();
                ui.label("Kind");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut g.kind, "addon".to_string(), "Add-on");
                    ui.radio_value(&mut g.kind, "program".to_string(), "Program");
                });
                ui.end_row();
                ui.label("Version");
                ui.add(egui::TextEdit::singleline(&mut g.version).desired_width(120.0));
                ui.end_row();
                ui.label("What it does");
                ui.add(egui::TextEdit::singleline(&mut g.summary).hint_text("One line").desired_width(420.0));
                ui.end_row();
                ui.label("Your name");
                ui.add(egui::TextEdit::singleline(&mut form.author).hint_text("Shown with it in the library").desired_width(300.0));
                ui.end_row();
                ui.label("Contact (optional)");
                ui.add(egui::TextEdit::singleline(&mut form.contact).hint_text("In case there are questions").desired_width(300.0));
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(RichText::new(format!("{}, {}", files_text(g.files.len()), size_text(g.size as u64))).color(theme::MUTED()));
            for p in &g.problems {
                ui.label(RichText::new(format!("✕ {p}")).color(theme::LIVE()));
            }
            for n in &g.notes {
                ui.label(RichText::new(format!("• {n}")).color(theme::DIM()));
            }
            ui.add_space(6.0);
            ui.checkbox(&mut form.agree, "I made it (or may share it), and it's Rust only");
            ui.add_space(8.0);
            let sending = st.plugin_send == BugSend::Sending;
            let ready = g.problems.is_empty() && !g.name.trim().is_empty() && form.agree && !sending;
            ui.horizontal(|ui| {
                if ui.add_enabled(ready, theme::primary_button("Send for review")).clicked() {
                    send = true;
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
                if sending {
                    ui.spinner();
                }
            });
            if st.plugin_sends_done > form.sends_before {
                match &st.plugin_send {
                    BugSend::Sent => {
                        ui.label(RichText::new("Sent. Someone will look at it before it's built and listed.").color(theme::ONLINE()));
                    }
                    BugSend::Failed { message, .. } => {
                        ui.label(RichText::new(format!("Not sent: {message}")).color(theme::LIVE()));
                    }
                    _ => {}
                }
            }
        });
        if std::mem::take(&mut form.scroll) {
            ui.scroll_to_rect(shown.response.rect, Some(egui::Align::Center));
        }
        if send {
            let sub = form.gathered.submission(&form.author, &form.contact);
            // The library server, when it's mine: switch collecting on there first.
            let mine = ingle_engine::plugins::server_key().filter(|k| {
                st.server(k).is_some_and(|s| s.me.as_ref().is_some_and(|m| m.role == ingle_proto::control::Role::Owner))
            });
            acts.push(Act::SubmitPlugin { submission: sub, turn_on: mine });
        }
        if close {
            self.plugins.form = None;
        }
    }

    /// Open add-on panels: small windows over everything else.
    pub(crate) fn addon_panels(&mut self, ctx: &egui::Context, st: &AppState) {
        let (place, here) = self.addon_place(st);
        let mut events: Vec<(String, String, String)> = Vec::new();
        let ids: Vec<String> = self.plugins.panels.iter().map(|p| p.id.clone()).collect();
        for id in ids {
            let name = self.plugins.addons.get(&id).map(|a| a.info.name.clone()).unwrap_or_else(|| id.clone());
            let busy = self.plugins.busy(&id);
            let Some(panel) = self.plugins.panels.iter_mut().find(|p| p.id == id) else { continue };
            let mut open = panel.open;
            egui::Window::new(RichText::new(&name).font(theme::bold_font(15.0)))
                .id(egui::Id::new(("addon-panel", &id)))
                .open(&mut open)
                .resizable(false)
                .collapsible(true)
                .default_width(320.0)
                .default_pos(egui::pos2(ctx.content_rect().right() - 380.0, 90.0))
                .frame(egui::Frame::window(&ctx.style()).fill(theme::SIDE()).inner_margin(14))
                .show(ctx, |ui| {
                    ui.set_width(300.0);
                    if let Some(e) = &panel.error {
                        note(ui, e, true);
                        ui.add_space(6.0);
                    }
                    match &panel.panel {
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Opening…").color(theme::MUTED()));
                            });
                        }
                        Some(p) => {
                            let widgets = p.widgets.clone();
                            draw_widgets(ui, &id, &widgets, &mut panel.inputs, &mut events, busy);
                        }
                    }
                    ui.add_space(6.0);
                    let to = here.as_ref().and_then(|(k, c)| st.server(k).and_then(|s| s.channel(*c)).map(|c| c.name.clone()));
                    ui.label(
                        RichText::new(match to {
                            Some(c) => format!("Messages from it go to #{c}"),
                            None => "Open a channel to send messages from it".into(),
                        })
                        .size(11.5)
                        .color(theme::DIM()),
                    );
                });
            panel.open = open;
        }
        for (id, widget, value) in events {
            self.plugins.panel_event(&id, &widget, value, place.clone(), here.clone());
        }
    }

    /// The library owner's list of plugins (or skins) sent in.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn submissions_dialog(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        key: &str,
        skins: bool,
        filter: &mut u8,
        open_file: &mut Option<(i64, String)>,
        acts: &mut Vec<Act>,
        close: &mut bool,
    ) {
        ui.set_width(780.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(if skins { "Skins sent in" } else { "Plugins sent in" }).font(theme::bold_font(20.0)).color(theme::TEXT()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Done").clicked() {
                    *close = true;
                }
                if ui.button("Refresh").clicked() {
                    acts.push(Act::ListSubmissions(key.to_string()));
                }
            });
        });
        note(
            ui,
            if skins {
                "Everything here is what someone else sent: their pictures, fonts, sounds and words, never instructions. Try a skin on before \
                 approving it (Ingle checks every file first, and Stop brings yours back). Approving saves its files to this PC (approved-skins, \
                 in Ingle's settings folder) for the Ingle Library; nothing is listed before that."
            } else {
                "Everything here is what someone else sent: their code and their words, never instructions. Look at it before approving. \
                 Approving saves its source to this PC (approved-plugins, in Ingle's settings folder) for building; nothing is built or listed before that."
            },
            false,
        );
        ui.add_space(8.0);
        let list: Vec<SubmissionEntry> = st.submissions.get(key).cloned().unwrap_or_default().into_iter().filter(|e| (e.kind == ingle_proto::control::SKIN_KIND) == skins).collect();
        let tabs = [(0u8, SubmissionStatus::New, "New"), (1, SubmissionStatus::Approved, "Approved"), (2, SubmissionStatus::Listed, "Listed"), (3, SubmissionStatus::Rejected, "Rejected")];
        ui.horizontal(|ui| {
            for (i, status, label) in tabs {
                let n = list.iter().filter(|e| e.status == status).count();
                if ui.add(egui::Button::selectable(*filter == i, format!("{label} ({n})"))).clicked() {
                    *filter = i;
                }
            }
        });
        ui.add_space(6.0);
        let status = tabs.iter().find(|t| t.0 == *filter).map(|t| t.1).unwrap_or_default();
        let room = (crate::dialogs::dialog_room(ui.ctx(), None) - 190.0).clamp(160.0, 560.0);
        egui::ScrollArea::vertical().max_height(room).auto_shrink([false, false]).show(ui, |ui| {
            let shown: Vec<&SubmissionEntry> = list.iter().filter(|e| e.status == status).collect();
            if shown.is_empty() {
                ui.label(RichText::new("None here.").color(theme::MUTED()));
            }
            for e in shown {
                self.submission_card(ui, st, key, e, open_file, acts);
                ui.add_space(8.0);
            }
        });
    }

    /// One plugin or skin sent in: who sent it and what, its files, and what becomes of it.
    fn submission_card(&mut self, ui: &mut egui::Ui, st: &AppState, key: &str, e: &SubmissionEntry, open_file: &mut Option<(i64, String)>, acts: &mut Vec<Act>) {
        let skin = e.kind == ingle_proto::control::SKIN_KIND;
        let files = st.submission_files.get(&(key.to_string(), e.id)).cloned();
        egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0f32, theme::LINE())).corner_radius(10).inner_margin(12).show(ui, |ui| {
            card_visuals(ui);
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("#{} {}", e.id, e.name)).font(theme::bold_font(15.0)).color(theme::TEXT()));
                if skin {
                    skin_pill(ui);
                } else {
                    kind_pill(ui, if e.kind == "program" { Kind::Program } else { Kind::Addon });
                }
                ui.label(RichText::new(format!("version {}", e.version)).color(theme::DIM()));
            });
            if e.update_of != 0 {
                let what = if e.changes.is_empty() { String::new() } else { format!(": {}", e.changes) };
                ui.add(egui::Label::new(RichText::new(format!("A new version of #{}, from the same sender{what}", e.update_of)).size(12.5).color(theme::EMBER2())).wrap());
            }
            let when = chrono::DateTime::from_timestamp_millis(e.created_at).map(|d| d.with_timezone(&chrono::Local).format("%b %-d, %Y %H:%M").to_string()).unwrap_or_default();
            let who = if e.contact.is_empty() { e.author.clone() } else { format!("{} ({})", e.author, e.contact) };
            ui.label(RichText::new(format!("From {} · {when} · Ingle {} · {}, {}", if who.is_empty() { "someone" } else { &who }, e.app_version, files_text(e.files as usize), size_text(e.size))).size(12.5).color(theme::DIM()));
            let signed = if e.author_id.is_empty() {
                "Not signed (sent by an older Ingle): nobody can send a new version of it.".to_string()
            } else {
                let short: String = e.author_id.chars().take(8).collect();
                format!("Signed by the sender's Ingle identity ({short}…): only they can send new versions of it.")
            };
            ui.label(RichText::new(signed).size(12.5).color(theme::DIM()));
            ui.add(egui::Label::new(RichText::new(&e.summary).color(theme::BODY())).wrap().selectable(true));
            // A skin's picture and what it changes, once its files are here.
            if skin {
                if let Some(files) = &files {
                    let file = |p: &str| files.iter().find(|f| f.path == p).and_then(|f| ingle_proto::b64::decode(&f.data).ok());
                    let manifest = file(ingle_proto::skin::MANIFEST).and_then(|b| serde_json::from_slice::<ingle_proto::skin::Manifest>(&b).ok());
                    let picture = ["preview.png", "preview.jpg", "preview.webp"].into_iter().find_map(|p| file(p).map(|b| (p.to_string(), b))).or_else(|| {
                        let bg = manifest.as_ref()?.background.as_ref()?.file.clone();
                        file(&bg).map(|b| (bg, b))
                    });
                    ui.add_space(6.0);
                    ui.horizontal_top(|ui| {
                        if let Some((path, bytes)) = picture {
                            let (r, _) = ui.allocate_exact_size(vec2(224.0, 126.0), Sense::hover());
                            let img_key = format!("sub-picture:{key}:{}:{path}", e.id);
                            match self.images.get(ui.ctx(), &img_key, Some(std::sync::Arc::new(bytes)), 512).frame(0.0) {
                                Some((tex, ts, _)) => {
                                    let uv = cover_uv(ts, r.size());
                                    ui.painter().add(egui::epaint::RectShape::filled(r, CornerRadius::same(8), Color32::WHITE).with_texture(tex, uv));
                                }
                                None => {
                                    ui.painter().rect_filled(r, CornerRadius::same(8), theme::ELEV2());
                                }
                            }
                            ui.add_space(8.0);
                        }
                        ui.vertical(|ui| match &manifest {
                            Some(m) => skin_summary(ui, m),
                            None => {
                                ui.label(RichText::new("Its skin.json can't be read.").color(theme::LIVE()));
                            }
                        });
                    });
                }
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if files.is_none() && ui.button(if skin { "Get its files" } else { "Look at the files" }).clicked() {
                    acts.push(Act::OpenSubmission { key: key.to_string(), id: e.id });
                }
                if skin && files.is_some() {
                    let trying = matches!(self.looks.trying_name(), Some((_, crate::looks::Trial::SentIn { key: k, id })) if k == key && *id == e.id);
                    if trying {
                        if ui.button("Stop trying it").clicked() {
                            acts.push(Act::StopTryingSkin);
                        }
                    } else if ui.button("Try it on").on_hover_text("Ingle wears it until you stop (it isn't installed)").clicked() {
                        acts.push(Act::TrySentInSkin { key: key.to_string(), id: e.id });
                    }
                }
            });
            // A word for whoever sent it, sent with what becomes of it.
            ui.add_space(6.0);
            let text = self.ui.submission_notes.entry(e.id).or_default();
            ui.add(
                egui::TextEdit::singleline(text)
                    .hint_text("A note for whoever sent it (they see it with Approve or Reject)")
                    .char_limit(ingle_proto::control::SUBMISSION_NOTE_MAX)
                    .desired_width(f32::INFINITY),
            );
            let note_text = text.trim().to_string();
            if !e.note.is_empty() {
                ui.label(RichText::new(format!("Your note to them: {}", e.note)).size(12.5).color(theme::DIM()));
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let mut set = |s: SubmissionStatus| acts.push(Act::SetSubmissionStatus { key: key.to_string(), id: e.id, status: s, note: note_text.clone() });
                match e.status {
                    SubmissionStatus::New => {
                        let hover = if skin { "Saves its files to this PC for the Ingle Library" } else { "Saves its source to this PC for building" };
                        if ui.add(theme::primary_button("Approve for the library")).on_hover_text(hover).clicked() {
                            set(SubmissionStatus::Approved);
                        }
                        if ui.button("Reject").clicked() {
                            set(SubmissionStatus::Rejected);
                        }
                    }
                    SubmissionStatus::Approved => {
                        if ui.button("Mark listed").clicked() {
                            set(SubmissionStatus::Listed);
                        }
                        if ui.button("Back to new").clicked() {
                            set(SubmissionStatus::New);
                        }
                    }
                    _ => {
                        if ui.button("Back to new").clicked() {
                            set(SubmissionStatus::New);
                        }
                    }
                }
            });
            if let Some(files) = &files {
                ui.add_space(6.0);
                for f in files {
                    let selected = open_file.as_ref().is_some_and(|(id, p)| *id == e.id && *p == f.path);
                    let bytes = ingle_proto::b64::decode(&f.data).unwrap_or_default();
                    if ui.add(egui::Button::selectable(selected, RichText::new(format!("{}  ({})", f.path, size_text(bytes.len() as u64))).font(theme::mono(12.0)))).clicked() {
                        *open_file = if selected { None } else { Some((e.id, f.path.clone())) };
                    }
                    if !selected {
                        continue;
                    }
                    if ingle_proto::control::sniff_image(&bytes).is_some() {
                        let side = ui.available_width().min(560.0);
                        let (r, _) = ui.allocate_exact_size(vec2(side, side * 9.0 / 16.0), Sense::hover());
                        let img_key = format!("sub-file:{key}:{}:{}", e.id, f.path);
                        if let Some((tex, ts, _)) = self.images.get(ui.ctx(), &img_key, Some(std::sync::Arc::new(bytes)), 1024).frame(0.0) {
                            // The whole picture, fitted in.
                            let scale = (r.width() / ts.x).min(r.height() / ts.y);
                            let shown = Rect::from_center_size(r.center(), ts * scale);
                            ui.painter().image(tex, shown, Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                        }
                        continue;
                    }
                    let text = if ingle_proto::skin::is_font(&bytes) {
                        "(a font)".to_string()
                    } else if bytes.starts_with(ingle_proto::sound::MAGIC) {
                        "(a sound)".to_string()
                    } else {
                        match String::from_utf8(bytes) {
                            Ok(t) => t.lines().take(400).collect::<Vec<_>>().join("\n"),
                            Err(_) => "(not text)".to_string(),
                        }
                    };
                    egui::Frame::new().fill(theme::CODE_BG()).corner_radius(6).inner_margin(8).show(ui, |ui| {
                        egui::ScrollArea::both().max_height(320.0).id_salt(("sub-file", e.id, &f.path)).show(ui, |ui| {
                            ui.add(egui::Label::new(RichText::new(text).font(theme::mono(12.0)).color(theme::TEXT())).selectable(true).extend());
                        });
                    });
                }
            }
        });
    }
}

/// What a skin changes: its colors (as swatches), picture, fonts and sounds.
fn skin_summary(ui: &mut egui::Ui, m: &ingle_proto::skin::Manifest) {
    if !m.colors.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (name, value) in &m.colors {
                let Some([r, g, b]) = ingle_proto::skin::parse_color(value) else { continue };
                let (rect, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                ui.painter().rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(r, g, b));
                ui.painter().rect_stroke(rect, CornerRadius::same(4), Stroke::new(1.0f32, theme::LINE()), egui::StrokeKind::Inside);
                let what = ingle_proto::skin::COLORS.iter().find(|(n, _)| n == name).map(|(_, w)| *w).unwrap_or("");
                resp.on_hover_text(format!("{name} {value}: {what}"));
            }
        });
        ui.add_space(4.0);
    }
    let mut bits = Vec::new();
    if let Some(b) = &m.background {
        bits.push(format!("a picture behind {} ({}% showing through)", if b.whole_window() { "the whole window" } else { "the chat" }, (b.show * 100.0).round()));
    }
    let fonts: Vec<&str> = [&m.fonts.body, &m.fonts.bold].into_iter().flatten().map(|f| f.rsplit('/').next().unwrap_or(f)).collect();
    if !fonts.is_empty() {
        bits.push(format!("fonts ({})", fonts.join(", ")));
    }
    if !m.sounds.is_empty() {
        bits.push(format!("sounds for {}", m.sounds.keys().map(|k| k.replace('_', " ")).collect::<Vec<_>>().join(", ")));
    }
    let colors = if m.colors.is_empty() { "No colors of its own".to_string() } else { format!("{} colors", m.colors.len()) };
    let text = if bits.is_empty() { format!("{colors}.") } else { format!("{colors}, {}.", bits.join(", ")) };
    ui.add(egui::Label::new(RichText::new(text).color(theme::BODY())).wrap());
}

/// Draws an add-on's widgets; clicks (and Enter in text boxes) go into `events`.
fn draw_widgets(ui: &mut egui::Ui, id: &str, widgets: &[Widget], inputs: &mut std::collections::HashMap<String, String>, events: &mut Vec<(String, String, String)>, busy: bool) {
    for w in widgets {
        match w {
            Widget::Heading { text } => {
                ui.label(RichText::new(text).font(theme::bold_font(16.0)).color(theme::TEXT()));
            }
            Widget::Text { text } => {
                ui.add(egui::Label::new(RichText::new(text).color(theme::BODY())).wrap().selectable(true));
            }
            Widget::Button { id: wid, label } => {
                if ui.add_enabled(!busy, egui::Button::new(label).min_size(vec2(44.0, 28.0))).clicked() {
                    events.push((id.to_string(), wid.clone(), String::new()));
                }
            }
            Widget::Input { id: wid, hint } => {
                let text = inputs.entry(wid.clone()).or_default();
                let resp = ui.add(egui::TextEdit::singleline(text).hint_text(hint.as_str()).desired_width(f32::INFINITY));
                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) && !text.trim().is_empty() {
                    events.push((id.to_string(), wid.clone(), std::mem::take(text)));
                    resp.request_focus();
                }
            }
            Widget::Row { widgets } => {
                ui.horizontal_wrapped(|ui| draw_widgets(ui, id, widgets, inputs, events, busy));
            }
            Widget::Divider => {
                ui.separator();
            }
            Widget::Unknown => {}
        }
    }
}

/// Opens the owner's list of plugins (or skins) sent in (from the channel list).
pub(crate) fn open_submissions(app: &mut IngleApp, key: &str, acts: &mut Vec<Act>, skins: bool) {
    acts.push(Act::ListSubmissions(key.to_string()));
    app.ui.dialog = Some(Dialog::PluginSubmissions { key: key.to_string(), skins, filter: 0, open_file: None });
}
