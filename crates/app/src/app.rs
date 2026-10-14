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
    /// An invite link from outside the app (a web page, a launch, a link in a message): ask
    /// first, unless it's a server already joined.
    OfferJoin(String),
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
    /// Not looking at any channel (the friends list).
    ClearActive,
    /// Mute or unmute a server (no channel) or a channel / conversation.
    MuteChat { key: String, channel: Option<ChannelId>, muted: bool },
    /// Set (or remove) a server's icon from part of an image: (bytes, cx, cy, frac).
    SetServerIcon { key: String, bytes: Option<(Arc<Vec<u8>>, f32, f32, f32)> },
    AddEmoji { key: String, name: String, bytes: Vec<u8> },
    ListBugs(String),
    SetBugStatus { key: String, id: i64, status: ingle_proto::control::BugStatus },
    SetBugReports { key: String, on: bool },
    /// Send a bug report; `turn_on`: the bug server is mine, so switch collecting on there first.
    ReportBug { title: String, body: String, include_log: bool, turn_on: Option<String> },
    /// The server's GIPHY key (owner only; "" turns GIF search off).
    SetGiphyKey { key: String, giphy: String },
    /// The server's KLIPY app key (owner only; "" turns KLIPY off).
    SetKlipyKey { key: String, klipy: String },
    /// Quit for real (Settings → Quit Ingle).
    Quit,
    AddFriend(String),
    AcceptFriend(String),
    RemoveFriend(String),
    Block(String),
    Unblock(String),
    RemoveEmoji { key: String, id: String },
    RenameEmoji { key: String, id: String, name: String },
    /// Save part of an image as my profile picture (see `make_avatar_crop`).
    SetAvatarCropped { bytes: Arc<Vec<u8>>, cx: f32, cy: f32, frac: f32 },
    /// Play the message sound at this volume (settings).
    TestSound(f32),
    /// Open the direct message conversation with someone on this server.
    OpenDm { key: String, uid: String },
    Typing { key: String, channel: ChannelId },
    CreateChannel { key: String, name: String, kind: ChannelKind },
    UpdateChannel { key: String, channel: ChannelId, name: Option<String>, topic: Option<String> },
    DeleteChannel { key: String, channel: ChannelId },
    CreateInvite { key: String },
    Kick { key: String, uid: String, ban: bool },
    ServerMute { key: String, uid: String, muted: bool },
    ServerDeafen { key: String, uid: String, deafened: bool },
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
    /// Play a soundboard sound for everyone in the call (see `soundboard.rs`).
    PlaySound(ingle_engine::config::SoundRef),
    /// Hear a soundboard sound yourself, nobody else.
    PreviewSound(ingle_engine::config::SoundRef),
    /// Stop every soundboard sound playing here.
    StopSounds,
    /// Add part (`start` to `end` seconds) of a picked file to a server's soundboard.
    AddSound { key: String, name: String, emoji: String, volume: f32, pcm: Arc<Vec<f32>>, start: f32, end: f32 },
    EditSound { key: String, id: String, name: String, emoji: String, volume: f32 },
    RemoveSound { key: String, id: String },
    /// Don't play someone's soundboard sounds here (or do again).
    MuteSoundboard { uid: String, muted: bool },
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
    /// "Join this server?" for a link that came from outside the app.
    ConfirmJoin { link: String },
    /// Create or join?
    AddChoice,
    /// The create-a-server wizard (its state is in `UiState::create`).
    Create,
    CreateChannel { key: String, kind: ChannelKind, name: String },
    EditChannel { key: String, channel: ChannelId, name: String, topic: String },
    Invite { key: String },
    /// Tab 0: overview; 1: roles; 2: emoji; 3: backups; 4: soundboard.
    ServerSettings { key: String, name: String, tab: u8 },
    LeaveServer { key: String, name: String },
    /// "Delete message?" (see `chat.rs`).
    DeleteMessage { key: String, channel: ChannelId, id: MessageId },
    Settings { tab: u8 },
    ScreenPicker { selected: Option<String> },
    /// Report a bug in Ingle (see the engine's `bugs.rs`).
    ReportBug { title: String, body: String, include_log: bool },
    /// The owner's inbox of Ingle bug reports for a server; `filter`: 0 new, 1 approved,
    /// 2 fixed, 3 rejected.
    BugReports { key: String, filter: u8 },
    /// Add or edit a soundboard sound (its state is in `UiState::sb`).
    SoundEditor,
    /// "Delete this sound?" (`from_settings`: go back to Server Settings after).
    RemoveSound { key: String, id: String, name: String, from_settings: bool },
}

/// What the microphone buttons say when a moderator has server muted you.
pub(crate) const SERVER_MUTED: &str = "A moderator server muted you here. Only a moderator can unmute you.";
/// And the deafen buttons, server deafened.
pub(crate) const SERVER_DEAFENED: &str = "A moderator server deafened you here. Only a moderator can undeafen you.";

#[derive(Default)]
pub struct UiState {
    pub server: Option<String>,
    /// Someone's profile card, from clicking their name in a voice channel: (server, user, the
    /// name's row, which the card opens beside).
    pub profile_card: Option<(String, String, Rect)>,
    pub channel: HashMap<String, ChannelId>,
    pub call_view: bool,
    pub drafts: HashMap<(String, ChannelId), String>,
    pub reply: Option<(MessageId, String)>,
    pub editing: Option<(MessageId, String)>,
    pub members_open: bool,
    pub call_chat_open: bool,
    pub focus: Option<VideoKey>,
    /// With nothing picked, the big tile follows whoever talks (like Discord): who it is now,
    /// and since when. `grid_view` turns that off (everyone the same size).
    pub speaker_focus: Option<(VideoKey, Instant)>,
    pub grid_view: bool,
    /// Volumes from before "Mute" in a tile's menu, to go back to on "Unmute".
    pub premute_volume: HashMap<(String, bool), f32>,
    /// A stream filling the whole screen, and whether the window is in full screen for it.
    pub fullscreen: Option<VideoKey>,
    pub fullscreen_on: bool,
    /// A click on a video tile, acted on once it's clearly not a double-click.
    pub pending_click: Option<(VideoKey, Instant)>,
    /// The share picker's live preview: what it shows, and its picture (with when it came).
    pub preview_source: Option<String>,
    pub preview_tex: Option<(egui::TextureHandle, Instant)>,
    pub dialog: Option<Dialog>,
    /// Which dialog (and wizard page) was showing last frame, so a dialog that just opened, or
    /// the wizard's next page, starts at its top.
    pub dialog_shown: Option<(std::mem::Discriminant<Dialog>, u8)>,
    pub react_picker: Option<MessageId>,
    pub composer_emoji: bool,
    /// The GIF picker over the message box (the GIF button, or typing /giphy), its search
    /// words, and whether its search box should take the cursor.
    pub gif_picker: bool,
    pub gif_search: String,
    pub gif_focus: bool,
    /// Error notices closed with Dismiss, and the one just copied (for "Copied").
    pub toasts_dismissed: HashSet<u64>,
    /// A bug report's text from when Send was clicked until it has gone (a report that
    /// doesn't go is never lost), and the last finished send acted on.
    pub bug_draft: Option<(String, String, bool)>,
    pub bug_seen: u64,
    pub toast_copied: Option<(u64, Instant)>,
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
    /// Where each conversation is scrolled to, kept steady while things load (see `chat.rs`).
    pub chat_scroll: HashMap<(String, ChannelId), crate::chat::ChatScroll>,
    /// While drawing a conversation: pictures load only inside this (near the screen); `None`
    /// elsewhere, where they always load.
    pub load_rect: Option<Rect>,
    /// How big GIFs were, so their space is kept before they load.
    pub gif_sizes: crate::giphy::GifSizes,
    /// GIFs near the screen this frame whose size isn't known until they load.
    pub gifs_waiting: usize,
    pub role_edit: Option<RoleEdit>,
    /// Ctrl+V was down last frame (to catch an image paste once).
    pub paste_down: bool,
    /// The home screen (friends and direct messages) instead of a server.
    pub home: bool,
    /// The direct message conversation open on the home screen: (server, conversation).
    pub dm: Option<(String, ChannelId)>,
    /// Friends list shows everyone, not just who's online.

