//! The main window: server rail, channel list, chat / call view, member list, dialogs.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, pos2, vec2};
use ingle_engine::Engine;
use ingle_engine::config::Settings;
use ingle_engine::state::{AppState, ConnStatus, ServerView, VideoKey};
use ingle_proto::control::{Attachment, ChannelId, ChannelKind, MessageId, Role, RoleId, SessionId, perm};

use crate::{icons, theme};

pub enum Act {
    AddServer(String),
    /// Join a server this app just set up, with its owner link (keeps the setup dialog open).
    JoinOwned(String),
    /// Show the "Join a server" dialog.
    OpenJoin,
    RemoveServer(String),
    Retry(String),
    Send { key: String, channel: ChannelId, text: String, reply_to: Option<MessageId>, files: Vec<(String, Vec<u8>)> },
    Edit { key: String, id: MessageId, text: String },
    Delete { key: String, id: MessageId },
    React { key: String, id: MessageId, emoji: String, add: bool },
    LoadHistory { key: String, channel: ChannelId, older: bool },
    SetActive { key: String, channel: ChannelId },
    Typing { key: String, channel: ChannelId },
    CreateChannel { key: String, name: String, kind: ChannelKind },
    UpdateChannel { key: String, channel: ChannelId, name: Option<String>, topic: Option<String> },
    DeleteChannel { key: String, channel: ChannelId },
    CreateInvite { key: String },
    Kick { key: String, uid: String, ban: bool },
    SetRole { key: String, uid: String, role: Role },
    RenameServer { key: String, name: String },
    SetName(String),
    Join { key: String, channel: ChannelId },
    Leave,
    Mute(bool),
    Deafen(bool),
    Camera(bool),
    StartScreen(String),
    StopScreen,
    Watch { session: SessionId, track: u8, on: bool },
    Volume { uid: String, volume: f32 },
    StreamVolume { uid: String, volume: f32 },
    Settings(Settings),
    MicTest(bool),
    RefreshDevices,
    DiscardPending { key: String, channel: ChannelId, nonce: String },
    Copy(String),
    CreateRole { key: String, name: String, color: u32, perms: u64, hoist: bool },
    UpdateRole { key: String, id: RoleId, name: Option<String>, color: Option<u32>, perms: Option<u64>, hoist: Option<bool>, shift: i32 },
    DeleteRole { key: String, id: RoleId },
    SetMemberRoles { key: String, uid: String, roles: Vec<RoleId> },
    /// A new profile picture (any image; it's cropped and resized), or none.
    SetAvatar(Option<Vec<u8>>),
    SaveImage { name: String, bytes: Arc<Vec<u8>> },
    Toast(String, bool),
}

/// The role being edited in server settings.
#[derive(Clone, Debug, PartialEq)]
pub struct RoleEdit {
    pub id: RoleId,
    pub name: String,
    pub color: [u8; 3],
    pub perms: u64,
    pub hoist: bool,
}

#[derive(Clone)]
pub enum Dialog {
    /// "Join a server" with an invite link.
    AddServer { link: String, error: Option<String> },
    /// Create or join?
    AddChoice,
    /// The create-a-server wizard (its state is in `UiState::create`).
    Create,
    CreateChannel { key: String, kind: ChannelKind, name: String },
    EditChannel { key: String, channel: ChannelId, name: String, topic: String },
    Invite { key: String },
    /// Tab 0: overview; 1: roles.
    ServerSettings { key: String, name: String, tab: u8 },
    LeaveServer { key: String, name: String },
    Settings { tab: u8 },
    ScreenPicker { selected: Option<String> },
}

#[derive(Default)]
pub struct UiState {
    pub server: Option<String>,
    pub channel: HashMap<String, ChannelId>,
    pub call_view: bool,
    pub drafts: HashMap<(String, ChannelId), String>,
    pub reply: Option<(MessageId, String)>,
    pub editing: Option<(MessageId, String)>,
    pub members_open: bool,
    pub call_chat_open: bool,
    pub focus: Option<VideoKey>,
    /// A stream filling the whole screen, and whether the window is in full screen for it.
    pub fullscreen: Option<VideoKey>,
    pub fullscreen_on: bool,
    /// A click on a video tile, acted on once it's clearly not a double-click.
    pub pending_click: Option<(VideoKey, Instant)>,
    pub dialog: Option<Dialog>,
    pub react_picker: Option<MessageId>,
    pub composer_emoji: bool,
    pub last_typing: Option<Instant>,
    pub requested: HashSet<(String, ChannelId)>,
    pub settings_draft: Option<Settings>,
    pub name_draft: String,
    pub welcome_name: String,
    pub create: Option<crate::create::CreateState>,
    /// Images waiting to be sent, per channel: (file name, bytes).
    pub attach: HashMap<(String, ChannelId), Vec<(String, Vec<u8>)>>,
    /// An image open in the full-size viewer: (server key, the image).
    pub viewer: Option<(String, Attachment)>,
    pub role_edit: Option<RoleEdit>,
    /// Ctrl+V was down last frame (to catch an image paste once).
    pub paste_down: bool,
}

pub struct IngleApp {
    pub engine: Arc<Engine>,
    pub ui: UiState,
    pub textures: HashMap<VideoKey, (egui::TextureHandle, u64)>,
    /// Per video tile: (window start, frames in window, last measured fps)
    pub fps: HashMap<VideoKey, (Instant, u32, f32)>,
    started: Instant,
    /// Invite links handed over by later launches (e.g. clicking an ingle:// link).
    handoff: crate::instance::Handoff,
    /// Invite link the app was started with, joined on the first frame.
    initial_join: Option<String>,
    /// The icon by the clock (Windows); kept here so it goes away with the app.
    _tray: Option<crate::tray::Tray>,
    /// Showing the one-time note about the close button hiding Ingle to the tray.
    pub tray_note: bool,
    /// Decoded images and profile pictures.
    pub images: crate::images::Images,
    pub updater: crate::update::Updater,
    /// The update we last told the person about (pinged once per version).
    update_seen: Option<ingle_proto::release::Manifest>,
    /// After an update: where to put the person back, and since when we've been trying.
    resume: Option<(crate::update::Resume, Instant)>,
}

impl IngleApp {
    pub fn new(
        engine: Arc<Engine>,
        handoff: crate::instance::Handoff,
        initial_join: Option<String>,
        tray: Option<crate::tray::Tray>,
        updater: crate::update::Updater,
    ) -> IngleApp {
        engine.refresh_devices();
        let resume = crate::update::take_resume(&engine.shared.config_dir).map(|r| (r, Instant::now()));
        let ui = UiState { members_open: true, call_chat_open: false, ..Default::default() };
        IngleApp {
            engine,
            ui,
            textures: HashMap::new(),
            fps: HashMap::new(),
            started: Instant::now(),
            handoff,
            initial_join,
            _tray: tray,
            tray_note: false,
            images: crate::images::Images::default(),
            updater,
            update_seen: None,
            resume,
        }
    }

