//! Text chat: message list (grouping, replies, reactions, edits) and the composer.

use std::time::{Duration, Instant};

use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{self, Align, Color32, CornerRadius, Key, KeyboardShortcut, Layout, Modifiers, Rect, RichText, Sense, Stroke, pos2, vec2};
use ingle_engine::state::{AppState, ChannelLog, ServerView};
use ingle_proto::control::{Attachment, ChannelId, ChannelKind, FILES_FEATURE, Message, MessageId, perm};

use crate::app::{Act, IngleApp, avatar, icon_button, name_color};
use crate::{icons, markdown, theme};

const GROUP_WINDOW_MS: i64 = 7 * 60 * 1000;

fn local(ms: i64) -> chrono::DateTime<Local> {
    Local.timestamp_millis_opt(ms).single().unwrap_or_else(Local::now)
}

pub fn when(ms: i64) -> String {
    let t = local(ms);
    let today = Local::now().date_naive();
    let d = t.date_naive();
    let time = t.format("%-I:%M %p").to_string();
    if d == today {
        format!("Today at {time}")
    } else if Some(d) == today.pred_opt() {
        format!("Yesterday at {time}")
    } else {
        format!("{}/{}/{} {time}", d.month(), d.day(), d.year())
    }
}

/// "Oct 11, 2026": when someone joined a server.
pub fn member_since(ms: i64) -> String {
    local(ms).format("%b %-d, %Y").to_string()
}

fn day_label(ms: i64) -> String {
    let t = local(ms);
    let today = Local::now().date_naive();
    let d = t.date_naive();
    if d == today {
        "Today".into()
    } else if Some(d) == today.pred_opt() {
        "Yesterday".into()
    } else {
        t.format("%B %-d, %Y").to_string()
    }
}

fn snippet(s: &str, n: usize) -> String {
    let one_line: String = s.replace('\n', " ");
    if one_line.chars().count() > n { format!("{}…", one_line.chars().take(n).collect::<String>()) } else { one_line }
}

/// Where a conversation is scrolled to, from one frame to the next. Like Discord: at the
/// newest messages it stays there as messages, pictures and GIFs come in; scrolled up, the
/// message at the top stays put while things above it (older messages, pictures loading)
/// change size, instead of the view jumping.
#[derive(Clone, Debug)]
pub struct ChatScroll {
    /// The scroll area's id (its offset is kept by egui).
    pub id: egui::Id,
    /// At the newest messages.
    pub stuck: bool,
    /// The message at the top of the view, and how far its top is from the view's top.
    pub anchor: Option<(MessageId, f32)>,
    /// Shown yet (see `REVEAL_WAIT`), and since when it's been laid out.
    pub shown: bool,
    pub since: Instant,
    /// How tall it was laid out last time.
    pub h: f32,
}

/// The longest a conversation just opened is kept from showing while GIFs on screen whose
/// size isn't known yet load (only those: everything else has its size before it loads).
const REVEAL_WAIT: Duration = Duration::from_millis(1500);

/// Where a conversation should be scrolled to after laying it out (`drawn`: where this pass
/// drew it; `placed`: each message's top and bottom): where you scrolled to, else the newest
/// messages if you were there, else wherever keeps your top message in place.
fn steady_scroll(out: &egui::scroll_area::ScrollAreaOutput<()>, drawn: f32, memo: Option<&ChatScroll>, placed: &[(MessageId, f32, f32)]) -> f32 {
    let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
    let now = out.state.offset.y;
    let Some(memo) = memo else { return max };
    if (now - drawn).abs() > 0.5 {
        // You scrolled (wheel, scroll bar, keys).
        return now.clamp(0.0, max);
    }
    if memo.stuck {
        return max;
    }
    match memo.anchor.and_then(|(id, from_top)| placed.iter().find(|m| m.0 == id).map(|m| m.1 - from_top)) {
        Some(at) => at.clamp(0.0, max),
        None => now.clamp(0.0, max),
    }
}

