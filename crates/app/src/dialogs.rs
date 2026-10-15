//! Modal dialogs: join server, invites, channels, settings, screen picker.

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Sense, pos2, vec2};
use ingle_engine::config::{CameraQuality, InputMode, NoiseSuppression, ScreenPreset};
use ingle_engine::hotkey;
use ingle_engine::state::{AppState, BugSend, ConnStatus};
use ingle_engine::video::SourceKind;
use ingle_proto::control::{ChannelKind, FILES_FEATURE, ROLES_FEATURE, perm};

use crate::app::{Act, Dialog, IngleApp, RoleEdit, avatar, rgb};
use crate::{icons, theme};

fn modal_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::SIDE)
        .stroke(egui::Stroke::new(1.0f32, theme::LINE))
        .corner_radius(12)
        .inner_margin(egui::Margin::same(22))
}

pub(crate) fn title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).font(theme::bold_font(20.0)).color(theme::TEXT));
    ui.add_space(4.0);
}

pub(crate) fn field_label(ui: &mut egui::Ui, text: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(text.to_uppercase()).font(theme::bold_font(11.5)).color(theme::MUTED));
}

/// The most a dialog's contents can take up: the window's height, less the dialog's margins
/// and a little room around it (`top`: where a dialog pinned to the top starts).
pub(crate) fn dialog_room(ctx: &egui::Context, top: Option<f32>) -> f32 {
    let h = ctx.content_rect().height();
    let around = match top {
        Some(t) => t + 16.0,
        None => 32.0,
    };
    (h - around - 2.0 * 22.0).max(160.0)
}

pub(crate) fn input(ui: &mut egui::Ui, text: &mut String, hint: &str) -> egui::Response {
    ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)))
}

/// Puts `insert` where the text box `id`'s cursor is (at the end if it has none) and moves the
/// cursor after it.
fn insert_at_cursor(ctx: &egui::Context, id: egui::Id, text: &mut String, insert: &str) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::TextEdit::load_state(ctx, id);
    let at = state.as_ref().and_then(|s| s.cursor.char_range()).map(|r| r.primary.index).unwrap_or(usize::MAX).min(text.chars().count());
    let byte = text.char_indices().nth(at).map(|(b, _)| b).unwrap_or(text.len());
    text.insert_str(byte, insert);
    if let Some(s) = state.as_mut() {
        s.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(at + insert.chars().count()))));
        s.clone().store(ctx, id);
    }
}

/// How the notice about a bug report that didn't go starts (its "Open it" brings it back).
pub(crate) const BUG_NOT_SENT: &str = "Your bug report wasn't sent";

/// Ingle's bug server, when it's one of my servers, I'm its owner and I'm connected to it: its
/// key, its name, and whether it collects reports yet.
fn my_bug_server(st: &AppState) -> Option<(String, String, bool)> {
    let key = ingle_engine::bugs::server_key()?;
    let s = st.server(&key)?;
    let owner = s.me.as_ref().map(|m| m.role == ingle_proto::control::Role::Owner).unwrap_or(false);
    (owner && s.status == ConnStatus::Connected && s.has(ingle_proto::control::BUGS_FEATURE)).then(|| (key, s.info.name.clone(), s.info.bug_reports))
}

/// A box of text in a dialog: a note, or (`error`) what went wrong.
fn notice(ui: &mut egui::Ui, text: &str, error: bool) {
    egui::Frame::new()
        .fill(if error { Color32::from_rgb(0x4a, 0x1f, 0x1f) } else { theme::ELEV })
        .stroke(egui::Stroke::new(1.0f32, if error { theme::LIVE } else { theme::LINE }))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(theme::TEXT));
        });
}