    /// The (server, channel) last reported to the engine as being looked at.
    pub active_view: Option<(String, ChannelId)>,
    /// Highlighted row in the @mention list.
    pub mention_sel: usize,
    /// A new profile picture being framed (zoom and drag) before it's saved.
    pub avatar_crop: Option<crate::avatar_edit::AvatarCrop>,
    /// The emoji picker's search and scroll.
    pub picker: crate::emoji_picker::PickerState,
    /// The GIF search panel.
    pub giphy: crate::giphy::GiphyUi,
    /// The KLIPY tab's search, and which tab the GIF picker is on.
    pub klipy: crate::giphy::GiphyUi,
    pub gif_provider: crate::giphy::Provider,
    /// GIFs from GIPHY being shown.
    pub gifs: crate::giphy::Downloads,
    /// Folded runs of blocked messages that were opened (by their first message).
    pub shown_blocked: std::collections::HashSet<MessageId>,
    /// Friends page tab: 0 online, 1 all, 2 pending, 3 blocked, 4 add friend.
    pub friends_tab: u8,
    pub friend_search: String,
    /// Emoji names being edited in server settings, by emoji id.
    pub emoji_names: HashMap<String, String>,
    /// Spellcheck for the message box.
    pub spell: crate::spell::Speller,
    /// What the message box's right-click menu was opened on.
    pub composer_menu: Option<crate::chat::ComposerMenu>,
    /// The soundboard: its picker over the call buttons and the add-a-sound dialog.
    pub sb: crate::soundboard::SoundboardUi,
    /// A dialog to open once this frame's dialog is done with (one dialog opening another).
    pub next_dialog: Option<Dialog>,
    /// Text selected in a conversation, and the messages' right-click menu.
    pub sel: crate::select::Selection,
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
    /// Decoded images and profile pictures.
    pub images: crate::images::Images,
    pub updater: crate::update::Updater,
    /// The update we last told the person about (pinged once per version).
    update_seen: Option<ingle_proto::release::Manifest>,
    /// After an update: where to put the person back, and since when we've been trying.
    resume: Option<(crate::update::Resume, Instant)>,
    /// Join / leave / stream chimes.
    voice_sounds: crate::sounds::VoiceSounds,
    /// The window's title, and what it shows now (with what's unread, see `unread::Unread`).
    title: String,
    title_shown: String,
}

impl IngleApp {
    pub fn new(
        engine: Arc<Engine>,
        handoff: crate::instance::Handoff,
        initial_join: Option<String>,
        tray: Option<crate::tray::Tray>,
        updater: crate::update::Updater,
        title: String,
    ) -> IngleApp {
        engine.refresh_devices();
        let resume = crate::update::take_resume(&engine.shared.config_dir).map(|r| (r, Instant::now()));
        let gif_sizes = crate::giphy::GifSizes::load(&engine.shared.config_dir);
        let ui = UiState { members_open: true, call_chat_open: false, gif_sizes, ..Default::default() };
        IngleApp {
            engine,
            ui,
            textures: HashMap::new(),
            fps: HashMap::new(),
            started: Instant::now(),
            handoff,
            initial_join,
            _tray: tray,
            images: crate::images::Images::default(),
            updater,
            update_seen: None,
            resume,
            voice_sounds: Default::default(),
            title_shown: title.clone(),
            title,
        }
    }