impl IngleApp {
    pub fn chat_view(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        s: &ServerView,
        channel: ChannelId,
        in_call_panel: bool,
        acts: &mut Vec<Act>,
    ) {
        // A direct message shows like a text channel named after the other person; a group,
        // after itself.
        let dm_with = s.dm(channel).map(|d| d.with.clone());
        let group = s.group(channel).cloned();
        let ch = s.channel(channel).cloned().or_else(|| {
            let name = match (&dm_with, &group) {
                (Some(w), _) => format!("@{}", s.member_name(w)),
                (None, Some(g)) => s.group_name(g, &st.my_uid),
                _ => return None,
            };
            Some(ingle_proto::control::Channel {
                id: channel,
                kind: ChannelKind::Text,
                name,
                topic: String::new(),
                position: 0,
                last_message: None,
                last_from: None,
            })
        });
        let Some(ch) = ch else { return };
        let key = s.key.clone();
        let names = mention_names(s);

        // History + read markers
        let log = s.logs.get(&channel);
        let loaded = log.map(|l| l.loaded).unwrap_or(false);
        if s.status.is_connected() && !loaded && !log.map(|l| l.loading).unwrap_or(false) && !self.ui.requested.contains(&(key.clone(), channel)) {
            self.ui.requested.insert((key.clone(), channel));
            acts.push(Act::LoadHistory { key: key.clone(), channel, older: false });
        }
        if loaded {
            self.ui.requested.remove(&(key.clone(), channel));
        }
        // This is what's on screen (the call's chat beside the call included): what comes in
        // here counts as read while the window is in front, and anywhere else as unread. Said
        // again while it has unread messages, which clears them once the window is in front.
        let here = (key.clone(), channel);
        if s.unread.contains(&channel) || s.mentions.contains_key(&channel) || self.ui.active_view.as_ref() != Some(&here) {
            acts.push(Act::SetActive { key: key.clone(), channel });
            self.ui.active_view = Some(here);
        }
        if !in_call_panel && dm_with.is_none() && group.is_none() {
            self.ui.channel.insert(key.clone(), channel);
        }

        // Header
        egui::TopBottomPanel::top(ui.id().with("chat-header"))
            .exact_height(48.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::MAIN).inner_margin(egui::Margin::symmetric(16, 0)))
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(g) = &group {
                        let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                        let p = ui.painter().clone();
                        crate::calls::group_picture(self, &p, ui.ctx(), s, g, &st.my_uid, r.center(), 13.0, theme::MAIN);
                        let w = (ui.available_width() - 200.0).max(80.0);
                        crate::emoji::label_line(ui, &ch.name, theme::bold_font(15.5), theme::TEXT, w, &mut |_| None);
                        ui.label(RichText::new(format!("{} members", g.members.len())).size(12.5).color(theme::DIM));
                    } else if let Some(w) = &dm_with {
                        let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                        let face = self.face(ui.ctx(), s, w);
                        avatar(ui.painter(), r.center(), 12.0, w, &s.member_name(w), false, theme::MAIN, face);
                        ui.label(RichText::new(s.member_name(w)).font(theme::bold_font(15.5)).color(theme::TEXT));
                        if s.members.get(w).map(|m| m.online).unwrap_or(false) {
                            crate::app::status_dot(ui.painter(), ui.min_rect().right_center() + vec2(10.0, 0.0), true, theme::MAIN);
                        }
                    } else {
                        let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                        if ch.kind == ChannelKind::Voice {
                            icons::chat(ui.painter(), r, theme::DIM);
                        } else {
                            icons::hash(ui.painter(), r, theme::DIM);
                        }
                        crate::emoji::label(ui, &ch.name, theme::bold_font(15.5), theme::TEXT, None, &mut |_| None);
                    }
                    if !ch.topic.is_empty() && !in_call_panel {
                        let (sep, _) = ui.allocate_exact_size(vec2(1.0, 22.0), Sense::hover());
                        ui.painter().rect_filled(sep, 0, theme::LINE);
                        ui.add(egui::Label::new(RichText::new(&ch.topic).size(13.0).color(theme::MUTED)).truncate());
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if in_call_panel {
                            if icon_button(ui, 28.0, "Close chat", false, |p, r, c| icons::close(p, r, c)).clicked() {
                                self.ui.call_chat_open = false;
                            }
                        } else if dm_with.is_none() && group.is_none() {
                            let open = self.ui.members_open;
                            if icon_button(ui, 30.0, if open { "Hide member list" } else { "Show member list" }, open, |p, r, c| icons::people(p, r, c)).clicked() {
                                self.ui.members_open = !open;
                            }
                        } else {
                            // A group's people (beside its chat), and adding people: to the group,
                            // or to make one from this conversation (like Discord's).
                            if group.is_some() {
                                let open = self.ui.members_open;
                                if icon_button(ui, 30.0, if open { "Hide members" } else { "Show members" }, open, |p, r, c| icons::people(p, r, c)).clicked() {
                                    self.ui.members_open = !open;
                                }
                            }
                            let can_add = crate::calls::can_group(s)
                                && group.as_ref().map(|g| g.members.len() < ingle_proto::control::MAX_GROUP).unwrap_or(true)
                                && dm_with.as_ref().is_none_or(|w| !st.blocked.contains(w));
                            if can_add && icon_button(ui, 30.0, "Add people", false, |p, r, c| icons::add_person(p, r, c)).clicked() {
                                self.ui.dialog = Some(crate::app::Dialog::AddPeople { key: s.key.clone(), channel: Some(channel), picked: Vec::new(), search: String::new() });
                            }
                            // Calls, like Discord's: voice, and video (not while you're in it).
                            let blocked = dm_with.as_ref().is_some_and(|w| st.blocked.contains(w));
                            let can_call = s.has(ingle_proto::control::DM_CALLS_FEATURE) && s.status.is_connected() && !blocked;
                            let in_it = st.call.as_ref().is_some_and(|c| c.server_key == s.key && c.channel_id == channel);
                            if can_call && !in_it {
                                if icon_button(ui, 30.0, "Start a video call", false, |p, r, c| icons::camera(p, r, c, false)).clicked() {
                                    acts.push(Act::CallDm { key: s.key.clone(), channel, video: true });
                                }
                                if icon_button(ui, 30.0, "Start a voice call", false, |p, r, c| icons::phone(p, r, c)).clicked() {
                                    acts.push(Act::CallDm { key: s.key.clone(), channel, video: false });
                                }
                            }
                        }
                    });
                });
                let r = ui.max_rect();
                ui.painter().line_segment([pos2(r.left() - 16.0, r.bottom()), pos2(r.right() + 16.0, r.bottom())], Stroke::new(1.0f32, theme::SHADOW_LINE));
            });

        // Someone's in this conversation's call and you aren't: join it.
        if (dm_with.is_some() || group.is_some()) && !in_call_panel {
            self.dm_call_banner(ui, st, s, channel, acts);
        }
        // A group's people, down the right.
        if let Some(g) = group.as_ref().filter(|_| self.ui.members_open && !in_call_panel) {
            egui::SidePanel::right(ui.id().with("group-members"))
                .exact_width(220.0)
                .resizable(false)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(theme::SIDE))
                .show_inside(ui, |ui| self.group_members_panel(ui, st, s, g, acts));
        }

        // Composer at the bottom
        egui::TopBottomPanel::bottom(ui.id().with("composer"))
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::MAIN).inner_margin(egui::Margin { left: 16, right: 16, top: 0, bottom: 18 }))
            .show_inside(ui, |ui| match dm_with.as_ref().filter(|w| st.blocked.contains(*w)) {
                Some(w) => {
                    egui::Frame::new().fill(theme::ELEV).corner_radius(8).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("You can't send messages to someone you've blocked.").color(theme::MUTED));
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.button("Unblock").clicked() {
                                    acts.push(Act::Unblock(w.clone()));
                                }
                            });
                        });
                    });
                }
                None => self.composer(ui, st, s, &ch, acts),
            });

        // Messages
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::MAIN)).show_inside(ui, |ui| {
            let Some(log) = s.logs.get(&channel) else {
                ui.centered_and_justified(|ui| ui.label(RichText::new("Loading…").color(theme::DIM)));
                return;
            };
            // Kept steady: at the newest messages, or on the message you were reading, whatever
            // loads above or below (see `ChatScroll`).
            let memo_key = (key.clone(), channel);
            let memo = self.ui.chat_scroll.get(&memo_key).cloned();
            // Just opened: laid out unseen until what's on screen has its size (see below).
            let hidden = memo.as_ref().is_none_or(|m| !m.shown);
            self.ui.gifs_waiting = 0;
            // Where this pass draws (what the last one left).
            let drawn = memo.as_ref().and_then(|m| egui::scroll_area::State::load(ui.ctx(), m.id)).map(|st| st.offset.y).unwrap_or(0.0);
            // Where each message is: (id, top, bottom), from the top of the list.
            let mut placed: Vec<(MessageId, f32, f32)> = Vec::new();
            let panel = ui.max_rect();
            // The messages' background, under everything in them: pressing and dragging on it
            // selects text (see `select.rs`); right-click opens the message menu.
            let mut select_bg: Option<egui::Response> = None;
            let out = egui::ScrollArea::vertical()
                .id_salt(("msgs", &key, channel))
                .auto_shrink([false, false])
                .scroll_bar_visibility(if hidden {
                    egui::scroll_area::ScrollBarVisibility::AlwaysHidden
                } else {
                    egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
                })
                // Dragging selects text here, like on a web page (not scrolling, as on a phone).
                .scroll_source(egui::scroll_area::ScrollSource { drag: false, ..Default::default() })
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    if hidden {
                        ui.set_invisible();
                    }
                    // Message text is selected across the whole conversation, not label by label
                    // (names and times aren't selectable on their own).
                    ui.style_mut().interaction.selectable_labels = false;
                    let bg = ui.interact(ui.clip_rect(), ui.id().with("select"), Sense::click_and_drag());
                    if let Some(url) = self.ui.sel.input(ui, &bg, &memo_key) {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                    }
                    select_bg = Some(bg);
                    let origin = ui.cursor().top();
                    // Pictures load only near the screen. The first time this conversation is
                    // drawn, this pass's place isn't where it'll be shown (that's the newest
                    // messages, set right after), so none load from it.
                    let clip = ui.clip_rect();
                    self.ui.load_rect = Some(if memo.is_some() { clip.expand2(vec2(0.0, clip.height())) } else { Rect::NOTHING });
                    ui.add_space(8.0);
                    if log.complete || log.messages.is_empty() && log.loaded {
                        match (&dm_with, &group) {
                            (Some(w), _) => self.dm_intro(ui, s, w),
                            (None, Some(g)) => self.group_intro(ui, st, s, g),
                            _ => self.channel_intro(ui, &ch),
                        }
                    } else if log.loading {
                        ui.vertical_centered(|ui| ui.label(RichText::new("Loading older messages…").color(theme::DIM)));
                    }
                    let mut prev: Option<&Message> = None;
                    let msgs = &log.messages;
                    // The red "NEW" line above the first message that came while you weren't
                    // looking (it stays until you leave the conversation or write in it).
                    let mut new_line = s.new_since.get(&channel).copied();
                    let mut i = 0;
                    while i < msgs.len() {
                        let m = &msgs[i];
                        // Messages from people I blocked fold into one line, like Discord.
                        if st.blocked.contains(&m.author) {
                            let mut j = i;
                            while j < msgs.len() && st.blocked.contains(&msgs[j].author) {
                                j += 1;
                            }
                            let open = self.ui.shown_blocked.contains(&m.id);
                            let n = j - i;
                            ui.horizontal(|ui| {
                                ui.add_space(22.0);
                                let label = format!("{n} blocked message{}", if n == 1 { "" } else { "s" });
                                ui.label(RichText::new(label).size(13.0).color(theme::DIM));
                                let b = ui.add(egui::Label::new(RichText::new(if open { "Hide" } else { "Show" }).size(13.0).color(theme::LINK)).sense(Sense::click()));
                                if b.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                                    if open {
                                        self.ui.shown_blocked.remove(&m.id);
                                    } else {
                                        self.ui.shown_blocked.insert(m.id);
                                    }
                                }
                            });
                            if open {
                                for mm in &msgs[i..j] {
                                    let top = ui.cursor().top() - origin;
                                    self.message(ui, st, s, log, mm, false, &names, acts);
                                    placed.push((mm.id, top, ui.cursor().top() - origin));
                                }
                            }
                            prev = None;
                            i = j;
                            continue;
                        }
                        i += 1;
                        let new_day = prev.map(|p| local(p.created_at).date_naive() != local(m.created_at).date_naive()).unwrap_or(true);
                        if new_day {
                            day_divider(ui, &day_label(m.created_at));
                        }
                        let first_new = new_line.is_some_and(|since| m.id >= since && m.author != st.my_uid);
                        if first_new {
                            new_divider(ui);
                            new_line = None;
                        }
                        // After the "NEW" line the name and picture show again: who wrote it.
                        let grouped = !new_day
                            && !first_new
                            && m.reply_to.is_none()
                            && prev.map(|p| p.author == m.author && m.created_at - p.created_at < GROUP_WINDOW_MS).unwrap_or(false);
                        let top = ui.cursor().top() - origin;
                        self.message(ui, st, s, log, m, grouped, &names, acts);
                        placed.push((m.id, top, ui.cursor().top() - origin));
                        prev = Some(m);
                    }
                    for p in &log.pending {
                        self.pending_message(ui, st, &key, channel, p, acts);
                    }
                    ui.add_space(6.0);
                });
            self.ui.load_rect = None;
            // Right-click on the messages: Copy (what's selected), Copy link, and the message's own.
            if let Some(bg) = select_bg.filter(|_| self.ui.sel.menu.is_some()) {
                let open = bg.context_menu(|ui| self.message_menu(ui, st, s, log, acts)).is_some();
                if !open {
                    self.ui.sel.menu = None;
                }
            }
            let offset = steady_scroll(&out, drawn, memo.as_ref(), &placed);
            if (offset - out.state.offset.y).abs() > 0.5 {
                let mut state = out.state;
                state.offset.y = offset;
                state.store(ui.ctx(), out.id);
            }
            // Shown once the GIFs on screen have their sizes (or after a moment), so the first
            // thing you see is the conversation as it stays, not one that jumps as GIFs load.
            let since = memo.as_ref().map(|m| m.since).unwrap_or_else(Instant::now);
            let h = out.content_size.y;
            // (And laid out the same as last time: the first frame shown is the one that stays.)
            let settled = memo.as_ref().is_some_and(|m| (m.h - h).abs() < 0.5) && self.ui.gifs_waiting == 0;
            let shown = !hidden || (memo.is_some() && (settled || since.elapsed() >= REVEAL_WAIT));
            if hidden && shown {
                ui.ctx().request_repaint();
            }
            // This pass was drawn somewhere else (things above changed size, or the window did):
            // it's drawn again, right away, before anything shows, so nothing jumps.
            let misplaced = (offset - drawn).abs() > 0.5 && (out.state.offset.y - drawn).abs() <= 0.5;
            if misplaced && !hidden {
                ui.ctx().request_discard("conversation kept steady while it loads");
            }
            if !shown {
                // (Just where "Loading…" was while the messages were on their way.)
                let mut over = ui.new_child(egui::UiBuilder::new().max_rect(panel));
                over.centered_and_justified(|ui| ui.label(RichText::new("Loading…").color(theme::DIM)));
                ui.ctx().request_repaint_after(Duration::from_millis(100));
            }
            let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
            let anchor = placed.iter().find(|m| m.2 > offset).map(|m| (m.0, m.1 - offset));
            self.ui.chat_scroll.insert(memo_key, ChatScroll { id: out.id, stuck: offset >= max - 1.0, anchor, shown, since, h });
            if offset < 40.0 && !log.complete && log.loaded && !log.loading && out.content_size.y > out.inner_rect.height() {
                acts.push(Act::LoadHistory { key: key.clone(), channel, older: true });
            }
        });
    }

    /// The right-click menu over the messages (what it was opened on is `UiState::sel.menu`):
    /// Copy for selected text, Copy link and Open link over a link, then the message's own.
    fn message_menu(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, log: &ChannelLog, acts: &mut Vec<Act>) {
        ui.set_min_width(200.0);
        let Some(menu) = self.ui.sel.menu.clone() else {
            ui.close();
            return;
        };
        let mut above = false;
        if let Some(text) = &menu.copy {
            if ui.button("Copy").clicked() {
                ui.ctx().copy_text(text.clone());
                ui.close();
            }
            above = true;
        }
        if let Some(url) = &menu.link {
            if ui.button("Copy link").clicked() {
                ui.ctx().copy_text(url.clone());
                ui.close();
            }
            if ui.button("Open link").clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(url.clone()));
                ui.close();
            }
            above = true;
        }
        let Some(m) = menu.msg.and_then(|id| log.find(id)) else {
            if !above {
                ui.close();
            }
            return;
        };
        if above {
            ui.separator();
        }
        let key = s.key.clone();
        let mine = m.author == st.my_uid;
        // Moderators can't delete other people's direct messages.
        let admin = s.can(perm::MANAGE_MESSAGES) && !s.is_private(m.channel_id);
        if !m.content.is_empty() && ui.button("Copy text").clicked() {
            ui.ctx().copy_text(m.content.clone());
            ui.close();
        }
        if let Some(g) = crate::giphy::gif_links(&m.content).first() {
            let fav = crate::giphy::is_favorite(&st.settings.gif_favorites, g);
            if ui.button(if fav { "Remove GIF from favorites" } else { "Add GIF to favorites" }).clicked() {
                let shown = self.ui.gif_sizes.get(g);
                self.toggle_gif_favorite(st, crate::giphy::favorite_from_link(g, shown), acts);
                ui.close();
            }
        }
        if ui.button("Add reaction").clicked() {
            self.ui.react_picker = Some(m.id);
            self.ui.picker.search.clear();
            self.ui.picker.focus_search = true;
            ui.close();
        }
        if ui.button("Reply").clicked() {
            self.ui.reply = Some((m.id, s.member_name(&m.author)));
            ui.close();
        }
        if mine && ui.button("Edit message").clicked() {
            self.ui.editing = Some((m.id, m.content.clone()));
            ui.close();
        }
        if (mine || admin) && ui.button(RichText::new("Delete message").color(theme::LIVE)).clicked() {
            // Holding Shift deletes at once, as with the trash button.
            if ui.input(|i| i.modifiers.shift) || !st.settings.confirm_delete {
                acts.push(Act::Delete { key, id: m.id });
            } else {
                self.ui.dialog = Some(crate::app::Dialog::DeleteMessage { key, channel: m.channel_id, id: m.id });
            }
            ui.close();
        }
    }

    /// The top of a group's conversation: its picture and name.
    fn group_intro(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, g: &ingle_proto::control::Group) {
        let name = s.group_name(g, &st.my_uid);
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.vertical(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
                let p = ui.painter().clone();
                crate::calls::group_picture(self, &p, ui.ctx(), s, g, &st.my_uid, r.center(), 40.0, theme::MAIN);
                ui.add_space(8.0);
                crate::emoji::label(ui, &name, theme::bold_font(26.0), theme::TEXT, None, &mut |_| None);
                ui.label(
                    RichText::new(format!(
                        "Welcome to the beginning of this group. Only the {} people in it can see it (and the server it's on: it isn't end-to-end encrypted yet).",
                        g.members.len()
                    ))
                    .color(theme::MUTED),
                );
            });
        });
        ui.add_space(12.0);
    }

    fn dm_intro(&mut self, ui: &mut egui::Ui, s: &ServerView, with: &str) {
        let name = s.member_name(with);
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.vertical(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(80.0, 80.0), Sense::hover());
                let face = self.face(ui.ctx(), s, with);
                avatar(ui.painter(), r.center(), 40.0, with, &name, false, theme::MAIN, face);
                ui.add_space(8.0);
                ui.label(RichText::new(&name).font(theme::bold_font(26.0)).color(theme::TEXT));
                ui.label(
                    RichText::new(format!("This is the beginning of your direct message history with {name}. Other members can't see it (the server it's on can: it isn't end-to-end encrypted yet)."))
                        .color(theme::MUTED),
                );
            });
        });
        ui.add_space(12.0);
    }

    fn channel_intro(&self, ui: &mut egui::Ui, ch: &ingle_proto::control::Channel) {
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.vertical(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 32.0, theme::ELEV2);
                if ch.kind == ChannelKind::Voice {
                    icons::chat(ui.painter(), r.shrink(18.0), theme::TEXT);
                } else {
                    icons::hash(ui.painter(), r.shrink(16.0), theme::TEXT);
                }
                ui.add_space(8.0);
                let title = if ch.kind == ChannelKind::Voice { format!("{} chat", ch.name) } else { format!("Welcome to #{}!", ch.name) };
                let w = ui.available_width();
                crate::emoji::label(ui, &title, theme::bold_font(26.0), theme::TEXT, Some(w), &mut |_| None);
                let sub = if ch.kind == ChannelKind::Voice {
                    "Messages here stay with the voice channel — handy for links while you talk.".to_string()
                } else {
                    format!("This is the start of the #{} channel.", ch.name)
                };
                // The channel's topic first, when it has one (it's also in the header).
                if !ch.topic.trim().is_empty() {
                    ui.add(egui::Label::new(RichText::new(ch.topic.trim()).size(15.0).color(theme::TEXT)).wrap());
                    ui.add_space(2.0);
                }
                let w = ui.available_width();
                crate::emoji::label(ui, &sub, theme::font(14.5), theme::MUTED, Some(w), &mut |_| None);
            });
        });
        ui.add_space(12.0);
    }

    #[allow(clippy::too_many_arguments)]
    fn message(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        s: &ServerView,
        log: &ingle_engine::state::ChannelLog,
        m: &Message,
        grouped: bool,
        names: &[String],
        acts: &mut Vec<Act>,
    ) {
        let key = s.key.clone();
        let mine = m.author == st.my_uid;
        let author = s.members.get(&m.author);
        let name = s.member_name(&m.author);
        let mention = !mine && s.pings_me(&m.author, &m.content);
        let bg_idx = ui.painter().add(egui::Shape::Noop);
        let top = ui.cursor().top();
        ui.add_space(if grouped { 1.0 } else { 10.0 });

        // Reply reference
        if let Some(rid) = m.reply_to {
            let (text, ruid) = match log.find(rid) {
                Some(r) => (format!("{}  {}", s.member_name(&r.author), snippet(&r.content, 80)), Some(r.author.clone())),
                None => ("Original message was deleted or is older".to_string(), None),
            };
            ui.horizontal(|ui| {
                ui.add_space(36.0);
                let (r, _) = ui.allocate_exact_size(vec2(30.0, 16.0), Sense::hover());
                ui.painter().add(egui::Shape::line(
                    vec![pos2(r.left() + 2.0, r.bottom() + 6.0), pos2(r.left() + 2.0, r.center().y), pos2(r.right(), r.center().y)],
                    Stroke::new(2.0f32, theme::LINE),
                ));
                if let Some(u) = &ruid {
                    let (ar, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                    let face = self.face(ui.ctx(), s, u);
                    avatar(ui.painter(), ar.center(), 8.0, u, &s.member_name(u), false, theme::MAIN, face);
                }
                // (Its emoji in colour, like the message itself.)
                let ctx = ui.ctx().clone();
                let w = ui.available_width();
                crate::emoji::label_line(ui, &text, theme::font(12.5), theme::MUTED, w, &mut |id| self.custom_emoji_tex(&ctx, &key, id));
            });
        }

        let editing = self.ui.editing.as_ref().filter(|(id, _)| *id == m.id).map(|(_, t)| t.clone());
        ui.horizontal_top(|ui| {
            ui.add_space(16.0);
            if grouped {
                let (r, _) = ui.allocate_exact_size(vec2(40.0, 18.0), Sense::hover());
                if ui.rect_contains_pointer(Rect::from_min_max(pos2(ui.max_rect().left(), r.top()), pos2(ui.max_rect().right(), r.bottom() + 4.0))) {
                    let t = local(m.created_at).format("%-I:%M").to_string();
                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, t, theme::font(10.5), theme::DIM);
                }
            } else {
                let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                let face = self.face(ui.ctx(), s, &m.author);
                avatar(ui.painter(), r.center(), 20.0, &m.author, &name, false, theme::MAIN, face);
            }
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width() - 16.0);
                if !grouped {
                    ui.horizontal(|ui| {
                        let col = author.map(|a| name_color(s, &m.author, a.role)).unwrap_or(theme::TEXT);
                        ui.label(RichText::new(&name).font(theme::bold_font(14.5)).color(col));
                        ui.label(RichText::new(crate::chat::when(m.created_at)).size(11.5).color(theme::DIM));
                    });
                }
                match editing {
                    Some(mut text) => {
                        let resp = ui.add(
                            egui::TextEdit::multiline(&mut text)
                                .desired_rows(1)
                                .desired_width(f32::INFINITY)
                                .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter)),
                        );
                        if self.ui.editing.as_ref().map(|(_, t)| t != &text).unwrap_or(false) {
                            self.ui.editing = Some((m.id, text.clone()));
                        }
                        if !resp.has_focus() && !resp.lost_focus() {
                            resp.request_focus();
                        }
                        let enter = ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                        ui.label(RichText::new("escape to cancel • enter to save").size(11.0).color(theme::DIM));
                        if ui.input(|i| i.key_pressed(Key::Escape)) {
                            self.ui.editing = None;
                        } else if enter {
                            if text.trim() != m.content.trim() && !text.trim().is_empty() {
                                acts.push(Act::Edit { key: key.clone(), id: m.id, text: text.trim().to_string() });
                            }
                            self.ui.editing = None;
                        }
                    }
                    None => {
                        let gifs = crate::giphy::gif_links(&m.content);
                        let only_gif = self.only_gif(&m.content, &gifs);
                        if !m.content.is_empty() && !only_gif {
                            let ctx = ui.ctx().clone();
                            // (Out of `self` while drawn: the emoji lookup borrows it too.)
                            let mut sel = std::mem::take(&mut self.ui.sel);
                            let chat = (key.clone(), m.channel_id);
                            let at = markdown::Selectable { sel: &mut sel, chat: &chat, msg: m.id };
                            markdown::show_selectable(ui, &m.content, theme::BODY, names, &mut |id| self.custom_emoji_tex(&ctx, &key, id), at);
                            self.ui.sel = sel;
                        }
                        for g in &gifs {
                            self.gif_embed(ui, st, g, m.id, acts);
                        }
                        if m.edited_at.is_some() {
                            ui.label(RichText::new("(edited)").size(10.5).color(theme::DIM));
                        }
                    }
                }
                if !m.attachments.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for a in &m.attachments {
                            self.attachment(ui, st, s, a, acts);
                        }
                    });
                }
                if !m.reactions.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for r in &m.reactions {
                            let me_in = r.users.iter().any(|u| u == &st.my_uid);
                            let ctx = ui.ctx().clone();
                            let pill = crate::emoji_picker::reaction_pill(ui, &r.emoji, r.users.len(), me_in, &mut |id| {
                                self.custom_emoji_tex(&ctx, &key, id)
                            });
                            let names: Vec<String> = r.users.iter().map(|u| s.member_name(u)).collect();
                            // Who reacted, with the emoji drawn as itself (as text it can come out as
                            // a box).
                            let who = format!("{} reacted with {}", names.join(", "), r.emoji);
                            let ctx = ui.ctx().clone();
                            let pill = pill.on_hover_ui(|ui| {
                                crate::emoji::label(ui, &who, theme::font(13.0), theme::TEXT, Some(280.0), &mut |id| {
                                    self.custom_emoji_tex(&ctx, &key, id)
                                });
                            });
                            if pill.clicked() {
                                acts.push(Act::React { key: key.clone(), id: m.id, emoji: r.emoji.clone(), add: !me_in });
                            }
                        }
                    });
                }
            });
        });
        ui.add_space(2.0);
        let rect = Rect::from_min_max(pos2(ui.max_rect().left(), top), pos2(ui.max_rect().right(), ui.cursor().top()));
        self.ui.sel.row(&(key.clone(), m.channel_id), m.id, rect);
        let hovered = ui.rect_contains_pointer(rect) || self.ui.react_picker == Some(m.id);
        let bg = if mention {
            Color32::from_rgba_unmultiplied(0xff, 0xaa, 0x4d, 14)
        } else if hovered {
            theme::HOVER
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().set(bg_idx, egui::Shape::rect_filled(rect, 0, bg));
        if mention {
            ui.painter().line_segment([rect.left_top(), rect.left_bottom()], Stroke::new(2.0f32, theme::GLOW));
        }

        // Hover tools
        if hovered && self.ui.editing.as_ref().map(|(id, _)| *id != m.id).unwrap_or(true) {
            // Moderators can't delete other people's direct messages.
            let admin = s.can(perm::MANAGE_MESSAGES) && !s.is_private(m.channel_id);
            let n = 2 + mine as usize + (mine || admin) as usize;
            let w = n as f32 * 30.0 + 8.0;
            let tools = Rect::from_min_size(pos2(rect.right() - w - 16.0, rect.top() - 10.0), vec2(w, 32.0));
            ui.painter().rect_filled(tools, CornerRadius::same(6), theme::ELEV);
            ui.painter().rect_stroke(tools, CornerRadius::same(6), Stroke::new(1.0f32, theme::LINE), egui::StrokeKind::Inside);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tools.shrink2(vec2(4.0, 2.0))).layout(Layout::left_to_right(Align::Center)));
            child.spacing_mut().item_spacing.x = 2.0;
            if icon_button(&mut child, 28.0, "Add reaction", self.ui.react_picker == Some(m.id), |p, r, c| icons::smile(p, r, c)).clicked() {
                self.ui.react_picker = if self.ui.react_picker == Some(m.id) { None } else { Some(m.id) };
                self.ui.picker.search.clear();
                self.ui.picker.focus_search = true;
            }
            if icon_button(&mut child, 28.0, "Reply", false, |p, r, c| icons::reply(p, r, c)).clicked() {
                self.ui.reply = Some((m.id, name.clone()));
            }
            if mine && icon_button(&mut child, 28.0, "Edit", false, |p, r, c| icons::pencil(p, r, c)).clicked() {
                self.ui.editing = Some((m.id, m.content.clone()));
            }
            if mine || admin {
                // Holding Shift deletes at once (no question), and the button shows it.
                let now = ui.input(|i| i.modifiers.shift);
                let del = if now {
                    let (r, resp) = child.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                    child.painter().rect_filled(r, CornerRadius::same(6), theme::LIVE);
                    icons::trash(child.painter(), r.shrink(28.0 * 0.2), Color32::WHITE);
                    resp.on_hover_text("Delete now")
                } else {
                    icon_button(&mut child, 28.0, "Delete", false, |p, r, _| icons::trash(p, r, theme::LIVE))
                };
                if del.clicked() {
                    if now || !st.settings.confirm_delete {
                        acts.push(Act::Delete { key: key.clone(), id: m.id });
                    } else {
                        self.ui.dialog = Some(crate::app::Dialog::DeleteMessage { key: key.clone(), channel: m.channel_id, id: m.id });
                    }
                }
            }
            if self.ui.react_picker == Some(m.id) {
                let area = egui::Area::new(ui.id().with(("react", m.id)))
                    .order(egui::Order::Foreground)
                    .pivot(egui::Align2::RIGHT_TOP)
                    .fixed_pos(pos2(tools.right(), tools.bottom() + 4.0))
                    .constrain(true)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).fill(theme::SIDE).inner_margin(10).show(ui, |ui| {
                            if let Some(e) = self.emoji_picker(ui, st, Some(&key), acts) {
                                let already = m.reactions.iter().any(|r| r.emoji == e && r.users.iter().any(|u| u == &st.my_uid));
                                acts.push(Act::React { key: key.clone(), id: m.id, emoji: e, add: !already });
                                self.ui.react_picker = None;
                            }
                        });
                    });
                let clicked_outside = ui.input(|i| i.pointer.any_pressed())
                    && !area.response.contains_pointer()
                    && !tools.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default());
                if clicked_outside || ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.ui.react_picker = None;
                }
            }
        }
    }

    fn pending_message(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        key: &str,
        channel: ChannelId,
        p: &ingle_engine::state::PendingMessage,
        acts: &mut Vec<Act>,
    ) {
        ui.add_space(4.0);
        ui.horizontal_top(|ui| {
            ui.add_space(62.0);
            ui.vertical(|ui| {
                let col = if p.failed.is_some() { theme::LIVE } else { theme::DIM };
                if !p.content.is_empty() {
                    let ctx = ui.ctx().clone();
                    let k = key.to_string();
                    markdown::show(ui, &p.content, col, &mention_names(s_of(st, key)), &mut |id| self.custom_emoji_tex(&ctx, &k, id));
                }
                if !p.files.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        for (i, (name, bytes)) in p.files.iter().enumerate() {
                            let k = format!("pending:{}:{i}", p.nonce);
                            let t = ui.input(|i| i.time);
                            let (r, _) = ui.allocate_exact_size(vec2(120.0, 90.0), Sense::hover());
                            match self.images.get(ui.ctx(), &k, Some(bytes.clone()), 400).frame(t) {
                                Some((tex, size, _)) => {
                                    let (fit, uv) = fit_in(r, size);
                                    let mut mesh = egui::Mesh::with_texture(tex);
                                    mesh.add_rect_with_uv(fit, uv, Color32::WHITE.gamma_multiply(0.6));
                                    ui.painter().add(egui::Shape::mesh(mesh));
                                }
                                None => {
                                    ui.painter().rect_filled(r, CornerRadius::same(6), theme::ELEV);
                                    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, name, theme::font(11.0), theme::DIM);
                                }
                            }
                        }
                    });
                }
                if let Some(progress) = &p.progress {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(progress).size(11.5).color(theme::DIM));
                    });
                }
                if let Some(err) = &p.failed {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Not sent: {err}")).size(11.5).color(theme::LIVE));
                        if ui.small_button("Retry").clicked() {
                            acts.push(Act::DiscardPending { key: key.to_string(), channel, nonce: p.nonce.clone() });
                            let files = p.files.iter().map(|(n, b)| (n.clone(), b.to_vec())).collect();
                            acts.push(Act::Send { key: key.to_string(), channel, text: p.content.clone(), reply_to: p.reply_to, files });
                        }
                        if ui.small_button("Discard").clicked() {
                            acts.push(Act::DiscardPending { key: key.to_string(), channel, nonce: p.nonce.clone() });
                        }
                    });
                }
            });
        });
    }

    fn composer(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, ch: &ingle_proto::control::Channel, acts: &mut Vec<Act>) {
        let key = s.key.clone();
        // Typing indicator
        let now = Instant::now();
        let typers: Vec<String> = s
            .typing
            .get(&ch.id)
            .map(|m| m.iter().filter(|(u, t)| **u != st.my_uid && now.duration_since(**t) < Duration::from_secs(7)).map(|(u, _)| s.member_name(u)).collect())
            .unwrap_or_default();
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
        if !typers.is_empty() {
            let text = match typers.len() {
                1 => format!("{} is typing…", typers[0]),
                2 => format!("{} and {} are typing…", typers[0], typers[1]),
                _ => "Several people are typing…".to_string(),
            };
            let t = ui.input(|i| i.time) as f32;
            for k in 0..3 {
                let a = ((t * 5.0 - k as f32 * 0.8).sin() * 0.5 + 0.5) * 0.7 + 0.3;
                ui.painter().circle_filled(pos2(r.left() + 6.0 + k as f32 * 7.0, r.center().y), 2.5, theme::MUTED.gamma_multiply(a));
            }
            ui.painter().text(pos2(r.left() + 28.0, r.center().y), egui::Align2::LEFT_CENTER, text, theme::font(12.5), theme::MUTED);
            ui.ctx().request_repaint_after(Duration::from_millis(50));
        }

        let draft_key = (key.clone(), ch.id);
        let mut draft = self.ui.drafts.get(&draft_key).cloned().unwrap_or_default();
        let before = draft.clone();
        let connected = s.status.is_connected();
        let files_ok = s.has(FILES_FEATURE);
        egui::Frame::new().fill(theme::ELEV).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
            // Images waiting to be sent.
            let mut remove: Option<usize> = None;
            if let Some(files) = self.ui.attach.get(&draft_key).cloned().filter(|f| !f.is_empty()) {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    for (i, (name, bytes)) in files.iter().enumerate() {
                        let (r, _) = ui.allocate_exact_size(vec2(110.0, 96.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(6), theme::SIDE);
                        let img = r.shrink(6.0).with_max_y(r.bottom() - 20.0);
                        let k = format!("draft:{}:{}", ingle_proto::release::sha256_hex(bytes), bytes.len());
                        let t = ui.input(|i| i.time);
                        if let Some((tex, size, next)) = self.images.get(ui.ctx(), &k, Some(std::sync::Arc::new(bytes.clone())), 400).frame(t) {
                            let (fit, uv) = fit_in(img, size);
                            let mut mesh = egui::Mesh::with_texture(tex);
                            mesh.add_rect_with_uv(fit, uv, Color32::WHITE);
                            ui.painter().add(egui::Shape::mesh(mesh));
                            if let Some(n) = next {
                                ui.ctx().request_repaint_after(n);
                            }
                        }
                        let label = ui.fonts_mut(|f| {
                            f.layout(name.clone(), theme::font(11.0), theme::MUTED, r.width() - 10.0)
                        });
                        ui.painter().galley(pos2(r.left() + 5.0, r.bottom() - 17.0), label, theme::MUTED);
                        let x = Rect::from_min_size(r.right_top() + vec2(-24.0, 4.0), vec2(20.0, 20.0));
                        let xr = ui.interact(x, ui.id().with(("unattach", i)), Sense::click());
                        ui.painter().circle_filled(x.center(), 10.0, if xr.hovered() { theme::LIVE } else { Color32::from_black_alpha(170) });
                        icons::close(ui.painter(), x.shrink(5.0), Color32::WHITE);
                        if xr.on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                    }
                });
                ui.add_space(4.0);
            }
            if let (Some(i), Some(files)) = (remove, self.ui.attach.get_mut(&draft_key)) {
                files.remove(i);
            }
            if let Some((_, who)) = &self.ui.reply {
                let who = who.clone();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Replying to ").size(12.5).color(theme::MUTED));
                    ui.label(RichText::new(who).font(theme::bold_font(12.5)).color(theme::TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, 20.0, "Cancel reply", false, |p, r, c| icons::close(p, r, c)).clicked() {
                            self.ui.reply = None;
                        }
                    });
                });
            }
            // Typing "@": people, roles and @everyone to pick from (arrows + Enter/Tab, or click).
            // Typing ":smi": emoji. Typing "/giphy cats": GIFs.
            let composer_id = egui::Id::new(("composer", &key, ch.id));
            // Typing "/giphy" (or "/giphy cats", or /klipy) brings up the GIF search, the words
            // going into its search box.
            if let Some((q, from)) = gif_query(&draft) {
                self.open_gif_picker(q, from);
                draft.clear();
            }
            if !self.ui.gif_picker && (self.ui.giphy.searched.is_some() || self.ui.klipy.searched.is_some()) {
                self.ui.giphy.reset();
                self.ui.klipy.reset();
            }
            // Typing "/": add-ons' commands (Tab fills in the first, a click any).
            if let Some(partial) = draft.strip_prefix('/').filter(|p| !p.contains(char::is_whitespace)) {
                let cmds = self.plugins.commands_like(partial);
                if !cmds.is_empty() {
                    let mut fill = None;
                    if ui.ctx().memory(|m| m.focused()) == Some(composer_id) && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
                        fill = Some(cmds[0].0.clone());
                    }
                    egui::Frame::new().fill(theme::ELEV).corner_radius(8).inner_margin(6).stroke(Stroke::new(1.0f32, theme::LINE)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.label(RichText::new("ADD-ON COMMANDS").font(theme::bold_font(11.0)).color(theme::DIM));
                        for (name, help) in &cmds {
                            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                            if resp.hovered() {
                                ui.painter().rect_filled(r, CornerRadius::same(5), theme::HOVER);
                            }
                            let g = ui.painter().layout_no_wrap(format!("/{name}"), theme::bold_font(14.0), theme::TEXT);
                            let w = g.size().x;
                            ui.painter().galley(pos2(r.left() + 8.0, r.center().y - g.size().y / 2.0), g, theme::TEXT);
                            ui.painter().text(pos2(r.left() + 18.0 + w, r.center().y), egui::Align2::LEFT_CENTER, help, theme::font(12.5), theme::DIM);
                            if resp.clicked() {
                                fill = Some(name.clone());
                            }
                        }
                    });
                    if let Some(name) = fill {
                        draft = format!("/{name} ");
                        // Put the cursor at the end, after the space.
                        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), composer_id).unwrap_or_default();
                        let end = draft.chars().count();
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
                        state.store(ui.ctx(), composer_id);
                        ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                    }
                }
            }
            let suggestions = {
                let m = mention_suggestions(s, &draft);
                if m.is_empty() { emoji_suggestions(s, &draft, st.settings.emoji_tone) } else { m }
            };
            if suggestions.is_empty() {
                self.ui.mention_sel = 0;
            } else {
                self.ui.mention_sel = self.ui.mention_sel.min(suggestions.len() - 1);
                let mut pick = None;
                if ui.ctx().memory(|m| m.focused()) == Some(composer_id) {
                    ui.input_mut(|i| {
                        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                            self.ui.mention_sel = (self.ui.mention_sel + 1) % suggestions.len();
                        }
                        if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                            self.ui.mention_sel = (self.ui.mention_sel + suggestions.len() - 1) % suggestions.len();
                        }
                        if i.consume_key(Modifiers::NONE, Key::Enter) || i.consume_key(Modifiers::NONE, Key::Tab) {
                            pick = Some(self.ui.mention_sel);
                        }
                    });
                }
                let header = match (&suggestions[0].emoji, emoji_token(&draft)) {
                    (Some(_), Some((_, p))) => format!("EMOJI MATCHING :{}", p.to_uppercase()),
                    _ => "MENTION".to_string(),
                };
                egui::Frame::new().fill(theme::ELEV).corner_radius(8).inner_margin(6).stroke(Stroke::new(1.0f32, theme::LINE)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(header).font(theme::bold_font(11.0)).color(theme::DIM));
                    for (i, sg) in suggestions.iter().enumerate() {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
                        if resp.hovered() {
                            self.ui.mention_sel = i;
                        }
                        if i == self.ui.mention_sel {
                            ui.painter().rect_filled(r, CornerRadius::same(5), theme::HOVER);
                        }
                        let mut x = r.left() + 8.0;
                        if let Some(e) = &sg.emoji {
                            let ctx = ui.ctx().clone();
                            let er = Rect::from_center_size(pos2(x + 11.0, r.center().y), vec2(22.0, 22.0));
                            crate::emoji::paint(ui.painter(), er, e, &mut |id| self.custom_emoji_tex(&ctx, &key, id));
                            x += 30.0;
                        }
                        if let Some(uid) = &sg.uid {
                            let face = self.face(ui.ctx(), s, uid);
                            avatar(ui.painter(), pos2(x + 10.0, r.center().y), 10.0, uid, &sg.label, false, theme::ELEV, face);
                            x += 28.0;
                        }
                        let galley = ui.painter().layout_no_wrap(sg.label.clone(), theme::bold_font(14.0), sg.color);
                        let w = galley.size().x;
                        ui.painter().galley(pos2(x, r.center().y - galley.size().y / 2.0), galley, sg.color);
                        ui.painter().text(pos2(x + w + 10.0, r.center().y), egui::Align2::LEFT_CENTER, &sg.detail, theme::font(12.5), theme::DIM);
                        if resp.clicked() {
                            pick = Some(i);
                        }
                    }
                });
                if let Some(i) = pick {
                    if suggestions[i].emoji.is_some() {
                        complete_emoji(&mut draft, &suggestions[i].insert);
                    } else {
                        complete_mention(&mut draft, &suggestions[i].insert);
                    }
                    self.ui.mention_sel = 0;
                    // Put the cursor after the name.
                    let mut state = egui::text_edit::TextEditState::load(ui.ctx(), composer_id).unwrap_or_default();
                    let end = draft.chars().count();
                    state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
                    state.store(ui.ctx(), composer_id);
                    ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                }
            }
            ui.horizontal(|ui| {
                // (The hint is plain text, which can't show emoji: they're left out of the name.)
                let name = crate::emoji::without_emoji(&ch.name);
                let hint = if !connected {
                    "Reconnecting…".to_string()
                } else if name.is_empty() {
                    "Send a message".to_string()
                } else if ch.kind == ChannelKind::Voice || ch.name.starts_with('@') || s.group(ch.id).is_some() {
                    format!("Message {name}")
                } else {
                    format!("Message #{name}")
                };
                if files_ok
                    && icon_button(ui, 30.0, "Attach images (or paste or drop them here)", false, |p, r, c| icons::plus(p, r, c, 2.0)).clicked()
                {
                    crate::images::pick_async(crate::images::PickFor::Attach(key.clone(), ch.id), true, ui.ctx());
                }
                // Room for the GIF, emoji and send buttons on the right.
                let width = ui.available_width() - 114.0;
                let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                    let job = composer_job(buf.as_str(), wrap);
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                // A right-click inside selected text keeps it selected (egui would move the
                // cursor to the click), so the menu can cut or copy it.
                let kept_selection = if ui.input(|i| i.pointer.secondary_pressed()) {
                    egui::text_edit::TextEditState::load(ui.ctx(), composer_id).and_then(|s| s.cursor.char_range()).filter(|r| !r.is_empty())
                } else {
                    None
                };
                let out = ui
                    .add_enabled_ui(connected, |ui| {
                        egui::TextEdit::multiline(&mut draft)
                            .id(composer_id)
                            .hint_text(RichText::new(hint).color(theme::DIM))
                            .desired_rows(1)
                            .desired_width(width)
                            .frame(false)
                            .font(theme::font(14.5))
                            .layouter(&mut layouter)
                            .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter))
                            .show(ui)
                    })
                    .inner;
                let resp = out.response.clone();
                let pointer_char = |ui: &egui::Ui| {
                    ui.input(|i| i.pointer.interact_pos()).map(|p| out.galley.cursor_from_pos(p - out.galley_pos).index)
                };
                if let Some(sel) = kept_selection {
                    let (a, b) = (sel.primary.index.min(sel.secondary.index), sel.primary.index.max(sel.secondary.index));
                    if pointer_char(ui).map(|at| a <= at && at <= b).unwrap_or(false) {
                        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), composer_id).unwrap_or_default();
                        state.cursor.set_char_range(Some(sel));
                        state.store(ui.ctx(), composer_id);
                    }
                }
                // Spellcheck: a red squiggle under misspelled words (not the one being typed).
                let spell_on = st.settings.spellcheck && self.ui.spell.ready(&st.settings.spell_language, ui.ctx());
                if spell_on && !out.galley.text().is_empty() {
                    self.ui.spell.know(s, &st.settings.spell_words);
                    let typing_at = out.cursor_range.filter(|_| resp.has_focus()).map(|c| c.primary.index);
                    let bad = self.ui.spell.misspelled(out.galley.text(), typing_at);
                    let painter = ui.painter().with_clip_rect(out.text_clip_rect.expand(3.0));
                    crate::spell::paint_squiggles(&painter, &out.galley, out.galley_pos, &bad);
                }
                // Right-click: what's under the pointer decides the menu (corrections for a
                // misspelled word, Cut and Copy for selected text).
                if resp.secondary_clicked() {
                    let text = out.galley.text().to_string();
                    let chars: Vec<char> = text.chars().collect();
                    let selection = egui::text_edit::TextEditState::load(ui.ctx(), composer_id)
                        .and_then(|s| s.cursor.char_range())
                        .map(|r| r.primary.index.min(r.secondary.index)..r.primary.index.max(r.secondary.index))
                        .filter(|r| !r.is_empty() && r.end <= chars.len());
                    let word = pointer_char(ui)
                        .filter(|_| spell_on)
                        .and_then(|at| crate::spell::word_at(&text, at))
                        .map(|r| (r.clone(), chars[r].iter().collect::<String>()))
                        .filter(|(_, w)| !self.ui.spell.correct(w))
                        .map(|(r, w)| {
                            let fixes = self.ui.spell.suggest(&w);
                            (r, w, fixes)
                        });
                    self.ui.composer_menu = Some(ComposerMenu { word, selection });
                }
                resp.context_menu(|ui| {
                    ui.set_min_width(220.0);
                    let menu = self.ui.composer_menu.clone().unwrap_or_default();
                    let put_cursor = |ui: &egui::Ui, at: usize| {
                        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), composer_id).unwrap_or_default();
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(at))));
                        state.store(ui.ctx(), composer_id);
                        ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                    };
                    if let Some((range, word, fixes)) = &menu.word {
                        if fixes.is_empty() {
                            ui.label(RichText::new("No spelling suggestions").color(theme::DIM));
                        }
                        for fix in fixes {
                            let still_there = draft.chars().skip(range.start).take(range.len()).collect::<String>() == *word;
                            if ui.button(RichText::new(fix).strong()).clicked() && still_there {
                                replace_chars(&mut draft, range.clone(), fix);
                                put_cursor(ui, range.start + fix.chars().count());
                                ui.close();
                            }
                        }
                        if ui.button("Add to dictionary").clicked() {
                            let mut set = st.settings.clone();
                            if !set.spell_words.iter().any(|w| w.eq_ignore_ascii_case(word)) {
                                set.spell_words.push(word.clone());
                            }
                            acts.push(Act::Settings(set));
                            ui.close();
                        }
                        ui.separator();
                    }
                    if menu_check(ui, "Spellcheck", st.settings.spellcheck, false).clicked() {
                        acts.push(Act::Settings(ingle_engine::config::Settings { spellcheck: !st.settings.spellcheck, ..st.settings.clone() }));
                        ui.close();
                    }
                    ui.menu_button("Languages", |ui| {
                        ui.set_min_width(200.0);
                        let current = crate::spell::lang(&st.settings.spell_language).code;
                        for l in crate::spell::LANGS {
                            if menu_check(ui, l.name, l.code == current, true).clicked() {
                                acts.push(Act::Settings(ingle_engine::config::Settings {
                                    spell_language: l.code.to_string(),
                                    spellcheck: true,
                                    ..st.settings.clone()
                                }));
                                ui.close();
                            }
                        }
                    });
                    ui.separator();
                    if let Some(sel) = menu.selection.clone().filter(|r| r.end <= draft.chars().count()) {
                        let picked: String = draft.chars().skip(sel.start).take(sel.len()).collect();
                        if ui.add(egui::Button::new("Cut").shortcut_text("Ctrl+X")).clicked() {
                            ui.ctx().copy_text(picked.clone());
                            replace_chars(&mut draft, sel.clone(), "");
                            put_cursor(ui, sel.start);
                            ui.close();
                        }
                        if ui.add(egui::Button::new("Copy").shortcut_text("Ctrl+C")).clicked() {
                            ui.ctx().copy_text(picked);
                            ui.close();
                        }
                    }
                    if ui.add(egui::Button::new("Paste").shortcut_text("Ctrl+V")).clicked() {
                        // Text goes in at the cursor (over selected text); a picture is attached.
                        match arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()).filter(|t| !t.is_empty()) {
                            Some(t) => {
                                let at = egui::text_edit::TextEditState::load(ui.ctx(), composer_id)
                                    .and_then(|s| s.cursor.char_range())
                                    .map(|r| r.primary.index.min(r.secondary.index)..r.primary.index.max(r.secondary.index))
                                    .unwrap_or_else(|| { let n = draft.chars().count(); n..n });
                                let at = at.start.min(draft.chars().count())..at.end.min(draft.chars().count());
                                replace_chars(&mut draft, at.clone(), &t);
                                put_cursor(ui, at.start + t.chars().count());
                            }
                            None if files_ok => {
                                if let Some(png) = crate::images::clipboard_png() {
                                    add_attachment(&mut self.ui.attach, &draft_key, "pasted-image.png".into(), png, acts);
                                }
                                ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                            }
                            None => {}
                        }
                        ui.close();
                    }
                });
                // Colour emoji over the spaces the layout left for them.
                {
                    let found = crate::emoji::find_all(out.galley.text());
                    if !found.is_empty() {
                        let starts: Vec<usize> = found.iter().map(|f| f.0).collect();
                        let ctx = ui.ctx().clone();
                        let painter = ui.painter().with_clip_rect(out.text_clip_rect);
                        for (r, f) in crate::emoji::placed(&out.galley, &starts).into_iter().zip(&found) {
                            if r.is_positive() {
                                crate::emoji::paint(&painter, r.translate(out.galley_pos.to_vec2()), &f.2, &mut |id| {
                                    self.custom_emoji_tex(&ctx, &key, id)
                                });
                            }
                        }
                    }
                }
                if self.ui.editing.is_none()
                    && !self.ui.composer_emoji
                    && !self.ui.gif_picker
                    && self.ui.react_picker.is_none()
                    && self.ui.dialog.is_none()
                    && self.ui.viewer.is_none()
                    && !resp.has_focus()
                    && ui.ctx().memory(|m| m.focused().is_none())
                {
                    resp.request_focus();
                }
                // Ctrl+V with an image on the clipboard: egui only reports text pastes, so
                // look at the keys ourselves.
                if files_ok && resp.has_focus() {
                    let down = ui.input(|i| i.modifiers.command) && paste_keys_down();
                    if down && !self.ui.paste_down {
                        if let Some(png) = crate::images::clipboard_png() {
                            add_attachment(&mut self.ui.attach, &draft_key, "pasted-image.png".into(), png, acts);
                        }
                    }
                    self.ui.paste_down = down;
                }
                let tip = if s.has(ingle_proto::control::KLIPY_FEATURE) && !s.info.klipy_key.is_empty() {
                    "Search GIFs (or type /giphy or /klipy)"
                } else {
                    "Search GIFs (or type /giphy)"
                };
                let gb = icon_button(ui, 30.0, tip, self.ui.gif_picker, |p, r, c| icons::gif(p, r.shrink(1.0), c));
                if gb.clicked() {
                    if self.ui.gif_picker {
                        self.ui.gif_picker = false;
                    } else {
                        self.open_gif_picker(String::new(), None);
                    }
                }
                let eb = icon_button(ui, 30.0, "Emoji", self.ui.composer_emoji, |p, r, c| icons::smile(p, r, c));
                if eb.clicked() {
                    self.ui.composer_emoji = !self.ui.composer_emoji;
                    self.ui.gif_picker = false;
                    self.ui.picker.search.clear();
                    self.ui.picker.focus_search = true;
                }
                let files = self.ui.attach.get(&draft_key).cloned().unwrap_or_default();
                // Send, at the far right (lit up once there's something to send).
                let ready = connected && (!draft.trim().is_empty() || !files.is_empty());
                let sb = send_button(ui, ready);
                // Any Enter press counts (even one egui takes for a repeat because it missed a key
                // release, e.g. to a file dialog).
                let enter = ui.input(|i| {
                    i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift))
                });
                let send = resp.has_focus() && enter || sb.clicked() && connected;
                // "/giphy …", "/klipy …" (or a bare "/gif") is a GIF search, never a message.
                let gif_command = gif_query(draft.trim()).or_else(|| (draft.trim() == "/gif").then(|| (String::new(), None)));
                // "/roll 2d6": an add-on's command. It answers on its own thread, and the answer
                // is sent here as my message.
                let addon_command = self.plugins.command_in(draft.trim());
                if send && let Some((addon, name, args)) = addon_command {
                    let reply_to = self.ui.reply.take().map(|(id, _)| id);
                    acts.push(Act::AddonCommand { addon, name, args, key: key.clone(), channel: ch.id, reply_to });
                    draft.clear();
                    resp.request_focus();
                } else if send && let Some((q, from)) = gif_command {
                    self.open_gif_picker(q, from);
                    draft.clear();
                } else if send && (!draft.trim().is_empty() || !files.is_empty()) {
                    let reply_to = self.ui.reply.take().map(|(id, _)| id);
                    let text = expand_shortcodes(s, draft.trim(), st.settings.emoji_tone);
                    acts.push(Act::Send { key: key.clone(), channel: ch.id, text, reply_to, files });
                    self.ui.attach.remove(&draft_key);
                    draft.clear();
                    resp.request_focus();
                } else if sb.clicked() {
                    resp.request_focus();
                }
                if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.ui.reply = None;
                }
                if self.ui.gif_picker {
                    let area = egui::Area::new(egui::Id::new(("composer-gif", &key, ch.id)))
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::RIGHT_BOTTOM)
                        .fixed_pos(pos2(sb.rect.right() + 6.0, gb.rect.top() - 12.0))
                        .constrain(true)
                        .show(ui.ctx(), |ui| egui::Frame::popup(ui.style()).fill(theme::SIDE).inner_margin(10).show(ui, |ui| self.gif_picker(ui, st, s, acts)).inner);
                    if let Some(url) = area.inner {
                        let reply_to = self.ui.reply.take().map(|(id, _)| id);
                        // A favorite picture (a meme, say) goes as a picture: from its copy here.
                        if crate::giphy::local_favorite(&url).is_some() {
                            match crate::giphy::kept_picture(&st.settings.gif_favorites, &url) {
                                Some(file) => acts.push(Act::Send { key: key.clone(), channel: ch.id, text: String::new(), reply_to, files: vec![file] }),
                                None => acts.push(Act::Toast("That favorite picture isn't on this PC any more.".into(), true)),
                            }
                        } else {
                            acts.push(Act::Send { key: key.clone(), channel: ch.id, text: url, reply_to, files: Vec::new() });
                        }
                        self.ui.gif_picker = false;
                        ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                    }
                    let clicked_outside = ui.input(|i| i.pointer.any_pressed())
                        && !area.response.contains_pointer()
                        && !gb.rect.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default());
                    if clicked_outside || ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.ui.gif_picker = false;
                        ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                    }
                }
                if self.ui.composer_emoji {
                    let area = egui::Area::new(egui::Id::new(("composer-emoji", &key, ch.id)))
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::RIGHT_BOTTOM)
                        .fixed_pos(pos2(sb.rect.right() + 6.0, eb.rect.top() - 12.0))
                        .constrain(true)
                        .show(ui.ctx(), |ui| {
                            egui::Frame::popup(ui.style()).fill(theme::SIDE).inner_margin(10).show(ui, |ui| self.emoji_picker(ui, st, Some(&key), acts)).inner
                        });
                    if let Some(e) = area.inner {
                        // In at the cursor.
                        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), composer_id).unwrap_or_default();
                        let at = state.cursor.char_range().map(|r| r.primary.index).unwrap_or(draft.chars().count()).min(draft.chars().count());
                        let byte = draft.char_indices().nth(at).map(|(b, _)| b).unwrap_or(draft.len());
                        draft.insert_str(byte, &e);
                        let end = at + e.chars().count();
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(end))));
                        state.store(ui.ctx(), composer_id);
                        if !ui.input(|i| i.modifiers.shift) {
                            self.ui.composer_emoji = false;
                        }
                        ui.ctx().memory_mut(|m| m.request_focus(composer_id));
                    }
                    let clicked_outside = ui.input(|i| i.pointer.any_pressed())
                        && !area.response.contains_pointer()
                        && !eb.rect.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default());
                    if clicked_outside || ui.input(|i| i.key_pressed(Key::Escape)) {
                        self.ui.composer_emoji = false;
                    }
                }
            });
        });
        if draft != before {
            if !draft.is_empty() && self.ui.last_typing.map(|t| t.elapsed() > Duration::from_secs(3)).unwrap_or(true) {
                self.ui.last_typing = Some(Instant::now());
                acts.push(Act::Typing { key: key.clone(), channel: ch.id });
            }
            self.ui.drafts.insert(draft_key, draft);
        }
    }
}