impl IngleApp {
    /// A channel name box with an emoji button beside it (the emoji goes where the cursor is,
    /// like Discord's 🎮-gaming). `salt` keeps each dialog's picker apart.
    fn channel_name_input(&mut self, ui: &mut egui::Ui, st: &AppState, salt: &str, name: &mut String, hint: &str, acts: &mut Vec<Act>) -> egui::Response {
        let open_id = egui::Id::new(("channel-emoji-open", salt));
        let mut open: bool = ui.data(|d| d.get_temp(open_id)).unwrap_or(false);
        let (resp, button) = ui
            .horizontal(|ui| {
                let w = (ui.available_width() - 44.0).max(80.0);
                let r = ui.add(egui::TextEdit::singleline(name).hint_text(hint).desired_width(w).margin(egui::Margin::symmetric(10, 8)));
                let b = crate::app::icon_button(ui, 34.0, "Add an emoji", open, |p, r, c| icons::smile(p, r, c));
                (r, b)
            })
            .inner;
        if button.clicked() {
            open = !open;
            if open {
                // Typing searches the picker, like Discord's.
                self.ui.picker.search.clear();
                self.ui.picker.focus_search = true;
            }
        }
        if open {
            let area = egui::Area::new(egui::Id::new(("channel-emoji", salt)))
                .order(egui::Order::Tooltip)
                .pivot(egui::Align2::LEFT_TOP)
                .fixed_pos(pos2(button.rect.right() + 10.0, button.rect.top()))
                .constrain(true)
                .show(ui.ctx(), |ui| egui::Frame::popup(ui.style()).fill(theme::SIDE).inner_margin(10).show(ui, |ui| self.emoji_picker(ui, st, None, acts)).inner);
            if let Some(e) = area.inner {
                insert_at_cursor(ui.ctx(), resp.id, name, &e);
                resp.request_focus();
                open = false;
            }
            let pressed_at = ui.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
            let outside = pressed_at.is_some_and(|p| !area.response.rect.contains(p) && !button.rect.contains(p));
            // (Escape closes the picker, not the dialog.)
            if outside || ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
                open = false;
            }
        }
        ui.data_mut(|d| d.insert_temp(open_id, open));
        resp
    }

    /// Report a bug, with the report that didn't go yet if there is one (to send again).
    pub(crate) fn report_bug_dialog(&self) -> Dialog {
        let (title, body, include_log) = self.ui.bug_draft.clone().unwrap_or((String::new(), String::new(), true));
        Dialog::ReportBug { title, body, include_log }
    }

    /// What became of a bug report sent from Report a bug: gone (the window closes; the
    /// engine says thanks), or not (the window shows why, with the report kept; if the window
    /// was closed meanwhile, a notice says so and can bring it back).
    fn bug_outcomes(&mut self, st: &AppState, acts: &mut Vec<Act>) {
        if st.bug_sends_done == self.ui.bug_seen {
            return;
        }
        self.ui.bug_seen = st.bug_sends_done;
        let open = matches!(self.ui.dialog, Some(Dialog::ReportBug { .. }));
        match &st.bug_send {
            BugSend::Sent => {
                self.ui.bug_draft = None;
                if open {
                    self.ui.dialog = None;
                }
            }
            BugSend::Failed { message, not_collecting } if !open && self.ui.bug_draft.is_some() => {
                let why = if *not_collecting { "Ingle's developers aren't taking bug reports right now" } else { message.as_str() };
                acts.push(Act::Toast(format!("{BUG_NOT_SENT}: {why}. It's kept, ready to send again."), true));
            }
            _ => {}
        }
    }

    pub fn dialogs(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        self.bug_outcomes(st, acts);
        self.dialog_ui(ctx, st, acts);
        // One dialog opening another (Server Settings' "Add a sound", say, and back).
        if let Some(d) = self.ui.next_dialog.take() {
            self.ui.dialog = Some(d);
        }
    }

    fn dialog_ui(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        let Some(dialog) = self.ui.dialog.clone() else {
            self.ui.dialog_shown = None;
            return;
        };
        let was_create = matches!(dialog, Dialog::Create);
        let kind = std::mem::discriminant(&dialog);
        let page = if was_create { self.ui.create.as_ref().map(|c| c.page as u8 + 1).unwrap_or(0) } else { 0 };
        let opened = self.ui.dialog_shown != Some((kind, page));
        self.ui.dialog_shown = Some((kind, page));
        if opened {
            // A dialog is placed by its size last frame (another dialog's or page's, or none):
            // lay it out again so it never shows at the wrong size or place.
            ctx.request_discard("a dialog opened");
        }
        let mut close = false;
        let mut modal = egui::Modal::new(egui::Id::new("dialog")).frame(modal_frame()).backdrop_color(Color32::from_black_alpha(170));
        let mut top = None;
        if was_create {
            // The wizard changes height as you go; pinning its top keeps buttons from jumping.
            let t = (ctx.content_rect().height() * 0.07).clamp(16.0, 80.0);
            top = Some(t);
            modal = modal.area(egui::Modal::default_area(egui::Id::new("dialog-create")).anchor(egui::Align2::CENTER_TOP, vec2(0.0, t)));
        }
        // Every dialog fits the window, however small: the screen picker and Settings fit
        // themselves (their buttons always show), and the others scroll when they must.
        let room = dialog_room(ctx, top);
        let fits_itself = matches!(dialog, Dialog::ScreenPicker { .. } | Dialog::Settings { .. });
        let resp = modal.show(ctx, |ui| {
            // The modal lays its contents out in last frame's size, which would let a dialog
            // shrink but never grow (a short dialog before it, or a wizard step getting
            // longer, would cut it off): it gets the whole room every frame instead.
            ui.set_max_height(room);
            let body = |ui: &mut egui::Ui| match dialog {
                Dialog::AddServer { mut link, error } => {
                    ui.set_width(460.0);
                    title(ui, "Join a server");
                    ui.label(RichText::new("Paste an invite link someone shared with you.").color(theme::MUTED));
                    field_label(ui, "Invite link");
                    let r = input(ui, &mut link, "ingle://host/abcd1234/xxxxxxxxxxxx");
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if !enter {
                        r.request_focus();
                    }
                    if let Some(e) = &error {
                        ui.label(RichText::new(e).color(theme::LIVE).size(12.5));
                    }
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("The link includes the server's fingerprint, so your connection is checked against it and can't be impersonated.")
                            .size(12.0)
                            .color(theme::DIM),
                    );
                    ui.add_space(14.0);
                    let mut create = false;
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button("Join").min_size(vec2(100.0, 34.0))).clicked() || enter {
                            acts.push(Act::AddServer(link.trim().to_string()));
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(egui::Button::new(RichText::new("No invite? Create your own server").color(theme::LINK)).frame(false)).clicked() {
                                create = true;
                            }
                        });
                    });
                    if create {
                        self.ui.create = Some(crate::create::CreateState::new(&st.my_name, ingle_proto::crypto::backup_key(&self.engine.shared.identity)));
                        self.ui.dialog = Some(Dialog::Create);
                    } else if let Some(Dialog::AddServer { link: l, .. }) = self.ui.dialog.as_mut() {
                        *l = link;
                    }
                }
                Dialog::AddChoice => {
                    ui.set_width(500.0);
                    title(ui, "Add a server");
                    ui.add_space(6.0);
                    if crate::create::choice_card(
                        ui,
                        "add-create",
                        None,
                        "Create my own",
                        "A server for you and your friends, on any VPS from any provider: one you already have, or a new one in your own DigitalOcean account. Ingle sets it up.",
                        icons::cloud,
                    ) {
                        self.ui.create = Some(crate::create::CreateState::new(&st.my_name, ingle_proto::crypto::backup_key(&self.engine.shared.identity)));
                        self.ui.dialog = Some(Dialog::Create);
                    }
                    if crate::create::choice_card(
                        ui,
                        "add-join",
                        None,
                        "Join with an invite link",
                        "Someone sent you a link that starts with ingle://",
                        icons::link,
                    ) {
                        self.ui.dialog = Some(Dialog::AddServer { link: String::new(), error: None });
                    }
                    ui.add_space(4.0);
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                }
                Dialog::Create => {
                    self.create_dialog(ui, st, acts, &mut close);
                }
                Dialog::CreateChannel { key, kind, mut name } => {
                    ui.set_width(420.0);
                    title(ui, "Create channel");
                    let mut k = kind;
                    field_label(ui, "Channel type");
                    ui.radio_value(&mut k, ChannelKind::Text, "Text — messages, links and emoji");
                    ui.radio_value(&mut k, ChannelKind::Voice, "Voice — talk, cameras and screen share (with its own chat)");
                    field_label(ui, "Channel name");
                    let open_id = egui::Id::new(("channel-emoji-open", "create"));
                    let was_picking = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
                    let r = self.channel_name_input(ui, st, "create", &mut name, if k == ChannelKind::Text { "New channel" } else { "Game Night" }, acts);
                    let picking = ui.data(|d| d.get_temp::<bool>(open_id)).unwrap_or(false);
                    // (Enter in the emoji picker's search picks an emoji: it doesn't create the
                    // channel. The cursor stays in the name, except while that search has it.)
                    let enter = !was_picking && !picking && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if !enter && !picking && !was_picking {
                        r.request_focus();
                    }
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if (ui.add(theme::primary_button("Create").min_size(vec2(100.0, 34.0))).clicked() || enter) && !name.trim().is_empty() {
                            acts.push(Act::CreateChannel { key: key.clone(), name: name.trim().to_string(), kind: k });
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                    if !close {
                        self.ui.dialog = Some(Dialog::CreateChannel { key, kind: k, name });
                    }
                }
                Dialog::EditChannel { key, channel, mut name, mut topic } => {
                    ui.set_width(440.0);
                    title(ui, "Edit channel");
                    field_label(ui, "Name");
                    self.channel_name_input(ui, st, "edit", &mut name, "", acts);
                    field_label(ui, "Topic");
                    input(ui, &mut topic, "What's this channel about?");
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button("Save").min_size(vec2(100.0, 34.0))).clicked() {
                            acts.push(Act::UpdateChannel { key: key.clone(), channel, name: Some(name.clone()), topic: Some(topic.clone()) });
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                    if !close {
                        self.ui.dialog = Some(Dialog::EditChannel { key, channel, name, topic });
                    }
                }
                Dialog::Invite { key } => {
                    ui.set_width(520.0);
                    let s = st.server(&key);
                    let name = s.map(|s| s.name.as_str()).unwrap_or("this server");
                    title(ui, &format!("Invite friends to {name}"));
                    ui.label(
                        RichText::new("Send them this link. It opens a page that gets Ingle for anyone who doesn't have it yet, and joins with one click. (It can also be pasted into Join a server.)")
                            .color(theme::MUTED),
                    );
                    ui.add_space(8.0);
                    match s.and_then(|s| s.last_invite.clone()) {
                        Some(link) => {
                            let web = ingle_proto::invite::InviteLink::web(&link, s.map(|s| s.name.as_str()).unwrap_or(""));
                            egui::Frame::new().fill(theme::CODE_BG).corner_radius(6).inner_margin(10).show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(&web).font(theme::mono(12.5)).color(theme::TEXT)).wrap());
                            });
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui.add(theme::primary_button("Copy link").min_size(vec2(110.0, 34.0))).clicked() {
                                    acts.push(Act::Copy(web.clone()));
                                }
                                if ui.button("Copy the ingle:// link").on_hover_text(link.as_str()).clicked() {
                                    acts.push(Act::Copy(link.clone()));
                                }
                                if ui.button("Done").clicked() {
                                    close = true;
                                }
                            });
                        }
                        None => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Creating invite…").color(theme::MUTED));
                            });
                            if ui.button("Close").clicked() {
                                close = true;
                            }
                        }
                    }
                }
                Dialog::ServerSettings { key, mut name, mut tab } => {
                    let server = st.server(&key).cloned();
                    let can_server = server.as_ref().map(|s| s.can(perm::MANAGE_SERVER)).unwrap_or(false);
                    let can_roles = server.as_ref().map(|s| s.can(perm::MANAGE_ROLES) && s.has(ROLES_FEATURE)).unwrap_or(false);
                    let can_emoji =
                        server.as_ref().map(|s| s.can(perm::MANAGE_EMOJI) && s.has(ingle_proto::control::EMOJI_FEATURE)).unwrap_or(false);
                    let can_backup = self.engine.can_back_up(&key);
                    // The soundboard tab shows for anyone who manages emoji (it says when the
                    // server needs updating first).
                    let can_sounds = server.as_ref().is_some_and(|s| s.can(perm::MANAGE_EMOJI) && s.status.is_connected());
                    if tab == 0 && !can_server && can_emoji {
                        tab = 2;
                    }
                    ui.set_width(match tab {
                        1 => 680.0,
                        2 | 4 => 560.0,
                        _ => 440.0,
                    });
                    ui.horizontal(|ui| {
                        title(ui, "Server settings");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Done").clicked() {
                                close = true;
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        if can_server && ui.add(egui::Button::selectable(tab == 0, "Overview")).clicked() {
                            tab = 0;
                        }
                        if can_roles && ui.add(egui::Button::selectable(tab == 1, "Roles")).clicked() {
                            tab = 1;
                        }
                        if can_emoji && ui.add(egui::Button::selectable(tab == 2, "Emoji")).clicked() {
                            tab = 2;
                        }
                        if can_sounds && ui.add(egui::Button::selectable(tab == 4, "Soundboard")).clicked() {
                            tab = 4;
                        }
                        if can_backup && ui.add(egui::Button::selectable(tab == 3, "Backups")).clicked() {
                            tab = 3;
                        }
                    });
                    ui.separator();
                    if tab == 4 {
                        if let Some(s) = &server {
                            let back = Dialog::ServerSettings { key: key.clone(), name: name.clone(), tab: 4 };
                            self.server_sounds_tab(ui, s, back);
                        }
                    } else if tab == 3 {
                        self.backups_tab(ui, st, &key);
                    } else if tab == 1 {
                        if let Some(s) = &server {
                            self.roles_editor(ui, s, acts);
                        }
                    } else if tab == 2 {
                        if let Some(s) = &server {
                            self.emoji_editor(ui, s, acts);
                        }
                    } else {
                        if let Some(s) = server.as_ref().filter(|s| s.has(ingle_proto::control::ICONS_FEATURE)) {
                            field_label(ui, "Server icon");
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
                                match self.server_icon(ui.ctx(), s) {
                                    Some(t) => {
                                        ui.painter().add(
                                            egui::epaint::RectShape::filled(r, CornerRadius::same(24), Color32::WHITE)
                                                .with_texture(t, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))),
                                        );
                                    }
                                    None => {
                                        ui.painter().rect_filled(r, CornerRadius::same(24), theme::ELEV);
                                        let initials: String =
                                            s.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase();
                                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, initials, theme::bold_font(24.0), theme::TEXT);
                                    }
                                }
                                ui.add_space(12.0);
                                ui.vertical(|ui| {
                                    ui.add_space(14.0);
                                    ui.horizontal(|ui| {
                                        if ui.add(theme::primary_button("Upload an icon")).clicked() {
                                            crate::images::pick_async(crate::images::PickFor::ServerIcon(key.clone()), false, ui.ctx());
                                        }
                                        if s.info.icon.is_some() && ui.button("Remove").clicked() {
                                            acts.push(Act::SetServerIcon { key: key.clone(), bytes: None });
                                        }
                                    });
                                    ui.label(RichText::new("Shows in everyone's server list. At least 128×128 looks best.").size(12.0).color(theme::DIM));
                                });
                            });
                            ui.add_space(12.0);
                        }
                        if let Some(s) = server.as_ref().filter(|s| {
                            s.has(ingle_proto::control::BUGS_FEATURE) && s.me.as_ref().map(|m| m.role == ingle_proto::control::Role::Owner).unwrap_or(false)
                        }) {
                            field_label(ui, "Ingle bug reports");
                            let mut on = s.info.bug_reports;
                            if ui.checkbox(&mut on, "Collect Ingle bug reports on this server").changed() {
                                acts.push(Act::SetBugReports { key: key.clone(), on });
                            }
                            if s.has(ingle_proto::control::PLUGINS_FEATURE) {
                                let mut on = s.info.plugin_submissions;
                                if ui.checkbox(&mut on, "Collect plugins for the Ingle Library on this server").changed() {
                                    acts.push(Act::SetPluginSubmissions { key: key.clone(), on });
                                }
                            }
                            ui.label(
                                RichText::new("For the server Ingle's own Report a bug sends to. Reports show under bug-reports in the channel list, only for you.")
                                    .size(12.0)
                                    .color(theme::DIM),
                            );
                            ui.add_space(12.0);
                        }
                        if let Some(s) = server.as_ref().filter(|s| {
                            s.has(ingle_proto::control::GIPHY_FEATURE) && s.me.as_ref().map(|m| m.role == ingle_proto::control::Role::Owner).unwrap_or(false)
                        }) {
                            field_label(ui, "GIF search (GIPHY)");
                            let on = !s.info.giphy_key.is_empty();
                            ui.label(
                                RichText::new(if on {
                                    "On for everyone in this server (/giphy and the GIF button). Only you, the owner, can change it."
                                } else {
                                    "Off. Paste a GIPHY API key (free from developers.giphy.com) to turn on /giphy and the GIF button for everyone here."
                                })
                                .size(12.0)
                                .color(theme::DIM),
                            );
                            let id = egui::Id::new(("giphy-key-edit", key.clone()));
                            let mut k: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
                            ui.horizontal(|ui| {
                                let hint = if on { "New GIPHY API key" } else { "GIPHY API key" };
                                if ui.add(egui::TextEdit::singleline(&mut k).hint_text(hint).desired_width(260.0)).changed() {
                                    ui.data_mut(|d| d.insert_temp(id, k.clone()));
                                }
                                if ui.add_enabled(!k.trim().is_empty(), theme::primary_button(if on { "Change" } else { "Turn on" })).clicked() {
                                    acts.push(Act::SetGiphyKey { key: key.clone(), giphy: k.trim().to_string() });
                                    ui.data_mut(|d| d.remove::<String>(id));
                                }
                                if on && ui.button("Turn off").clicked() {
                                    acts.push(Act::SetGiphyKey { key: key.clone(), giphy: String::new() });
                                }
                            });
                            ui.label(
                                RichText::new(
                                    "Everyone's Ingle here gets this key: GIPHY wants each search to come straight from the app, never through a server. Use a key made just for this server, and change it if it's ever misused."
                                )
                                .size(12.0)
                                .color(theme::DIM),
                            );
                            ui.add_space(12.0);
                        }
                        if let Some(s) = server.as_ref().filter(|s| {
                            s.has(ingle_proto::control::KLIPY_FEATURE) && s.me.as_ref().map(|m| m.role == ingle_proto::control::Role::Owner).unwrap_or(false)
                        }) {
                            field_label(ui, "GIF search (KLIPY)");
                            let on = !s.info.klipy_key.is_empty();
                            ui.label(
                                RichText::new(if on {
                                    "On for everyone in this server (/klipy, and its own tab in the GIF button). Only you, the owner, can change it."
                                } else {
                                    "Off. Paste a KLIPY app key (free from partner.klipy.com) to turn on /klipy and a KLIPY tab in the GIF button for everyone here."
                                })
                                .size(12.0)
                                .color(theme::DIM),
                            );
                            let id = egui::Id::new(("klipy-key-edit", key.clone()));
                            let mut k: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
                            ui.horizontal(|ui| {
                                let hint = if on { "New KLIPY app key" } else { "KLIPY app key" };
                                if ui.add(egui::TextEdit::singleline(&mut k).hint_text(hint).desired_width(260.0)).changed() {
                                    ui.data_mut(|d| d.insert_temp(id, k.clone()));
                                }
                                if ui.add_enabled(!k.trim().is_empty(), theme::primary_button(if on { "Change" } else { "Turn on" })).clicked() {
                                    acts.push(Act::SetKlipyKey { key: key.clone(), klipy: k.trim().to_string() });
                                    ui.data_mut(|d| d.remove::<String>(id));
                                }
                                if on && ui.button("Turn off").clicked() {
                                    acts.push(Act::SetKlipyKey { key: key.clone(), klipy: String::new() });
                                }
                            });
                            ui.label(
                                RichText::new(
                                    "Everyone's Ingle here gets this key: KLIPY wants each search to come straight from the app, never through a server. Use a key made just for this server, and change it if it's ever misused."
                                )
                                .size(12.0)
                                .color(theme::DIM),
                            );
                            ui.add_space(12.0);
                        }
                        field_label(ui, "Server name");
                        input(ui, &mut name, "");
                        ui.add_space(14.0);
                        if ui.add(theme::primary_button("Save").min_size(vec2(100.0, 34.0))).clicked() && !name.trim().is_empty() {
                            acts.push(Act::RenameServer { key: key.clone(), name: name.trim().to_string() });
                        }
                    }
                    if !close {
                        self.ui.dialog = Some(Dialog::ServerSettings { key, name, tab });
                    }
                }
                Dialog::ConfirmJoin { link } => {
                    ui.set_width(440.0);
                    title(ui, "Join this server?");
                    match ingle_proto::invite::InviteLink::parse(&link) {
                        Ok(l) => {
                            ui.label(RichText::new(l.authority()).font(theme::mono(14.0)).color(theme::TEXT));
                            ui.add_space(6.0);
                            ui.label(
                                RichText::new(
                                    "A link asked Ingle to join this server. Joining connects to it: the server sees your \
                                     IP address and your Ingle identity. Only join servers from people you trust.",
                                )
                                .color(theme::MUTED),
                            );
                            ui.add_space(14.0);
                            ui.horizontal(|ui| {
                                if ui.add(theme::primary_button("Join").min_size(vec2(100.0, 34.0))).clicked() {
                                    acts.push(Act::AddServer(link.clone()));
                                    close = true;
                                }
                                if ui.button("Cancel").clicked() {
                                    close = true;
                                }
                            });
                        }
                        Err(e) => {
                            ui.label(RichText::new(format!("That link doesn't work: {e}")).color(theme::LIVE));
                            if ui.button("OK").clicked() {
                                close = true;
                            }
                        }
                    }
                }
                Dialog::LeaveServer { key, name } => {
                    ui.set_width(400.0);
                    title(ui, &format!("Leave '{name}'?"));
                    ui.label(RichText::new("You'll need a new invite to come back.").color(theme::MUTED));
                    ui.add_space(14.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::danger_button("Leave server").min_size(vec2(120.0, 34.0))).clicked() {
                            acts.push(Act::RemoveServer(key.clone()));
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                }
                Dialog::DeleteMessage { key, channel, id } => {
                    self.delete_dialog(ui, st, &key, channel, id, acts, &mut close);
                }
                Dialog::ReportBug { mut title, mut body, mut include_log } => {
                    ui.set_width(520.0);
                    super::dialogs::title(ui, "Report a bug");
                    ui.label(RichText::new("It goes to Ingle's developers. Nothing else about you is sent: no name, no account.").color(theme::MUTED));
                    let sending = st.bug_send == BugSend::Sending;
                    ui.add_enabled_ui(!sending, |ui| {
                        field_label(ui, "In a few words");
                        ui.add(
                            egui::TextEdit::singleline(&mut title)
                                .hint_text("Voice cuts out when someone shares their screen")
                                .desired_width(f32::INFINITY)
                                .char_limit(140),
                        );
                        field_label(ui, "What happened? What did you expect?");
                        ui.add(egui::TextEdit::multiline(&mut body).desired_rows(6).desired_width(f32::INFINITY).char_limit(7000));
                        ui.add_space(6.0);
                        ui.checkbox(&mut include_log, "Include Ingle's log (helps find the cause; it has server addresses and names, not messages)");
                    });
                    // Ingle's bug server, when it's my own server: reports only arrive once it
                    // collects them, so sending switches that on.
                    let mine = my_bug_server(st);
                    let turn_on = mine.as_ref().filter(|m| !m.2).map(|m| m.0.clone());
                    // Why the last try didn't go (the text above is that report, kept).
                    let failed = match (&st.bug_send, &self.ui.bug_draft) {
                        (BugSend::Failed { message, not_collecting }, Some(_)) => Some((message.clone(), *not_collecting)),
                        _ => None,
                    };
                    if let Some((message, not_collecting)) = &failed {
                        // (For my own server, the note below says what to do.)
                        if !(*not_collecting && turn_on.is_some()) {
                            ui.add_space(10.0);
                            let text = if *not_collecting {
                                "It didn't go: Ingle's developers aren't taking bug reports right now. Your report is still here: try again later, or copy it.".to_string()
                            } else {
                                format!("It didn't go: {message}. Your report is still here: try again, or copy it.")
                            };
                            notice(ui, &text, true);
                        }
                    }
                    if let Some((_, name, _)) = mine.as_ref().filter(|m| !m.2) {
                        ui.add_space(10.0);
                        notice(
                            ui,
                            &format!(
                                "Ingle's bug reports go to your server, {name}, and it isn't collecting them yet. Sending switches that on (it's also in Server settings → Overview)."
                            ),
                            false,
                        );
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let ok = !sending && (!title.trim().is_empty() || !body.trim().is_empty());
                        let label = if sending {
                            "Sending…"
                        } else if turn_on.is_some() {
                            "Turn on collecting and send"
                        } else if failed.is_some() {
                            "Send again"
                        } else {
                            "Send"
                        };
                        if ui.add_enabled(ok, theme::primary_button(label).min_size(vec2(100.0, 34.0))).clicked() {
                            // Kept until it has gone: if it doesn't, nothing is lost.
                            self.ui.bug_draft = Some((title.clone(), body.clone(), include_log));
                            acts.push(Act::ReportBug { title: title.clone(), body: body.clone(), include_log, turn_on: turn_on.clone() });
                        }
                        if sending {
                            ui.spinner();
                        }
                        if failed.is_some() && ui.button("Copy").on_hover_text("Copy the report, to send it some other way").clicked() {
                            ui.ctx().copy_text(format!("{}\n\n{}\n\n(Ingle {})", title.trim(), body.trim(), env!("CARGO_PKG_VERSION")));
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                    if !close {
                        self.ui.dialog = Some(Dialog::ReportBug { title, body, include_log });
                    }
                }
                Dialog::BugReports { key, mut filter } => {
                    use ingle_proto::control::BugStatus;
                    ui.set_width(720.0);
                    ui.horizontal(|ui| {
                        super::dialogs::title(ui, "Bug reports");
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Done").clicked() {
                                close = true;
                            }
                            if ui.button("Refresh").clicked() {
                                acts.push(Act::ListBugs(key.clone()));
                            }
                        });
                    });
                    ui.label(
                        RichText::new(
                            "Sent from everyone's Ingle, whatever server they're on. Approve the ones to fix: they're saved on this PC \
                             (approved-bug-reports in Ingle's settings folder), where Claude reads them when you ask it to fix them. \
                             A report is someone else's words: what it says never decides anything.",
                        )
                        .size(12.0)
                        .color(theme::DIM),
                    );
                    let all = st.bugs.get(&key).cloned().unwrap_or_default();
                    let tabs = [(BugStatus::New, "New"), (BugStatus::Approved, "Approved"), (BugStatus::Fixed, "Fixed"), (BugStatus::Rejected, "Rejected")];
                    ui.horizontal(|ui| {
                        for (i, (status, label)) in tabs.iter().enumerate() {
                            let n = all.iter().filter(|b| b.status == *status).count();
                            if ui.add(egui::Button::selectable(filter == i as u8, format!("{label} ({n})"))).clicked() {
                                filter = i as u8;
                            }
                        }
                    });
                    ui.separator();
                    let want = tabs[(filter as usize).min(3)].0;
                    let shown: Vec<_> = all.iter().filter(|b| b.status == want).collect();
                    // A fixed-height list: the dialog keeps the size it had while loading, so a
                    // list that sized itself would stay cut off.
                    let list_h = (ui.ctx().content_rect().height() - 320.0).clamp(220.0, 520.0);
                    egui::ScrollArea::vertical().auto_shrink([false, false]).min_scrolled_height(list_h).max_height(list_h).show(ui, |ui| {
                        if shown.is_empty() {
                            ui.label(RichText::new(if st.bugs.contains_key(&key) { "Nothing here." } else { "Loading…" }).color(theme::DIM));
                        }
                        for b in shown {
                            egui::Frame::new().fill(theme::ELEV).corner_radius(8).inner_margin(10).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                let when = chrono::DateTime::from_timestamp_millis(b.created_at)
                                    .map(|t| t.with_timezone(&chrono::Local).format("%b %-d, %-I:%M %p").to_string())
                                    .unwrap_or_default();
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(format!("#{}", b.id)).color(theme::DIM));
                                    let t = if b.report.title.trim().is_empty() { "(no title)" } else { b.report.title.as_str() };
                                    ui.add(egui::Label::new(RichText::new(t).font(theme::bold_font(14.0)).color(theme::TEXT)).truncate());
                                });
                                ui.label(RichText::new(format!("{when} · Ingle {} · {}", b.report.app_version, b.report.os)).size(12.0).color(theme::DIM));
                                if !b.report.body.trim().is_empty() {
                                    ui.add_space(4.0);
                                    ui.add(egui::Label::new(RichText::new(&b.report.body).color(theme::MUTED)).selectable(true));
                                }
                                if !b.report.log.is_empty() {
                                    egui::CollapsingHeader::new(RichText::new("Log").size(12.0)).id_salt(("bug-log", b.id)).show(ui, |ui| {
                                        egui::ScrollArea::vertical().max_height(160.0).id_salt(("bug-log-scroll", b.id)).show(ui, |ui| {
                                            ui.add(egui::Label::new(RichText::new(&b.report.log).monospace().size(11.0).color(theme::DIM)).selectable(true));
                                        });
                                    });
                                }
                                ui.add_space(6.0);
                                ui.horizontal(|ui| {
                                    let mut set = |ui: &mut egui::Ui, label: &str, s: BugStatus, primary: bool| {
                                        if b.status != s {
                                            let btn = if primary { theme::primary_button(label) } else { egui::Button::new(label) };
                                            if ui.add(btn).clicked() {
                                                acts.push(Act::SetBugStatus { key: key.clone(), id: b.id, status: s });
                                            }
                                        }
                                    };
                                    set(ui, "Approve for fixing", BugStatus::Approved, true);
                                    set(ui, "Reject", BugStatus::Rejected, false);
                                    set(ui, "Mark fixed", BugStatus::Fixed, false);
                                    if b.status != BugStatus::New {
                                        set(ui, "Back to new", BugStatus::New, false);
                                    }
                                    if ui.button("Copy").clicked() {
                                        ui.ctx().copy_text(format!(
                                            "Bug #{} ({:?}): {}\n{}\nIngle {} · {}",
                                            b.id, b.status, b.report.title, b.report.body, b.report.app_version, b.report.os
                                        ));
                                    }
                                });
                            });
                            ui.add_space(6.0);
                        }
                    });
                    if !close {
                        self.ui.dialog = Some(Dialog::BugReports { key, filter });
                    }
                }
                Dialog::PluginSubmissions { key, mut filter, mut open_file } => {
                    self.submissions_dialog(ui, st, &key, &mut filter, &mut open_file, acts, &mut close);
                    if !close {
                        self.ui.dialog = Some(Dialog::PluginSubmissions { key, filter, open_file });
                    }
                }
                Dialog::ScreenPicker { selected } => {
                    ui.set_width(560.0);
                    title(ui, "Share your screen");
                    // The title and the buttons always show (Go Live never goes off the
                    // window); what's between fits what's left: the lists and the preview get
                    // smaller in a short window, and the rest scrolls if it must.
                    let body_h = (room - 44.0 - 56.0).max(120.0);
                    // Quality, sound and the note under the preview, and the preview's label.
                    let fixed = 205.0;
                    // The preview as big as fits (leaving the screens and windows room), and the
                    // list of them in what's left: it scrolls when there are more than fit.
                    let preview_h = ((body_h - fixed - 180.0) * 0.6).clamp(64.0, 220.0);
                    let sources_h = (body_h - fixed - preview_h).max(100.0);
                    let preset = self.ui.settings_draft.as_ref().map(|s| s.screen_preset).unwrap_or(st.settings.screen_preset);
                    let mut chosen = selected.clone();
                    let screens = &st.devices.screens;
                    egui::ScrollArea::vertical().id_salt("share-body").max_height(body_h).auto_shrink([false, true]).show(ui, |ui| {
                        if screens.is_empty() {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Looking for screens and windows…").color(theme::MUTED));
                            });
                        }
                        egui::ScrollArea::vertical().id_salt("share-sources").max_height(sources_h).auto_shrink([false, true]).show(ui, |ui| {
                            for (label, kind) in [("SCREENS", SourceKind::Monitor), ("WINDOWS", SourceKind::Window), ("OTHER", SourceKind::Test)] {
                                let items: Vec<_> = screens.iter().filter(|s| s.kind == kind).collect();
                                if items.is_empty() {
                                    continue;
                                }
                                field_label(ui, label);
                                for s in items {
                                    let sel = chosen.as_deref() == Some(s.id.as_str());
                                    let text = if s.width > 0 { format!("{}   ({}×{})", s.name, s.width, s.height) } else { s.name.clone() };
                                    if ui.add(egui::Button::selectable(sel, text).min_size(vec2(ui.available_width(), 30.0))).clicked() {
                                        chosen = Some(s.id.clone());
                                    }
                                }
                            }
                        });
                        if chosen.is_none() {
                            chosen = screens.iter().find(|s| s.kind == SourceKind::Monitor).map(|s| s.id.clone());
                        }
                        self.share_preview(ui, chosen.as_deref(), preview_h);
                        field_label(ui, "Quality");
                        let mut p = preset;
                        egui::ComboBox::from_id_salt("screen-preset").selected_text(p.label()).width(260.0).show_ui(ui, |ui| {
                            for q in ScreenPreset::ALL {
                                ui.selectable_value(&mut p, q, q.label());
                            }
                        });
                        if p != preset {
                            let mut s = st.settings.clone();
                            s.screen_preset = p;
                            acts.push(Act::Settings(s));
                        }
                        ui.add_space(8.0);
                        let sound_ok = chosen.as_deref().map(ingle_engine::can_share_sound).unwrap_or(false);
                        let what = match chosen.as_deref() {
                            Some(id) if id.starts_with("win:") => "Share this app's sound",
                            Some(id) if id.starts_with("test:") => "Share sound (a test tone)",
                            _ => "Share sound (everything you hear, except this call)",
                        };
                        // Shown unticked when this source can't carry sound, so it never looks like it will.
                        let mut share = st.settings.share_audio && sound_ok;
                        let cb = ui.add_enabled(sound_ok, egui::Checkbox::new(&mut share, if sound_ok { what } else { "Share sound" }));
                        if !sound_ok {
                            cb.on_disabled_hover_text("Sharing sound needs Windows 10 (build 20348+) or Windows 11 for now.");
                        } else if share != st.settings.share_audio {
                            let mut s = st.settings.clone();
                            s.share_audio = share;
                            acts.push(Act::Settings(s));
                        }
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Free, no subscription. Viewers choose to watch, so it only uses bandwidth for people who are watching.")
                                .size(12.0)
                                .color(theme::DIM),
                        );
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        let ok = chosen.is_some();
                        if ui.add_enabled(ok, theme::primary_button("Go Live").min_size(vec2(110.0, 34.0))).clicked() {
                            if let Some(id) = chosen.clone() {
                                acts.push(Act::StartScreen(id));
                                close = true;
                            }
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("Refresh").clicked() {
                                acts.push(Act::RefreshDevices);
                            }
                        });
                    });
                    if !close {
                        self.ui.dialog = Some(Dialog::ScreenPicker { selected: chosen });
                    }
                }
                Dialog::Settings { tab } => {
                    self.settings_dialog(ui, st, tab, acts, &mut close);
                }
                Dialog::SoundEditor => {
                    self.sound_editor_dialog(ui, st, acts, &mut close);
                }
                Dialog::RemoveSound { key, id, name, from_settings } => {
                    self.remove_sound_dialog(ui, st, &key, &id, &name, from_settings, acts, &mut close);
                }
            };
            if fits_itself {
                body(ui);
            } else {
                let mut scroll = egui::ScrollArea::vertical().id_salt(("dialog-body", kind)).max_height(room).auto_shrink([true, true]);
                if opened {
                    scroll = scroll.vertical_scroll_offset(0.0);
                }
                scroll.show(ui, body);
            }
        });
        // A setup that's running can't be dismissed by clicking outside or pressing Escape.
        let busy = matches!(self.ui.dialog, Some(Dialog::Create)) && self.ui.create.as_ref().map(|c| c.busy()).unwrap_or(false);
        if close || (resp.should_close() && !busy) {
            if matches!(self.ui.dialog, Some(Dialog::Settings { .. })) && st.mic_test {
                acts.push(Act::MicTest(false));
            }
            // The wizard's state goes with its dialog (opening another dialog from it keeps it
            // only until then).
            if matches!(self.ui.dialog, Some(Dialog::Create)) || was_create {
                self.ui.create = None;
            }
            // The add-a-sound dialog closed with Escape (or a click outside): back to where it
            // was opened from.
            if matches!(self.ui.dialog, Some(Dialog::SoundEditor)) {
                self.sound_editor_closed();
            }
            if let Some(Dialog::RemoveSound { key, from_settings: true, .. }) = &self.ui.dialog {
                if self.ui.next_dialog.is_none() {
                    let name = st.server(key).map(|s| s.name.clone()).unwrap_or_default();
                    self.ui.next_dialog = Some(Dialog::ServerSettings { key: key.clone(), name, tab: 4 });
                }
            }
            self.ui.dialog = None;
        }
    }

    fn settings_dialog(&mut self, ui: &mut egui::Ui, st: &AppState, tab: u8, acts: &mut Vec<Act>, close: &mut bool) {
        ui.set_width(640.0);
        let mut report_bug = false;
        let mut draft = self.ui.settings_draft.clone().unwrap_or_else(|| st.settings.clone());
        // Things changed elsewhere while this dialog is open (the tray notice, the emoji
        // picker): don't save stale copies of them.
        draft.tray_note_seen = st.settings.tray_note_seen;
        draft.recent_emoji = st.settings.recent_emoji.clone();
        draft.emoji_tone = st.settings.emoji_tone;
        draft.giphy_key = st.settings.giphy_key.clone();
        draft.spellcheck = st.settings.spellcheck;
        draft.spell_language = st.settings.spell_language.clone();
        draft.spell_words = st.settings.spell_words.clone();
        // (Sounds played by their keys while this is open, and the soundboard's favorites.)
        draft.sound_recent = st.settings.sound_recent.clone();
        draft.sound_favorites = st.settings.sound_favorites.clone();
        // (And GIFs starred in chat or the GIF picker.)
        draft.gif_favorites = st.settings.gif_favorites.clone();
        let before = draft.clone();
        let mut t = tab;
        ui.horizontal(|ui| {
            title(ui, "Settings");
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Done").clicked() {
                    *close = true;
                }
            });
        });
        // Tab ids stay the same everywhere (About is 2); the Windows tab only exists there.
        let tabs: &[(u8, &str)] = if cfg!(windows) {
            &[(0, "My Account"), (1, "Voice & Video"), (4, "Notifications"), (3, "Windows"), (2, "About")]
        } else {
            &[(0, "My Account"), (1, "Voice & Video"), (4, "Notifications"), (2, "About")]
        };
        ui.horizontal(|ui| {
            for &(id, name) in tabs {
                if ui.add(egui::Button::selectable(t == id, name)).clicked() {
                    t = id;
                }
            }
        });
        ui.separator();
        // As tall as the window allows (the title, tabs and the version at the foot always show).
        let body_h = (dialog_room(ui.ctx(), None) - 150.0).clamp(140.0, 470.0);
        egui::ScrollArea::vertical().max_height(body_h).min_scrolled_height(body_h).show(ui, |ui| match t {
            0 => {
                field_label(ui, "Profile picture");
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
                    let face = self.my_face(ui.ctx());
                    avatar(ui.painter(), r.center(), 40.0, &st.my_uid, &st.my_name, false, theme::SIDE, face);
                    ui.add_space(12.0);
                    ui.vertical(|ui| {
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            if ui.add(theme::primary_button("Upload a picture")).clicked() {
                                crate::images::pick_async(crate::images::PickFor::Avatar, false, ui.ctx());
                            }
                            if face.is_some() && ui.button("Remove").clicked() {
                                acts.push(Act::SetAvatar(None));
                            }
                        });
                        ui.label(
                            RichText::new("PNG, JPEG, GIF or WebP. You can zoom in and move it before saving. It shows on every server you're in.")
                                .size(12.0)
                                .color(theme::DIM),
                        );
                    });
                });
                if st.servers.iter().any(|s| s.status.is_connected() && !s.has(FILES_FEATURE)) {
                    ui.label(
                        RichText::new("Some of your servers run an older Ingle Server and won't show it until they're updated.")
                            .size(12.0)
                            .color(theme::DIM),
                    );
                }
                field_label(ui, "Display name");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.ui.name_draft).desired_width(260.0).margin(egui::Margin::symmetric(10, 8)));
                    if ui.add(theme::primary_button("Save")).clicked() {
                        acts.push(Act::SetName(self.ui.name_draft.trim().to_string()));
                    }
                });
                field_label(ui, "Your identity");
                ui.label(
                    RichText::new(
                        "There's no account or password. Your identity is a key stored on this computer; servers recognise you by it.",
                    )
                    .color(theme::MUTED),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Safety code").color(theme::MUTED));
                    ui.label(RichText::new(&st.safety_code).font(theme::mono(14.0)).color(theme::TEXT));
                    if ui.small_button("Copy").clicked() {
                        acts.push(Act::Copy(st.safety_code.clone()));
                    }
                });
                ui.label(RichText::new("Friends can compare this code with you to be sure they're talking to the real you.").size(12.0).color(theme::DIM));
                ui.add_space(6.0);
                let path = self.engine.identity_path().display().to_string();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Key file").color(theme::MUTED));
                    ui.add(egui::Label::new(RichText::new(&path).font(theme::mono(11.5)).color(theme::DIM)).truncate());
                    if ui.small_button("Copy path").clicked() {
                        acts.push(Act::Copy(path.clone()));
                    }
                });
                ui.label(RichText::new("Back this file up to keep your identity if you reinstall. Don't share it.").size(12.0).color(theme::DIM));
            }
            1 => {
                let devices = &st.devices;
                ui.columns(2, |cols| {
                    field_label(&mut cols[0], "Input device");
                    device_combo(&mut cols[0], "in-dev", &mut draft.input_device, &devices.inputs);
                    field_label(&mut cols[1], "Output device");
                    device_combo(&mut cols[1], "out-dev", &mut draft.output_device, &devices.outputs);
                    field_label(&mut cols[0], "Input volume");
                    let mut g = draft.input_gain * 100.0;
                    if cols[0].add(egui::Slider::new(&mut g, 0.0..=200.0).suffix("%")).changed() {
                        draft.input_gain = g / 100.0;
                    }
                    field_label(&mut cols[1], "Output volume");
                    let mut o = draft.output_volume * 100.0;
                    if cols[1].add(egui::Slider::new(&mut o, 0.0..=200.0).suffix("%")).changed() {
                        draft.output_volume = o / 100.0;
                    }
                });
                field_label(ui, "Mic test");
                ui.horizontal(|ui| {
                    let label = if st.mic_test { "Stop testing" } else { "Let's check" };
                    if ui.add(theme::primary_button(label)).clicked() {
                        acts.push(Act::MicTest(!st.mic_test));
                    }
                    meter(ui, st.mic_level_db, if draft.vad_auto { None } else { Some(draft.vad_threshold_db) });
                });
                ui.label(RichText::new("Plays your processed microphone back to you so you can hear what others hear.").size(12.0).color(theme::DIM));

                field_label(ui, "Input mode");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut draft.input_mode, InputMode::VoiceActivity, "Voice activity");
                    ui.radio_value(&mut draft.input_mode, InputMode::PushToTalk, "Push to talk");
                });
                match draft.input_mode {
                    InputMode::VoiceActivity => {
                        ui.checkbox(&mut draft.vad_auto, "Automatically detect when I'm talking");
                        if !draft.vad_auto {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new("Sensitivity").color(theme::MUTED));
                                ui.add(egui::Slider::new(&mut draft.vad_threshold_db, -80.0..=-10.0).suffix(" dB"));
                            });
                        }
                    }
                    InputMode::PushToTalk => {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Key").color(theme::MUTED));
                            egui::ComboBox::from_id_salt("ptt-key").selected_text(hotkey::key_label(&draft.ptt_key)).show_ui(ui, |ui| {
                                for k in hotkey::KEY_CHOICES {
                                    ui.selectable_value(&mut draft.ptt_key, k.to_string(), hotkey::key_label(k));
                                }
                            });
                            ui.label(RichText::new("Release delay").color(theme::MUTED));
                            let mut d = draft.ptt_release_ms as f32;
                            if ui.add(egui::Slider::new(&mut d, 0.0..=1000.0).suffix(" ms")).changed() {
                                draft.ptt_release_ms = d as u32;
                            }
                        });
                        ui.label(RichText::new("Works while other apps are focused, and doesn't block the key in your game.").size(12.0).color(theme::DIM));
                    }
                }

                field_label(ui, "Voice processing");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Noise suppression").color(theme::MUTED));
                    ui.radio_value(&mut draft.noise_suppression, NoiseSuppression::Off, "Off");
                    ui.radio_value(&mut draft.noise_suppression, NoiseSuppression::Standard, "Standard");
                    ui.radio_value(&mut draft.noise_suppression, NoiseSuppression::Strong, "Strong (AI)");
                    ui.radio_value(&mut draft.noise_suppression, NoiseSuppression::Isolation, "Voice isolation");
                });
                if draft.noise_suppression == NoiseSuppression::Isolation {
                    ui.label(
                        RichText::new(
                            "Only your voice gets through: typing, mouse clicks, fans, machines, TVs, people talking in the room \
                             and sounds between your words are cut. Someone talking right next to you as loudly as you can still be \
                             heard. Try it with Let's check.",
                        )
                        .size(12.0)
                        .color(theme::DIM),
                    );
                }
                ui.checkbox(&mut draft.echo_cancellation, "Echo cancellation (needed if you use speakers instead of headphones)");
                ui.checkbox(&mut draft.auto_gain, "Automatic gain control");
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Voice quality").color(theme::MUTED));
                    let mut b = draft.voice_bitrate_kbps as f32;
                    if ui.add(egui::Slider::new(&mut b, 24.0..=192.0).step_by(8.0).suffix(" kbps")).changed() {
                        draft.voice_bitrate_kbps = b as u32;
                    }
                });
                ui.label(RichText::new("Opus with in-band error correction. Discord's default is 64 kbps; anything above ~64 kbps is overkill for voice but great for music.").size(12.0).color(theme::DIM));
                ui.checkbox(&mut draft.voice_priority, "Prioritize voice packets (Quality of Service)");
                ui.label(
                    RichText::new(
                        "Tells your router and Wi-Fi to send Ingle's voice ahead of downloads and other traffic. \
                         Turn it off if your voice cuts out or won't connect (a few routers don't handle it).",
                    )
                    .size(12.0)
                    .color(theme::DIM),
                );

                self.soundboard_settings(ui, st, &mut draft);

                field_label(ui, "Camera");
                ui.horizontal(|ui| {
                    device_combo(ui, "cam-dev", &mut draft.camera_device, &devices.cameras);
                    egui::ComboBox::from_id_salt("cam-q").selected_text(draft.camera_quality.label()).show_ui(ui, |ui| {
                        for q in [CameraQuality::Low, CameraQuality::Standard, CameraQuality::High] {
                            ui.selectable_value(&mut draft.camera_quality, q, q.label());
                        }
                    });
                });
                field_label(ui, "Screen share quality");
                egui::ComboBox::from_id_salt("scr-q").selected_text(draft.screen_preset.label()).width(260.0).show_ui(ui, |ui| {
                    for q in ScreenPreset::ALL {
                        ui.selectable_value(&mut draft.screen_preset, q, q.label());
                    }
                });
                ui.add_space(6.0);
                if ui.small_button("Re-scan devices").clicked() {
                    acts.push(Act::RefreshDevices);
                }
            }
            4 => {
                field_label(ui, "Sounds");
                ui.checkbox(&mut draft.sounds, "Play sounds");
                ui.label(
                    RichText::new(
                        "A ping for new messages (not in muted channels, or the one you're looking at), and chimes when people \
                         join or leave your voice channel, start or stop sharing their screen, and when you tune in to a stream.",
                    )
                    .size(12.0)
                    .color(theme::DIM),
                );
                ui.add_space(8.0);
                ui.add_enabled_ui(draft.sounds, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Volume");
                        let mut pct = draft.sound_volume * 100.0;
                        if ui.add(egui::Slider::new(&mut pct, 0.0..=150.0).suffix("%")).changed() {
                            draft.sound_volume = pct / 100.0;
                        }
                        if ui.button("Test").clicked() {
                            acts.push(Act::TestSound(draft.sound_volume));
                        }
                    });
                });
                ui.add_space(10.0);
                field_label(ui, "Messages");
                ui.checkbox(&mut draft.confirm_delete, "Ask before deleting a message");
                ui.label(
                    RichText::new("Holding Shift when you click Delete skips the question either way.")
                        .size(12.0)
                        .color(theme::DIM),
                );
                ui.add_space(10.0);
                field_label(ui, "Muting");
                ui.label(
                    RichText::new("Right-click a channel, a server or a direct message conversation to mute it.")
                        .size(12.0)
                        .color(theme::DIM),
                );
            }
            3 => {
                field_label(ui, "Icon by the clock");
                ui.label(
                    RichText::new(
                        "Closing the window keeps Ingle running by the clock, so you stay in voice and friends can still reach \
                         you. Click the campfire icon to open Ingle; right-click it to mute, leave voice or quit.",
                    )
                    .color(theme::MUTED),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Windows may tuck the icon under the ^ arrow. To keep it in view, drag it from there to the taskbar \
                         next to the clock.",
                    )
                    .size(12.0)
                    .color(theme::DIM),
                );
                if !crate::tray::is_active() {
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Windows isn't showing the icon right now, so closing the window minimizes Ingle instead.")
                            .size(12.0)
                            .color(theme::LIVE),
                    );
                }
                ui.add_space(12.0);
                field_label(ui, "When Windows starts");
                ui.checkbox(&mut draft.start_with_windows, "Open Ingle when Windows starts");
                ui.add_enabled(draft.start_with_windows, egui::Checkbox::new(&mut draft.start_hidden, "Start in the tray (just the icon by the clock)"));
                ui.label(
                    RichText::new("Like Discord: you're online and friends can reach you as soon as your PC is on.")
                        .size(12.0)
                        .color(theme::DIM),
                );
                ui.add_space(12.0);
                if ui.add(theme::danger_button("Quit Ingle").min_size(vec2(110.0, 32.0))).clicked() {
                    acts.push(Act::Quit);
                }
            }
            _ => {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} {}", ingle_proto::APP_NAME, env!("CARGO_PKG_VERSION"))).font(theme::bold_font(16.0)).color(theme::TEXT));
                ui.label(RichText::new("Lean voice, video, screen share and chat. Written in Rust.").color(theme::MUTED));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let checking = self.updater.checking();
                    if ui.add_enabled(!checking, egui::Button::new(if checking { "Checking…" } else { "Check for updates" })).clicked() {
                        acts.push(Act::CheckUpdates);
                    }
                    if ingle_engine::bugs::can_report() && ui.button("Report a bug").clicked() {
                        report_bug = true;
                    }
                });
                ui.add_space(10.0);
                for (h, b) in [
                    ("End-to-end encrypted calls", "Voice, cameras and screen shares are encrypted on your PC with keys the server never sees (X25519 + ChaCha20-Poly1305, new keys whenever someone joins or leaves)."),
                    ("No accounts", "Your identity is a key on this computer. No email, phone number or ID."),
                    ("Self-hosted", "Communities run their own small server. Text history lives on that server."),
                    ("Free quality", "1080p60 screen sharing and high-bitrate audio without a subscription."),
                ] {
                    ui.label(RichText::new(h).font(theme::bold_font(14.0)).color(theme::TEXT));
                    ui.label(RichText::new(b).color(theme::MUTED));
                    ui.add_space(6.0);
                }
                ui.label(RichText::new("Text messages are stored on the server you're talking on (like Discord), not end-to-end encrypted.").size(12.0).color(theme::DIM));
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Emoji graphics: Twemoji by Twitter, Inc. and other contributors, licensed CC-BY 4.0. Emoji names: Emojibase (MIT). \
                         GIF search powered by GIPHY and KLIPY. Voice isolation: DeepFilterNet (MIT/Apache-2.0) and RNNoise (BSD). Spellcheck: \
                         spellbook (MPL-2.0) with dictionaries from SCOWL (MIT/BSD), RLA-ES (MPL-1.1) and Grammalecte (MPL-2.0). \
                         Reading sound files for the soundboard: Symphonia (MPL-2.0). Ingle's own soundboard sounds are \
                         synthesized by Ingle (no recordings).",
                    )
                        .size(12.0)
                        .color(theme::DIM),
                );
            }
        });
        // Like Discord: the version at the foot of Settings, on every tab.
        ui.add_space(10.0);
        ui.separator();
        ui.horizontal(|ui| {
            let v = format!("Ingle {} · {} {}", env!("CARGO_PKG_VERSION"), std::env::consts::OS, std::env::consts::ARCH);
            ui.label(RichText::new(&v).size(11.5).color(theme::DIM));
            if ui.small_button("Copy").on_hover_text("Copy the version (for a bug report)").clicked() {
                ui.ctx().copy_text(v);
            }
        });
        if draft != before {
            acts.push(Act::Settings(draft.clone()));
        }
        self.ui.settings_draft = Some(draft);
        if report_bug {
            if st.mic_test {
                acts.push(Act::MicTest(false));
            }
            // Replaces this dialog (not "close", which would clear it).
            self.ui.dialog = Some(self.report_bug_dialog());
            return;
        }
        if !*close {
            self.ui.dialog = Some(Dialog::Settings { tab: t });
        }
    }
}