    fn apply(&mut self, a: Act, ctx: &egui::Context) {
        let e = &self.engine;
        match a {
            Act::AddServer(link) => match e.add_server(&link) {
                Ok(key) => {
                    self.ui.server = Some(key);
                    self.ui.dialog = None;
                }
                Err(err) => self.ui.dialog = Some(Dialog::AddServer { link, error: Some(err.to_string()) }),
            },
            Act::JoinOwned(link) => match e.add_server(&link) {
                Ok(key) => {
                    self.ui.server = Some(key);
                    self.ui.call_view = false;
                }
                Err(err) => {
                    if let Some(c) = self.ui.create.as_mut() {
                        c.join_error = Some(err.to_string());
                    }
                }
            },
            Act::OpenJoin => self.ui.dialog = Some(Dialog::AddServer { link: String::new(), error: None }),
            Act::RemoveServer(k) => {
                e.remove_server(&k);
                if self.ui.server.as_deref() == Some(k.as_str()) {
                    self.ui.server = None;
                }
            }
            Act::Retry(k) => e.retry_server(&k),
            Act::Send { key, channel, text, reply_to, files } => e.send_message_with(&key, channel, &text, reply_to, files),
            Act::Edit { key, id, text } => e.edit_message(&key, id, &text),
            Act::Delete { key, id } => e.delete_message(&key, id),
            Act::React { key, id, emoji, add } => e.react(&key, id, &emoji, add),
            Act::LoadHistory { key, channel, older } => e.load_history(&key, channel, older),
            Act::SetActive { key, channel } => e.set_active_channel(&key, channel),
            Act::Typing { key, channel } => e.typing(&key, channel),
            Act::CreateChannel { key, name, kind } => e.create_channel(&key, &name, kind),
            Act::UpdateChannel { key, channel, name, topic } => e.update_channel(&key, channel, name, topic),
            Act::DeleteChannel { key, channel } => e.delete_channel(&key, channel),
            Act::CreateInvite { key } => e.create_invite(&key, None, None),
            Act::Kick { key, uid, ban } => e.kick(&key, &uid, ban),
            Act::SetRole { key, uid, role } => e.set_role(&key, &uid, role),
            Act::RenameServer { key, name } => e.update_server(&key, Some(name), None),
            Act::SetName(n) => e.set_display_name(&n),
            Act::Join { key, channel } => {
                e.join_voice(&key, channel);
                self.ui.call_view = true;
            }
            Act::Leave => {
                e.leave_voice();
                self.ui.call_view = false;
                self.ui.focus = None;
            }
            Act::Mute(m) => e.set_mute(m),
            Act::Deafen(d) => e.set_deafen(d),
            Act::Camera(on) => {
                if on {
                    e.start_camera()
                } else {
                    e.stop_camera()
                }
            }
            Act::StartScreen(src) => e.start_screen(&src),
            Act::StopScreen => e.stop_screen(),
            Act::Watch { session, track, on } => {
                e.watch(session, track, on);
                if on {
                    self.ui.focus = Some(VideoKey { session, track });
                }
            }
            Act::Volume { uid, volume } => e.set_user_volume(&uid, volume),
            Act::StreamVolume { uid, volume } => e.set_stream_volume(&uid, volume),
            Act::Settings(s) => e.update_settings(s),
            Act::MicTest(on) => {
                if on {
                    e.start_mic_test()
                } else {
                    e.stop_mic_test()
                }
            }
            Act::RefreshDevices => e.refresh_devices(),
            Act::DiscardPending { key, channel, nonce } => e.discard_pending(&key, channel, &nonce),
            Act::Copy(text) => ctx.copy_text(text),
            Act::CreateRole { key, name, color, perms, hoist } => e.create_role(&key, &name, color, perms, hoist),
            Act::UpdateRole { key, id, name, color, perms, hoist, shift } => e.update_role(&key, id, name, color, perms, hoist, shift),
            Act::DeleteRole { key, id } => e.delete_role(&key, id),
            Act::SetMemberRoles { key, uid, roles } => e.set_member_roles(&key, &uid, roles),
            Act::SetAvatar(bytes) => match bytes.map(|b| crate::images::make_avatar(&b)).transpose() {
                Ok(png) => e.set_avatar(png),
                Err(err) => self.toast(err, true),
            },
            Act::SaveImage { name, bytes } => {
                if let Err(err) = crate::images::save_as(&name, &bytes) {
                    self.toast(format!("Couldn't save the image: {err}"), true);
                }
            }
            Act::Toast(text, error) => self.toast(text, error),
        }
    }

    fn toast(&self, text: impl Into<String>, error: bool) {
        self.engine.shared.state.write().toast(text, error);
    }

    /// Someone's profile picture as a texture, if they have one and it's loaded.
    pub fn face(&mut self, ctx: &egui::Context, s: &ServerView, uid: &str) -> Option<egui::TextureId> {
        let id = s.members.get(uid)?.avatar.clone()?;
        let bytes = match self.engine.file(&s.key, &id) {
            ingle_engine::files::FileState::Ready(b) => Some(b),
            _ => None,
        };
        self.images.get(ctx, &format!("av:{id}"), bytes, 256).frame(0.0).map(|f| f.0)
    }

    /// My own profile picture (from this PC), as a texture.
    pub fn my_face(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        let bytes = self.engine.my_avatar()?;
        let key = format!("av:me:{}", ingle_proto::release::sha256_hex(&bytes));
        self.images.get(ctx, &key, Some(Arc::new(bytes)), 256).frame(0.0).map(|f| f.0)
    }

    /// Upload new video frames to textures; returns the texture for a tile.
    pub fn texture(&mut self, ctx: &egui::Context, key: VideoKey) -> Option<(egui::TextureId, egui::Vec2)> {
        let frame = self.engine.video_frame(key)?;
        let size = [frame.width as usize, frame.height as usize];
        match self.textures.get_mut(&key) {
            Some((tex, generation)) => {
                if *generation != frame.generation {
                    tex.set(egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba), egui::TextureOptions::LINEAR);
                    *generation = frame.generation;
                    let e = self.fps.entry(key).or_insert((Instant::now(), 0, 0.0));
                    e.1 += 1;
                    let dt = e.0.elapsed().as_secs_f32();
                    if dt >= 1.0 {
                        e.2 = e.1 as f32 / dt;
                        *e = (Instant::now(), 0, e.2);
                    }
                }
            }
            None => {
                let tex = ctx.load_texture(
                    format!("video-{}-{}", key.session, key.track),
                    egui::ColorImage::from_rgba_unmultiplied(size, &frame.rgba),
                    egui::TextureOptions::LINEAR,
                );
                self.textures.insert(key, (tex, frame.generation));
            }
        }
        let (tex, _) = self.textures.get(&key)?;
        Some((tex.id(), vec2(frame.width as f32, frame.height as f32)))
    }
}