/// What the message box's right-click menu was opened on.
#[derive(Clone, Default)]
pub struct ComposerMenu {
    /// The misspelled word under the pointer: where it is (chars), the word, its corrections.
    pub word: Option<(std::ops::Range<usize>, String, Vec<String>)>,
    /// Selected text (chars), for Cut and Copy.
    pub selection: Option<std::ops::Range<usize>>,
}

/// Put `with` in place of the characters `range` of `text`.
fn replace_chars(text: &mut String, range: std::ops::Range<usize>, with: &str) {
    let byte = |i: usize| text.char_indices().nth(i).map(|(b, _)| b).unwrap_or(text.len());
    let (a, b) = (byte(range.start), byte(range.end));
    text.replace_range(a..b, with);
}

/// A menu row with a checkbox (or, for one of several, a round radio mark) on its right, like
/// Discord's.
fn menu_check(ui: &mut egui::Ui, label: &str, on: bool, radio: bool) -> egui::Response {
    // Room on the right for the box.
    let resp = ui.add(egui::Button::new(label).right_text("      "));
    let b = Rect::from_center_size(pos2(resp.rect.right() - 16.0, resp.rect.center().y), vec2(16.0, 16.0));
    let p = ui.painter();
    match (radio, on) {
        (false, true) => {
            p.rect_filled(b, CornerRadius::same(4), theme::EMBER);
            icons::tick(p, b, Color32::WHITE);
        }
        (false, false) => {
            p.rect_stroke(b, CornerRadius::same(4), Stroke::new(1.5f32, theme::MUTED), egui::StrokeKind::Inside);
        }
        (true, on) => {
            p.circle_stroke(b.center(), 7.0, Stroke::new(1.5f32, if on { theme::EMBER } else { theme::MUTED }));
            if on {
                p.circle_filled(b.center(), 3.8, theme::EMBER);
            }
        }
    }
    resp
}