fn device_combo(ui: &mut egui::Ui, id: &str, value: &mut Option<String>, devices: &[(String, String)]) {
    let current = value
        .as_ref()
        .and_then(|v| devices.iter().find(|d| &d.0 == v).map(|d| d.1.clone()))
        .unwrap_or_else(|| "Default".into());
    egui::ComboBox::from_id_salt(id).selected_text(current).width(250.0).show_ui(ui, |ui| {
        ui.selectable_value(value, None, "Default");
        for (dev_id, name) in devices {
            ui.selectable_value(value, Some(dev_id.clone()), name);
        }
    });
}

fn meter(ui: &mut egui::Ui, level_db: f32, threshold: Option<f32>) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width().min(320.0), 10.0), Sense::hover());
    ui.painter().rect_filled(r, CornerRadius::same(5), theme::ELEV);
    let frac = ((level_db + 80.0) / 80.0).clamp(0.0, 1.0);
    let fill = egui::Rect::from_min_size(r.min, vec2(r.width() * frac, r.height()));
    let col = if level_db > -12.0 { theme::LIVE } else { theme::ONLINE };
    ui.painter().rect_filled(fill, CornerRadius::same(5), col);
    if let Some(t) = threshold {
        let x = r.left() + r.width() * ((t + 80.0) / 80.0).clamp(0.0, 1.0);
        ui.painter().line_segment([egui::pos2(x, r.top() - 3.0), egui::pos2(x, r.bottom() + 3.0)], egui::Stroke::new(2.0f32, theme::EMBER2));
    }
}

