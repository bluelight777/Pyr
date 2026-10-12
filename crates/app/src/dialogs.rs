//! Modal dialogs: join server, invites, channels, settings, screen picker.

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, RichText, Sense, pos2, vec2};
use ingle_engine::config::{CameraQuality, InputMode, NoiseSuppression, ScreenPreset};
use ingle_engine::hotkey;
use ingle_engine::state::AppState;
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

pub(crate) fn input(ui: &mut egui::Ui, text: &mut String, hint: &str) -> egui::Response {
    ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(f32::INFINITY).margin(egui::Margin::symmetric(10, 8)))
}

impl IngleApp {
    pub fn dialogs(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        let Some(dialog) = self.ui.dialog.clone() else { return };
        let was_create = matches!(dialog, Dialog::Create);
        let mut close = false;
        let mut modal = egui::Modal::new(egui::Id::new("dialog")).frame(modal_frame()).backdrop_color(Color32::from_black_alpha(170));
        if was_create {
            // The wizard changes height as you go; pinning its top keeps buttons from jumping.
            let top = (ctx.content_rect().height() * 0.07).clamp(16.0, 80.0);
            modal = modal.area(egui::Modal::default_area(egui::Id::new("dialog-create")).anchor(egui::Align2::CENTER_TOP, vec2(0.0, top)));
        }
        let resp = modal.show(ctx, |ui| {
            match dialog {
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
                        self.ui.create = Some(crate::create::CreateState::new(&st.my_name));
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
                        "A server for you and your friends. Rent one for a few dollars a month, or use a server you already have.",
                        icons::cloud,
                    ) {
                        self.ui.create = Some(crate::create::CreateState::new(&st.my_name));
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
                    let r = input(ui, &mut name, if k == ChannelKind::Text { "new-channel" } else { "Game Night" });
                    let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if !enter {
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
                    input(ui, &mut name, "");
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
                    title(ui, &format!("Invite friends to {}", s.map(|s| s.name.as_str()).unwrap_or("this server")));
                    ui.label(RichText::new("Send them this link. They paste it into \"Join a server\".").color(theme::MUTED));
                    ui.add_space(8.0);
                    match s.and_then(|s| s.last_invite.clone()) {
                        Some(link) => {
                            egui::Frame::new().fill(theme::CODE_BG).corner_radius(6).inner_margin(10).show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(&link).font(theme::mono(12.5)).color(theme::TEXT)).wrap());
                            });
                            ui.add_space(10.0);
                            ui.horizontal(|ui| {
                                if ui.add(theme::primary_button("Copy link").min_size(vec2(110.0, 34.0))).clicked() {
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
                    ui.set_width(if tab == 1 { 680.0 } else { 440.0 });
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
                    });
                    ui.separator();
                    if tab == 1 {
                        if let Some(s) = &server {
                            self.roles_editor(ui, s, acts);
                        }
                    } else {
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
                Dialog::ScreenPicker { selected } => {
                    ui.set_width(560.0);
                    title(ui, "Share your screen");
                    let preset = self.ui.settings_draft.as_ref().map(|s| s.screen_preset).unwrap_or(st.settings.screen_preset);
                    let mut chosen = selected.clone();
                    let screens = &st.devices.screens;
                    if screens.is_empty() {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new("Looking for screens and windows…").color(theme::MUTED));
                        });
                    }
                    for (label, kind) in [("SCREENS", SourceKind::Monitor), ("WINDOWS", SourceKind::Window), ("OTHER", SourceKind::Test)] {
                        let items: Vec<_> = screens.iter().filter(|s| s.kind == kind).collect();
                        if items.is_empty() {
                            continue;
                        }
                        field_label(ui, label);
                        egui::ScrollArea::vertical().id_salt(label).max_height(if kind == SourceKind::Window { 180.0 } else { 120.0 }).show(ui, |ui| {
                            for s in items {
                                let sel = chosen.as_deref() == Some(s.id.as_str());
                                let text = if s.width > 0 { format!("{}   ({}×{})", s.name, s.width, s.height) } else { s.name.clone() };
                                if ui.add(egui::Button::selectable(sel, text).min_size(vec2(ui.available_width(), 30.0))).clicked() {
                                    chosen = Some(s.id.clone());
                                }
                            }
                        });
                    }
                    if chosen.is_none() {
                        chosen = screens.iter().find(|s| s.kind == SourceKind::Monitor).map(|s| s.id.clone());
                    }
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
                    ui.label(RichText::new("Free, no subscription. Viewers choose to watch, so it only uses bandwidth for people who are watching.").size(12.0).color(theme::DIM));
                    ui.add_space(14.0);
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
            self.ui.dialog = None;
        }
    }

    fn settings_dialog(&mut self, ui: &mut egui::Ui, st: &AppState, tab: u8, acts: &mut Vec<Act>, close: &mut bool) {
        ui.set_width(640.0);
        let mut draft = self.ui.settings_draft.clone().unwrap_or_else(|| st.settings.clone());
        // The tray note can change these while this dialog is open; don't save stale copies.
        draft.close_to_tray = st.settings.close_to_tray;
        draft.tray_note_seen = st.settings.tray_note_seen;
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
        egui::ScrollArea::vertical().max_height(470.0).min_scrolled_height(470.0).show(ui, |ui| match t {
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
                                if let Some((_, bytes)) = crate::images::pick_image() {
                                    acts.push(Act::SetAvatar(Some(bytes)));
                                }
                            }
                            if face.is_some() && ui.button("Remove").clicked() {
                                acts.push(Act::SetAvatar(None));
                            }
                        });
                        ui.label(
                            RichText::new("PNG, JPEG, GIF or WebP. The middle square is used. It shows on every server you're in.")
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
                            "Only your voice gets through: TVs, people talking in the room and sounds between your words are cut. \
                             Someone talking right next to you as loudly as you can still be heard. Try it with Let's check.",
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
                field_label(ui, "Muting");
                ui.label(
                    RichText::new("Right-click a channel, a server or a direct message conversation to mute it.")
                        .size(12.0)
                        .color(theme::DIM),
                );
            }
            3 => {
                field_label(ui, "Icon by the clock");
                ui.checkbox(&mut draft.close_to_tray, "Keep Ingle running in the tray when I close the window");
                ui.label(
                    RichText::new(
                        "You stay in voice and friends can still reach you. Click the campfire icon by the clock to open Ingle; \
                         right-click it to mute, leave voice or quit.",
                    )
                    .size(12.0)
                    .color(theme::DIM),
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
                        RichText::new("The tray icon couldn't be added right now, so closing the window quits Ingle.")
                            .size(12.0)
                            .color(theme::LIVE),
                    );
                }
            }
            _ => {
                ui.add_space(8.0);
                ui.label(RichText::new(format!("{} {}", ingle_proto::APP_NAME, env!("CARGO_PKG_VERSION"))).font(theme::bold_font(16.0)).color(theme::TEXT));
                ui.label(RichText::new("Lean voice, video, screen share and chat. Written in Rust.").color(theme::MUTED));
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
            }
        });
        if draft != before {
            acts.push(Act::Settings(draft.clone()));
        }
        self.ui.settings_draft = Some(draft);
        if !*close {
            self.ui.dialog = Some(Dialog::Settings { tab: t });
        }
    }
}