    fn apply(&mut self, a: Act, ctx: &egui::Context) {
        let e = &self.engine;
        match a {
            Act::OfferJoin(link) => {
                let known = ingle_proto::invite::InviteLink::parse(&link).ok().and_then(|l| {
                    e.state().servers.iter().find(|s| {
                        ingle_proto::b64::decode_array::<32>(&s.key).map(|fp| l.matches(&fp)).unwrap_or(false)
                    }).map(|s| s.key.clone())
                });
                match known {
                    // Already a member there: just open it.
                    Some(key) => {
                        self.ui.home = false;
                        self.ui.server = Some(key);
                    }
                    None => self.ui.dialog = Some(Dialog::ConfirmJoin { link }),
                }
            }
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
            Act::ClearActive => {
                e.clear_active_channel();
                self.ui.active_view = None;
            }
            Act::OpenDm { key, uid } => e.open_dm(&key, &uid),
            Act::MuteChat { key, channel, muted } => e.set_muted(&key, channel, muted),
            Act::TestSound(v) => {
                let mut s = e.state().settings.clone();
                s.sounds = true;
                s.sound_volume = v;
                e.play_sound_with(ingle_engine::sfx::Sound::Message, &s);
            }
            Act::Typing { key, channel } => e.typing(&key, channel),
            Act::CreateChannel { key, name, kind } => e.create_channel(&key, &name, kind),
            Act::UpdateChannel { key, channel, name, topic } => e.update_channel(&key, channel, name, topic),
            Act::DeleteChannel { key, channel } => e.delete_channel(&key, channel),
            Act::CreateInvite { key } => e.create_invite(&key, None, None),
            Act::Kick { key, uid, ban } => e.kick(&key, &uid, ban),
            Act::ServerMute { key, uid, muted } => e.server_mute(&key, &uid, muted),
            Act::ServerDeafen { key, uid, deafened } => e.server_deafen(&key, &uid, deafened),
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
            Act::Settings(s) => {
                if e.shared.config_dir == ingle_engine::config::config_dir(None) {
                    crate::autostart::apply(s.start_with_windows);
                }
                e.update_settings(s)
            }
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
            Act::SetAvatarCropped { bytes, cx, cy, frac } => match crate::images::make_avatar_crop(&bytes, cx, cy, frac) {
                Ok(png) => e.set_avatar(Some(png)),
                Err(err) => self.toast(err, true),
            },
            Act::AddEmoji { key, name, bytes } => match crate::images::make_emoji(&bytes) {
                Ok((b, animated)) => e.add_emoji(&key, &name, b, animated),
                Err(err) => self.toast(err, true),
            },
            Act::RemoveEmoji { key, id } => e.remove_emoji(&key, &id),
            Act::Quit => {
                #[cfg(windows)]
                crate::tray::quit();
                #[cfg(not(windows))]
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            Act::AddFriend(uid) => e.add_friend(&uid),
            Act::AcceptFriend(uid) => e.accept_friend(&uid),
            Act::RemoveFriend(uid) => e.remove_friend(&uid),
            Act::Block(uid) => e.block(&uid),
            Act::Unblock(uid) => e.unblock(&uid),
            Act::RenameEmoji { key, id, name } => e.rename_emoji(&key, &id, &name),
            Act::SetGiphyKey { key, giphy } => e.set_giphy_key(&key, &giphy),
            Act::SetKlipyKey { key, klipy } => e.set_klipy_key(&key, &klipy),
            Act::ListBugs(key) => e.list_bugs(&key),
            Act::SetBugStatus { key, id, status } => e.set_bug_status(&key, id, status),
            Act::SetBugReports { key, on } => e.set_bug_reports(&key, on),
            Act::ReportBug { title, body, include_log, turn_on } => e.report_bug(&title, &body, include_log, turn_on.as_deref()),
            Act::SetServerIcon { key, bytes } => match bytes {
                None => e.set_server_icon(&key, None),
                Some((b, cx, cy, frac)) => match crate::images::make_avatar_crop(&b, cx, cy, frac) {
                    Ok(png) => e.set_server_icon(&key, Some(png)),
                    Err(err) => self.toast(err, true),
                },
            },
            Act::SetAvatar(bytes) => match bytes.map(|b| crate::images::make_avatar(&b)).transpose() {
                Ok(png) => e.set_avatar(png),
                Err(err) => self.toast(err, true),
            },
            Act::SaveImage { name, bytes } => {
                // (Between frames: nothing holds the shared state while the dialog is open.)
                if let Err(err) = crate::images::save_as(&name, &bytes) {
                    self.toast(format!("Couldn't save the image: {err}"), true);
                }
                ctx.input_mut(|i| i.keys_down.clear());
            }
            Act::Toast(text, error) => self.toast(text, error),
            Act::PlaySound(sound) => e.play_soundboard(sound),
            Act::PreviewSound(sound) => e.preview_sound(sound),
            Act::StopSounds => e.stop_sounds(),
            Act::AddSound { key, name, emoji, volume, pcm, start, end } => e.add_sound_from(&key, &name, &emoji, volume, pcm, start, end),
            Act::EditSound { key, id, name, emoji, volume } => e.edit_sound(&key, &id, Some(name), Some(emoji), Some(volume)),
            Act::RemoveSound { key, id } => e.remove_sound(&key, &id),
            Act::MuteSoundboard { uid, muted } => {
                let mut s = e.state().settings.clone();
                s.soundboard_muted.retain(|u| *u != uid);
                if muted {
                    s.soundboard_muted.push(uid);
                }
                e.update_settings(s);
            }
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

    /// A server emoji's picture (animated ones play), once downloaded.
    pub fn custom_emoji_tex(&mut self, ctx: &egui::Context, key: &str, id: &str) -> Option<egui::TextureId> {
        let bytes = match self.engine.file(key, id) {
            ingle_engine::files::FileState::Ready(b) => Some(b),
            _ => None,
        };
        let t = ctx.input(|i| i.time);
        let (tex, _, next) = self.images.get(ctx, &format!("emoji:{id}"), bytes, 128).frame(t)?;
        if let Some(n) = next {
            ctx.request_repaint_after(n);
        }
        Some(tex)
    }

    /// A server's icon, once downloaded.
    pub fn server_icon(&mut self, ctx: &egui::Context, s: &ServerView) -> Option<egui::TextureId> {
        let id = s.info.icon.clone()?;
        let bytes = match self.engine.file(&s.key, &id) {
            ingle_engine::files::FileState::Ready(b) => Some(b),
            _ => None,
        };
        self.images.get(ctx, &format!("icon:{id}"), bytes, 256).frame(0.0).map(|f| f.0)
    }

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
        crate::watchdog::step("starting a frame");
        let engine = self.engine.clone();
        let mut acts: Vec<Act> = Vec::new();
        for e in ingle_engine::errors::take() {
            engine.shared.state.write().toast(e, true);
        }
        // The share picker closed, or the window was minimized with it open: stop previewing
        // (a minimized picker closes: nothing keeps capturing your screen out of sight).
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        if minimized && matches!(self.ui.dialog, Some(Dialog::ScreenPicker { .. })) {
            self.ui.dialog = None;
        }
        if self.ui.preview_source.is_some() && !matches!(self.ui.dialog, Some(Dialog::ScreenPicker { .. })) {
            self.ui.preview_source = None;
            self.ui.preview_tex = None;
            engine.preview_screen(None);
        }
        if let Some(link) = self.initial_join.take() {
            acts.push(Act::OfferJoin(link));
        }
        // Another launch handed us an invite link (or just wants the window): come to the front.
        if let Some(link) = self.handoff.poll() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            if !link.is_empty() {
                acts.push(Act::OfferJoin(link));
            }
        }
        // The close button (and Alt+F4) never quits while there's an icon by the clock: Ingle
        // keeps running there, like Discord, until you choose Quit Ingle on the icon. The first
        // time, the icon says so.
        if ctx.input(|i| i.viewport().close_requested()) && crate::tray::available() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if crate::tray::ensure_icon() {
                // Off to the tray: the share picker closes, and its preview stops now (a hidden
                // window may not draw again until it's back).
                if matches!(self.ui.dialog, Some(Dialog::ScreenPicker { .. })) {
                    self.ui.dialog = None;
                }
                if self.ui.preview_source.take().is_some() {
                    self.ui.preview_tex = None;
                    engine.preview_screen(None);
                }
                crate::tray::hide_main();
                let settings = engine.state().settings.clone();
                if !settings.tray_note_seen {
                    crate::tray::notify(
                        "Ingle is still running",
                        "Click the campfire icon to open Ingle. To quit, right-click it and choose Quit Ingle.",
                    );
                    acts.push(Act::Settings(ingle_engine::config::Settings { tray_note_seen: true, ..settings }));
                }
            } else {
                // No icon to come back from (Windows wouldn't show it): just minimize.
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            }
        }
        self.update_step(&mut acts);
        if crate::images::dialog_closed() {
            ctx.input_mut(|i| i.keys_down.clear());
        }
        // Pictures chosen in a file dialog (which runs on its own thread).
        for (what, files) in crate::images::take_picked() {
            use crate::images::PickFor;
            match what {
                PickFor::Avatar | PickFor::ServerIcon(_) => {
                    let target = match what {
                        PickFor::ServerIcon(k) => crate::avatar_edit::CropTarget::Server(k),
                        _ => crate::avatar_edit::CropTarget::Me,
                    };
                    if let Some((_, bytes)) = files.into_iter().next() {
                        match crate::avatar_edit::AvatarCrop::new(bytes, target) {
                            Ok(c) => self.ui.avatar_crop = Some(c),
                            Err(e) => acts.push(Act::Toast(e, true)),
                        }
                    }
                }
                PickFor::Emoji(key) => {
                    for (name, bytes) in files {
                        acts.push(Act::AddEmoji { key: key.clone(), name: crate::images::emoji_name_from(&name), bytes });
                    }
                }
                PickFor::Attach(key, ch) => {
                    for (name, bytes) in files {
                        crate::chat::add_attachment(&mut self.ui.attach, &(key.clone(), ch), name, bytes, &mut acts);
                    }
                }
                PickFor::Sound(key) => {
                    if let Some(ed) = self.ui.sb.editor.as_mut().filter(|e| e.key == key) {
                        ed.picked(files.into_iter().next(), ctx);
                    }
                }
            }
        }
        // Folders chosen in a dialog (opened between frames).
        for (what, dir) in crate::images::take_folders() {
            match what {
                crate::images::FolderFor::Backups => engine.set_backup_dir(Some(dir)),
                crate::images::FolderFor::Restore => {
                    if let Some(c) = self.ui.create.as_mut() {
                        c.set_restore(dir);
                    }
                }
            }
        }
        // A direct message conversation was opened (from the member list or friends): show it.
        if let Some((key, id)) = engine.take_opened_dm() {
            self.ui.home = true;
            self.ui.dm = Some((key, id));
            self.ui.call_view = false;
        }
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        crate::titlebar::show(ctx, ingle_proto::APP_NAME);
        crate::watchdog::step("waiting to read the shared state");
        let chimes = {
            let st = engine.state();
            crate::watchdog::step("drawing the window");
            let chimes = self.voice_sounds.step(&st);
            self.frame_ui(ctx, &st, &mut acts);
            // The title (on the taskbar button and in Alt+Tab) says when something's unread.
            let title = crate::unread::Unread::of(&st).title(&self.title);
            if title != self.title_shown {
                ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
                self.title_shown = title;
            }
            chimes
        };
        crate::watchdog::step("playing sounds");
        // Only now that the state is let go (see Engine::state).
        for c in chimes {
            engine.play_sound(c);
        }
        let focused = ctx.input(|i| i.viewport().focused.unwrap_or(true) && !i.viewport().minimized.unwrap_or(false));
        engine.set_window_focused(focused);
        crate::watchdog::step("doing what was clicked");
        for a in acts {
            self.apply(a, ctx);
        }
        crate::watchdog::step("in a file dialog");
        crate::images::run_wanted_dialog();
        // Drop textures for tiles that are gone.
        let live: Vec<VideoKey> = self.textures.keys().copied().collect();
        for k in live {
            if self.engine.video_frame(k).is_none() {
                self.textures.remove(&k);
            }
        }
        self.ui.gif_sizes.save(false);
        // Keep animating while toasts/typing indicators/speaking rings are visible.
        ctx.request_repaint_after(Duration::from_millis(250));
        // eframe paints the frame after this returns, then waits for the next one.
        crate::watchdog::step("painting or idle");
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.ui.gif_sizes.save(true);
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
            Status::Ready(path, sha256) => {
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
                match crate::update::install(&path, &sha256) {
                    Ok(()) => {
                        tracing::info!("quitting to install the update");
                        self.ui.gif_sizes.save(true);
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
        let missed = match self.updater.status() {
            Status::Available(m) => m.missed(crate::update::current_version()),
            _ => 0,
        };
        let (label, frac, manifest) = match self.updater.status() {
            Status::Available(m) => {
                let behind = if missed > 1 { format!(" ({missed} updates behind)") } else { String::new() };
                (format!("Update to Ingle {}{behind} (restarts Ingle and puts you back where you were)", m.version), None, Some(m))
            }
            Status::Downloading(done, total) => {
                ("Downloading the update…".to_string(), Some(done as f32 / total.max(1) as f32), None)
            }
            Status::Ready(..) => ("Installing…".to_string(), Some(1.0), None),
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
                // Several updates missed: how many, in a red badge (like Discord's counts).
                if missed > 1 {
                    let t = if missed > 9 { "9+".to_string() } else { missed.to_string() };
                    let b = egui::Rect::from_center_size(rect.right_bottom() + vec2(-6.0, -6.0), vec2(if missed > 9 { 24.0 } else { 20.0 }, 20.0));
                    ui.painter().rect_filled(b.expand(3.0), CornerRadius::same(13), theme::RAIL);
                    ui.painter().rect_filled(b, CornerRadius::same(10), theme::LIVE);
                    ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, t, theme::bold_font(12.0), Color32::WHITE);
                }
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

        if self.ui.home {
            self.home_ui(ctx, st, acts);
            self.soundboard_popup(ctx, st, acts);
            self.dialogs(ctx, st, acts);
            self.viewer(ctx, acts);
            self.avatar_editor(ctx, acts);
            self.drop_files(ctx, st, acts);
            self.toasts(ctx, st);
            return;
        }
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
                    } else if self.ui.active_view.is_some() {
                        // No chat on screen: what comes in meanwhile is unread, wherever it is.
                        acts.push(Act::ClearActive);
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
                                if self.ui.active_view.is_some() {
                                    acts.push(Act::ClearActive);
                                }
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
                if self.ui.active_view.is_some() {
                    acts.push(Act::ClearActive);
                }
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(theme::MAIN))
                    .show(ctx, |ui| self.welcome(ui, st, acts));
            }
        }

        self.soundboard_popup(ctx, st, acts);
        self.profile_card(ctx, st, acts);
        self.dialogs(ctx, st, acts);
        self.viewer(ctx, acts);
        self.avatar_editor(ctx, acts);
        self.drop_files(ctx, st, acts);
        self.toasts(ctx, st);
    }