impl eframe::App for IngleApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let engine = self.engine.clone();
        let mut acts: Vec<Act> = Vec::new();
        if let Some(link) = self.initial_join.take() {
            acts.push(Act::AddServer(link));
        }
        // Another launch handed us an invite link (or just wants the window): come to the front.
        if let Some(link) = self.handoff.poll() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            if !link.is_empty() {
                acts.push(Act::AddServer(link));
            }
        }
        // With the icon by the clock, the close button hides Ingle instead of quitting, so calls
        // and messages keep coming in (the first time, a note says so).
        if ctx.input(|i| i.viewport().close_requested()) && crate::tray::is_active() {
            let (keep_running, note_seen) = {
                let s = &engine.state().settings;
                (s.close_to_tray, s.tray_note_seen)
            };
            if keep_running {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                if note_seen {
                    crate::tray::hide_main();
                } else {
                    self.tray_note = true;
                }
            }
        }
        self.update_step(&mut acts);
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        {
            let st = engine.state();
            self.frame_ui(ctx, &st, &mut acts);
        }
        for a in acts {
            self.apply(a, ctx);
        }
        // Drop textures for tiles that are gone.
        let live: Vec<VideoKey> = self.textures.keys().copied().collect();
        for k in live {
            if self.engine.video_frame(k).is_none() {
                self.textures.remove(&k);
            }
        }
        // Keep animating while toasts/typing indicators/speaking rings are visible.
        ctx.request_repaint_after(Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.engine.shutdown();
    }
}

impl IngleApp {
    /// Update pings, installing a downloaded update, and rejoining after one.
    fn update_step(&mut self, acts: &mut Vec<Act>) {
        use crate::update::Status;
        match self.updater.status() {
            Status::Available(m) => {
                if self.update_seen.as_ref().map(|s| s.version != m.version).unwrap_or(true) {
                    self.engine
                        .shared
                        .state
                        .write()
                        .toast(format!("Ingle {} is out. Click the green arrow at the top left to update.", m.version), false);
                    self.update_seen = Some(m);
                }
            }
            Status::Failed(e) => {
                self.engine.shared.state.write().toast(format!("Update failed: {e}"), true);
                self.updater.dismiss_error(self.update_seen.clone());
            }
            Status::Ready(path) => {
                // Remember where we are, start the installer, and get out of its way.
                {
                    let st = self.engine.state();
                    match &st.call {
                        Some(c) => crate::update::save_resume(&self.engine.shared.config_dir, &c.server_key, Some(c.channel_id)),
                        None => {
                            if let Some(k) = &self.ui.server {
                                crate::update::save_resume(&self.engine.shared.config_dir, k, None);
                            }
                        }
                    }
                }
                match crate::update::install(&path) {
                    Ok(()) => {
                        tracing::info!("quitting to install the update");
                        crate::tray::prepare_exit();
                        self.engine.shutdown();
                        std::thread::sleep(Duration::from_millis(300));
                        std::process::exit(0);
                    }
                    Err(e) => {
                        self.engine.shared.state.write().toast(format!("Update failed: {e}"), true);
                        self.updater.dismiss_error(self.update_seen.clone());
                    }
                }
            }
            Status::Idle | Status::Downloading(..) => {}
        }
        // Back from an update: open the same server and rejoin the voice channel once it's
        // connected (give up after a minute and a half).
        if let Some((r, since)) = &self.resume {
            if since.elapsed() > Duration::from_secs(90) {
                self.resume = None;
            } else {
                let st = self.engine.state();
                if let Some(s) = st.server(&r.server_key) {
                    if matches!(s.status, ConnStatus::Connected) {
                        self.ui.server = Some(r.server_key.clone());
                        if let Some(c) = r.channel_id.filter(|c| s.channel(*c).is_some()) {
                            acts.push(Act::Join { key: r.server_key.clone(), channel: c });
                        }
                        drop(st);
                        self.resume = None;
                    }
                }
            }
        }
    }

    /// Discord's green arrow: shown in the server rail when an update is out.
    fn update_button(&mut self, ui: &mut egui::Ui) {
        use crate::update::Status;
        let (label, frac, manifest) = match self.updater.status() {
            Status::Available(m) => (format!("Update to Ingle {} (restarts Ingle and puts you back where you were)", m.version), None, Some(m)),
            Status::Downloading(done, total) => {
                ("Downloading the update…".to_string(), Some(done as f32 / total.max(1) as f32), None)
            }
            Status::Ready(_) => ("Installing…".to_string(), Some(1.0), None),
            _ => return,
        };
        let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
        let green = Color32::from_rgb(0x23, 0xa5, 0x5a);
        let fill = if resp.hovered() { Color32::from_rgb(0x2d, 0xc7, 0x6d) } else { green };
        ui.painter().rect_filled(rect, CornerRadius::same(if resp.hovered() { 16 } else { 24 }), fill);
        match frac {
            None => {
                // A down arrow into a tray.
                let c = rect.center();
                let s = Stroke::new(3.0f32, Color32::WHITE);
                ui.painter().line_segment([c + vec2(0.0, -11.0), c + vec2(0.0, 5.0)], s);
                ui.painter().line_segment([c + vec2(-7.0, -2.0), c + vec2(0.0, 5.0)], s);
                ui.painter().line_segment([c + vec2(7.0, -2.0), c + vec2(0.0, 5.0)], s);
                ui.painter().line_segment([c + vec2(-10.0, 11.0), c + vec2(10.0, 11.0)], s);
            }
            Some(f) => {
                let pct = format!("{}%", (f * 100.0).round() as u32);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, pct, theme::bold_font(13.0), Color32::WHITE);
                ui.ctx().request_repaint_after(Duration::from_millis(200));
            }
        }
        let resp = resp.on_hover_text(label);
        if resp.clicked() {
            if let Some(m) = manifest {
                self.updater.download(m);
            }
        }
        ui.add_space(8.0);
    }
}

// ------------------------------------------------------------------- helpers

#[allow(clippy::too_many_arguments)]
pub fn avatar(
    p: &egui::Painter,
    center: egui::Pos2,
    radius: f32,
    uid: &str,
    name: &str,
    speaking: bool,
    ring_bg: Color32,
    face: Option<egui::TextureId>,
) {
    if speaking {
        p.circle_filled(center, radius + 4.0, theme::GLOW.gamma_multiply(0.25));
        p.circle_stroke(center, radius + 2.5, Stroke::new(2.5f32, theme::GLOW));
        p.circle_stroke(center, radius + 0.5, Stroke::new(1.5f32, ring_bg));
    }
    if let Some(tex) = face {
        let rect = Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        p.add(egui::epaint::RectShape::filled(rect, CornerRadius::same(radius.min(255.0) as u8), Color32::WHITE).with_texture(tex, uv));
        return;
    }
    p.circle_filled(center, radius, theme::avatar_color(uid));
    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_else(|| "?".into());
    p.text(center, egui::Align2::CENTER_CENTER, initial, theme::bold_font(radius * 0.95), Color32::from_rgb(0x1b, 0x14, 0x11));
}

pub fn status_dot(p: &egui::Painter, center: egui::Pos2, online: bool, bg: Color32) {
    p.circle_filled(center, 6.0, bg);
    if online {
        p.circle_filled(center, 4.0, theme::ONLINE);
    } else {
        p.circle_stroke(center, 3.0, Stroke::new(1.5f32, theme::DIM));
    }
}

/// A square icon button that paints its icon with `draw`. Returns the response.
pub fn icon_button(
    ui: &mut egui::Ui,
    size: f32,
    tooltip: &str,
    active: bool,
    draw: impl FnOnce(&egui::Painter, Rect, Color32),
) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let bg = if active {
        Color32::from_rgb(0x3a, 0x2a, 0x20)
    } else if resp.hovered() {
        theme::ELEV
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
    let color = if active { theme::EMBER2 } else if resp.hovered() { theme::TEXT } else { theme::MUTED };
    draw(ui.painter(), rect.shrink(size * 0.2), color);
    resp.on_hover_text(tooltip)
}