/// The paper plane at the right of the message box.
fn send_button(ui: &mut egui::Ui, ready: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(30.0, 30.0), if ready { Sense::click() } else { Sense::hover() });
    if ready && resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(6), theme::ELEV2);
    }
    let color = if !ready {
        theme::DIM
    } else if resp.hovered() {
        theme::EMBER2
    } else {
        theme::EMBER
    };
    icons::send(ui.painter(), rect.shrink(7.0), color);
    if ready {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Send message (Enter)")
    } else {
        resp.on_hover_text("Send message")
    }
}

/// Add an image to a channel's draft, after checking it.
pub fn add_attachment(
    attach: &mut std::collections::HashMap<(String, ChannelId), Vec<(String, Vec<u8>)>>,
    draft: &(String, ChannelId),
    name: String,
    bytes: Vec<u8>,
    acts: &mut Vec<Act>,
) {
    if let Err(e) = crate::images::check_attachment(&name, &bytes) {
        acts.push(Act::Toast(e, true));
        return;
    }
    let list = attach.entry(draft.clone()).or_default();
    if list.len() >= ingle_proto::control::MAX_ATTACHMENTS_PER_MESSAGE {
        acts.push(Act::Toast("You can send up to 10 images at a time.".into(), true));
        return;
    }
    list.push((name, bytes));
}