impl IngleApp {
    /// Shown the first time the close button hides Ingle to the tray (Windows).
    pub fn tray_note(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        if !self.tray_note {
            return;
        }
        // Some(true): keep running in the tray; Some(false): the close button quits.
        let mut choice: Option<bool> = None;
        let modal = egui::Modal::new(egui::Id::new("tray-note")).frame(modal_frame()).backdrop_color(Color32::from_black_alpha(170));
        let resp = modal.show(ctx, |ui| {
            ui.set_width(460.0);
            title(ui, "Ingle keeps running by the clock");
            ui.label(
                RichText::new("Closing the window leaves Ingle running in the tray, so you stay in voice and friends can still reach you.")
                    .color(theme::MUTED),
            );
            ui.add_space(6.0);
            ui.label(
                RichText::new(
                    "Click the campfire icon by the clock to come back. Windows may tuck it under the ^ arrow. \
                     To quit, right-click it and choose Quit Ingle.",
                )
                .color(theme::MUTED),
            );
            ui.add_space(6.0);
            ui.label(RichText::new("You can change this any time in Settings, under Windows.").size(12.0).color(theme::DIM));
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button("Keep running").min_size(vec2(130.0, 34.0))).clicked() {
                    choice = Some(true);
                }
                if ui.button("Quit when I close it").clicked() {
                    choice = Some(false);
                }
            });
        });
        if let Some(keep_running) = choice {
            let mut s = st.settings.clone();
            s.close_to_tray = keep_running;
            s.tray_note_seen = true;
            acts.push(Act::Settings(s));
            self.tray_note = false;
            if keep_running {
                crate::tray::hide_main();
            } else {
                // Settings apply at the end of this frame, so the close isn't stopped again.
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        } else if resp.should_close() {
            // Escape or a click outside: stay open, and ask again next time.
            self.tray_note = false;
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
            ui.horizontal(|ui| {
                ui.label(RichText::new("Roles are listed highest first. People get every permission of every role they have; colors and groups come from their highest role.").size(12.0).color(theme::DIM));
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
                ui.label(RichText::new("Everyone can chat, react, join voice, share and create invites.").size(11.5).color(theme::DIM));
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
                    if ui.button("Move up").clicked() {
                        acts.push(Act::UpdateRole { key: key.clone(), id: e.id, name: None, color: None, perms: None, hoist: None, shift: 1 });
                    }
                    if ui.button("Move down").clicked() {
                        acts.push(Act::UpdateRole { key: key.clone(), id: e.id, name: None, color: None, perms: None, hoist: None, shift: -1 });
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::danger_button("Delete")).clicked() {
                            acts.push(Act::DeleteRole { key: key.clone(), id: e.id });
                        }
                    });
                });
                ui.label(RichText::new("Give someone a role by right-clicking them in the member list.").size(11.5).color(theme::DIM));
            });
            self.ui.role_edit = Some(e);
        });
    }
}
