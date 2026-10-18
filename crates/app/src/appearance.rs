//! Settings → Appearance (which skin Pyr wears: Campfire, its own, or one from the Pyr
//! Library) and Settings → Claude (Pyr's Claude connector, for making plugins and skins with
//! Claude: see `connector`).

use std::time::{Duration, Instant};

use eframe::egui::{self, Color32, RichText, Sense, Stroke, vec2};
use pyr_engine::state::AppState;

use crate::app::{Act, PyrApp};
use crate::connector::{self, Setup};
use crate::dialogs::{field_label, notice};
use crate::looks::Trial;
use crate::theme;

/// What Settings → Claude last found (looked at again every couple of seconds while it shows).
pub struct ClaudeSetup {
    at: Instant,
    exe: Option<std::path::PathBuf>,
    configs: Vec<(std::path::PathBuf, Setup)>,
    /// Claude Code still starts Ingle's connector (from before Pyr was called Pyr).
    code_has_ingles: bool,
    /// What the last button did (or why it didn't).
    pub said: Option<(String, bool)>,
}

impl PyrApp {
    pub(crate) fn appearance_tab(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>, close: &mut bool) {
        if let Some((name, author, what)) = self.looks.trying_name() {
            let from = match what {
                Trial::Claude { .. } => "from Claude",
                Trial::SentIn { .. } => "sent in for the Pyr Library",
                Trial::Folder { .. } => "from a folder",
            };
            let by = if author.is_empty() { String::new() } else { format!(" by {author}") };
            let text = format!("Trying {name}{by} ({from}). Your own skin comes back when you stop.");
            ui.horizontal(|ui| {
                ui.label(RichText::new(text).color(theme::TEXT()));
                if ui.button("Stop trying it").clicked() {
                    acts.push(Act::StopTryingSkin);
                }
            });
            ui.add_space(6.0);
        }
        if let Some(p) = &self.looks.problem {
            notice(ui, &format!("Your skin couldn't be put on, so Pyr looks like Campfire for now: {p}"), true);
            ui.add_space(6.0);
        }
        field_label(ui, "Skin");
        ui.label(RichText::new("Colors, a picture behind the chat, fonts and sounds. Skins have no code, and Pyr checks every file before it uses one.").color(theme::MUTED()));
        ui.add_space(6.0);
        let worn = st.settings.skin.clone();
        let row = |ui: &mut egui::Ui, app: &mut PyrApp, id: &str, name: &str, by: &str, version: &str, acts: &mut Vec<Act>| {
            let wearing = worn == id;
            egui::Frame::new()
                .fill(theme::ELEV())
                .stroke(Stroke::new(1.0f32, if wearing { theme::EMBER2() } else { theme::LINE() }))
                .corner_radius(10)
                .inner_margin(10)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        if id.is_empty() {
                            campfire_swatch(ui, vec2(64.0, 40.0));
                        } else {
                            app.skin_thumbnail(ui, id, vec2(64.0, 40.0));
                        }
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new(name).font(theme::bold_font(15.0)).color(theme::TEXT()));
                            let mut bits = Vec::new();
                            if id.is_empty() {
                                bits.push(by.to_string());
                            } else if !by.is_empty() {
                                bits.push(format!("by {by}"));
                            }
                            if !version.is_empty() {
                                bits.push(format!("version {version}"));
                            }
                            if !bits.is_empty() {
                                ui.label(RichText::new(bits.join(" · ")).size(12.5).color(theme::DIM()));
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if wearing {
                                ui.label(RichText::new("✓ Wearing").color(theme::ONLINE()));
                            } else if ui.add(egui::Button::new("Wear").min_size(vec2(72.0, 30.0))).clicked() {
                                acts.push(Act::WearSkin(id.to_string()));
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        };
        row(ui, self, "", "Campfire", "Pyr's own look, by Firelit Games", "", acts);
        let installed = self.plugins.skins.skins.clone();
        for s in &installed {
            row(ui, self, &s.id, &s.name, &s.author, &s.version, acts);
        }
        if installed.is_empty() {
            ui.label(RichText::new("No skins installed yet.").color(theme::DIM()));
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button("Find skins in the Gallery")).clicked() {
                self.open_place(crate::plugins::Page::Skins);
                self.plugins.skins_tab = crate::plugins::SkinsTab::Gallery;
                self.plugins.skin_detail = None;
                *close = true;
            }
            if ui.button("Make one with Claude…").clicked() {
                self.ui.settings_tab_next = Some(6);
            }
            if ui.button("Try a skin folder…").on_hover_text("A skin you're making yourself: Pyr wears it until you stop").clicked() {
                crate::images::pick_folder_async(crate::images::FolderFor::TrySkin, "Choose the skin's folder (with its skin.json)", ui.ctx());
            }
        });
    }

    /// An installed skin's picture from the Library (or a palette while there's none).
    fn skin_thumbnail(&mut self, ui: &mut egui::Ui, id: &str, size: egui::Vec2) {
        let (r, _) = ui.allocate_exact_size(size, Sense::hover());
        let bytes = self.plugins.icons.get(&format!("skin:{id}")).cloned();
        let side = crate::plugins::ui::picture_side(ui.ctx(), size);
        let tex = bytes.and_then(|b| self.images.get(ui.ctx(), &format!("skin-preview:{id}:{side}"), Some(b), side).frame(0.0));
        let rounding = crate::theme::r(8);
        match tex {
            Some((tex, ts, _)) => {
                let uv = crate::plugins::ui::cover_uv(ts, r.size());
                ui.painter().add(egui::epaint::RectShape::filled(r, rounding, Color32::WHITE).with_texture(tex, uv));
            }
            None => {
                ui.painter().rect_filled(r, rounding, theme::ELEV2());
                crate::icons::palette(ui.painter(), r.shrink(8.0));
            }
        }
    }

    pub(crate) fn claude_tab(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        let _ = (st, acts);
        field_label(ui, "Make plugins and skins with Claude");
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Pyr's Claude connector lets Claude (the Claude app, or Claude Code) start a skin or an add-on with you, put a skin on Pyr \
                     while you talk it over so you see each change, check it the way the Pyr Library will, send it in for review, and send \
                     new versions of yours later. In a server that adds Claude (Server Settings → Bots), it can read and write there too, \
                     when you ask it to.",
                )
                .color(theme::BODY()),
            )
            .wrap(),
        );
        ui.add_space(4.0);
        ui.add(
            egui::Label::new(
                RichText::new(
                    "It signs what it sends to the Library with your Pyr identity (the key on this PC), so only you can send a new version \
                     of something you sent. It never reads or writes in a server as you: in chat it's Claude, a bot with an identity of its \
                     own, only in the servers that add it and where its roles let it. In Pyr itself, all it can change is the skin you're \
                     trying.",
                )
                .size(12.5)
                .color(theme::DIM()),
            )
            .wrap(),
        );
        // What's set up where (looked at again every couple of seconds).
        let stale = self.ui.claude_setup.as_ref().is_none_or(|c| c.at.elapsed() > Duration::from_secs(2));
        let args = connector::args(&self.engine.shared.config_dir);
        if stale {
            let exe = connector::connector();
            let configs = match &exe {
                Some(exe) => connector::claude_configs().into_iter().map(|c| {
                    let s = connector::status(&c, exe, &args);
                    (c, s)
                }).collect(),
                None => Vec::new(),
            };
            let said = self.ui.claude_setup.take().and_then(|c| c.said);
            let code_has_ingles = connector::claude_code_config().is_some_and(|c| connector::claude_code_has_ingles(&c));
            self.ui.claude_setup = Some(ClaudeSetup { at: Instant::now(), exe, configs, code_has_ingles, said });
            ui.ctx().request_repaint_after(Duration::from_secs(2));
        }
        let Some(setup) = self.ui.claude_setup.as_ref() else { return };
        let Some(exe) = setup.exe.clone() else {
            ui.add_space(8.0);
            notice(ui, "The connector (pyr-mcp) isn't next to Pyr on this PC. Reinstall Pyr to get it.", true);
            return;
        };
        let configs = setup.configs.clone();
        let said = setup.said.clone();
        let code_has_ingles = setup.code_has_ingles;
        let mut did: Option<(String, bool)> = None;
        if cfg!(windows) || cfg!(target_os = "macos") {
            field_label(ui, "The Claude app");
            let here = configs.iter().any(|(_, s)| *s == Setup::Here);
            let unreadable: Vec<&String> = configs.iter().filter_map(|(_, s)| if let Setup::Unreadable(e) = s { Some(e) } else { None }).collect();
            let elsewhere = configs.iter().find_map(|(_, s)| if let Setup::Elsewhere(p) = s { Some(p.clone()) } else { None });
            let (text, color) = if here {
                ("Set up. In the Claude app, ask Claude to make you a skin, or an add-on for Pyr.".to_string(), theme::ONLINE())
            } else if let Some(p) = &elsewhere {
                let same_exe = if cfg!(windows) { p.eq_ignore_ascii_case(&exe.display().to_string()) } else { std::path::Path::new(p) == exe };
                if same_exe {
                    ("Set up for this Pyr, but with another settings folder (so another Pyr identity). Use this one's so Claude sends things as you.".to_string(), theme::MUTED())
                } else {
                    (format!("Set up for another copy of Pyr ({p})."), theme::MUTED())
                }
            } else {
                ("Not set up yet.".to_string(), theme::MUTED())
            };
            ui.label(RichText::new(text).color(color));
            for e in unreadable {
                ui.add(egui::Label::new(RichText::new(e).size(12.5).color(theme::LIVE())).wrap());
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let label = if here { "Set it up again" } else if elsewhere.is_some() { "Use this Pyr's connector" } else { "Add it to the Claude app" };
                if ui.add(theme::primary_button(label)).clicked() {
                    // Into every settings file the Claude app may read (the first is made if
                    // there are none yet).
                    let mut targets: Vec<std::path::PathBuf> = configs.iter().filter(|(c, s)| !matches!(s, Setup::Unreadable(_)) && (c.exists() || c.parent().is_some_and(|d| d.is_dir()))).map(|(c, _)| c.clone()).collect();
                    if targets.is_empty() {
                        targets.extend(configs.first().map(|(c, _)| c.clone()));
                    }
                    let errors: Vec<String> = targets.iter().filter_map(|c| connector::add(c, &exe, &args).err()).collect();
                    did = Some(if targets.is_empty() {
                        ("Couldn't find where the Claude app keeps its settings.".into(), true)
                    } else if errors.is_empty() {
                        ("Added. Now quit the Claude app completely (right-click its icon by the clock and choose Quit) and open it again: its tools show up under \"pyr\".".into(), false)
                    } else {
                        (errors.join(" "), true)
                    });
                }
                if (here || elsewhere.is_some()) && ui.button("Take it out").clicked() {
                    let errors: Vec<String> = configs.iter().filter(|(_, s)| matches!(s, Setup::Here | Setup::Elsewhere(_))).filter_map(|(c, _)| connector::remove(c).err()).collect();
                    did = Some(if errors.is_empty() { ("Taken out. Restart the Claude app for it to go.".into(), false) } else { (errors.join(" "), true) });
                }
            });
            if let Some((text, error)) = did.clone().or(said) {
                ui.add_space(4.0);
                notice(ui, &text, error);
            }
        }
        field_label(ui, "Claude Code");
        let command = |ui: &mut egui::Ui, line: &str| {
            ui.horizontal(|ui| {
                egui::Frame::new().fill(theme::CODE_BG()).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
                    ui.set_max_width(480.0);
                    ui.add(egui::Label::new(RichText::new(line).font(theme::mono(12.0)).color(theme::TEXT())).wrap().selectable(true));
                });
                if ui.small_button("Copy").clicked() {
                    ui.ctx().copy_text(line.to_string());
                }
            });
        };
        if code_has_ingles {
            // (Claude Code's settings file is only read: Claude Code's own commands change it.)
            notice(
                ui,
                "Claude Code still starts Ingle's connector, from before Pyr was called Pyr (it went with Ingle). Run these two in a \
                 terminal to switch it to Pyr's, then start Claude Code again:",
                true,
            );
            ui.add_space(4.0);
            for line in connector::claude_code_switch(&exe, &args) {
                command(ui, &line);
                ui.add_space(2.0);
            }
        } else {
            ui.label(RichText::new("Run this once in a terminal:").color(theme::MUTED()));
            command(ui, &connector::claude_code_line(&exe, &args));
        }
        field_label(ui, "Other apps");
        ui.horizontal(|ui| {
            ui.label(RichText::new("Apps that take MCP servers the same way:").color(theme::MUTED()));
            if ui.small_button("Copy the setup").clicked() {
                ui.ctx().copy_text(connector::snippet(&exe, &args));
            }
        });
        if did.is_some() {
            // Look again now, and keep what the button said.
            self.ui.claude_setup = Some(ClaudeSetup { at: Instant::now() - Duration::from_secs(10), exe: Some(exe), configs, code_has_ingles, said: did });
        }
    }
}

/// Campfire's own colors, side by side.
/// Campfire, Pyr's own skin, as a few dabs of its colors (it has no picture), `size` big.
pub(crate) fn campfire_swatch(ui: &mut egui::Ui, size: egui::Vec2) {
    let (r, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, crate::theme::r(8), Color32::from_rgb(0x1b, 0x14, 0x10));
    let colors = [pyr_proto::skin::CAMPFIRE[13], pyr_proto::skin::CAMPFIRE[14], pyr_proto::skin::CAMPFIRE[15], pyr_proto::skin::CAMPFIRE[9]];
    // Four dabs across the middle, sized to the box (64 x 40 is the small one).
    let k = (size.y / 40.0).min(size.x / 64.0);
    let step = 13.0 * k;
    let left = r.center().x - step * 1.5;
    for (i, [cr, cg, cb]) in colors.into_iter().enumerate() {
        let c = egui::pos2(left + i as f32 * step, r.center().y);
        p.circle_filled(c, 5.5 * k, Color32::from_rgb(cr, cg, cb));
    }
}
