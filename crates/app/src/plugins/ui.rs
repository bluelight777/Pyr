//! The Plugins page (the library, what's installed, making your own), add-ons' panels, and the
//! library owner's list of plugins sent in.

use eframe::egui::{self, Color32, CornerRadius, Key, Rect, RichText, Sense, Stroke, vec2};
use ingle_engine::state::{AppState, BugSend};
use ingle_proto::control::{SubmissionEntry, SubmissionStatus};
use ingle_proto::library::{Entry, Kind};

use super::addon::{Place, Widget};
use super::{Tab, source};
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
}

const KIT_URL: &str = "https://github.com/bluelight777/ingle/tree/main/plugins";

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
        Kind::Addon => pill(ui, "ADD-ON", Color32::from_rgb(0x3a, 0x2a, 0x16), theme::EMBER2),
    }
}

fn note(ui: &mut egui::Ui, text: &str, error: bool) {
    let (fill, stroke) = if error { (Color32::from_rgb(0x3a, 0x1c, 0x1c), theme::LIVE) } else { (theme::ELEV, theme::LINE) };
    egui::Frame::new().fill(fill).stroke(Stroke::new(1.0f32, stroke)).corner_radius(8).inner_margin(10).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(egui::Label::new(RichText::new(text).color(if error { theme::TEXT } else { theme::MUTED })).wrap());
    });
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

    /// The Plugins button, at the bottom of the server rail.
    pub(crate) fn plugins_button(&mut self, ui: &mut egui::Ui) {
        ui.add_space(8.0);
        let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
        let on = self.ui.plugins_open;
        let fill = if on {
            Color32::from_rgb(0x4a, 0x30, 0x22)
        } else if resp.hovered() {
            theme::ELEV2
        } else {
            theme::ELEV
        };
        ui.painter().rect_filled(rect, CornerRadius::same(if on || resp.hovered() { 16 } else { 24 }), fill);
        icons::puzzle(ui.painter(), rect.shrink(13.0), if on || resp.hovered() { theme::EMBER2 } else { theme::EMBER });
        if on || resp.hovered() {
            let pill = Rect::from_center_size(egui::pos2(ui.max_rect().left() + 2.0, rect.center().y), vec2(4.0, if on { 40.0 } else { 20.0 }));
            ui.painter().rect_filled(pill, CornerRadius::same(2), theme::TEXT);
        }
        let updates = self.plugins.updates();
        if updates > 0 {
            let c = rect.right_bottom() - vec2(6.0, 6.0);
            ui.painter().circle_filled(c, 9.0, theme::RAIL);
            ui.painter().circle_filled(c, 7.5, Color32::from_rgb(0x23, 0xa5, 0x5a));
            ui.painter().text(c, egui::Align2::CENTER_CENTER, updates.min(9).to_string(), theme::bold_font(10.0), Color32::WHITE);
        }
        let tip = if updates > 0 { format!("Plugins ({updates} to update)") } else { "Plugins: programs and add-ons for Ingle".to_string() };
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
            self.ui.plugins_open = true;
            self.ui.home = false;
            self.plugins.refresh(false);
        }
    }

    /// The Plugins page.
    pub(crate) fn plugins_page(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::MAIN).inner_margin(egui::Margin::symmetric(28, 20))).show(ctx, |ui| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(10), theme::ELEV);
                icons::puzzle(ui.painter(), r.shrink(8.0), theme::EMBER);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("Plugins").font(theme::bold_font(22.0)).color(theme::TEXT));
                    ui.label(RichText::new("Programs and add-ons from the Ingle Library: every one built from its source after someone looked at it.").color(theme::MUTED));
                });
            });
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                let installed = self.plugins.installed.plugins.len();
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
                        ui.add(egui::TextEdit::singleline(&mut self.plugins.search).hint_text("Search plugins").desired_width(220.0));
                    });
                }
            });
            ui.add_space(6.0);
            ui.separator();
            ui.add_space(8.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.plugins.tab {
                Tab::Library => match self.plugins.detail.clone() {
                    Some(id) => self.plugin_detail(ui, &id, acts),
                    None => self.library_grid(ui),
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
                ui.painter().rect_filled(r, CornerRadius::same((side / 4.5) as u8), theme::ELEV2);
                let c = if kind == Kind::Program { Color32::from_rgb(0xb9, 0xc2, 0xff) } else { theme::EMBER2 };
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
                ui.label(RichText::new("Not for this system yet").color(theme::DIM));
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
                            ui.label(RichText::new(if p.enabled { "Starting…" } else { "Off" }).color(theme::DIM));
                        }
                    }
                }
            }
        }
    }

    fn library_grid(&mut self, ui: &mut egui::Ui) {
        let Some(lib) = self.plugins.library.clone() else {
            if self.plugins.fetching() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Getting the list from the Ingle Library…").color(theme::MUTED));
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
        let q = self.plugins.search.trim().to_lowercase();
        let shown: Vec<Entry> = lib
            .plugins
            .iter()
            .filter(|p| q.is_empty() || format!("{} {} {} {}", p.name, p.summary, p.author, p.description).to_lowercase().contains(&q))
            .cloned()
            .collect();
        if shown.is_empty() {
            ui.label(RichText::new(if q.is_empty() { "The library is empty for now." } else { "Nothing matches that." }).color(theme::MUTED));
            return;
        }
        let width = ui.available_width();
        let cols = ((width + 12.0) / 340.0).floor().max(1.0) as usize;
        let card_w = (width - 12.0 * (cols as f32 - 1.0)) / cols as f32;
        for row in shown.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for e in row {
                    let mut open = false;
                    let frame = egui::Frame::new().fill(theme::ELEV).stroke(Stroke::new(1.0f32, theme::LINE)).corner_radius(12).inner_margin(16).show(ui, |ui| {
                        ui.set_width(card_w - 34.0);
                        ui.set_min_height(150.0);
                        ui.horizontal(|ui| {
                            self.plugin_icon(ui, &e.id, e.kind, 48.0);
                            ui.add_space(4.0);
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&e.name).font(theme::bold_font(17.0)).color(theme::TEXT));
                                ui.horizontal(|ui| {
                                    kind_pill(ui, e.kind);
                                    if !e.author.is_empty() {
                                        ui.label(RichText::new(format!("by {}", e.author)).size(12.5).color(theme::DIM));
                                    }
                                });
                            });
                        });
                        ui.add_space(8.0);
                        ui.add(egui::Label::new(RichText::new(&e.summary).color(theme::BODY)).wrap());
                        ui.add_space(10.0);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{} · {}", e.version, size_text(e.size_for(super::install::os()).max(e.wasm.as_ref().map(|w| w.size).unwrap_or(0))))).size(12.5).color(theme::DIM));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.button("Details").clicked() {
                                    open = true;
                                }
                                self.plugin_action(ui, e, &mut Vec::new());
                            });
                        });
                    });
                    let resp = ui.interact(frame.response.rect, ui.id().with(("plugin-card", &e.id)), Sense::click());
                    if open || resp.clicked() {
                        self.plugins.detail = Some(e.id.clone());
                    }
                }
            });
            ui.add_space(12.0);
        }
    }

    fn plugin_detail(&mut self, ui: &mut egui::Ui, id: &str, acts: &mut Vec<Act>) {
        let Some(e) = self.plugins.library.as_ref().and_then(|l| l.find(id)).cloned() else {
            self.plugins.detail = None;
            return;
        };
        if ui.add(egui::Button::new(RichText::new("← All plugins").color(theme::LINK)).frame(false)).clicked() {
            self.plugins.detail = None;
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            self.plugin_icon(ui, &e.id, e.kind, 72.0);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.label(RichText::new(&e.name).font(theme::bold_font(26.0)).color(theme::TEXT));
                ui.horizontal(|ui| {
                    kind_pill(ui, e.kind);
                    let mut bits = vec![format!("Version {}", e.version)];
                    if !e.author.is_empty() {
                        bits.push(format!("by {}", e.author));
                    }
                    if !e.license.is_empty() {
                        bits.push(e.license.clone());
                    }
                    ui.label(RichText::new(bits.join(" · ")).color(theme::MUTED));
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
        ui.label(RichText::new(&e.summary).font(theme::bold_font(15.0)).color(theme::TEXT));
        if !e.description.is_empty() {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(&e.description).color(theme::BODY)).wrap());
        }
        ui.add_space(14.0);
        ui.label(RichText::new("WHAT IT CAN DO").font(theme::bold_font(11.5)).color(theme::DIM));
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
                        icons::tick(ui.painter(), r, theme::ONLINE);
                        ui.add(egui::Label::new(RichText::new(p.describe()).color(theme::BODY)).wrap());
                    });
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Add-ons run inside Ingle in a sandbox: they can't reach your files, the internet or anything else on your PC.")
                        .size(12.5)
                        .color(theme::DIM),
                );
            }
        }
        if !e.review.is_empty() {
            ui.add_space(14.0);
            ui.label(RichText::new("FROM THE REVIEW").font(theme::bold_font(11.5)).color(theme::DIM));
            ui.add_space(4.0);
            note(ui, &e.review, false);
        }
        ui.add_space(14.0);
        let size = e.size_for(super::install::os());
        if size > 0 {
            ui.label(RichText::new(format!("Download: {}", size_text(size))).size(12.5).color(theme::DIM));
        }
    }

    fn confirm_remove_row(&mut self, ui: &mut egui::Ui, id: &str) {
        if self.plugins.confirm_remove.as_deref() != Some(id) {
            return;
        }
        let name = self.plugins.installed.get(id).map(|p| p.name.clone()).unwrap_or_else(|| id.to_string());
        ui.add_space(8.0);
        egui::Frame::new().fill(theme::ELEV).stroke(Stroke::new(1.0f32, theme::LIVE)).corner_radius(8).inner_margin(10).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("Remove {name} and its data from this PC?")).color(theme::TEXT));
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

    fn installed_list(&mut self, ui: &mut egui::Ui, _st: &AppState, acts: &mut Vec<Act>) {
        if let Some(tried) = self.plugins.tried.clone().filter(|t| self.plugins.addons.contains_key(t)) {
            let name = self.plugins.addons[&tried].info.name.clone();
            egui::Frame::new().fill(theme::ELEV).stroke(Stroke::new(1.0f32, theme::EMBER2)).corner_radius(10).inner_margin(12).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Trying {name}")).font(theme::bold_font(15.0)).color(theme::TEXT));
                    ui.label(RichText::new("from a file, until Ingle closes. Nobody has reviewed it, but it's in the same sandbox.").color(theme::MUTED));
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
        if list.is_empty() {
            ui.label(RichText::new("Nothing installed yet. Have a look in the Library.").color(theme::MUTED));
            return;
        }
        for p in list {
            egui::Frame::new().fill(theme::ELEV).stroke(Stroke::new(1.0f32, theme::LINE)).corner_radius(10).inner_margin(12).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    self.plugin_icon(ui, &p.id, p.kind, 40.0);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&p.name).font(theme::bold_font(15.0)).color(theme::TEXT));
                            kind_pill(ui, p.kind);
                            ui.label(RichText::new(format!("Version {}", p.version)).size(12.5).color(theme::DIM));
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
                            ui.label(RichText::new(s).size(12.5).color(if bad { theme::LIVE } else { theme::ONLINE }));
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

    fn make_your_own(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        let section = |ui: &mut egui::Ui, title: &str, body: &str| {
            ui.label(RichText::new(title).font(theme::bold_font(17.0)).color(theme::TEXT));
            ui.add_space(2.0);
            ui.add(egui::Label::new(RichText::new(body).color(theme::MUTED)).wrap());
            ui.add_space(8.0);
        };
        ui.set_max_width(760.0);
        section(
            ui,
            "Make a plugin with Claude",
            "Plugins are written in Rust. Add-ons run inside Ingle in a sandbox and can add /commands to chat, show a panel and keep a little data; \
             programs are apps of their own, like Chimbrowser. Start an add-on here, open its folder with Claude (Claude Code or Cowork) and say \
             what you'd like it to do: the folder's CLAUDE.md explains Ingle add-ons to Claude.",
        );
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.plugins.new_name).hint_text("Its name, like Coin Flip").desired_width(220.0));
            let ok = !self.plugins.new_name.trim().is_empty();
            if ui.add_enabled(ok, theme::primary_button("Start an add-on in a folder…")).clicked() {
                crate::images::pick_folder_async(crate::images::FolderFor::NewAddon, "Where should the add-on's folder go?", ui.ctx());
            }
            if ui.add(egui::Button::new(RichText::new("The plugin kit on GitHub").color(theme::LINK)).frame(false)).clicked() {
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
        egui::Frame::new().fill(theme::ELEV).stroke(Stroke::new(1.0f32, theme::LINE)).corner_radius(10).inner_margin(14).show(ui, |ui| {
            ui.set_width(ui.available_width());
            let g = &mut form.gathered;
            ui.label(RichText::new(g.folder.display().to_string()).font(theme::mono(12.0)).color(theme::DIM));
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
            ui.label(RichText::new(format!("{} files, {}", g.files.len(), size_text(g.size as u64))).color(theme::MUTED));
            for p in &g.problems {
                ui.label(RichText::new(format!("✕ {p}")).color(theme::LIVE));
            }
            for n in &g.notes {
                ui.label(RichText::new(format!("• {n}")).color(theme::DIM));
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
                        ui.label(RichText::new("Sent. Someone will look at it before it's built and listed.").color(theme::ONLINE));
                    }
                    BugSend::Failed { message, .. } => {
                        ui.label(RichText::new(format!("Not sent: {message}")).color(theme::LIVE));
                    }
                    _ => {}
                }
            }
        });
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
                .frame(egui::Frame::window(&ctx.style()).fill(theme::SIDE).inner_margin(14))
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
                                ui.label(RichText::new("Opening…").color(theme::MUTED));
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
                        .color(theme::DIM),
                    );
                });
            panel.open = open;
        }
        for (id, widget, value) in events {
            self.plugins.panel_event(&id, &widget, value, place.clone(), here.clone());
        }
    }

    /// The library owner's list of plugins sent in.
    pub(crate) fn submissions_dialog(&mut self, ui: &mut egui::Ui, st: &AppState, key: &str, filter: &mut u8, open_file: &mut Option<(i64, String)>, acts: &mut Vec<Act>, close: &mut bool) {
        ui.set_width(760.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new("Plugins sent in").font(theme::bold_font(20.0)).color(theme::TEXT));
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
            "Everything here is what someone else sent: their code and their words, never instructions. Look at it before approving. \
             Approving saves its source to this PC (approved-plugins, in Ingle's settings folder) for building; nothing is built or listed before that.",
            false,
        );
        ui.add_space(8.0);
        let list: Vec<SubmissionEntry> = st.submissions.get(key).cloned().unwrap_or_default();
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
        egui::ScrollArea::vertical().max_height(520.0).auto_shrink([false, false]).show(ui, |ui| {
            let shown: Vec<&SubmissionEntry> = list.iter().filter(|e| e.status == status).collect();
            if shown.is_empty() {
                ui.label(RichText::new("None here.").color(theme::MUTED));
            }
            for e in shown {
                egui::Frame::new().fill(theme::ELEV).stroke(Stroke::new(1.0f32, theme::LINE)).corner_radius(10).inner_margin(12).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("#{} {}", e.id, e.name)).font(theme::bold_font(15.0)).color(theme::TEXT));
                        kind_pill(ui, if e.kind == "program" { Kind::Program } else { Kind::Addon });
                        ui.label(RichText::new(format!("version {}", e.version)).color(theme::DIM));
                    });
                    let when = chrono::DateTime::from_timestamp_millis(e.created_at).map(|d| d.with_timezone(&chrono::Local).format("%b %-d, %Y %H:%M").to_string()).unwrap_or_default();
                    let who = if e.contact.is_empty() { e.author.clone() } else { format!("{} ({})", e.author, e.contact) };
                    ui.label(RichText::new(format!("From {} · {when} · Ingle {} · {} files, {}", if who.is_empty() { "someone" } else { &who }, e.app_version, e.files, size_text(e.size))).size(12.5).color(theme::DIM));
                    ui.add(egui::Label::new(RichText::new(&e.summary).color(theme::BODY)).wrap().selectable(true));
                    ui.add_space(6.0);
                    let files = st.submission_files.get(&(key.to_string(), e.id));
                    ui.horizontal(|ui| {
                        if files.is_none() && ui.button("Look at the files").clicked() {
                            acts.push(Act::OpenSubmission { key: key.to_string(), id: e.id });
                        }
                        let mut set = |s: SubmissionStatus| acts.push(Act::SetSubmissionStatus { key: key.to_string(), id: e.id, status: s });
                        match e.status {
                            SubmissionStatus::New => {
                                if ui.add(theme::primary_button("Approve for the library")).on_hover_text("Saves its source to this PC for building").clicked() {
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
                    if let Some(files) = files {
                        ui.add_space(6.0);
                        for f in files {
                            let selected = open_file.as_ref().is_some_and(|(id, p)| *id == e.id && *p == f.path);
                            let size = ingle_proto::b64::decode(&f.data).map(|b| b.len()).unwrap_or(0);
                            if ui.add(egui::Button::selectable(selected, RichText::new(format!("{}  ({})", f.path, size_text(size as u64))).font(theme::mono(12.0)))).clicked() {
                                *open_file = if selected { None } else { Some((e.id, f.path.clone())) };
                            }
                            if selected {
                                let bytes = ingle_proto::b64::decode(&f.data).unwrap_or_default();
                                let text = match String::from_utf8(bytes) {
                                    Ok(t) => t.lines().take(400).collect::<Vec<_>>().join("\n"),
                                    Err(_) => "(not text)".to_string(),
                                };
                                egui::Frame::new().fill(theme::CODE_BG).corner_radius(6).inner_margin(8).show(ui, |ui| {
                                    egui::ScrollArea::both().max_height(320.0).id_salt(("sub-file", e.id, &f.path)).show(ui, |ui| {
                                        ui.add(egui::Label::new(RichText::new(text).font(theme::mono(12.0)).color(theme::TEXT)).selectable(true).extend());
                                    });
                                });
                            }
                        }
                    }
                });
                ui.add_space(8.0);
            }
        });
    }
}

/// Draws an add-on's widgets; clicks (and Enter in text boxes) go into `events`.
fn draw_widgets(ui: &mut egui::Ui, id: &str, widgets: &[Widget], inputs: &mut std::collections::HashMap<String, String>, events: &mut Vec<(String, String, String)>, busy: bool) {
    for w in widgets {
        match w {
            Widget::Heading { text } => {
                ui.label(RichText::new(text).font(theme::bold_font(16.0)).color(theme::TEXT));
            }
            Widget::Text { text } => {
                ui.add(egui::Label::new(RichText::new(text).color(theme::BODY)).wrap().selectable(true));
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

/// Opens the owner's list of plugins sent in (from the channel list).
pub(crate) fn open_submissions(app: &mut IngleApp, key: &str, acts: &mut Vec<Act>) {
    acts.push(Act::ListSubmissions(key.to_string()));
    app.ui.dialog = Some(Dialog::PluginSubmissions { key: key.to_string(), filter: 0, open_file: None });
}