pub fn rgb(c: u32) -> Color32 {
    Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)
}

/// A member's name color: their highest colored role's, else by owner/admin.
pub fn name_color(s: &ServerView, uid: &str, role: Role) -> Color32 {
    s.name_color(uid).map(rgb).unwrap_or_else(|| role_color(role))
}

pub fn role_color(role: Role) -> Color32 {
    match role {
        Role::Owner => theme::OWNER,
        Role::Admin => theme::ADMIN,
        Role::Member => theme::TEXT,
    }
}

pub fn short_status(s: &ConnStatus) -> Option<(String, bool)> {
    match s {
        ConnStatus::Connected => None,
        ConnStatus::Connecting => Some(("Connecting…".into(), false)),
        ConnStatus::Reconnecting { attempt, error } => Some((format!("Reconnecting (try {attempt}): {error}"), false)),
        ConnStatus::Failed(e) => Some((e.clone(), true)),
    }
}

impl IngleApp {
    fn frame_ui(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        if self.fullscreen_ui(ctx, st, acts) {
            self.toasts(ctx, st);
            return;
        }
        // Pick a server if none selected.
        if self.ui.server.as_ref().map(|k| st.server(k).is_none()).unwrap_or(true) {
            self.ui.server = st.servers.first().map(|s| s.key.clone());
        }
        if self.ui.name_draft.is_empty() {
            self.ui.name_draft = st.my_name.clone();
        }

        egui::SidePanel::left("rail")
            .exact_width(72.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::RAIL))
            .show(ctx, |ui| self.rail(ui, st, acts));