    /// Someone's profile (from clicking their name in a voice channel), like Discord's: their
    /// picture, name, roles, when they joined and how you know them, and a button to message them.
    /// (Right-clicking the name has the other options.)
    fn profile_card(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        use ingle_proto::control::RelationKind;
        let Some((key, uid, row)) = self.ui.profile_card.clone() else { return };
        let at = pos2(row.right() + 10.0, row.top());
        let Some(s) = st.server(&key) else {
            self.ui.profile_card = None;
            return;
        };
        let mut chose = false;
        let area = egui::Area::new(egui::Id::new("profile-card")).order(egui::Order::Foreground).fixed_pos(at).constrain(true).show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(theme::SIDE).inner_margin(16).corner_radius(10).show(ui, |ui| {
                ui.set_width(260.0);
                let member = s.members.get(&uid);
                let name = s.member_name(&uid);
                let role = member.map(|m| m.role).unwrap_or(Role::Member);
                let online = member.map(|m| m.online).unwrap_or(false);
                let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                let face = self.face(ui.ctx(), s, &uid);
                avatar(ui.painter(), r.center(), 34.0, &uid, &name, false, theme::SIDE, face);
                status_dot(ui.painter(), r.center() + vec2(25.0, 25.0), online, theme::SIDE);
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).font(theme::bold_font(19.0)).color(name_color(s, &uid, role)));
                    if role == Role::Owner {
                        let (cr, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                        icons::crown(ui.painter(), cr, theme::OWNER);
                    }
                });
                let how = match st.relation(&uid) {
                    _ if uid == st.my_uid => Some("This is you"),
                    Some(RelationKind::Friend) => Some("Friend"),
                    Some(RelationKind::Incoming) => Some("Sent you a friend request"),
                    Some(RelationKind::Outgoing) => Some("Friend request sent"),
                    Some(RelationKind::Blocked) => Some("Blocked"),
                    None => None,
                };
                if let Some(how) = how {
                    ui.label(RichText::new(how).size(12.5).color(theme::MUTED));
                }
                ui.add_space(8.0);
                ui.separator();
                let roles = s.roles_of(&uid);
                if !roles.is_empty() {
                    ui.label(RichText::new("ROLES").font(theme::bold_font(11.0)).color(theme::DIM));
                    ui.horizontal_wrapped(|ui| {
                        for r in roles {
                            let col = if r.color != 0 { rgb(r.color) } else { theme::TEXT };
                            egui::Frame::new().fill(theme::ELEV).corner_radius(4).inner_margin(egui::Margin::symmetric(6, 2)).show(ui, |ui| {
                                ui.label(RichText::new(&r.name).size(12.0).color(col));
                            });
                        }
                    });
                    ui.add_space(6.0);
                }
                if let Some(joined) = member.map(|m| m.joined_at).filter(|t| *t > 0) {
                    ui.label(RichText::new("MEMBER SINCE").font(theme::bold_font(11.0)).color(theme::DIM));
                    ui.label(RichText::new(crate::chat::member_since(joined)).size(13.0).color(theme::TEXT));
                    ui.add_space(6.0);
                }
                ui.label(RichText::new("SAFETY CODE").font(theme::bold_font(11.0)).color(theme::DIM));
                ui.label(RichText::new(ingle_proto::crypto::safety_code(&uid)).font(theme::mono(12.5)).color(theme::MUTED))
                    .on_hover_text("Compare it with theirs (on the welcome screen) to be sure it's really them.");
                let can_dm = uid != st.my_uid && s.has(ingle_proto::control::DMS_FEATURE) && s.status.is_connected();
                if can_dm {
                    ui.add_space(10.0);
                    if ui.add(theme::primary_button(&format!("Message {name}")).min_size(vec2(ui.available_width(), 32.0))).clicked() {
                        acts.push(Act::OpenDm { key: s.key.clone(), uid: uid.clone() });
                        chose = true;
                    }
                }
            });
        });
        // A click elsewhere closes it; one on the name itself is left to toggle it (a quick click
        // can press and release within one frame).
        let pressed_at = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        let clicked_outside = pressed_at.is_some_and(|p| !area.response.rect.contains(p) && !row.contains(p));
        if chose || clicked_outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.ui.profile_card = None;
        }
    }

    /// What you can do with someone, in the menu from right-clicking their name: message them,
    /// set how loud they are for you, friend or block them, copy their safety code, and for
    /// moderators, kick them. True when something was chosen (moving the volume slider isn't).
    pub(crate) fn person_options(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, uid: &str, acts: &mut Vec<Act>) -> bool {
        let me = uid == st.my_uid;
        let name = s.member_name(uid);
        let mut chose = false;
        let can_dm = !me && s.has(ingle_proto::control::DMS_FEATURE) && s.status.is_connected();
        if can_dm && ui.button("Message").clicked() {
            acts.push(Act::OpenDm { key: s.key.clone(), uid: uid.to_string() });
            chose = true;
        }
        if !me {
            ui.separator();
            ui.label(RichText::new("User volume").size(12.0).color(theme::MUTED));
            let mut v = st.user_volumes.get(uid).copied().unwrap_or(1.0) * 100.0;
            if ui.add(egui::Slider::new(&mut v, 0.0..=200.0).suffix("%")).changed() {
                acts.push(Act::Volume { uid: uid.to_string(), volume: v / 100.0 });
            }
            // In my call: their soundboard sounds can be muted, for me.
            let in_my_call = st.call.as_ref().is_some_and(|c| c.server_key == s.key && s.voice_in(c.channel_id).iter().any(|p| p.user_id == uid));
            if in_my_call {
                let muted = st.settings.soundboard_muted.iter().any(|u| u == uid);
                if ui.checkbox(&mut muted.clone(), "Mute their soundboard").clicked() {
                    acts.push(Act::MuteSoundboard { uid: uid.to_string(), muted: !muted });
                    chose = true;
                }
            }
            ui.separator();
            let n = acts.len();
            person_menu(ui, st, uid, acts);
            chose |= acts.len() > n;
        }
        // A moderator's Server Mute and Server Deafen, in a group of their own.
        let (can_mute, can_deafen) = moderate(st, s, uid);
        if (can_mute || can_deafen) && s.voice.values().any(|p| p.user_id == uid) {
            if !me {
                ui.separator();
            }
            server_voice_items(ui, st, s, uid, acts);
            ui.separator();
        }
        if ui.button("Copy safety code").clicked() {
            acts.push(Act::Copy(ingle_proto::crypto::safety_code(uid)));
            chose = true;
        }
        if !me && s.can(perm::KICK) && s.rank(uid) < s.rank(&st.my_uid) {
            ui.separator();
            if ui.button(RichText::new(format!("Kick {name}")).color(theme::LIVE)).clicked() {
                acts.push(Act::Kick { key: s.key.clone(), uid: uid.to_string(), ban: false });
                chose = true;
            }
        }
        chose
    }

    /// Whether a moderator muted or deafened me in my call (`VoiceParticipant::server_mute`,
    /// `server_deaf`).
    pub(crate) fn moderated_me(st: &AppState) -> (bool, bool) {
        st.call
            .as_ref()
            .and_then(|c| st.server(&c.server_key).and_then(|s| s.voice.get(&c.session)))
            .map(|p| (p.server_mute, p.server_deaf))
            .unwrap_or_default()
    }

    /// Images dragged onto the window go to the open channel's composer.
    fn drop_files(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        let key = if self.ui.home { self.ui.dm.as_ref().map(|d| d.0.clone()) } else { self.ui.server.clone() };
        let Some(server) = key.and_then(|k| st.server(&k).cloned()) else { return };
        if !server.has(ingle_proto::control::FILES_FEATURE) {
            return;
        }
        let channel = if self.ui.home {
            self.ui.dm.as_ref().map(|d| d.1)
        } else if self.ui.call_view {
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
                let name = server
                    .channel(channel)
                    .map(|c| format!("#{}", c.name))
                    .or_else(|| server.dm(channel).map(|d| format!("your chat with {}", server.member_name(&d.with))))
                    .unwrap_or_default();
                let b = Rect::from_center_size(r.center(), vec2(420.0, 120.0));
                ui.painter().rect_filled(b, CornerRadius::same(14), theme::EMBER);
                ui.painter().text(b.center() - vec2(0.0, 12.0), egui::Align2::CENTER_CENTER, "Drop images to send", theme::bold_font(20.0), Color32::from_rgb(0x1b, 0x14, 0x11));
                ui.painter().text(b.center() + vec2(0.0, 18.0), egui::Align2::CENTER_CENTER, format!("in {name}"), theme::font(14.0), Color32::from_rgb(0x3a, 0x22, 0x14));
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
            // Home: friends and direct messages.
            let (r, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
            let home = self.ui.home;
            let fill = if home { Color32::from_rgb(0x4a, 0x30, 0x22) } else if resp.hovered() { theme::ELEV2 } else { Color32::from_rgb(0x24, 0x1d, 0x19) };
            ui.painter().rect_filled(r, CornerRadius::same(if home || resp.hovered() { 16 } else { 24 }), fill);
            icons::flame(ui.painter(), r.shrink(10.0));
            if home || resp.hovered() {
                let pill = Rect::from_center_size(pos2(ui.max_rect().left() + 2.0, r.center().y), vec2(4.0, if home { 40.0 } else { 20.0 }));
                ui.painter().rect_filled(pill, CornerRadius::same(2), theme::TEXT);
            }
            let dm_count: u32 = st.servers.iter().map(|s| s.dms.iter().filter_map(|d| s.mentions.get(&d.id)).sum::<u32>()).sum::<u32>()
                + st.relations().iter().filter(|r| r.1 == ingle_proto::control::RelationKind::Incoming).count() as u32;
            if dm_count > 0 {
                let c = r.right_bottom() - vec2(6.0, 6.0);
                ui.painter().circle_filled(c, 9.0, theme::RAIL);
                ui.painter().circle_filled(c, 7.5, theme::LIVE);
                ui.painter().text(c, egui::Align2::CENTER_CENTER, dm_count.min(99).to_string(), theme::bold_font(10.0), Color32::WHITE);
            }
            if resp.on_hover_text("Direct messages").clicked() {
                self.ui.home = true;
            }
            // Unread direct messages under it, by who sent them (like Discord): their picture
            // with how many, wherever you are. Click to read.
            let mut waiting: Vec<(&ServerView, ChannelId, String, u32)> = Vec::new();
            for s in &st.servers {
                for d in &s.dms {
                    let n = s.mentions.get(&d.id).copied().unwrap_or(0);
                    // (A muted server still lets direct messages through; a muted conversation doesn't.)
                    let muted = st.muted_channels.contains(&(s.key.clone(), d.id));
                    if n > 0 && !muted && !st.blocked.contains(&d.with) {
                        waiting.push((s, d.id, d.with.clone(), n));
                    }
                }
            }
            for (s, id, with, n) in waiting.into_iter().take(5) {
                ui.add_space(8.0);
                let (r, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
                let name = s.member_name(&with);
                let face = self.face(ui.ctx(), s, &with);
                avatar(ui.painter(), r.center(), 24.0, &with, &name, false, theme::RAIL, face);
                let pill = Rect::from_center_size(pos2(ui.max_rect().left() + 2.0, r.center().y), vec2(4.0, if resp.hovered() { 20.0 } else { 8.0 }));
                ui.painter().rect_filled(pill, CornerRadius::same(2), theme::TEXT);
                let c = r.right_bottom() - vec2(6.0, 6.0);
                ui.painter().circle_filled(c, 9.0, theme::RAIL);
                ui.painter().circle_filled(c, 7.5, theme::LIVE);
                ui.painter().text(c, egui::Align2::CENTER_CENTER, n.min(99).to_string(), theme::bold_font(10.0), Color32::WHITE);
                let tip = format!("{name}: {n} unread message{}", if n == 1 { "" } else { "s" });
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                    self.ui.home = true;
                    self.ui.dm = Some((s.key.clone(), id));
                }
            }
            ui.add_space(6.0);
            let (sep, _) = ui.allocate_exact_size(vec2(32.0, 2.0), Sense::hover());
            ui.painter().rect_filled(sep, CornerRadius::same(1), theme::LINE);
            ui.add_space(6.0);
            self.update_button(ui);
            for s in &st.servers {
                let selected = !self.ui.home && self.ui.server.as_deref() == Some(s.key.as_str());
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
                let icon = self.server_icon(ui.ctx(), s);
                // A server's own icon is drawn as it is (see-through parts stay see-through, like
                // Discord); the coloured tile is only behind initials. The pill on the left shows
                // which server is open.
                if icon.is_none() {
                    ui.painter().rect_filled(rect, CornerRadius::same(radius), fill);
                }
                if let Some(t) = icon {
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
                    ui.painter().add(egui::epaint::RectShape::filled(rect, CornerRadius::same(radius), Color32::WHITE).with_texture(t, uv));
                }
                let initials: String = s
                    .name
                    .split_whitespace()
                    .filter_map(|w| w.chars().next())
                    .take(2)
                    .collect::<String>()
                    .to_uppercase();
                let tc = if selected { Color32::from_rgb(0x1b, 0x14, 0x11) } else { theme::TEXT };
                if icon.is_none() {
                    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, initials, theme::bold_font(15.0), tc);
                }
                // pill indicator
                // Direct messages count on the home button, not the server's.
                let unread = s.unread.iter().any(|c| s.dm(*c).is_none());
                let pill_h = if selected { 40.0 } else if hovered { 20.0 } else if unread { 8.0 } else { 0.0 };
                if pill_h > 0.0 {
                    let pill = Rect::from_center_size(pos2(ui.max_rect().left() + 2.0, rect.center().y), vec2(4.0, pill_h));
                    ui.painter().rect_filled(pill, CornerRadius::same(2), theme::TEXT);
                }
                let mentions: u32 = s.mentions.iter().filter(|(c, _)| s.dm(**c).is_none()).map(|(_, n)| n).sum();
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
                    self.ui.home = false;
                    self.ui.server = Some(s.key.clone());
                    self.ui.call_view = st.call.as_ref().map(|c| c.server_key == s.key).unwrap_or(false) && self.ui.call_view;
                }
                let name = s.name.clone();
                let key = s.key.clone();
                let admin = s.can(perm::MANAGE_SERVER) || s.can(perm::MANAGE_ROLES);
                let server_muted = st.muted_servers.contains(&s.key);
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
                    let label = if server_muted { "Unmute server" } else { "Mute server" };
                    if ui.button(label).on_hover_text("No message sounds or unread dots (mentions still ping)").clicked() {
                        acts.push(Act::MuteChat { key: key.clone(), channel: None, muted: !server_muted });
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
        } else if st.voice_rejoin.is_some() {
            egui::TopBottomPanel::bottom("voice-panel")
                .exact_height(56.0)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(theme::SIDE_DARK).inner_margin(egui::Margin::symmetric(10, 8)))
                .show_inside(ui, |ui| self.rejoin_panel(ui, st, acts));
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
            // The owner's bug report inbox (when this server collects Ingle bug reports).
            let owner = s.me.as_ref().map(|m| m.role == ingle_proto::control::Role::Owner).unwrap_or(false);
            if owner && s.info.bug_reports {
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                let row = r.shrink2(vec2(8.0, 1.0));
                if resp.hovered() {
                    ui.painter().rect_filled(row, CornerRadius::same(6), theme::ELEV);
                }
                ui.painter().text(row.left_center() + vec2(12.0, 0.0), egui::Align2::LEFT_CENTER, "!", theme::bold_font(17.0), theme::MUTED);
                ui.painter().text(row.left_center() + vec2(32.0, 0.0), egui::Align2::LEFT_CENTER, "bug-reports", theme::font(15.0), theme::MUTED);
                let n = s.info.new_bug_reports;
                if n > 0 {
                    let t = if n > 99 { "99+".to_string() } else { n.to_string() };
                    let b = Rect::from_center_size(pos2(row.right() - 18.0, row.center().y), vec2(14.0 + 7.0 * t.len() as f32, 18.0));
                    ui.painter().rect_filled(b, CornerRadius::same(9), theme::LIVE);
                    ui.painter().text(b.center(), egui::Align2::CENTER_CENTER, t, theme::bold_font(11.5), Color32::WHITE);
                }
                if resp.on_hover_text("Ingle bug reports people sent here. Only you see this.").clicked() {
                    acts.push(Act::ListBugs(s.key.clone()));
                    self.ui.dialog = Some(Dialog::BugReports { key: s.key.clone(), filter: 0 });
                }
                ui.add_space(6.0);
            }
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
                    // Hover by where the pointer is, not by which widget has it: over the chat
                    // bubble the row itself isn't "hovered", and going by that made the bubble
                    // blink (and miss clicks).
                    let hot = ui.rect_contains_pointer(rect);
                    let bg = if selected { theme::ELEV2 } else if hot { theme::ELEV } else { Color32::TRANSPARENT };
                    ui.painter().rect_filled(rect, CornerRadius::same(6), bg);
                    let icon_rect = Rect::from_min_size(rect.left_top() + vec2(8.0, 7.0), vec2(18.0, 18.0));
                    let fg = if selected || unread || hot {
                        theme::TEXT
                    } else if st.is_muted(&s.key, c.id) {
                        theme::DIM
                    } else {
                        theme::MUTED
                    };
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
                    }
                    // Unread (text channels, and voice channels' chats): the dot on the left.
                    if unread || mentions > 0 {
                        let p = pos2(rect.left() - 6.0, rect.center().y);
                        ui.painter().rect_filled(Rect::from_center_size(p, vec2(4.0, 8.0)), CornerRadius::same(2), theme::TEXT);
                    }
                    // Voice channels: a chat bubble opens the channel's text chat without joining.
                    // Always there to click; shown while the pointer is on the row (or unread).
                    let mut chat_clicked = false;
                    if kind == ChannelKind::Voice {
                        // (Left of the mentions badge when there is one.)
                        let x = if mentions > 0 { 46.0 } else { 16.0 };
                        let cr = Rect::from_center_size(rect.right_center() - vec2(x, 0.0), vec2(26.0, 26.0));
                        let cresp = ui.interact(cr, ui.id().with(("chat", c.id)), Sense::click());
                        if hot || unread {
                            if cresp.hovered() {
                                ui.painter().rect_filled(cr, CornerRadius::same(5), theme::ELEV2);
                            }
                            icons::chat(ui.painter(), cr.shrink(5.0), if cresp.hovered() || unread { theme::TEXT } else { theme::MUTED });
                        }
                        if cresp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Open chat").clicked() {
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
                    let muted = st.is_muted(&s.key, cid);
                    let can_manage = s.can(perm::MANAGE_CHANNELS);
                    {
                        resp.context_menu(|ui| {
                            let label = if muted { "Unmute channel" } else { "Mute channel" };
                            if ui.button(label).clicked() {
                                acts.push(Act::MuteChat { key: key.clone(), channel: Some(cid), muted: !muted });
                                ui.close();
                            }
                            if !can_manage {
                                return;
                            }
                            ui.separator();
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
                            let row = r.translate(vec2(8.0, 0.0)).with_max_x(r.right());
                            let uid = p.user_id.clone();
                            let pointer_in = presp.contains_pointer();
                            let card_open = self.ui.profile_card.as_ref().is_some_and(|(k, u, _)| *k == s.key && *u == uid);
                            if pointer_in || card_open {
                                ui.painter().rect_filled(row, CornerRadius::same(6), theme::HOVER);
                            }
                            let center = pos2(r.left() + 46.0, r.center().y);
                            let face = self.face(ui.ctx(), s, &uid);
                            avatar(ui.painter(), center, 11.0, &uid, &name, speaking.contains(&p.session), theme::SIDE, face);
                            let col = if speaking.contains(&p.session) || pointer_in { theme::TEXT } else { theme::MUTED };
                            let name_rect = ui.painter().text(pos2(r.left() + 64.0, r.center().y), egui::Align2::LEFT_CENTER, &name, theme::font(13.5), col);
                            // A soundboard sound they just played: its emoji by their name.
                            if let Some((x, age)) = st.call.as_ref().filter(|c| c.server_key == s.key).and_then(|c| crate::soundboard::recent_pop(c, p.session)) {
                                let tex = crate::soundboard::custom_emoji_id(&x.emoji).and_then(|id| self.custom_emoji_tex(ui.ctx(), &s.key, &id));
                                let at = pos2(name_rect.right() + 13.0, r.center().y + 4.0);
                                crate::soundboard::paint_pop(ui.painter(), at, 18.0, &x.emoji, age, &mut |_| tex);
                                ui.ctx().request_repaint();
                            }
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
                            // Deafened (red: by a moderator), and muted (red: by a moderator), like
                            // Discord's; deafening yourself mutes you too, so that shows just the one.
                            if p.server_deaf || p.self_deaf {
                                let c = if p.server_deaf { theme::LIVE } else { theme::DIM };
                                icons::headphones(ui.painter(), Rect::from_center_size(pos2(x - 8.0, r.center().y), vec2(16.0, 16.0)), c, true);
                                x -= 20.0;
                            }
                            if p.server_mute {
                                icons::mic(ui.painter(), Rect::from_center_size(pos2(x - 8.0, r.center().y), vec2(16.0, 16.0)), theme::LIVE, true);
                                x -= 20.0;
                            } else if p.self_mute && !p.self_deaf {
                                icons::mic(ui.painter(), Rect::from_center_size(pos2(x - 8.0, r.center().y), vec2(16.0, 16.0)), theme::DIM, true);
                                x -= 20.0;
                            }
                            // Hovering someone: a button to message them (like the chat bubble on a
                            // voice channel).
                            let can_dm = uid != st.my_uid && s.has(ingle_proto::control::DMS_FEATURE) && s.status.is_connected();
                            let mut dm_clicked = false;
                            if can_dm && pointer_in {
                                let br = Rect::from_center_size(pos2(x - 10.0, r.center().y), vec2(22.0, 22.0));
                                let bresp = ui.interact(br, ui.id().with(("dm", c.id, &uid)), Sense::click());
                                icons::chat(ui.painter(), br.shrink(3.0), if bresp.hovered() { theme::TEXT } else { theme::MUTED });
                                if bresp.on_hover_text(format!("Message {name}")).clicked() {
                                    acts.push(Act::OpenDm { key: s.key.clone(), uid: uid.clone() });
                                    dm_clicked = true;
                                }
                            }
                            // Left-click: their profile. Right-click: what you can do.
                            if presp.clicked() && !dm_clicked {
                                self.ui.profile_card = if card_open { None } else { Some((s.key.clone(), uid.clone(), row)) };
                            }
                            // (Ticking Mute or Deafen leaves the menu open, like Discord; the
                            // other items close it.)
                            egui::Popup::context_menu(&presp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
                                if ui.button("Profile").clicked() {
                                    self.ui.profile_card = Some((s.key.clone(), uid.clone(), row));
                                    ui.close();
                                }
                                if uid == st.my_uid {
                                    // Like Discord: your own Mute and Deafen, the same as the
                                    // buttons by your name (deafened counts as muted too).
                                    let (muted, deafened) = self.engine.voice_prefs();
                                    ui.separator();
                                    let mut m = muted || deafened;
                                    if ui.checkbox(&mut m, "Mute").clicked() {
                                        acts.push(Act::Mute(m));
                                    }
                                    let mut d = deafened;
                                    if ui.checkbox(&mut d, "Deafen").clicked() {
                                        acts.push(Act::Deafen(d));
                                    }
                                    ui.separator();
                                }
                                if self.person_options(ui, st, s, &uid, acts) {
                                    ui.close();
                                }
                            });
                        }
                    }
                }
                ui.add_space(10.0);
            }
        });
    }

    /// Shown while getting back into a call after the server dropped (an update, say).
    pub(crate) fn rejoin_panel(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        let Some(r) = st.voice_rejoin.as_ref() else { return };
        let channel_name = st.server(&r.server_key).and_then(|s| s.channel(r.channel_id)).map(|c| c.name.clone()).unwrap_or_default();
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
            icons::signal(ui.painter(), rect, theme::EMBER2);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("Reconnecting…").font(theme::bold_font(13.5)).color(theme::EMBER2));
                ui.add(egui::Label::new(RichText::new(format!("{channel_name} · the server is restarting")).size(12.0).color(theme::MUTED)).truncate());
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, 32.0, "Disconnect", false, |p, r, _| icons::hangup(p, r, theme::TEXT)).clicked() {
                    acts.push(Act::Leave);
                }
            });
        });
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
    }

    pub(crate) fn voice_panel(&mut self, ui: &mut egui::Ui, st: &AppState, call: &ingle_engine::state::CallView, acts: &mut Vec<Act>) {
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
            let w = (ui.available_width() - 3.0 * ui.spacing().item_spacing.x) / 4.0;
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
            let sb = btn(ui, self.soundboard_open(), "Soundboard", &|p, r, c| icons::soundboard(p, r, c));
            self.soundboard_anchor(crate::soundboard::FROM_PANEL, sb.rect);
            if sb.clicked() {
                self.toggle_soundboard(st, crate::soundboard::FROM_PANEL);
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

    pub(crate) fn user_panel(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
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
                let (server_muted, server_deafened) = Self::moderated_me(st);
                let deaf_tip = if server_deafened { SERVER_DEAFENED } else if deafened { "Undeafen" } else { "Deafen" };
                let deaf = deafened || server_deafened;
                if icon_button(ui, 32.0, deaf_tip, false, |p, r, c| icons::headphones(p, r, if deaf { theme::LIVE } else { c }, deaf)).clicked() {
                    if server_deafened {
                        acts.push(Act::Toast(SERVER_DEAFENED.into(), false));
                    } else {
                        acts.push(Act::Deafen(!deafened));
                    }
                }
                // Deafened is muted too (unmuting undeafens), like Discord.
                let muted = muted || deafened;
                let mute_tip = if server_muted { SERVER_MUTED } else if muted { "Unmute" } else { "Mute" };
                if icon_button(ui, 32.0, mute_tip, false, |p, r, c| icons::mic(p, r, if muted || server_muted { theme::LIVE } else { c }, muted || server_muted)).clicked() {
                    if server_muted {
                        acts.push(Act::Toast(SERVER_MUTED.into(), false));
                    } else {
                        acts.push(Act::Mute(!muted));
                    }
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
                    let can_dm = !me && s.has(ingle_proto::control::DMS_FEATURE) && s.status.is_connected();
                    if can_dm && resp.clicked() {
                        acts.push(Act::OpenDm { key: key.clone(), uid: uid.clone() });
                    }
                    resp.on_hover_text(hover).context_menu(|ui| {
                        if can_dm && ui.button("Message").clicked() {
                            acts.push(Act::OpenDm { key: key.clone(), uid: uid.clone() });
                            ui.close();
                        }
                        person_menu(ui, st, &uid, acts);
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
        // Notices fade after 6 seconds; errors stay until dismissed, so they can be read and
        // copied (to send to whoever runs the server, say).
        let visible: Vec<_> = st
            .toasts
            .iter()
            .filter(|t| if t.error { !self.ui.toasts_dismissed.contains(&t.id) } else { now.duration_since(t.at) < Duration::from_secs(6) })
            .collect();
        if visible.is_empty() {
            return;
        }
        if visible.iter().any(|t| !t.error) {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let mut dismiss = None;
        let mut copy = None;
        let mut report: Option<String> = None;
        let mut reopen_report = false;
        egui::Area::new(egui::Id::new("toasts"))
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-20.0, -90.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                for t in visible.iter().rev().take(3) {
                    egui::Frame::new()
                        .fill(if t.error { Color32::from_rgb(0x4a, 0x1f, 0x1f) } else { theme::ELEV2 })
                        .stroke(Stroke::new(1.0f32, if t.error { theme::LIVE } else { theme::LINE }))
                        .corner_radius(8)
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_max_width(360.0);
                            if !t.error {
                                ui.label(RichText::new(&t.text).color(theme::TEXT));
                                return;
                            }
                            ui.label(RichText::new("Something went wrong").font(theme::bold_font(13.0)).color(theme::TEXT));
                            ui.add_space(2.0);
                            ui.add(egui::Label::new(RichText::new(&t.text).color(theme::TEXT)).selectable(true));
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                let copied = self.ui.toast_copied.map(|(id, at)| id == t.id && now.duration_since(at) < Duration::from_secs(2)).unwrap_or(false);
                                if ui.button(if copied { "Copied" } else { "Copy" }).on_hover_text("Copy this error, to paste it to someone").clicked() {
                                    copy = Some(t.id);
                                }
                                if ui.button("Dismiss").clicked() {
                                    dismiss = Some(t.id);
                                }
                                if t.text.starts_with(crate::dialogs::BUG_NOT_SENT) {
                                    // That report, as it was, to send again.
                                    if ui.button("Open it").clicked() {
                                        reopen_report = true;
                                        dismiss = Some(t.id);
                                    }
                                } else if ingle_engine::bugs::can_report()
                                    && ui.button("Report").on_hover_text("Tell Ingle's developers about this").clicked()
                                {
                                    report = Some(t.text.clone());
                                    dismiss = Some(t.id);
                                }
                            });
                        });
                    ui.add_space(6.0);
                }
            });
        if let Some(id) = copy {
            if let Some(t) = st.toasts.iter().find(|t| t.id == id) {
                let when = chrono::Local::now().format("%Y-%m-%d %H:%M");
                ctx.copy_text(format!("{}\n(Ingle {}, {when})", t.text, env!("CARGO_PKG_VERSION")));
                self.ui.toast_copied = Some((id, now));
                ctx.request_repaint_after(Duration::from_secs(2));
            }
        }
        if let Some(id) = dismiss {
            self.ui.toasts_dismissed.insert(id);
        }
        if let Some(text) = report {
            self.ui.dialog = Some(Dialog::ReportBug {
                title: text.chars().take(80).collect(),
                body: format!("This error came up:\n{text}\n\nWhat I was doing: "),
                include_log: true,
            });
        }
        if reopen_report {
            self.ui.dialog = Some(self.report_bug_dialog());
        }
        let _ = self.started;
    }
}