impl IngleApp {
    /// Server settings → Roles: the list on the left, the selected role on the right.
    fn roles_editor(&mut self, ui: &mut egui::Ui, s: &ingle_engine::state::ServerView, acts: &mut Vec<Act>) {
        let key = s.key.clone();
        // Keep the edited role in step with the server (it may have changed, or be gone).
        if let Some(e) = &self.ui.role_edit {
            if s.role(e.id).is_none() {
                self.ui.role_edit = None;
            }
        }
        ui.columns(2, |cols| {
            let ui = &mut cols[0];
            // (Wrapped to the column, not justified: on one line it stretched the dialog off
            // the window.)
            ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                ui.add(
                    egui::Label::new(
                        RichText::new("Roles are listed highest first. People get every permission of every role they have; colors and groups come from their highest role.")
                            .size(12.0)
                            .color(theme::DIM),
                    )
                    .wrap(),
                );
            });
            ui.add_space(6.0);
            if ui.add(theme::primary_button("Create role")).clicked() {
                acts.push(Act::CreateRole { key: key.clone(), name: "new role".into(), color: 0x99aab5, perms: 0, hoist: false });
            }
            ui.add_space(6.0);
            egui::ScrollArea::vertical().id_salt("roles-list").max_height(380.0).show(ui, |ui| {
                if s.roles.is_empty() {
                    ui.label(RichText::new("No roles yet.").color(theme::MUTED));
                }
                for r in &s.roles {
                    let count = s.members.values().filter(|m| m.roles.contains(&r.id)).count();
                    let selected = self.ui.role_edit.as_ref().map(|e| e.id == r.id).unwrap_or(false);
                    let resp = ui.add(egui::Button::selectable(selected, "").min_size(vec2(ui.available_width(), 34.0)));
                    let rect = resp.rect;
                    let col = if r.color != 0 { rgb(r.color) } else { theme::MUTED };
                    ui.painter().circle_filled(pos2(rect.left() + 14.0, rect.center().y), 6.0, col);
                    ui.painter().text(pos2(rect.left() + 28.0, rect.center().y), egui::Align2::LEFT_CENTER, &r.name, theme::bold_font(13.5), theme::TEXT);
                    let people = if count == 1 { "1 person".to_string() } else { format!("{count} people") };
                    ui.painter().text(pos2(rect.right() - 10.0, rect.center().y), egui::Align2::RIGHT_CENTER, people, theme::font(12.0), theme::DIM);
                    if resp.clicked() {
                        let [rr, gg, bb] = [(r.color >> 16) as u8, (r.color >> 8) as u8, r.color as u8];
                        self.ui.role_edit = Some(RoleEdit { id: r.id, name: r.name.clone(), color: [rr, gg, bb], perms: r.perms, hoist: r.hoist });
                    }
                }
            });

            let ui = &mut cols[1];
            let Some(mut e) = self.ui.role_edit.clone() else {
                ui.add_space(30.0);
                ui.label(RichText::new("Pick a role to edit it.").color(theme::MUTED));
                return;
            };
            let Some(role) = s.role(e.id).cloned() else { return };
            let editable = s.can_manage_role(&role);
            if !editable {
                ui.label(RichText::new("This role is at or above your own highest role, so you can't change it.").size(12.0).color(theme::LIVE));
            }
            ui.add_enabled_ui(editable, |ui| {
                field_label(ui, "Name");
                ui.add(egui::TextEdit::singleline(&mut e.name).desired_width(f32::INFINITY).char_limit(32));
                field_label(ui, "Color");
                ui.horizontal(|ui| {
                    ui.color_edit_button_srgb(&mut e.color);
                    for c in [0x1abc9c, 0x2ecc71, 0x3498db, 0x9b59b6, 0xe91e63, 0xf1c40f, 0xe67e22, 0xe74c3c, 0x95a5a6] {
                        let (r, resp) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::click());
                        ui.painter().rect_filled(r, CornerRadius::same(4), rgb(c));
                        if resp.clicked() {
                            e.color = [(c >> 16) as u8, (c >> 8) as u8, c as u8];
                        }
                    }
                });
                ui.checkbox(&mut e.hoist, "Show members with this role separately in the member list");
                field_label(ui, "Permissions");
                let mine = s.my_perms();
                for (bit, name, what) in perm::LIST {
                    let mut on = e.perms & bit != 0;
                    let allowed = mine & bit != 0;
                    let resp = ui.add_enabled(allowed, egui::Checkbox::new(&mut on, name)).on_hover_text(what);
                    if !allowed {
                        resp.on_disabled_hover_text("You can only give permissions you have yourself.");
                    }
                    if on {
                        e.perms |= bit;
                    } else {
                        e.perms &= !bit;
                    }
                }
                // (Notes wrap to the column, left aligned, and the buttons go on two rows: on one
                // they ran into each other in a narrow column.)
                let note = |ui: &mut egui::Ui, text: &str| {
                    ui.with_layout(Layout::top_down(Align::LEFT), |ui| {
                        ui.add(egui::Label::new(RichText::new(text).size(11.5).color(theme::DIM)).wrap());
                    });
                };
                note(ui, "Everyone can chat, react, join voice, share and create invites.");
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let changed = e.name.trim() != role.name
                        || u32::from(e.color[0]) << 16 | u32::from(e.color[1]) << 8 | u32::from(e.color[2]) != role.color
                        || e.perms != role.perms
                        || e.hoist != role.hoist;
                    if ui.add_enabled(changed && !e.name.trim().is_empty(), theme::primary_button("Save changes")).clicked() {
                        acts.push(Act::UpdateRole {
                            key: key.clone(),
                            id: e.id,
                            name: Some(e.name.trim().to_string()),
                            color: Some(u32::from(e.color[0]) << 16 | u32::from(e.color[1]) << 8 | u32::from(e.color[2])),
                            perms: Some(e.perms),
                            hoist: Some(e.hoist),
                            shift: 0,
                        });
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::danger_button("Delete")).clicked() {
                            acts.push(Act::DeleteRole { key: key.clone(), id: e.id });
                        }
                    });
                });
                ui.horizontal(|ui| {
                    if ui.button("Move up").clicked() {
                        acts.push(Act::UpdateRole { key: key.clone(), id: e.id, name: None, color: None, perms: None, hoist: None, shift: 1 });
                    }
                    if ui.button("Move down").clicked() {
                        acts.push(Act::UpdateRole { key: key.clone(), id: e.id, name: None, color: None, perms: None, hoist: None, shift: -1 });
                    }
                });
                note(ui, "Give someone a role by right-clicking them in the member list.");
            });
            self.ui.role_edit = Some(e);
        });
    }
}