        let selected = self.ui.server.clone().and_then(|k| st.server(&k).cloned());
        match selected {
            Some(server) => {
                egui::SidePanel::left("channels")
                    .exact_width(240.0)
                    .resizable(false)
                    .show_separator_line(false)
                    .frame(egui::Frame::new().fill(theme::SIDE))
                    .show(ctx, |ui| self.channel_sidebar(ui, st, &server, acts));
                let in_call_here = st.call.as_ref().map(|c| c.server_key == server.key).unwrap_or(false);
                if self.ui.call_view && in_call_here {
                    let call = st.call.as_ref().expect("checked");
                    if self.ui.call_chat_open {
                        egui::SidePanel::right("call-chat")
                            .exact_width(340.0)
                            .resizable(false)
                            .show_separator_line(false)
                            .frame(egui::Frame::new().fill(theme::MAIN))
                            .show(ctx, |ui| self.chat_view(ui, st, &server, call.channel_id, true, acts));
                    }
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(theme::STAGE))
                        .show(ctx, |ui| self.stage(ui, ctx, st, &server, acts));
                } else {
                    let channel = self.current_channel(&server);
                    if self.ui.members_open {
                        egui::SidePanel::right("members")
                            .exact_width(240.0)
                            .resizable(false)
                            .show_separator_line(false)
                            .frame(egui::Frame::new().fill(theme::SIDE))
                            .show(ctx, |ui| self.member_list(ui, st, &server, acts));
                    }
                    egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::MAIN)).show(ctx, |ui| {
                        match channel {
                            Some(c) => self.chat_view(ui, st, &server, c, false, acts),
                            None => {
                                ui.centered_and_justified(|ui| {
                                    let msg = short_status(&server.status).map(|s| s.0).unwrap_or_else(|| "No channels yet.".into());
                                    ui.label(RichText::new(msg).color(theme::MUTED));
                                });
                            }
                        }
                    });
                }
            }
            None => {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(theme::MAIN))
                    .show(ctx, |ui| self.welcome(ui, st, acts));
            }
        }

        self.dialogs(ctx, st, acts);
        self.tray_note(ctx, st, acts);
        self.viewer(ctx, acts);
        self.drop_files(ctx, st, acts);
        self.toasts(ctx, st);
    }

    /// Images dragged onto the window go to the open channel's composer.
    fn drop_files(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        let Some(server) = self.ui.server.clone().and_then(|k| st.server(&k).cloned()) else { return };
        if !server.has(ingle_proto::control::FILES_FEATURE) {
            return;
        }
        let channel = if self.ui.call_view {
            st.call.as_ref().filter(|c| c.server_key == server.key && self.ui.call_chat_open).map(|c| c.channel_id)
        } else {
            self.current_channel(&server)
        };
        let Some(channel) = channel else { return };
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());
        if hovering {
            let screen = ctx.content_rect();
            egui::Area::new(egui::Id::new("drop-overlay")).order(egui::Order::Foreground).fixed_pos(screen.min).interactable(false).show(ctx, |ui| {
                let (r, _) = ui.allocate_exact_size(screen.size(), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(160));
                let name = server.channel(channel).map(|c| c.name.clone()).unwrap_or_default();
                let b = Rect::from_center_size(r.center(), vec2(420.0, 120.0));
                ui.painter().rect_filled(b, CornerRadius::same(14), theme::EMBER);
                ui.painter().text(b.center() - vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, "Drop images to send", theme::bold_font(20.0), Color32::from_rgb(0x1b, 0x14, 0x11));
                ui.painter().text(b.center() + vec2(0.0, 18.0), egui::Align2::CENTER_CENTER, format!("in #{name}"), theme::font(14.0), Color32::from_rgb(0x3a, 0x22, 0x14));
            });
        }
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let draft = (server.key.clone(), channel);
        for f in dropped {
            let item = match (&f.bytes, &f.path) {
                (Some(b), _) => Some((f.name.clone(), b.to_vec())),
                (None, Some(p)) => crate::images::read_files(std::slice::from_ref(p)).into_iter().next(),
                _ => None,
            };
            if let Some((name, bytes)) = item {
                crate::chat::add_attachment(&mut self.ui.attach, &draft, name, bytes, acts);
            }
        }
    }

    fn current_channel(&mut self, server: &ServerView) -> Option<ChannelId> {
        let sel = self.ui.channel.get(&server.key).copied();
        if let Some(c) = sel {
            if server.channel(c).is_some() {
                return Some(c);
            }
        }
        let first = server.channels.iter().find(|c| c.kind == ChannelKind::Text).or(server.channels.first())?.id;
        self.ui.channel.insert(server.key.clone(), first);
        Some(first)
    }

    // ------------------------------------------------------------ server rail

    fn rail(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        ui.add_space(12.0);
        ui.vertical_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(16), Color32::from_rgb(0x24, 0x1d, 0x19));
            icons::flame(ui.painter(), r.shrink(10.0));
            ui.add_space(6.0);
            let (sep, _) = ui.allocate_exact_size(vec2(32.0, 2.0), Sense::hover());
            ui.painter().rect_filled(sep, CornerRadius::same(1), theme::LINE);
            ui.add_space(6.0);
            self.update_button(ui);
            for s in &st.servers {
                let selected = self.ui.server.as_deref() == Some(s.key.as_str());
                let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
                let hovered = resp.hovered();
                let radius = if selected || hovered { 16 } else { 24 };
                let fill = if selected {
                    theme::EMBER
                } else if hovered {
                    theme::ELEV2
                } else {
                    theme::ELEV
                };
                ui.painter().rect_filled(rect, CornerRadius::same(radius), fill);
                let initials: String = s
                    .name
                    .split_whitespace()
                    .filter_map(|w| w.chars().next())
                    .take(2)
                    .collect::<String>()
                    .to_uppercase();
                let tc = if selected { Color32::from_rgb(0x1b, 0x14, 0x11) } else { theme::TEXT };
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initials, theme::bold_font(15.0), tc);
                // pill indicator
                let unread = !s.unread.is_empty();
                let pill_h = if selected { 40.0 } else if hovered { 20.0 } else if unread { 8.0 } else { 0.0 };
                if pill_h > 0.0 {
                    let pill = Rect::from_center_size(pos2(ui.max_rect().left() + 2.0, rect.center().y), vec2(4.0, pill_h));
                    ui.painter().rect_filled(pill, CornerRadius::same(2), theme::TEXT);
                }
                let mentions: u32 = s.mentions.values().sum();
                if mentions > 0 {
                    let c = rect.right_bottom() - vec2(6.0, 6.0);
                    ui.painter().circle_filled(c, 9.0, theme::RAIL);
                    ui.painter().circle_filled(c, 7.5, theme::LIVE);
                    ui.painter().text(c, egui::Align2::CENTER_CENTER, mentions.min(99).to_string(), theme::bold_font(10.0), Color32::WHITE);
                }
                if !s.status.is_connected() {
                    let c = rect.right_top() + vec2(-6.0, 6.0);
                    let col = if matches!(s.status, ConnStatus::Failed(_)) { theme::LIVE } else { theme::DIM };
                    ui.painter().circle_filled(c, 6.0, theme::RAIL);
                    ui.painter().circle_filled(c, 4.0, col);
                }
                if resp.clicked() {
                    self.ui.server = Some(s.key.clone());
                    self.ui.call_view = st.call.as_ref().map(|c| c.server_key == s.key).unwrap_or(false) && self.ui.call_view;
                }
                let name = s.name.clone();
                let key = s.key.clone();
                let admin = s.can(perm::MANAGE_SERVER) || s.can(perm::MANAGE_ROLES);
                let resp = resp.on_hover_text(&name);
                resp.context_menu(|ui| {
                    if ui.button("Invite people").clicked() {
                        acts.push(Act::CreateInvite { key: key.clone() });
                        self.ui.dialog = Some(Dialog::Invite { key: key.clone() });
                        ui.close();
                    }
                    if admin && ui.button("Server settings").clicked() {
                        self.ui.dialog = Some(Dialog::ServerSettings { key: key.clone(), name: name.clone(), tab: 0 });
                        ui.close();
                    }
                    if ui.button("Reconnect").clicked() {
                        acts.push(Act::Retry(key.clone()));
                        ui.close();
                    }
                    ui.separator();
                    if ui.button(RichText::new("Leave server").color(theme::LIVE)).clicked() {
                        self.ui.dialog = Some(Dialog::LeaveServer { key: key.clone(), name: name.clone() });
                        ui.close();
                    }
                });
                ui.add_space(8.0);
            }
            let (rect, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
            let fill = if resp.hovered() { theme::EMBER } else { theme::ELEV };
            ui.painter().rect_filled(rect, CornerRadius::same(if resp.hovered() { 16 } else { 24 }), fill);
            icons::plus(ui.painter(), rect.shrink(12.0), if resp.hovered() { theme::RAIL } else { theme::EMBER }, 2.5);
            if resp.on_hover_text("Add a server").clicked() {
                self.ui.dialog = Some(Dialog::AddChoice);
            }
        });
    }

    // ----------------------------------------------------------- channel list

    fn channel_sidebar(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, acts: &mut Vec<Act>) {
        // Bottom: user panel, and the voice panel above it when in a call.
        egui::TopBottomPanel::bottom("user-panel")
            .exact_height(56.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::SIDE_DARK).inner_margin(egui::Margin::symmetric(8, 0)))
            .show_inside(ui, |ui| self.user_panel(ui, st, acts));
        if let Some(call) = st.call.as_ref() {
            egui::TopBottomPanel::bottom("voice-panel")
                .exact_height(86.0)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(theme::SIDE_DARK).inner_margin(egui::Margin::symmetric(10, 8)))
                .show_inside(ui, |ui| self.voice_panel(ui, st, call, acts));
        }
        // Header
        egui::TopBottomPanel::top("server-header")
            .exact_height(48.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::SIDE).inner_margin(egui::Margin::symmetric(16, 0)))
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let title = ui.add(egui::Label::new(RichText::new(&s.name).font(theme::bold_font(15.5)).color(theme::TEXT)).truncate().sense(Sense::click()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, 26.0, "Invite people", false, |p, r, c| icons::plus(p, r, c, 2.0)).clicked() {
                            acts.push(Act::CreateInvite { key: s.key.clone() });
                            self.ui.dialog = Some(Dialog::Invite { key: s.key.clone() });
                        }
                    });
                    if title.clicked() && (s.can(perm::MANAGE_SERVER) || s.can(perm::MANAGE_ROLES)) {
                        let tab = if s.can(perm::MANAGE_SERVER) { 0 } else { 1 };
                        self.ui.dialog = Some(Dialog::ServerSettings { key: s.key.clone(), name: s.name.clone(), tab });
                    }
                });
                let r = ui.max_rect();
                ui.painter().line_segment([pos2(r.left() - 16.0, r.bottom()), pos2(r.right() + 16.0, r.bottom())], Stroke::new(1.0f32, theme::SHADOW_LINE));
            });

        if let Some((msg, failed)) = short_status(&s.status) {
            egui::Frame::new().fill(if failed { Color32::from_rgb(0x4a, 0x1f, 0x1f) } else { theme::ELEV }).inner_margin(8).corner_radius(6).outer_margin(8).show(ui, |ui| {
                ui.label(RichText::new(msg).color(theme::TEXT).size(12.5));
                if ui.small_button("Retry now").clicked() {
                    acts.push(Act::Retry(s.key.clone()));
                }
            });
        }

        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(8.0);
            let current = self.ui.channel.get(&s.key).copied();
            for kind in [ChannelKind::Text, ChannelKind::Voice] {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    let label = if kind == ChannelKind::Text { "TEXT CHANNELS" } else { "VOICE CHANNELS" };
                    ui.label(RichText::new(label).font(theme::bold_font(11.5)).color(theme::DIM));
                    if s.can(perm::MANAGE_CHANNELS) {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.add_space(8.0);
                            if icon_button(ui, 20.0, "Create channel", false, |p, r, c| icons::plus(p, r, c, 1.8)).clicked() {
                                self.ui.dialog = Some(Dialog::CreateChannel { key: s.key.clone(), kind, name: String::new() });
                            }
                        });
                    }
                });
                for c in s.channels.iter().filter(|c| c.kind == kind) {
                    let is_current = current == Some(c.id)
                        && (kind == ChannelKind::Text || !self.ui.call_view || st.call.as_ref().map(|x| x.channel_id != c.id).unwrap_or(true));
                    let in_this_call = st.call.as_ref().map(|x| x.server_key == s.key && x.channel_id == c.id).unwrap_or(false);
                    let selected = (kind == ChannelKind::Text && is_current && !self.ui.call_view)
                        || (kind == ChannelKind::Voice && ((in_this_call && self.ui.call_view) || (is_current && !self.ui.call_view)));
                    let unread = s.unread.contains(&c.id);
                    let mentions = s.mentions.get(&c.id).copied().unwrap_or(0);
                    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 32.0), Sense::click());
                    let rect = rect.translate(vec2(8.0, 0.0)).with_max_x(rect.right());
                    let bg = if selected { theme::ELEV2 } else if resp.hovered() { theme::ELEV } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
                    let icon_rect = Rect::from_min_size(rect.left_top() + vec2(8.0, 7.0), vec2(18.0, 18.0));
                    let fg = if selected || unread || resp.hovered() { theme::TEXT } else { theme::MUTED };
                    if kind == ChannelKind::Text {
                        icons::hash(ui.painter(), icon_rect, theme::DIM);
                    } else {
                        icons::speaker(ui.painter(), icon_rect, if in_this_call { theme::ONLINE } else { theme::DIM });
                    }
                    let font = if unread { theme::bold_font(14.5) } else { theme::font(14.5) };
                    ui.painter().text(rect.left_center() + vec2(34.0, 0.0), egui::Align2::LEFT_CENTER, &c.name, font, fg);
                    if mentions > 0 {
                        let b = Rect::from_center_size(rect.right_center() - vec2(18.0, 0.0), vec2(22.0, 16.0));
                        ui.painter().rect_filled(b, CornerRadius::same(8), theme::LIVE);
                        ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, mentions.to_string(), theme::bold_font(11.0), Color32::WHITE);
                    } else if unread && kind == ChannelKind::Text {
                        let p = pos2(rect.left() - 6.0, rect.center().y);
                        ui.painter().rect_filled(Rect::from_center_size(p, vec2(4.0, 8.0)), CornerRadius::same(2), theme::TEXT);
                    }
                    // Voice channels: a chat bubble opens the channel's text chat without joining.
                    let mut chat_clicked = false;
                    if kind == ChannelKind::Voice && (resp.hovered() || unread) {
                        let cr = Rect::from_center_size(rect.right_center() - vec2(16.0, 0.0), vec2(22.0, 22.0));
                        let cresp = ui.interact(cr, ui.id().with(("chat", c.id)), Sense::click());
                        icons::chat(ui.painter(), cr.shrink(3.0), if cresp.hovered() || unread { theme::TEXT } else { theme::MUTED });
                        if cresp.on_hover_text("Open chat").clicked() {
                            chat_clicked = true;
                        }
                    }
                    if chat_clicked {
                        self.ui.channel.insert(s.key.clone(), c.id);
                        self.ui.call_view = false;
                    } else if resp.clicked() {
                        match kind {
                            ChannelKind::Text => {
                                self.ui.channel.insert(s.key.clone(), c.id);
                                self.ui.call_view = false;
                            }
                            ChannelKind::Voice => {
                                if !in_this_call {
                                    acts.push(Act::Join { key: s.key.clone(), channel: c.id });
                                }
                                self.ui.call_view = true;
                            }
                        }
                    }
                    let (key, cid, cname, ctopic) = (s.key.clone(), c.id, c.name.clone(), c.topic.clone());
                    if s.can(perm::MANAGE_CHANNELS) {
                        resp.context_menu(|ui| {
                            if ui.button("Edit channel").clicked() {
                                self.ui.dialog = Some(Dialog::EditChannel { key: key.clone(), channel: cid, name: cname.clone(), topic: ctopic.clone() });
                                ui.close();
                            }
                            if ui.button(RichText::new("Delete channel").color(theme::LIVE)).clicked() {
                                acts.push(Act::DeleteChannel { key: key.clone(), channel: cid });
                                ui.close();
                            }
                        });
                    }
                    // People in this voice channel
                    if kind == ChannelKind::Voice {
                        let speaking: HashSet<SessionId> = st.call.as_ref().map(|c| c.speaking.clone()).unwrap_or_default();
                        for p in s.voice_in(c.id) {
                            let name = s.member_name(&p.user_id);
                            let (r, presp) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 28.0), Sense::click());
                            if presp.hovered() {
                                ui.painter().rect_filled(r.translate(vec2(8.0, 0.0)).with_max_x(r.right()), CornerRadius::same(6), theme::HOVER);
                            }
                            let center = pos2(r.left() + 46.0, r.center().y);
                            let face = self.face(ui.ctx(), s, &p.user_id);
                            avatar(ui.painter(), center, 11.0, &p.user_id, &name, speaking.contains(&p.session), theme::SIDE, face);
                            let col = if speaking.contains(&p.session) { theme::TEXT } else { theme::MUTED };
                            ui.painter().text(pos2(r.left() + 64.0, r.center().y), egui::Align2::LEFT_CENTER, &name, theme::font(13.5), col);
                            let mut x = r.right() - 10.0;
                            if p.tracks.iter().any(|t| t.id == ingle_proto::media::track::SCREEN) {
                                let b = Rect::from_min_max(pos2(x - 34.0, r.center().y - 8.0), pos2(x, r.center().y + 8.0));
                                ui.painter().rect_filled(b, CornerRadius::same(4), theme::LIVE);
                                ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, "LIVE", theme::bold_font(10.0), Color32::WHITE);
                                x -= 40.0;
                            }
                            if p.tracks.iter().any(|t| t.id == ingle_proto::media::track::CAMERA) {
                                icons::camera(ui.painter(), Rect::from_center_size(pos2(x - 8.0, r.center().y), vec2(16.0, 16.0)), theme::DIM, false);
                                x -= 20.0;
                            }
                            if p.self_deaf {
                                icons::headphones(ui.painter(), Rect::from_center_size(pos2(x - 8.0, r.center().y), vec2(16.0, 16.0)), theme::DIM, true);
                            } else if p.self_mute {
                                icons::mic(ui.painter(), Rect::from_center_size(pos2(x - 8.0, r.center().y), vec2(16.0, 16.0)), theme::DIM, true);
                            }
                            if p.user_id != st.my_uid {
                                let uid = p.user_id.clone();
                                let vol = st.user_volumes.get(&uid).copied().unwrap_or(1.0);
                                let admin_can = s.can(perm::KICK) && s.rank(&uid) < s.rank(&st.my_uid);
                                let key = s.key.clone();
                                presp.context_menu(|ui| {
                                    ui.label(RichText::new(format!("{name} volume")).color(theme::MUTED).size(12.0));
                                    let mut v = vol * 100.0;
                                    if ui.add(egui::Slider::new(&mut v, 0.0..=200.0).suffix("%")).changed() {
                                        acts.push(Act::Volume { uid: uid.clone(), volume: v / 100.0 });
                                    }
                                    if admin_can {
                                        ui.separator();
                                        if ui.button(RichText::new(format!("Kick {name}")).color(theme::LIVE)).clicked() {
                                            acts.push(Act::Kick { key: key.clone(), uid: uid.clone(), ban: false });
                                            ui.close();
                                        }
                                    }
                                });
                            }
                        }
                    }
                }
                ui.add_space(10.0);
            }
        });
    }

    fn voice_panel(&mut self, ui: &mut egui::Ui, st: &AppState, call: &ingle_engine::state::CallView, acts: &mut Vec<Act>) {
        let server = st.server(&call.server_key);
        let channel_name = server.and_then(|s| s.channel(call.channel_id)).map(|c| c.name.clone()).unwrap_or_default();
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            icons::signal(ui.painter(), r, theme::ONLINE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                let label = ui.add(egui::Label::new(RichText::new("Voice Connected").font(theme::bold_font(13.5)).color(theme::ONLINE)).sense(Sense::click()));
                let srv = server.map(|s| s.name.clone()).unwrap_or_default();
                ui.add(egui::Label::new(RichText::new(format!("{channel_name} · {} ms", call.stats.rtt_ms)).size(12.0).color(theme::MUTED)).truncate())
                    .on_hover_text(format!("{channel_name} / {srv}"));
                if label.clicked() {
                    self.ui.server = Some(call.server_key.clone());
                    self.ui.call_view = true;
                }
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, 32.0, "Disconnect", false, |p, r, _| icons::hangup(p, r, theme::TEXT)).clicked() {
                    acts.push(Act::Leave);
                }
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 16.0) / 3.0;
            let btn = |ui: &mut egui::Ui, active: bool, tip: &str, draw: &dyn Fn(&egui::Painter, Rect, Color32)| {
                let (rect, resp) = ui.allocate_exact_size(vec2(w, 30.0), Sense::click());
                let bg = if active { Color32::from_rgb(0x3a, 0x2a, 0x20) } else if resp.hovered() { theme::ELEV2 } else { theme::ELEV };
                ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
                draw(ui.painter(), Rect::from_center_size(rect.center(), vec2(18.0, 18.0)), if active { theme::EMBER2 } else { theme::TEXT });
                resp.on_hover_text(tip)
            };
            if btn(ui, call.camera_on, if call.camera_on { "Turn off camera" } else { "Turn on camera" }, &|p, r, c| icons::camera(p, r, c, false)).clicked() {
                acts.push(Act::Camera(!call.camera_on));
            }
            if btn(ui, call.screen_on, if call.screen_on { "Stop sharing" } else { "Share your screen" }, &|p, r, c| icons::screen(p, r, c)).clicked() {
                if call.screen_on {
                    acts.push(Act::StopScreen);
                } else {
                    acts.push(Act::RefreshDevices);
                    self.ui.dialog = Some(Dialog::ScreenPicker { selected: None });
                }
            }
            let code = call.call_code.clone();
            let tip = if call.e2ee_ready {
                format!("End-to-end encrypted.\nCall code: {code}\nIf everyone sees the same code, nobody is listening in.")
            } else {
                "Exchanging encryption keys…".to_string()
            };
            if btn(ui, false, &tip, &|p, r, c| icons::lock(p, r, if c == theme::TEXT { theme::ONLINE } else { c })).clicked() {
                acts.push(Act::Copy(code));
            }
        });
    }

    fn user_panel(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        let (muted, deafened) = self.engine.voice_prefs();
        ui.horizontal_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
            let speaking = st.call.as_ref().map(|c| c.speaking.contains(&c.session)).unwrap_or(false);
            let face = self.my_face(ui.ctx());
            avatar(ui.painter(), r.center(), 16.0, &st.my_uid, &st.my_name, speaking, theme::SIDE_DARK, face);
            status_dot(ui.painter(), r.right_bottom() - vec2(3.0, 3.0), true, theme::SIDE_DARK);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add_space(9.0);
                ui.label(RichText::new(&st.my_name).font(theme::bold_font(13.5)).color(theme::TEXT));
                let sub = if st.call.is_some() { "In voice" } else { "Online" };
                ui.label(RichText::new(sub).size(11.5).color(theme::MUTED));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                if icon_button(ui, 32.0, "User settings", false, |p, r, c| icons::gear(p, r, c)).clicked() {
                    self.ui.dialog = Some(Dialog::Settings { tab: 1 });
                    self.ui.settings_draft = Some(st.settings.clone());
                    acts.push(Act::RefreshDevices);
                }
                let deaf_tip = if deafened { "Undeafen" } else { "Deafen" };
                if icon_button(ui, 32.0, deaf_tip, false, |p, r, c| icons::headphones(p, r, if deafened { theme::LIVE } else { c }, deafened)).clicked() {
                    acts.push(Act::Deafen(!deafened));
                }
                let mute_tip = if muted { "Unmute" } else { "Mute" };
                if icon_button(ui, 32.0, mute_tip, false, |p, r, c| icons::mic(p, r, if muted { theme::LIVE } else { c }, muted)).clicked() {
                    acts.push(Act::Mute(!muted));
                }
            });
        });
    }

    // ------------------------------------------------------------ member list

    fn member_list(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, acts: &mut Vec<Act>) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(14.0);
            let mut members: Vec<_> = s.members.values().collect();
            members.sort_by(|a, b| s.rank(&b.user_id).cmp(&s.rank(&a.user_id)).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
            // Like Discord: online people with a "shown separately" role are grouped under
            // their highest such role, other online people under Online, then Offline.
            let group_of = |m: &ingle_proto::control::Member| -> Option<RoleId> {
                if !m.online {
                    return None;
                }
                s.roles_of(&m.user_id).into_iter().find(|r| r.hoist).map(|r| r.id)
            };
            let mut groups: Vec<(String, Vec<&ingle_proto::control::Member>, bool)> = Vec::new();
            for r in s.roles.iter().filter(|r| r.hoist) {
                let g: Vec<_> = members.iter().copied().filter(|m| group_of(m) == Some(r.id)).collect();
                if !g.is_empty() {
                    groups.push((r.name.to_uppercase(), g, true));
                }
            }
            let online: Vec<_> = members.iter().copied().filter(|m| m.online && group_of(m).is_none()).collect();
            if !online.is_empty() {
                groups.push(("ONLINE".into(), online, true));
            }
            let offline: Vec<_> = members.iter().copied().filter(|m| !m.online).collect();
            if !offline.is_empty() {
                groups.push(("OFFLINE".into(), offline, false));
            }
            for (label, group, online) in groups {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.label(RichText::new(format!("{label} — {}", group.len())).font(theme::bold_font(11.5)).color(theme::DIM));
                });
                for m in group {
                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 8.0, 42.0), Sense::click());
                    let r = r.translate(vec2(8.0, 0.0)).with_max_x(r.right());
                    if resp.hovered() {
                        ui.painter().rect_filled(r, CornerRadius::same(6), theme::HOVER);
                    }
                    let alpha = if online { 1.0 } else { 0.45 };
                    let c = pos2(r.left() + 22.0, r.center().y);
                    let face = self.face(ui.ctx(), s, &m.user_id);
                    avatar(ui.painter(), c, 16.0, &m.user_id, &m.name, false, theme::SIDE, face);
                    if online {
                        status_dot(ui.painter(), c + vec2(12.0, 12.0), true, theme::SIDE);
                    }
                    ui.painter().text(
                        pos2(r.left() + 48.0, r.center().y),
                        egui::Align2::LEFT_CENTER,
                        &m.name,
                        theme::bold_font(14.0),
                        name_color(s, &m.user_id, m.role).gamma_multiply(alpha),
                    );
                    if m.role == Role::Owner {
                        let w = ui.fonts_mut(|f| f.layout_no_wrap(m.name.clone(), theme::bold_font(14.0), Color32::WHITE).size().x);
                        icons::crown(ui.painter(), Rect::from_center_size(pos2(r.left() + 58.0 + w, r.center().y), vec2(14.0, 14.0)), theme::OWNER);
                    }
                    let (uid, name, role) = (m.user_id.clone(), m.name.clone(), m.role);
                    let me = st.my_uid == uid;
                    let my_role = s.me.as_ref().map(|x| x.role).unwrap_or(Role::Member);
                    let below_me = s.rank(&uid) < s.rank(&st.my_uid);
                    let key = s.key.clone();
                    let their_roles: Vec<String> = s.roles_of(&uid).iter().map(|r| r.name.clone()).collect();
                    let mut hover = format!("{name}\nSafety code: {}", ingle_proto::crypto::safety_code(&uid));
                    if !their_roles.is_empty() {
                        hover.push_str(&format!("\nRoles: {}", their_roles.join(", ")));
                    }
                    resp.on_hover_text(hover).context_menu(|ui| {
                        if ui.button("Copy safety code").clicked() {
                            acts.push(Act::Copy(ingle_proto::crypto::safety_code(&uid)));
                            ui.close();
                        }
                        // Roles you can hand out (below your own highest), for people below you
                        // (or yourself).
                        let manageable: Vec<_> = s.roles.iter().filter(|r| s.can_manage_role(r)).collect();
                        if !manageable.is_empty() && (me || below_me || s.is_admin()) && !(role == Role::Owner && my_role != Role::Owner) {
                            ui.menu_button("Roles", |ui| {
                                let current: Vec<RoleId> = s.members.get(&uid).map(|m| m.roles.clone()).unwrap_or_default();
                                for r in &manageable {
                                    let mut on = current.contains(&r.id);
                                    let col = if r.color != 0 { rgb(r.color) } else { theme::TEXT };
                                    if ui.checkbox(&mut on, RichText::new(&r.name).color(col)).changed() {
                                        let mut next = current.clone();
                                        if on {
                                            next.push(r.id);
                                        } else {
                                            next.retain(|x| *x != r.id);
                                        }
                                        acts.push(Act::SetMemberRoles { key: key.clone(), uid: uid.clone(), roles: next });
                                    }
                                }
                            });
                        }
                        if !me && my_role == Role::Owner && role != Role::Owner {
                            let (label, new_role) = if role == Role::Admin { ("Remove admin", Role::Member) } else { ("Make admin", Role::Admin) };
                            if ui.button(label).clicked() {
                                acts.push(Act::SetRole { key: key.clone(), uid: uid.clone(), role: new_role });
                                ui.close();
                            }
                        }
                        if !me && below_me && (s.can(perm::KICK) || s.can(perm::BAN)) {
                            ui.separator();
                            if s.can(perm::KICK) && ui.button(RichText::new(format!("Kick {name}")).color(theme::LIVE)).clicked() {
                                acts.push(Act::Kick { key: key.clone(), uid: uid.clone(), ban: false });
                                ui.close();
                            }
                            if s.can(perm::BAN) && ui.button(RichText::new(format!("Ban {name}")).color(theme::LIVE)).clicked() {
                                acts.push(Act::Kick { key: key.clone(), uid: uid.clone(), ban: true });
                                ui.close();
                            }
                        }
                    });
                }
                ui.add_space(12.0);
            }
        });
    }

    // ---------------------------------------------------------------- welcome

    fn welcome(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.18).max(30.0));
            let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(22), Color32::from_rgb(0x24, 0x1d, 0x19));
            icons::flame(ui.painter(), r.shrink(14.0));
            ui.add_space(14.0);
            ui.label(RichText::new(format!("Welcome to {}", ingle_proto::APP_NAME)).font(theme::bold_font(28.0)).color(theme::TEXT));
            ui.add_space(6.0);
            ui.label(
                RichText::new("Voice, video, screen share and chat — end-to-end encrypted, no accounts, no ads.")
                    .size(15.0)
                    .color(theme::MUTED),
            );
            ui.add_space(26.0);
            ui.label(RichText::new("What should people call you?").color(theme::MUTED));
            ui.add_space(4.0);
            if self.ui.welcome_name.is_empty() {
                self.ui.welcome_name = st.my_name.clone();
            }
            let resp = ui.add(egui::TextEdit::singleline(&mut self.ui.welcome_name).desired_width(280.0).margin(egui::Margin::symmetric(10, 8)));
            if resp.lost_focus() && self.ui.welcome_name.trim() != st.my_name {
                acts.push(Act::SetName(self.ui.welcome_name.trim().to_string()));
            }
            ui.add_space(24.0);
            let name_changed = self.ui.welcome_name.trim() != st.my_name && !self.ui.welcome_name.trim().is_empty();
            // Two buttons side by side, centred.
            let w = 220.0 * 2.0 + 12.0;
            ui.allocate_ui_with_layout(vec2(w, 44.0), Layout::left_to_right(Align::Center), |ui| {
                if ui.add(theme::primary_button("Create a server").min_size(vec2(220.0, 42.0))).on_hover_text("Rent one for a few dollars a month, or use a server you have").clicked() {
                    if name_changed {
                        acts.push(Act::SetName(self.ui.welcome_name.trim().to_string()));
                    }
                    let name = if name_changed { self.ui.welcome_name.trim().to_string() } else { st.my_name.clone() };
                    self.ui.create = Some(crate::create::CreateState::new(&name));
                    self.ui.dialog = Some(Dialog::Create);
                }
                ui.add_space(4.0);
                if ui
                    .add(egui::Button::new(RichText::new("Join a server").font(theme::bold_font(14.0)).color(theme::TEXT)).fill(theme::ELEV2).min_size(vec2(220.0, 42.0)))
                    .on_hover_text("Paste an invite link a friend sent you")
                    .clicked()
                {
                    if name_changed {
                        acts.push(Act::SetName(self.ui.welcome_name.trim().to_string()));
                    }
                    self.ui.dialog = Some(Dialog::AddServer { link: String::new(), error: None });
                }
            });
            ui.add_space(18.0);
            ui.label(
                RichText::new("A server is where your group's channels live. Create one for your friends, or join one with an invite link.")
                    .size(13.0)
                    .color(theme::DIM),
            );
            ui.add_space(12.0);
            ui.label(RichText::new(format!("Your safety code: {}", st.safety_code)).size(12.0).color(theme::DIM));
        });
    }

    // ----------------------------------------------------------------- toasts

    fn toasts(&mut self, ctx: &egui::Context, st: &AppState) {
        let now = Instant::now();
        let visible: Vec<_> = st.toasts.iter().filter(|t| now.duration_since(t.at) < Duration::from_secs(6)).collect();
        if visible.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-20.0, -90.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                for t in visible.iter().rev().take(3) {
                    egui::Frame::new()
                        .fill(if t.error { Color32::from_rgb(0x4a, 0x1f, 0x1f) } else { theme::ELEV2 })
                        .stroke(Stroke::new(1.0f32, if t.error { theme::LIVE } else { theme::LINE }))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_max_width(340.0);
                            ui.label(RichText::new(&t.text).color(theme::TEXT));
                        });
                    ui.add_space(6.0);
                }
            });
        let _ = self.started;
    }
}