/// V and Ctrl (or Cmd) are held right now.
fn paste_keys_down() -> bool {
    use device_query::{DeviceQuery, DeviceState, Keycode};
    let keys = DeviceState::new().get_keys();
    keys.contains(&Keycode::V)
}

/// Fit an image of `size` inside `r`, centred, keeping its shape.
pub fn fit_in(r: Rect, size: egui::Vec2) -> (Rect, Rect) {
    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if size.x <= 0.0 || size.y <= 0.0 {
        return (r, uv);
    }
    let s = (r.width() / size.x).min(r.height() / size.y);
    (Rect::from_center_size(r.center(), size * s), uv)
}

/// What goes when a message is deleted, besides its words: its pictures and reactions, and
/// replies that will say it's gone.
fn goes_with_it(m: &Message, log: &ChannelLog) -> Option<String> {
    let pictures = m.attachments.len() + crate::giphy::gif_links(&m.content).len();
    let reactions: usize = m.reactions.iter().map(|r| r.users.len()).sum();
    let replies = log.messages.iter().filter(|x| x.reply_to == Some(m.id)).count();
    let mut parts = Vec::new();
    match pictures {
        0 => {}
        1 => parts.push("Its picture".to_string()),
        n => parts.push(format!("Its {n} pictures")),
    }
    match reactions {
        0 => {}
        1 => parts.push("1 reaction".to_string()),
        n => parts.push(format!("{n} reactions")),
    }
    let mut out = String::new();
    if !parts.is_empty() {
        let one = parts.len() == 1 && (pictures == 1 || reactions == 1);
        out = format!("{} {} with it.", parts.join(" and "), if one { "goes" } else { "go" });
    }
    if replies > 0 {
        if !out.is_empty() {
            out.push(' ');
        }
        out += &match replies {
            1 => "The reply to it will say it was deleted.".to_string(),
            n => format!("The {n} replies to it will say it was deleted."),
        };
    }
    (!out.is_empty()).then_some(out)
}

impl IngleApp {
    /// "Delete message": like Discord's, but the message shows as it is (pictures and all), it
    /// says what goes with it and, for someone else's, whose it is; Enter deletes, Escape keeps.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn delete_dialog(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        key: &str,
        channel: ChannelId,
        id: MessageId,
        acts: &mut Vec<Act>,
        close: &mut bool,
    ) {
        ui.set_width(460.0);
        // Gone already (deleted elsewhere, or the server's gone): nothing to ask.
        let Some((s, log, m)) = st.server(key).and_then(|s| s.logs.get(&channel).and_then(|l| l.find(id).map(|m| (s, l, m)))) else {
            *close = true;
            return;
        };
        let mine = m.author == st.my_uid;
        let place = if s.is_private(channel) {
            "this conversation".to_string()
        } else {
            s.channel(channel).map(|c| format!("#{}", c.name)).unwrap_or_else(|| "the channel".into())
        };
        crate::dialogs::title(ui, "Delete message");
        let ask = if mine {
            format!("Are you sure you want to delete this message? It goes for everyone in {place}.")
        } else {
            format!("This is {}'s message. Deleting it takes it away for everyone in {place}.", s.member_name(&m.author))
        };
        ui.label(RichText::new(ask).color(theme::MUTED));
        ui.add_space(12.0);
        egui::Frame::new()
            .fill(theme::MAIN)
            .stroke(Stroke::new(1.0f32, theme::LINE))
            .corner_radius(8)
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
                    self.message_preview(ui, st, s, log, m);
                });
            });
        if let Some(goes) = goes_with_it(m, log) {
            ui.add_space(8.0);
            ui.label(RichText::new(goes).size(12.5).color(theme::MUTED));
        }
        ui.add_space(10.0);
        ui.label(RichText::new("TIP").font(theme::bold_font(11.0)).color(theme::ONLINE));
        ui.label(RichText::new("Hold Shift when you click Delete to delete straight away, without this question.").size(12.5).color(theme::BODY));
        ui.add_space(14.0);
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        ui.horizontal(|ui| {
            let w = (ui.available_width() - ui.spacing().item_spacing.x) / 2.0;
            if ui.add(egui::Button::new("Cancel").min_size(vec2(w, 36.0))).clicked() {
                *close = true;
            }
            if ui.add(theme::danger_button("Delete").min_size(vec2(w, 36.0))).clicked() || enter {
                acts.push(Act::Delete { key: key.to_string(), id });
                *close = true;
            }
        });
    }

    /// A message as the chat shows it, for the delete question (clicking it does nothing).
    fn message_preview(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, log: &ChannelLog, m: &Message) {
        let key = s.key.clone();
        let name = s.member_name(&m.author);
        if let Some(rid) = m.reply_to {
            let text = match log.find(rid) {
                Some(r) => format!("Replying to {}: {}", s.member_name(&r.author), snippet(&r.content, 60)),
                None => "Replying to a message that was deleted or is older".to_string(),
            };
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(theme::DIM)).truncate());
            ui.add_space(4.0);
        }
        ui.horizontal_top(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
            let face = self.face(ui.ctx(), s, &m.author);
            avatar(ui.painter(), r.center(), 20.0, &m.author, &name, false, theme::MAIN, face);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width());
                ui.horizontal(|ui| {
                    let col = s.members.get(&m.author).map(|a| name_color(s, &m.author, a.role)).unwrap_or(theme::TEXT);
                    ui.label(RichText::new(&name).font(theme::bold_font(14.5)).color(col));
                    ui.label(RichText::new(when(m.created_at)).size(11.5).color(theme::DIM));
                });
                let gifs = crate::giphy::gif_links(&m.content);
                let only_gif = self.only_gif(&m.content, &gifs);
                if !m.content.is_empty() && !only_gif {
                    let ctx = ui.ctx().clone();
                    let names = mention_names(s);
                    markdown::show(ui, &m.content, theme::BODY, &names, &mut |id| self.custom_emoji_tex(&ctx, &key, id));
                }
                for g in &gifs {
                    self.gif_embed_sized(ui, g, vec2(240.0, 160.0), None);
                }
                if m.edited_at.is_some() {
                    ui.label(RichText::new("(edited)").size(10.5).color(theme::DIM));
                }
                if !m.attachments.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                        for a in &m.attachments {
                            self.attachment_sized(ui, s, a, vec2(240.0, 160.0), false, &[]);
                        }
                    });
                }
                if !m.reactions.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for r in &m.reactions {
                            let me_in = r.users.iter().any(|u| u == &st.my_uid);
                            let ctx = ui.ctx().clone();
                            let _ = crate::emoji_picker::reaction_pill(ui, &r.emoji, r.users.len(), me_in, &mut |id| {
                                self.custom_emoji_tex(&ctx, &key, id)
                            });
                        }
                    });
                }
            });
        });
    }

    /// An image or GIF in a message; click to see it full size. Pointed at, it has the
    /// favorite star in its corner, like a GIF from GIPHY or KLIPY: a favorite picture (a meme,
    /// say) is kept on this PC, in the GIF picker's Favorites, ready to send on any server.
    fn attachment(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, a: &Attachment, acts: &mut Vec<Act>) {
        let Some((url, bytes)) = self.attachment_sized(ui, s, a, vec2(400.0, 300.0), true, &st.settings.gif_favorites) else { return };
        match st.settings.gif_favorites.iter().find(|f| f.url == url) {
            // Starred again: out it goes.
            Some(f) => self.toggle_gif_favorite(st, f.clone(), acts),
            None => match crate::giphy::favorite_picture(&bytes, &a.name, Some((a.width, a.height))) {
                Ok(f) => self.toggle_gif_favorite(st, f, acts),
                Err(e) => acts.push(Act::Toast(e, true)),
            },
        }
    }

    /// An image or GIF at most `max` big; `open`: clicking shows it full size, and pointing at
    /// it shows the favorite star (filled when it's one of `favs`). Returns the picture's
    /// favorite link and what's in it when its star was clicked.
    fn attachment_sized(
        &mut self,
        ui: &mut egui::Ui,
        s: &ServerView,
        a: &Attachment,
        max: egui::Vec2,
        open: bool,
        favs: &[crate::giphy::GifFavorite],
    ) -> Option<(String, std::sync::Arc<Vec<u8>>)> {
        let mut starred = None;
        let (w, h) = if a.width > 0 && a.height > 0 { (a.width as f32, a.height as f32) } else { (320.0, 200.0) };
        let max = vec2(max.x.min(ui.available_width().max(120.0)), max.y);
        // Small images keep their size; big ones shrink to fit. Tiny ones get a minimum.
        let scale = (max.x / w).min(max.y / h).min(1.0);
        let size = vec2((w * scale).max(48.0), (h * scale).max(48.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        // Far from the screen: just its space. It's fetched when it comes near, so opening a
        // conversation doesn't load every picture in it at once.
        if self.ui.load_rect.is_some_and(|r| !r.intersects(rect)) && !self.images.has(&a.id) {
            ui.painter().rect_filled(rect, CornerRadius::same(6), theme::ELEV);
            return None;
        }
        let state = self.engine.file(&s.key, &a.id);
        let bytes = match &state {
            ingle_engine::files::FileState::Ready(b) => Some(b.clone()),
            _ => None,
        };
        let t = ui.input(|i| i.time);
        let tex = self.images.get(ui.ctx(), &a.id, bytes.clone(), crate::images::max_side_for_chat());
        match (tex.frame(t), &state) {
            (Some((id, isize, next)), _) => {
                let (fit, uv) = fit_in(rect, isize);
                ui.painter().add(
                    egui::epaint::RectShape::filled(fit, CornerRadius::same(6), Color32::WHITE).with_texture(id, uv),
                );
                if let Some(n) = next {
                    ui.ctx().request_repaint_after(n);
                }
                // The favorite star while it's pointed at (its link is worked out from what's
                // in it, once).
                if let (true, Some(b)) = (open && ui.rect_contains_pointer(fit), &bytes) {
                    let k = (s.key.clone(), a.id.clone());
                    let url = match self.ui.picture_favs.get(&k) {
                        Some(u) => u.clone(),
                        None => {
                            let u = crate::giphy::picture_favorite_link(b);
                            self.ui.picture_favs.insert(k, u.clone());
                            u
                        }
                    };
                    if !url.is_empty() && gif_star(ui, fit, ("picture-star", &a.id), crate::giphy::is_favorite(favs, &url)) {
                        starred = Some((url, b.clone()));
                    }
                }
            }
            (None, ingle_engine::files::FileState::Failed(e)) => {
                ui.painter().rect_filled(rect, CornerRadius::same(6), theme::ELEV);
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Image unavailable", theme::font(12.0), theme::DIM);
                let _ = e;
            }
            _ => {
                ui.painter().rect_filled(rect, CornerRadius::same(6), theme::ELEV);
                let c = rect.center();
                let a0 = t as f32 * 4.0;
                ui.painter().add(egui::Shape::line(
                    (0..12).map(|i| c + 9.0 * egui::Vec2::angled(a0 + i as f32 * 0.4)).collect(),
                    Stroke::new(2.5f32, theme::DIM),
                ));
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            }
        }
        let resp = resp.on_hover_text(format!("{}  ·  {}", a.name, human_size(a.size)));
        if open && resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if open && resp.clicked() && starred.is_none() {
            self.ui.viewer = Some((s.key.clone(), a.clone()));
        }
        starred
    }

    /// The full-size image viewer (click outside or press Esc to close).
    pub fn viewer(&mut self, ctx: &egui::Context, acts: &mut Vec<Act>) {
        let Some((key, a)) = self.ui.viewer.clone() else { return };
        let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
        let state = self.engine.file(&key, &a.id);
        let bytes = match &state {
            ingle_engine::files::FileState::Ready(b) => Some(b.clone()),
            _ => None,
        };
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::new("image-viewer")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, backdrop) = ui.allocate_exact_size(screen.size(), Sense::click());
            ui.painter().rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha(215));
            let area = r.shrink2(vec2(r.width() * 0.05, 70.0)).translate(vec2(0.0, -10.0));
            let t = ui.input(|i| i.time);
            let tex = self.images.get(ui.ctx(), &a.id, bytes.clone(), crate::images::max_side_for_chat());
            let mut img_rect = Rect::NOTHING;
            if let Some((id, size, next)) = tex.frame(t) {
                // Up to its real size, never stretched beyond it.
                let s = (area.width() / size.x).min(area.height() / size.y).min(1.0);
                img_rect = Rect::from_center_size(area.center(), size * s);
                ui.painter().image(id, img_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                if let Some(n) = next {
                    ui.ctx().request_repaint_after(n);
                }
            } else {
                ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, "Loading…", theme::font(15.0), theme::MUTED);
            }
            // Name, size, Save and Close along the bottom.
            let bar = Rect::from_min_max(pos2(r.left(), r.bottom() - 56.0), r.max);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(24.0, 10.0))).layout(Layout::left_to_right(Align::Center)));
            child.label(RichText::new(&a.name).font(theme::bold_font(14.0)).color(theme::TEXT));
            child.label(RichText::new(format!("{} · {}×{}", human_size(a.size), a.width, a.height)).size(12.5).color(theme::MUTED));
            child.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
                if let Some(b) = &bytes {
                    if ui.button("Save as…").clicked() {
                        acts.push(Act::SaveImage { name: a.name.clone(), bytes: b.clone() });
                    }
                }
            });
            if backdrop.clicked() && !img_rect.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default()) && !bar.contains(ui.input(|i| i.pointer.interact_pos()).unwrap_or_default()) {
                close = true;
            }
        });
        if close {
            self.ui.viewer = None;
        }
    }
}