impl IngleApp {
    /// Server settings → Emoji: upload, rename and remove the server's own emoji.
    fn emoji_editor(&mut self, ui: &mut egui::Ui, s: &ingle_engine::state::ServerView, acts: &mut Vec<Act>) {
        let max = ingle_proto::control::MAX_CUSTOM_EMOJI;
        ui.label(
            RichText::new(format!(
                "Add up to {max} emoji anyone in this server can use, by typing :name: or from the emoji picker. \
                 PNG, JPEG, WebP or GIF (animated GIFs move). Names are letters, numbers and underscores."
            ))
            .color(theme::MUTED),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button("Upload emoji")).clicked() {
                crate::images::pick_async(crate::images::PickFor::Emoji(s.key.clone()), true, ui.ctx());
            }
            ui.label(RichText::new(format!("{} of {max} used", s.info.emojis.len())).size(12.0).color(theme::DIM));
        });
        ui.add_space(8.0);
        if s.info.emojis.is_empty() {
            ui.label(RichText::new("No emoji yet.").color(theme::DIM));
            return;
        }
        let ctx = ui.ctx().clone();
        let rows_h = (s.info.emojis.len() as f32 * 40.0 + 30.0).min(380.0);
        egui::ScrollArea::vertical().max_height(380.0).min_scrolled_height(rows_h).show(ui, |ui| {
            egui::Grid::new("emoji-list").num_columns(4).spacing(vec2(12.0, 6.0)).striped(true).show(ui, |ui| {
                ui.label(RichText::new("IMAGE").font(theme::bold_font(11.0)).color(theme::DIM));
                ui.label(RichText::new("NAME").font(theme::bold_font(11.0)).color(theme::DIM));
                ui.label(RichText::new("ADDED BY").font(theme::bold_font(11.0)).color(theme::DIM));
                ui.label("");
                ui.end_row();
                for e in &s.info.emojis {
                    let r = crate::emoji::EmojiRef::Custom { name: e.name.clone(), id: e.id.clone(), animated: e.animated };
                    crate::emoji::widget(ui, 32.0, &r, &mut |id| self.custom_emoji_tex(&ctx, &s.key, id));
                    let mut name = self.ui.emoji_names.get(&e.id).cloned().unwrap_or_else(|| e.name.clone());
                    let resp = ui.add_sized([200.0, 26.0], egui::TextEdit::singleline(&mut name).char_limit(32));
                    if resp.changed() {
                        self.ui.emoji_names.insert(e.id.clone(), name.clone());
                    }
                    if resp.lost_focus() {
                        let n = name.trim().to_string();
                        if n != e.name && ingle_proto::control::valid_emoji_name(&n) {
                            acts.push(Act::RenameEmoji { key: s.key.clone(), id: e.id.clone(), name: n });
                        } else if n != e.name {
                            acts.push(Act::Toast("Emoji names are 2–32 letters, numbers or underscores.".into(), true));
                        }
                        self.ui.emoji_names.remove(&e.id);
                    }
                    ui.label(RichText::new(s.member_name(&e.by)).color(theme::MUTED));
                    if crate::app::icon_button(ui, 26.0, "Remove", false, |p, r, _| crate::icons::trash(p, r, theme::LIVE)).clicked() {
                        acts.push(Act::RemoveEmoji { key: s.key.clone(), id: e.id.clone() });
                    }
                    ui.end_row();
                }
            });
        });
    }
}