/// Server Mute and Server Deafen, in red like Discord's, for a moderator
/// (`perm::MUTE_MEMBERS`, `perm::DEAFEN_MEMBERS`) and someone in voice they may do it to
/// (themselves, or someone with a lower role).
pub(crate) fn server_voice_items(ui: &mut egui::Ui, st: &AppState, s: &ServerView, uid: &str, acts: &mut Vec<Act>) {
    let (can_mute, can_deafen) = moderate(st, s, uid);
    let Some(p) = s.voice.values().find(|p| p.user_id == uid) else { return };
    if can_mute {
        let mut on = p.server_mute;
        if ui.checkbox(&mut on, RichText::new("Server Mute").color(theme::LIVE)).on_hover_text("Nobody hears them until a moderator unmutes them, in any voice channel here.").clicked() {
            acts.push(Act::ServerMute { key: s.key.clone(), uid: uid.to_string(), muted: on });
        }
    }
    if can_deafen {
        let mut on = p.server_deaf;
        if ui.checkbox(&mut on, RichText::new("Server Deafen").color(theme::LIVE)).on_hover_text("They hear nobody, and nobody hears them, until a moderator undeafens them.").clicked() {
            acts.push(Act::ServerDeafen { key: s.key.clone(), uid: uid.to_string(), deafened: on });
        }
    }
}