pub fn human_size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", (n / 1024).max(1))
    }
}

/// Discord's red "NEW" line: where the messages you hadn't seen start.
fn new_divider(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    let c = r.center();
    let galley = ui.painter().layout_no_wrap("NEW".to_string(), theme::bold_font(10.0), Color32::WHITE);
    let tag = Rect::from_min_size(pos2(r.right() - 16.0 - galley.size().x - 10.0, c.y - 7.0), vec2(galley.size().x + 10.0, 14.0));
    ui.painter().line_segment([pos2(r.left() + 16.0, c.y), pos2(tag.left(), c.y)], Stroke::new(1.0f32, theme::LIVE));
    // A notch pointing at the line, then the tag.
    ui.painter().add(egui::Shape::convex_polygon(
        vec![pos2(tag.left() - 5.0, c.y), pos2(tag.left() + 1.0, tag.top()), pos2(tag.left() + 1.0, tag.bottom())],
        theme::LIVE,
        Stroke::NONE,
    ));
    ui.painter().rect_filled(tag, CornerRadius::same(3), theme::LIVE);
    ui.painter().galley(tag.center() - galley.size() / 2.0, galley, Color32::WHITE);
}

fn day_divider(ui: &mut egui::Ui, label: &str) {
    ui.add_space(12.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let tw = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), theme::bold_font(11.5), Color32::WHITE).size().x);
    let c = r.center();
    ui.painter().line_segment([pos2(r.left() + 16.0, c.y), pos2(c.x - tw / 2.0 - 8.0, c.y)], Stroke::new(1.0f32, theme::LINE));
    ui.painter().line_segment([pos2(c.x + tw / 2.0 + 8.0, c.y), pos2(r.right() - 16.0, c.y)], Stroke::new(1.0f32, theme::LINE));
    ui.painter().text(c, egui::Align2::CENTER_CENTER, label, theme::bold_font(11.5), theme::DIM);
}

/// Everything an @mention can name here (lowercase): people, roles, everyone and here.
pub fn mention_names(s: &ServerView) -> Vec<String> {
    let mut v: Vec<String> = s.members.keys().map(|u| s.member_name(u).to_lowercase()).collect();
    v.extend(s.roles.iter().map(|r| r.name.to_lowercase()));
    v.push("everyone".into());
    v.push("here".into());
    v
}

fn s_of<'a>(st: &'a AppState, key: &str) -> &'a ServerView {
    static EMPTY: std::sync::OnceLock<ServerView> = std::sync::OnceLock::new();
    st.server(key).unwrap_or_else(|| EMPTY.get_or_init(|| ServerView::new(String::new(), String::new(), String::new())))
}

/// One choice in the @ list under the composer.
pub struct Suggestion {
    /// What goes into the message after the @.
    pub insert: String,
    pub label: String,
    pub detail: String,
    pub color: Color32,
    pub uid: Option<String>,
    /// For emoji suggestions: the picture.
    pub emoji: Option<crate::emoji::EmojiRef>,
}

/// The "@partial" being typed at the end of the message, if any: (byte offset of the @, the
/// partial text).
fn mention_token(draft: &str) -> Option<(usize, String)> {
    let at = draft.rfind('@')?;
    if draft[..at].chars().next_back().map(|c| !c.is_whitespace()).unwrap_or(false) {
        return None;
    }
    let partial = &draft[at + 1..];
    if partial.contains('\n') || partial.chars().count() > 32 || partial.ends_with("  ") {
        return None;
    }
    Some((at, partial.to_lowercase()))
}

/// People, roles and @everyone/@here matching what's typed after the @.
pub fn mention_suggestions(s: &ServerView, draft: &str) -> Vec<Suggestion> {
    let Some((_, partial)) = mention_token(draft) else { return Vec::new() };
    let mut out = Vec::new();
    let mut members: Vec<_> = s.members.values().collect();
    members.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    for m in members {
        let name = s.member_name(&m.user_id);
        if name.to_lowercase().starts_with(&partial) {
            out.push(Suggestion {
                insert: name.clone(),
                label: name,
                detail: if m.online { "online".into() } else { String::new() },
                color: name_color(s, &m.user_id, m.role),
                uid: Some(m.user_id.clone()),
                emoji: None,
            });
        }
    }
    for r in &s.roles {
        if r.name.to_lowercase().starts_with(&partial) {
            let color = if r.color != 0 { crate::app::rgb(r.color) } else { theme::TEXT };
            out.push(Suggestion { insert: r.name.clone(), label: format!("@{}", r.name), detail: "role".into(), color, uid: None, emoji: None });
        }
    }
    if s.can(perm::MENTION_EVERYONE) {
        for (word, what) in [("everyone", "everyone in the server"), ("here", "everyone online")] {
            if word.starts_with(&partial) {
                out.push(Suggestion { insert: word.into(), label: format!("@{word}"), detail: what.into(), color: theme::GLOW, uid: None, emoji: None });
            }
        }
    }
    // A finished mention ("@Bob " or exactly "@Bob" already picked) needs no list.
    if out.len() == 1 && out[0].insert.to_lowercase() == partial {
        out.clear();
    }
    out.truncate(8);
    out
}

/// Puts the chosen name in place of the "@partial" being typed.
pub fn complete_mention(draft: &mut String, insert: &str) {
    if let Some((at, _)) = mention_token(draft) {
        draft.truncate(at);
        draft.push('@');
        draft.push_str(insert);
        draft.push(' ');
    }
}

/// "/giphy" or "/giphy cats" (or "/gif cats") in the box: a GIF search, and its words. A bare
/// "/gif" isn't one yet, since it may be the start of "/giphy".
pub fn giphy_query(draft: &str) -> Option<String> {
    let rest = draft.strip_prefix("/giphy").or_else(|| draft.strip_prefix("/gif").filter(|r| r.starts_with(' ')))?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some(rest.trim().to_string())
}

/// A GIF search typed in the box: its words, and where to search ("/giphy" GIPHY, "/klipy"
/// KLIPY, "/gif" wherever the picker was last).
pub fn gif_query(draft: &str) -> Option<(String, Option<crate::giphy::Provider>)> {
    use crate::giphy::Provider;
    if let Some(rest) = draft.strip_prefix("/klipy") {
        return (rest.is_empty() || rest.starts_with(' ')).then(|| (rest.trim().to_string(), Some(Provider::Klipy)));
    }
    let q = giphy_query(draft)?;
    Some((q, draft.starts_with("/giphy").then_some(Provider::Giphy)))
}

/// The ":partial" shortcode being typed at the end of the message: (byte offset of the
/// colon, the partial text).
fn emoji_token(draft: &str) -> Option<(usize, String)> {
    let colon = draft.rfind(':')?;
    if draft[..colon].chars().next_back().map(|c| !c.is_whitespace()).unwrap_or(false) {
        return None;
    }
    let partial = &draft[colon + 1..];
    if partial.chars().count() < 2 || partial.len() > 32 || !partial.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')) {
        return None;
    }
    Some((colon, partial.to_lowercase()))
}

/// Emoji whose shortcode starts with (or contains) the ":partial" being typed.
pub fn emoji_suggestions(s: &ServerView, draft: &str, tone: u8) -> Vec<Suggestion> {
    let Some((_, partial)) = emoji_token(draft) else { return Vec::new() };
    let mut out = Vec::new();
    for c in &s.info.emojis {
        if c.name.to_lowercase().contains(&partial) {
            out.push(Suggestion {
                insert: crate::emoji_picker::custom_text(&c.name, &c.id, c.animated),
                label: format!(":{}:", c.name),
                detail: String::new(),
                color: theme::TEXT,
                uid: None,
                emoji: Some(crate::emoji::EmojiRef::Custom { name: c.name.clone(), id: c.id.clone(), animated: c.animated }),
            });
        }
    }
    let mut scored: Vec<(u8, usize, Suggestion)> = Vec::new();
    for g in &crate::emoji::data().groups {
        for it in &g.items {
            let Some((rank, sc)) = it
                .s
                .iter()
                .filter_map(|sc| if sc.starts_with(&partial) { Some((0, sc)) } else if sc.contains(&partial) { Some((1, sc)) } else { None })
                .min_by_key(|x| (x.0, x.1.len()))
            else {
                continue;
            };
            let (text, _) = it.toned(tone);
            let Some(cell) = crate::emoji::cell_of(text) else { continue };
            scored.push((
                rank,
                sc.len(),
                Suggestion {
                    insert: text.to_string(),
                    label: format!(":{sc}:"),
                    detail: String::new(),
                    color: theme::TEXT,
                    uid: None,
                    emoji: Some(crate::emoji::EmojiRef::Unicode(cell)),
                },
            ));
        }
    }
    scored.sort_by_key(|x| (x.0, x.1));
    out.extend(scored.into_iter().map(|x| x.2));
    out.truncate(8);
    out
}

/// Puts the chosen emoji in place of the ":partial" being typed.
pub fn complete_emoji(draft: &mut String, insert: &str) {
    if let Some((colon, _)) = emoji_token(draft) {
        draft.truncate(colon);
        draft.push_str(insert);
        draft.push(' ');
    }
}