impl IngleApp {
    /// Server settings → Backups (owner only): this server's backups on this PC.
    fn backups_tab(&mut self, ui: &mut egui::Ui, st: &AppState, key: &str) {
        let folder = self.engine.backup_folder(key);
        let status = st.backups.get(key).cloned().unwrap_or_default();
        let last = status.last_ok.or_else(|| ingle_engine::backup::info(&folder).map(|i| i.last_backup));
        field_label(ui, "Backups on this PC");
        ui.label(
            RichText::new(
                "While Ingle is open, everything on this server is copied to this PC every hour: messages, channels, members, \
                 roles, emoji, pictures and files, and the server's identity. If the VPS is lost, replaced or you stop paying \
                 for it, set up a new one with Restore from a backup and the server comes back exactly as it was, as the same \
                 server for everyone.",
            )
            .color(theme::MUTED),
        );
        ui.add_space(8.0);
        let when = |ms: i64| {
            chrono::DateTime::from_timestamp_millis(ms)
                .map(|t| t.with_timezone(&chrono::Local).format("%b %-d, %Y at %-I:%M %p").to_string())
                .unwrap_or_default()
        };
        let line = if status.running {
            "Backing up now…".to_string()
        } else {
            match last {
                Some(t) => format!("Last backed up {}", when(t)),
                None => "Not backed up yet".to_string(),
            }
        };
        ui.label(RichText::new(line).color(theme::TEXT));
        if let Some(e) = &status.error {
            ui.label(RichText::new(format!("The last try failed: {e}")).color(theme::LIVE));
        }
        ui.add_space(4.0);
        ui.label(RichText::new(folder.display().to_string()).size(12.0).color(theme::DIM));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.add_enabled(!status.running, theme::primary_button("Back up now")).clicked() {
                self.engine.backup_now(key);
            }
            if ui.button("Open folder").clicked() {
                let _ = std::fs::create_dir_all(&folder);
                crate::create::open_folder(&folder);
            }
            if ui.button("Change folder…").on_hover_text("A OneDrive, Dropbox or Google Drive folder keeps a copy online too").clicked() {
                crate::images::pick_folder_async(crate::images::FolderFor::Backups, "Where to keep server backups", ui.ctx());
            }
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new("The backup includes direct messages and the server's private key: keep the folder to yourself.")
                .size(12.0)
                .color(theme::DIM),
        );
    }
}