/// Whether I may server mute and server deafen `uid` here: a moderator with the permission,
/// and them myself or below me.
pub(crate) fn moderate(st: &AppState, s: &ServerView, uid: &str) -> (bool, bool) {
    let me = uid == st.my_uid;
    let reach = s.has(ingle_proto::control::VOICE_MOD_FEATURE) && (me || s.rank(uid) < s.rank(&st.my_uid));
    // Only the owner undoes their own server mute or deafen: anyone else's is for someone above
    // them to undo (so it isn't offered).
    let owner = s.me.as_ref().is_some_and(|m| m.role == Role::Owner);
    let now = s.voice.values().find(|p| p.user_id == uid);
    let stuck_mute = me && !owner && now.is_some_and(|p| p.server_mute);
    let stuck_deaf = me && !owner && now.is_some_and(|p| p.server_deaf);
    (reach && s.can(perm::MUTE_MEMBERS) && !stuck_mute, reach && s.can(perm::DEAFEN_MEMBERS) && !stuck_deaf)
}

/// Friend and block choices for someone (right-click menus), like Discord's.
pub(crate) fn person_menu(ui: &mut egui::Ui, st: &AppState, uid: &str, acts: &mut Vec<Act>) {
    use ingle_proto::control::RelationKind;
    if uid == st.my_uid {
        return;
    }
    let rel = st.relation(uid);
    let mut item = |ui: &mut egui::Ui, label: &str, act: Act| {
        if ui.button(label).clicked() {
            acts.push(act);
            ui.close();
        }
    };
    match rel {
        Some(RelationKind::Friend) => item(ui, "Remove Friend", Act::RemoveFriend(uid.into())),
        Some(RelationKind::Incoming) => {
            item(ui, "Accept Friend Request", Act::AcceptFriend(uid.into()));
            item(ui, "Ignore Friend Request", Act::RemoveFriend(uid.into()));
        }
        Some(RelationKind::Outgoing) => item(ui, "Cancel Friend Request", Act::RemoveFriend(uid.into())),
        Some(RelationKind::Blocked) => {}
        None => item(ui, "Add Friend", Act::AddFriend(uid.into())),
    }
    if rel == Some(RelationKind::Blocked) {
        item(ui, "Unblock", Act::Unblock(uid.into()));
    } else {
        item(ui, "Block", Act::Block(uid.into()));
    }
}