/// Turns finished ":shortcodes:" (the server's own emoji too) into emoji when sending,
/// leaving code alone.
pub fn expand_shortcodes(s: &ServerView, text: &str, tone: u8) -> String {
    if !text.contains(':') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut in_code = false;
    let mut rest = text;
    while let Some(i) = rest.find([':', '`']) {
        let (before, after) = rest.split_at(i);
        out.push_str(before);
        if after.starts_with('`') {
            in_code = !in_code;
            out.push('`');
            rest = &after[1..];
            continue;
        }
        // after starts with ':'
        let body = &after[1..];
        let end = body.find(':');
        let word = end.map(|e| &body[..e]).filter(|w| !w.is_empty() && w.len() <= 32 && w.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')));
        let (Some(w), false) = (word, in_code) else {
            out.push(':');
            rest = body;
            continue;
        };
        // Part of a custom emoji already written out (<:name:id>)? Leave it.
        if out.ends_with('<') || out.ends_with("<a") {
            out.push(':');
            rest = body;
            continue;
        }
        let custom = s.info.emojis.iter().find(|c| c.name == w);
        let uni = crate::emoji::data().by_short.get(&w.to_lowercase()).map(|(g, i)| &crate::emoji::data().groups[*g].items[*i]);
        match (custom, uni) {
            (Some(c), _) => out.push_str(&crate::emoji_picker::custom_text(&c.name, &c.id, c.animated)),
            (None, Some(it)) => out.push_str(it.toned(tone).0),
            _ => {
                out.push(':');
                rest = body;
                continue;
            }
        }
        rest = &body[w.len() + 1..];
    }
    out.push_str(rest);
    out
}

/// The message box's text layout: plain text, with room left for colour emoji.
fn composer_job(text: &str, wrap: f32) -> egui::text::LayoutJob {
    let mut job = egui::text::LayoutJob::default();
    job.wrap.max_width = wrap;
    let plain = egui::text::TextFormat::simple(theme::font(14.5), theme::TEXT);
    let (lead, rest) = crate::emoji::formats(14.5 * crate::emoji::SCALE);
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    for (start, n, _) in crate::emoji::find_all(text) {
        if start > i {
            job.append(&chars[i..start].iter().collect::<String>(), 0.0, plain.clone());
        }
        job.append(&chars[start].to_string(), 0.0, lead.clone());
        if n > 1 {
            job.append(&chars[start + 1..start + n].iter().collect::<String>(), 0.0, rest.clone());
        }
        i = start + n;
    }
    if i < chars.len() || chars.is_empty() {
        job.append(&chars[i..].iter().collect::<String>(), 0.0, plain);
    }
    job
}

/// The favorite star in a GIF's top corner, like Discord's: gold, outlined, or filled when it's
/// a favorite already. Returns whether it was clicked.
fn gif_star(ui: &mut egui::Ui, r: Rect, id: impl std::hash::Hash, fav: bool) -> bool {
    let sr = Rect::from_min_size(pos2(r.right() - 32.0, r.top() + 6.0), vec2(26.0, 26.0));
    let resp = ui.interact(sr, ui.id().with(id), Sense::click());
    let lit = resp.hovered();
    ui.painter().rect_filled(sr, CornerRadius::same(7), if lit { theme::ELEV2 } else { Color32::from_black_alpha(150) });
    icons::star(ui.painter(), sr.shrink(if lit { 4.5 } else { 5.5 }), theme::GLOW, fav);
    resp.on_hover_text(if fav { "Remove from favorites" } else { "Add to favorites" }).clicked()
}

impl IngleApp {
    /// A GIF from GIPHY or KLIPY in a message, playing. Pointed at, it has a star in its corner
    /// that keeps it in your favorites (the GIF picker's first tab), like Discord's.
    /// Right-clicked, it has its message's menu (with Copy link and the favorite item).
    fn gif_embed(&mut self, ui: &mut egui::Ui, st: &AppState, url: &str, msg: MessageId, acts: &mut Vec<Act>) {
        let fav = crate::giphy::is_favorite(&st.settings.gif_favorites, url);
        let (starred, menu) = self.gif_embed_sized(ui, url, vec2(320.0, 240.0), Some(fav));
        if starred {
            let shown = self.ui.gif_sizes.get(url);
            self.toggle_gif_favorite(st, crate::giphy::favorite_from_link(url, shown), acts);
        }
        if menu {
            self.ui.sel.open_menu(ui.ctx(), Some(url.to_string()), msg);
        }
    }

    /// Adds a GIF to your favorites, or takes it out if it's there.
    pub(crate) fn toggle_gif_favorite(&mut self, st: &AppState, fav: crate::giphy::GifFavorite, acts: &mut Vec<Act>) {
        let removing = crate::giphy::is_favorite(&st.settings.gif_favorites, &fav.url);
        match crate::giphy::toggle_favorite(&st.settings.gif_favorites, fav.clone()) {
            Ok(v) => {
                // (A kept picture taken out: its copy goes too.)
                if removing {
                    crate::giphy::forget_picture(&fav);
                }
                acts.push(Act::Settings(ingle_engine::config::Settings { gif_favorites: v, ..st.settings.clone() }));
            }
            Err(e) => {
                if !removing && !fav.file.is_empty() {
                    crate::giphy::forget_picture(&fav);
                }
                acts.push(Act::Toast(e, false));
            }
        }
    }

    /// Whether a GIF in a message can't be shown (it didn't download, or isn't a picture): like
    /// Discord, its message then shows the link as text, so a message that's only a GIF isn't
    /// left empty.
    fn gif_failed(&self, url: &str) -> bool {
        self.ui.gifs.failed(url) || self.images.failed(&format!("gif:{url}"))
    }

    /// A message's text is only this GIF's link (so the GIF shows instead of it).
    fn only_gif(&self, content: &str, gifs: &[String]) -> bool {
        gifs.len() == 1 && content.trim() == gifs[0] && !self.gif_failed(&gifs[0])
    }

    /// A GIF at most `max` big. `star`: it's in a message (clicking it opens it on its site, and
    /// pointing at it shows the favorite star, filled when it's a favorite already); `None`:
    /// just the picture. Returns whether the star was clicked, and whether the GIF was
    /// right-clicked.
    fn gif_embed_sized(&mut self, ui: &mut egui::Ui, url: &str, max: egui::Vec2, star: Option<bool>) -> (bool, bool) {
        let open = star.is_some();
        let (mut starred, mut menu) = (false, false);
        let fit = |size: egui::Vec2| size * (max.x / size.x).min(max.y / size.y).min(2.0);
        // Its space before it loads: the size its link says (see `Gif::posted`), else the size
        // it had the last time it was shown here, so nothing moves when it shows; else a guess.
        let known = crate::giphy::link_size(url).or_else(|| self.ui.gif_sizes.get(url));
        let space = known.map(fit).unwrap_or(vec2(200.0, 120.0));
        let k = format!("gif:{url}");
        let far = self.ui.load_rect.is_some_and(|r| !r.intersects(Rect::from_min_size(ui.cursor().min, space)));
        if far && !self.images.has(&k) {
            let (r, _) = ui.allocate_exact_size(space, Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(8), theme::ELEV);
            return (false, false);
        }
        let ctx = ui.ctx().clone();
        let dl = self.ui.gifs.get(url, move || ctx.request_repaint());
        let t = ui.input(|i| i.time);
        // One that can't be shown leaves nothing here: its message shows the link instead (see
        // `gif_failed`).
        let shown = match dl {
            crate::giphy::Dl::Ready(bytes) => match self.images.get(ui.ctx(), &k, Some(bytes), 480) {
                crate::images::Tex::Failed => return (false, false),
                tex => tex.frame(t),
            },
            crate::giphy::Dl::Loading => None,
            crate::giphy::Dl::Failed => return (false, false),
        };
        match shown {
            Some((tex, size, next)) => {
                self.ui.gif_sizes.set(url, size);
                // In the space it was given (fitted), or at its own size.
                let (r, resp) = ui.allocate_exact_size(if known.is_some() { space } else { fit(size) }, Sense::click());
                let (img, uv) = fit_in(r, size);
                ui.painter().add(egui::epaint::RectShape::filled(img, CornerRadius::same(8), Color32::WHITE).with_texture(tex, uv));
                if let Some(n) = next {
                    ui.ctx().request_repaint_after(n);
                }
                let from = crate::giphy::media_provider(url).unwrap_or_default().name();
                menu = open && resp.secondary_clicked();
                if open && resp.on_hover_text(format!("GIF from {from} (click to open)")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(url.split('#').next().unwrap_or(url)));
                }
                // The favorite star, in the top corner while the GIF's pointed at (over the GIF,
                // so it takes its own clicks).
                if let Some(fav) = star.filter(|_| ui.rect_contains_pointer(img)) {
                    starred = gif_star(ui, img, ("gif-star", url), fav);
                }
            }
            None => {
                // Its size isn't known yet: a conversation just opened waits a moment for it
                // (see `chat_view`).
                if known.is_none() {
                    self.ui.gifs_waiting += 1;
                }
                let (r, _) = ui.allocate_exact_size(space, Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(8), theme::ELEV);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Loading GIF…", theme::font(12.5), theme::DIM);
            }
        }
        (starred, menu)
    }

    /// Opens the GIF picker with these search words, its search box taking the cursor (on
    /// `from`'s tab; `None`: the one it was on last).
    fn open_gif_picker(&mut self, words: String, from: Option<crate::giphy::Provider>) {
        self.ui.gif_picker = true;
        self.ui.composer_emoji = false;
        self.ui.gif_search = words;
        self.ui.gif_focus = true;
        if let Some(p) = from {
            self.ui.gif_provider = p;
        }
        if from.is_some() || !self.ui.gif_search.trim().is_empty() {
            self.ui.gif_favs = false;
        }
    }

    /// The GIF search state of `p`'s tab.
    fn gif_search_of(&mut self, p: crate::giphy::Provider) -> &mut crate::giphy::GiphyUi {
        match p {
            crate::giphy::Provider::Giphy => &mut self.ui.giphy,
            crate::giphy::Provider::Klipy => &mut self.ui.klipy,
        }
    }

    /// The GIF picker, like Discord's: your favorite GIFs (the first tab), and a search box over
    /// what's trending or the results from GIPHY or KLIPY (a tab each, when the server has
    /// them). Returns the GIF to send.
    fn gif_picker(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, acts: &mut Vec<Act>) -> Option<String> {
        use crate::giphy::Provider;
        ui.set_width(PICKER_W);
        let mut send = None;
        let search_id = egui::Id::new("gif-search");
        let owner = s.me.as_ref().is_some_and(|m| m.role == ingle_proto::control::Role::Owner);
        let key_of = |p: Provider| match p {
            Provider::Giphy => crate::giphy::key(&s.info.giphy_key),
            Provider::Klipy => crate::klipy::key(&s.info.klipy_key),
        };
        let feature = |p: Provider| s.has(if p == Provider::Giphy { ingle_proto::control::GIPHY_FEATURE } else { ingle_proto::control::KLIPY_FEATURE });
        // A tab for each source that's on here (or that the owner could turn on).
        let tabs: Vec<Provider> = [Provider::Giphy, Provider::Klipy].into_iter().filter(|p| feature(*p) && (owner || key_of(*p).is_some())).collect();
        if !tabs.contains(&self.ui.gif_provider) {
            self.ui.gif_provider = tabs.iter().copied().find(|p| key_of(*p).is_some()).or(tabs.first().copied()).unwrap_or_default();
        }
        // No GIF search here: favorites are still yours to send.
        if tabs.is_empty() {
            self.ui.gif_favs = true;
        }
        // While typing in the search box: Up/Down (or Tab) choose a GIF, Enter sends it.
        let typing = ui.ctx().memory(|m| m.has_focus(search_id));
        let (mut go_next, mut go_prev, mut enter) = (false, false, false);
        if typing {
            ui.input_mut(|i| {
                go_next = i.consume_key(Modifiers::NONE, Key::ArrowDown) || i.consume_key(Modifiers::NONE, Key::Tab);
                go_prev = i.consume_key(Modifiers::NONE, Key::ArrowUp) || i.consume_key(Modifiers::SHIFT, Key::Tab);
                enter = i.consume_key(Modifiers::NONE, Key::Enter);
            });
        }
        // The tabs: Favorites first, then where GIFs are searched.
        ui.horizontal(|ui| {
            let b = ui.add(egui::Button::selectable(self.ui.gif_favs, RichText::new("      Favorites").font(theme::bold_font(12.0))));
            icons::star(ui.painter(), Rect::from_center_size(pos2(b.rect.left() + 14.0, b.rect.center().y), vec2(11.0, 11.0)), theme::GLOW, true);
            if b.clicked() && !self.ui.gif_favs {
                self.ui.gif_favs = true;
                self.ui.gif_search.clear();
                self.ui.gif_focus = true;
            }
            for p in &tabs {
                let on = !self.ui.gif_favs && self.ui.gif_provider == *p;
                if ui.add(egui::Button::selectable(on, RichText::new(p.name()).font(theme::bold_font(12.0)))).clicked() && !on {
                    self.ui.gif_provider = *p;
                    self.ui.gif_favs = false;
                    self.ui.gif_focus = true;
                }
            }
            if !self.ui.gif_favs {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(format!("Powered by {}", self.ui.gif_provider.name())).font(theme::bold_font(10.5)).color(theme::MUTED));
                });
            }
        });
        let provider = self.ui.gif_provider;
        ui.add_space(6.0);
        // The search box (on Favorites, typing searches the last site used, like Discord's).
        if !tabs.is_empty() {
            // (KLIPY asks for exactly "Search KLIPY" here.)
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.ui.gif_search).hint_text(format!("Search {}", provider.name())).desired_width(PICKER_W).id(search_id),
            );
            if self.ui.gif_focus || (!r.has_focus() && ui.ctx().memory(|m| m.focused().is_none())) {
                r.request_focus();
                self.ui.gif_focus = false;
            }
            if self.ui.gif_favs && !self.ui.gif_search.trim().is_empty() {
                self.ui.gif_favs = false;
            }
            ui.add_space(8.0);
        }
        if self.ui.gif_favs {
            return self.gif_favorites(ui, st, s, (go_next, go_prev, enter), acts);
        }
        let query = self.ui.gif_search.clone();
        let words = crate::giphy::normalize(&query);
        let title = if words.is_empty() { "TRENDING GIFS".to_string() } else { format!("GIFS FOR \u{201c}{}\u{201d}", words.to_uppercase()) };
        ui.label(RichText::new(title).font(theme::bold_font(11.0)).color(theme::DIM));
        ui.add_space(4.0);
        let Some(api_key) = key_of(provider) else {
            if !feature(provider) {
                ui.label(RichText::new("This server needs updating before it can have GIF search.").color(theme::MUTED));
            } else if owner {
                let (text, hint, id) = match provider {
                    Provider::Giphy => (
                        "GIF search is off on your server. Paste a GIPHY API key (free from developers.giphy.com) to turn it on for everyone here:",
                        "GIPHY API key",
                        ui.id().with("giphy-key-setup"),
                    ),
                    Provider::Klipy => (
                        "KLIPY is off on your server. Paste a KLIPY app key (free from partner.klipy.com) to turn it on for everyone here:",
                        "KLIPY app key",
                        ui.id().with("klipy-key-setup"),
                    ),
                };
                ui.label(RichText::new(text).color(theme::MUTED));
                let mut k: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
                ui.horizontal(|ui| {
                    if ui.add(egui::TextEdit::singleline(&mut k).hint_text(hint).desired_width(300.0)).changed() {
                        ui.data_mut(|d| d.insert_temp(id, k.clone()));
                    }
                    if ui.add_enabled(!k.trim().is_empty(), theme::primary_button("Turn on")).clicked() {
                        acts.push(match provider {
                            Provider::Giphy => Act::SetGiphyKey { key: s.key.clone(), giphy: k.trim().to_string() },
                            Provider::Klipy => Act::SetKlipyKey { key: s.key.clone(), klipy: k.trim().to_string() },
                        });
                        ui.data_mut(|d| d.remove::<String>(id));
                    }
                });
                ui.label(
                    RichText::new(format!(
                        "Everyone's Ingle here gets this key: {} wants each search to come straight from the app, never through a server. Use a key made just for this server.",
                        provider.name()
                    ))
                    .size(12.0)
                    .color(theme::DIM),
                );
            } else {
                ui.label(
                    RichText::new("GIF search isn't set up on this server yet. The server's owner can turn it on in Server settings.")
                        .color(theme::MUTED),
                );
            }
            return None;
        };
        // One letter isn't worth a search (each one uses up some of the key's allowance).
        if words.chars().count() == 1 {
            ui.label(RichText::new(format!("Keep typing to search {}…", provider.name())).color(theme::MUTED));
            return None;
        }
        let ctx = ui.ctx().clone();
        self.gif_search_of(provider).update(provider, &query, &api_key, move || ctx.request_repaint());
        let gifs = match self.gif_search_of(provider).current() {
            Some(crate::giphy::Search::Ready(v)) => v,
            Some(crate::giphy::Search::Failed(e)) => {
                ui.label(RichText::new(e).color(theme::LIVE));
                return None;
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(format!("Searching {}…", provider.name())).color(theme::MUTED));
                });
                ui.ctx().request_repaint_after(Duration::from_millis(100));
                return None;
            }
        };
        if gifs.is_empty() {
            ui.label(RichText::new("No GIFs found. Try other words.").color(theme::MUTED));
            return None;
        }
        let n = gifs.len();
        let mut sel = self.gif_search_of(provider).sel.min(n - 1);
        if go_next {
            sel = (sel + 1) % n;
        }
        if go_prev {
            sel = (sel + n - 1) % n;
        }
        self.gif_search_of(provider).sel = sel;
        let favs = &st.settings.gif_favorites;
        let tiles: Vec<Tile> = gifs
            .iter()
            .map(|g| Tile { show: &g.preview, fallback: &g.preview, w: g.preview_w, h: g.preview_h, title: &g.title, fav: crate::giphy::is_favorite(favs, &g.url) })
            .collect();
        let (clicked, starred) = self.gif_grid(ui, ("gif-grid", provider.name()), &tiles, sel, go_next || go_prev);
        if let Some(g) = starred.and_then(|i| gifs.get(i)) {
            self.toggle_gif_favorite(st, g.favorite(), acts);
        }
        if let Some(g) = enter.then_some(sel).or(clicked).and_then(|i| gifs.get(i)) {
            send = Some(g.posted());
            // KLIPY counts what's shared (and what it was found by).
            if g.provider == Provider::Klipy {
                crate::klipy::share(&api_key, &g.slug, &words);
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Click a GIF to send it  ·  its star keeps it in Favorites  ·  ↑ ↓ and Enter").size(11.0).color(theme::DIM));
        send
    }

    /// The GIF picker's Favorites tab: your favorite GIFs, newest first (from any server, and
    /// sent to any). Returns the one to send.
    fn gif_favorites(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, keys: (bool, bool, bool), acts: &mut Vec<Act>) -> Option<String> {
        let (go_next, go_prev, enter) = keys;
        let favs = &st.settings.gif_favorites;
        ui.label(RichText::new("FAVORITES").font(theme::bold_font(11.0)).color(theme::DIM));
        ui.add_space(4.0);
        if favs.is_empty() {
            ui.add_space(4.0);
            ui.label(
                RichText::new("No favorites yet. Point at any GIF or picture (a meme, say), here or in chat, and click the star in its corner to keep it here.")
                    .color(theme::MUTED),
            );
            ui.add_space(8.0);
            return None;
        }
        let n = favs.len();
        let mut sel = self.ui.gif_fav_sel.min(n - 1);
        if go_next {
            sel = (sel + 1) % n;
        }
        if go_prev {
            sel = (sel + n - 1) % n;
        }
        self.ui.gif_fav_sel = sel;
        let tiles: Vec<Tile> = favs
            .iter()
            .map(|f| Tile {
                show: if f.preview.is_empty() { &f.url } else { &f.preview },
                fallback: &f.url,
                w: f32::from(f.w),
                h: f32::from(f.h),
                title: &f.title,
                fav: true,
            })
            .collect();
        let (clicked, starred) = self.gif_grid(ui, "gif-favorites", &tiles, sel, go_next || go_prev);
        if let Some(f) = starred.and_then(|i| favs.get(i)) {
            self.toggle_gif_favorite(st, f.clone(), acts);
        }
        let send = enter.then_some(sel).or(clicked).and_then(|i| favs.get(i)).map(|f| {
            // KLIPY counts what's shared, when it's on here and its name for the GIF is known.
            if crate::giphy::media_provider(&f.url) == Some(crate::giphy::Provider::Klipy) && !f.slug.is_empty() && s.has(ingle_proto::control::KLIPY_FEATURE) {
                if let Some(k) = crate::klipy::key(&s.info.klipy_key) {
                    crate::klipy::share(&k, &f.slug, "");
                }
            }
            f.url.clone()
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Click one to send it  ·  its star takes it out  ·  ↑ ↓ and Enter").size(11.0).color(theme::DIM));
        send
    }

    /// The GIF picker's grid: two columns, each GIF at its own height (like Discord's picker),
    /// each going under the shorter column, in the order given (KLIPY's must stay as KLIPY sends
    /// them). Returns the GIF clicked and the one whose star was clicked.
    fn gif_grid(&mut self, ui: &mut egui::Ui, id: impl std::hash::Hash, tiles: &[Tile], sel: usize, to_sel: bool) -> (Option<usize>, Option<usize>) {
        const GAP: f32 = 6.0;
        let col_w = (PICKER_W - GAP) / 2.0;
        let mut col_h = [0.0f32; 2];
        let places: Vec<(usize, f32, f32)> = tiles
            .iter()
            .map(|t| {
                // Its shape: as given, else as it was when last shown, else a guess.
                let size = (t.w > 0.0 && t.h > 0.0).then(|| vec2(t.w, t.h)).or_else(|| self.ui.gif_sizes.get(t.fallback));
                let h = size.map_or(col_w * 0.75, |s| (col_w * s.y / s.x.max(1.0)).clamp(60.0, 360.0));
                let c = usize::from(col_h[1] < col_h[0]);
                let y = col_h[c];
                col_h[c] += h + GAP;
                (c, y, h)
            })
            .collect();
        let total = (col_h[0].max(col_h[1]) - GAP).max(0.0);
        let (mut clicked, mut starred) = (None, None);
        egui::ScrollArea::vertical().id_salt(id).max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
            let (area, _) = ui.allocate_exact_size(vec2(PICKER_W, total), Sense::hover());
            for (i, (t, &(c, y, h))) in tiles.iter().zip(&places).enumerate() {
                let r = Rect::from_min_size(area.min + vec2(c as f32 * (col_w + GAP), y), vec2(col_w, h));
                let resp = ui.interact(r, ui.id().with(("gif", i)), Sense::click());
                if i == sel && to_sel {
                    ui.scroll_to_rect(r, None);
                }
                if !ui.is_rect_visible(r) {
                    continue;
                }
                ui.painter().rect_filled(r, CornerRadius::same(6), theme::ELEV);
                // Its small version, else the GIF itself.
                let ctx = ui.ctx().clone();
                let mut src = t.show;
                let mut dl = self.ui.gifs.get(src, move || ctx.request_repaint());
                if matches!(dl, crate::giphy::Dl::Failed) && t.fallback != t.show {
                    let ctx = ui.ctx().clone();
                    src = t.fallback;
                    dl = self.ui.gifs.get(src, move || ctx.request_repaint());
                }
                if let crate::giphy::Dl::Ready(bytes) = dl {
                    let time = ui.input(|i| i.time);
                    if let Some((tex, size, next)) = self.images.get(ui.ctx(), &format!("gif:{src}"), Some(bytes), 260).frame(time) {
                        if t.w <= 0.0 || t.h <= 0.0 {
                            self.ui.gif_sizes.set(t.fallback, size);
                        }
                        ui.painter().add(egui::epaint::RectShape::filled(r, CornerRadius::same(6), Color32::WHITE).with_texture(tex, cover_uv(r, size)));
                        if let Some(nx) = next {
                            ui.ctx().request_repaint_after(nx);
                        }
                    }
                }
                let inside = ui.rect_contains_pointer(r);
                if i == sel || inside {
                    ui.painter().rect_stroke(r, CornerRadius::same(6), Stroke::new(2.0f32, theme::EMBER), egui::StrokeKind::Inside);
                }
                let resp = if t.title.is_empty() { resp } else { resp.on_hover_text(t.title) };
                if resp.clicked() {
                    clicked = Some(i);
                }
                if inside && gif_star(ui, r, ("gif-tile-star", i), t.fav) {
                    starred = Some(i);
                }
            }
        });
        (clicked, starred)
    }
}