impl IngleApp {
    /// What's about to be shared, live (a few frames a second), in the share picker.
    fn share_preview(&mut self, ui: &mut egui::Ui, source: Option<&str>, height: f32) {
        if self.ui.preview_source.as_deref() != source {
            self.ui.preview_source = source.map(String::from);
            self.ui.preview_tex = None;
            self.engine.preview_screen(source);
        }
        if let Some((w, h, rgba)) = self.engine.preview_frame(640) {
            let img = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], &rgba);
            match self.ui.preview_tex.as_mut() {
                Some((t, at)) => {
                    t.set(img, egui::TextureOptions::LINEAR);
                    *at = std::time::Instant::now();
                }
                None => {
                    let t = ui.ctx().load_texture("share-preview", img, egui::TextureOptions::LINEAR);
                    self.ui.preview_tex = Some((t, std::time::Instant::now()));
                }
            }
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        field_label(ui, "PREVIEW");
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
        ui.painter().rect_filled(r, CornerRadius::same(8), Color32::BLACK);
        match &self.ui.preview_tex {
            Some((t, _)) => {
                let size = t.size_vec2();
                let scale = (r.width() / size.x).min(r.height() / size.y);
                let pr = egui::Rect::from_center_size(r.center(), size * scale);
                let mut mesh = egui::Mesh::with_texture(t.id());
                mesh.add_rect_with_uv(pr, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                ui.painter().add(egui::Shape::mesh(mesh));
            }
            None => {
                let text = if source.is_some() { "Getting a preview…" } else { "Pick a screen or window" };
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, theme::font(13.0), theme::DIM);
            }
        }
        ui.add_space(4.0);
    }
}