/// The GIF picker's width.
const PICKER_W: f32 = 460.0;

/// One GIF in the picker's grid.
struct Tile<'a> {
    /// What's shown: a small version, or `fallback` (the GIF itself) if that can't be had.
    show: &'a str,
    fallback: &'a str,
    /// Its size, for its shape (0 when unknown).
    w: f32,
    h: f32,
    title: &'a str,
    fav: bool,
}

/// The part of a picture of `size` that fills `r` (its shape kept, the overflow cut off evenly).
fn cover_uv(r: Rect, size: egui::Vec2) -> Rect {
    let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
    if size.x <= 0.0 || size.y <= 0.0 || r.height() <= 0.0 {
        return full;
    }
    let (ra, ia) = (r.width() / r.height(), size.x / size.y);
    if ia > ra {
        let w = ra / ia;
        Rect::from_min_max(pos2((1.0 - w) / 2.0, 0.0), pos2((1.0 + w) / 2.0, 1.0))
    } else {
        let h = ia / ra;
        Rect::from_min_max(pos2(0.0, (1.0 - h) / 2.0), pos2(1.0, (1.0 + h) / 2.0))
    }
}

#[cfg(test)]
mod delete_tests {
    use super::*;
    use ingle_proto::control::Reaction;

    fn msg(id: MessageId, reply_to: Option<MessageId>) -> Message {
        Message {
            id,
            channel_id: 1,
            author: "a".into(),
            content: "hi".into(),
            created_at: 0,
            edited_at: None,
            reply_to,
            reactions: Vec::new(),
            attachments: Vec::new(),
        }
    }

    fn picture() -> Attachment {
        Attachment { id: "f".into(), name: "dog.png".into(), mime: "image/png".into(), size: 10, width: 4, height: 4 }
    }

    #[test]
    fn the_delete_question_says_what_goes_with_the_message() {
        let mut log = ChannelLog::default();
        let mut m = msg(5, None);
        log.upsert(m.clone());
        assert_eq!(goes_with_it(&m, &log), None);
        m.attachments.push(picture());
        assert_eq!(goes_with_it(&m, &log).as_deref(), Some("Its picture goes with it."));
        m.reactions.push(Reaction { emoji: "😘".into(), users: vec!["b".into()] });
        assert_eq!(goes_with_it(&m, &log).as_deref(), Some("Its picture and 1 reaction go with it."));
        m.attachments.push(picture());
        m.reactions.push(Reaction { emoji: "🔥".into(), users: vec!["b".into(), "c".into()] });
        assert_eq!(goes_with_it(&m, &log).as_deref(), Some("Its 2 pictures and 3 reactions go with it."));
        m.attachments.clear();
        m.content = "https://media.giphy.com/media/abc/giphy.gif".into();
        log.upsert(msg(6, Some(5)));
        log.upsert(msg(7, Some(5)));
        log.upsert(msg(8, Some(6)));
        let said = goes_with_it(&m, &log).unwrap();
        assert!(said.ends_with("The 2 replies to it will say it was deleted."), "{said}");
        let just_reactions = Message { reactions: vec![Reaction { emoji: "👍".into(), users: vec!["b".into()] }], ..msg(9, None) };
        assert_eq!(goes_with_it(&just_reactions, &log).as_deref(), Some("1 reaction goes with it."));
    }
}

#[cfg(test)]
mod emoji_text_tests {
    use super::*;

    fn server() -> ServerView {
        let mut s = ServerView::new("k".into(), "S".into(), String::new());
        s.info.emojis.push(ingle_proto::control::CustomEmoji { id: "abcdefgh12".into(), name: "party_cat".into(), animated: false, by: String::new() });
        s
    }

    #[test]
    fn shortcodes_become_emoji_when_sent() {
        let s = server();
        assert_eq!(expand_shortcodes(&s, "hi :joy: and :party_cat:!", 0), "hi 😂 and <:party_cat:abcdefgh12>!");
        assert_eq!(expand_shortcodes(&s, "time 12:30:45 `:joy:` :nope:", 0), "time 12:30:45 `:joy:` :nope:");
        assert_eq!(expand_shortcodes(&s, "<:party_cat:abcdefgh12>", 0), "<:party_cat:abcdefgh12>");
        assert_eq!(expand_shortcodes(&s, ":thumbsup:", 3), "👍🏽");
    }

    #[test]
    fn emoji_and_giphy_suggestions() {
        let s = server();
        let v = emoji_suggestions(&s, "hey :part", 0);
        assert!(v.iter().any(|x| x.label == ":party_cat:"), "{:?}", v.iter().map(|x| &x.label).collect::<Vec<_>>());
        assert!(v.len() > 1);
        assert!(emoji_suggestions(&s, "12:30", 0).is_empty());
        let mut d = "hey :part".to_string();
        complete_emoji(&mut d, "🎉");
        assert_eq!(d, "hey 🎉 ");
        assert_eq!(giphy_query("/giphy cute cats").as_deref(), Some("cute cats"));
        assert_eq!(giphy_query("/giphy").as_deref(), Some(""));
        assert_eq!(giphy_query("/giphyx"), None);
        assert_eq!(giphy_query("hello"), None);
        // "/gif" might be the start of "/giphy": not a search until a space follows.
        assert_eq!(giphy_query("/gif"), None);
        assert_eq!(giphy_query("/gif dogs").as_deref(), Some("dogs"));
        assert_eq!(giphy_query("/gifts for mom"), None);
        use crate::giphy::Provider;
        assert_eq!(gif_query("/klipy cute cats"), Some(("cute cats".into(), Some(Provider::Klipy))));
        assert_eq!(gif_query("/klipy"), Some((String::new(), Some(Provider::Klipy))));
        assert_eq!(gif_query("/klipyx"), None);
        assert_eq!(gif_query("/giphy dogs"), Some(("dogs".into(), Some(Provider::Giphy))));
        assert_eq!(gif_query("/gif dogs"), Some(("dogs".into(), None)));
        assert_eq!(gif_query("hello /klipy"), None);
    }
}
