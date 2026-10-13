//! Text chat: message list (grouping, replies, reactions, edits) and the composer.

use std::time::{Duration, Instant};

use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{self, Align, Color32, CornerRadius, Key, KeyboardShortcut, Layout, Modifiers, Rect, RichText, Sense, Stroke, pos2, vec2};
use ingle_engine::state::{AppState, ServerView};
use ingle_proto::control::{Attachment, ChannelId, ChannelKind, FILES_FEATURE, Message, perm};

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
        // A direct message shows like a text channel named after the other person.
        let dm_with = s.dm(channel).map(|d| d.with.clone());
        let ch = s.channel(channel).cloned().or_else(|| {
            dm_with.as_ref().map(|w| ingle_proto::control::Channel {
                id: channel,
                kind: ChannelKind::Text,
                name: format!("@{}", s.member_name(w)),
                topic: String::new(),
                position: 0,
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
        let here = (key.clone(), channel);
        if s.unread.contains(&channel) || s.mentions.contains_key(&channel) || self.ui.active_view.as_ref() != Some(&here) && !in_call_panel {
            acts.push(Act::SetActive { key: key.clone(), channel });
            if !in_call_panel {
                self.ui.active_view = Some(here);
            }
        }
        if !in_call_panel {
            if dm_with.is_none() {
                self.ui.channel.insert(key.clone(), channel);
            }
        } else if s.unread.contains(&channel) {
            acts.push(Act::SetActive { key: key.clone(), channel });
        }

        // Header
        egui::TopBottomPanel::top(ui.id().with("chat-header"))
            .exact_height(48.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::MAIN).inner_margin(egui::Margin::symmetric(16, 0)))
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(w) = &dm_with {
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
                        ui.label(RichText::new(&ch.name).font(theme::bold_font(15.5)).color(theme::TEXT));
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
                        } else if dm_with.is_none() {
                            let open = self.ui.members_open;
                            if icon_button(ui, 30.0, if open { "Hide member list" } else { "Show member list" }, open, |p, r, c| icons::people(p, r, c)).clicked() {
                                self.ui.members_open = !open;
                            }
                        }
                    });
                });
                let r = ui.max_rect();
                ui.painter().line_segment([pos2(r.left() - 16.0, r.bottom()), pos2(r.right() + 16.0, r.bottom())], Stroke::new(1.0f32, theme::SHADOW_LINE));
            });

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
            let out = egui::ScrollArea::vertical()
                .id_salt(("msgs", &key, channel))
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add_space(8.0);
                    if log.complete || log.messages.is_empty() && log.loaded {
                        match &dm_with {
                            Some(w) => self.dm_intro(ui, s, w),
                            None => self.channel_intro(ui, &ch),
                        }
                    } else if log.loading {
                        ui.vertical_centered(|ui| ui.label(RichText::new("Loading older messages…").color(theme::DIM)));
                    }
                    let mut prev: Option<&Message> = None;
                    let msgs = &log.messages;
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
                                    self.message(ui, st, s, log, mm, false, &names, acts);
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
                        let grouped = !new_day
                            && m.reply_to.is_none()
                            && prev.map(|p| p.author == m.author && m.created_at - p.created_at < GROUP_WINDOW_MS).unwrap_or(false);
                        self.message(ui, st, s, log, m, grouped, &names, acts);
                        prev = Some(m);
                    }
                    for p in &log.pending {
                        self.pending_message(ui, st, &key, channel, p, acts);
                    }
                    ui.add_space(6.0);
                });
            if out.state.offset.y < 40.0 && !log.complete && log.loaded && !log.loading && out.content_size.y > out.inner_rect.height() {
                acts.push(Act::LoadHistory { key: key.clone(), channel, older: true });
            }
        });
    }

    /// The top of a direct message conversation: their picture and name.
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
                    RichText::new(format!("This is the beginning of your direct message history with {name}. Only the two of you can see it."))
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
                ui.label(RichText::new(title).font(theme::bold_font(26.0)).color(theme::TEXT));
                let sub = if ch.kind == ChannelKind::Voice {
                    "Messages here stay with the voice channel — handy for links while you talk.".to_string()
                } else {
                    format!("This is the start of the #{} channel.", ch.name)
                };
                ui.label(RichText::new(sub).color(theme::MUTED));
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
                ui.add(egui::Label::new(RichText::new(text).size(12.5).color(theme::MUTED)).truncate());
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
                        let only_gif = gifs.len() == 1 && m.content.trim() == gifs[0];
                        if !m.content.is_empty() && !only_gif {
                            let ctx = ui.ctx().clone();
                            markdown::show(ui, &m.content, theme::BODY, names, &mut |id| self.custom_emoji_tex(&ctx, &key, id));
                        }
                        for g in &gifs {
                            self.gif_embed(ui, g);
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
                            self.attachment(ui, s, a);
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
            let admin = s.can(perm::MANAGE_MESSAGES) && s.dm(m.channel_id).is_none();
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
            if (mine || admin) && icon_button(&mut child, 28.0, "Delete", false, |p, r, _| icons::trash(p, r, theme::LIVE)).clicked() {
                acts.push(Act::Delete { key: key.clone(), id: m.id });
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
            // Typing "/giphy" (or "/giphy cats") brings up the GIF search, the words going into
            // its search box.
            if let Some(q) = giphy_query(&draft) {
                self.open_gif_picker(q);
                draft.clear();
            }
            if !self.ui.gif_picker && self.ui.giphy.searched.is_some() {
                self.ui.giphy.reset();
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
                let hint = if !connected {
                    "Reconnecting…".to_string()
                } else if ch.kind == ChannelKind::Voice || ch.name.starts_with('@') {
                    format!("Message {}", ch.name)
                } else {
                    format!("Message #{}", ch.name)
                };
                if files_ok
                    && icon_button(ui, 30.0, "Attach images (or paste or drop them here)", false, |p, r, c| icons::plus(p, r, c, 2.0)).clicked()
                {
                    crate::images::pick_async(crate::images::PickFor::Attach(key.clone(), ch.id), true, ui.ctx());
                }
                let width = ui.available_width() - 76.0;
                let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                    let job = composer_job(buf.as_str(), wrap);
                    ui.fonts_mut(|f| f.layout_job(job))
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
                // Any Enter press counts (even one egui takes for a repeat because it missed a key
                // release, e.g. to a file dialog).
                let enter = ui.input(|i| {
                    i.events.iter().any(|e| matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift))
                });
                let send = resp.has_focus() && enter;
                let files = self.ui.attach.get(&draft_key).cloned().unwrap_or_default();
                // "/giphy …" (or a bare "/gif") is a GIF search, never a message.
                let gif_command = giphy_query(draft.trim()).or_else(|| (draft.trim() == "/gif").then(String::new));
                if send && gif_command.is_some() {
                    self.open_gif_picker(gif_command.unwrap_or_default());
                    draft.clear();
                } else if send && (!draft.trim().is_empty() || !files.is_empty()) {
                    let reply_to = self.ui.reply.take().map(|(id, _)| id);
                    let text = expand_shortcodes(s, draft.trim(), st.settings.emoji_tone);
                    acts.push(Act::Send { key: key.clone(), channel: ch.id, text, reply_to, files });
                    self.ui.attach.remove(&draft_key);
                    draft.clear();
                    resp.request_focus();
                }
                if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.ui.reply = None;
                }
                let gb = icon_button(ui, 30.0, "Search GIFs (or type /giphy)", self.ui.gif_picker, |p, r, c| icons::gif(p, r.shrink(1.0), c));
                if gb.clicked() {
                    if self.ui.gif_picker {
                        self.ui.gif_picker = false;
                    } else {
                        self.open_gif_picker(String::new());
                    }
                }
                let eb = icon_button(ui, 30.0, "Emoji", self.ui.composer_emoji, |p, r, c| icons::smile(p, r, c));
                if eb.clicked() {
                    self.ui.composer_emoji = !self.ui.composer_emoji;
                    self.ui.gif_picker = false;
                    self.ui.picker.search.clear();
                    self.ui.picker.focus_search = true;
                }
                if self.ui.gif_picker {
                    let area = egui::Area::new(egui::Id::new(("composer-gif", &key, ch.id)))
                        .order(egui::Order::Foreground)
                        .pivot(egui::Align2::RIGHT_BOTTOM)
                        .fixed_pos(pos2(eb.rect.right() + 6.0, gb.rect.top() - 12.0))
                        .constrain(true)
                        .show(ui.ctx(), |ui| egui::Frame::popup(ui.style()).fill(theme::SIDE).inner_margin(10).show(ui, |ui| self.gif_picker(ui, st, acts)).inner);
                    if let Some(url) = area.inner {
                        let reply_to = self.ui.reply.take().map(|(id, _)| id);
                        acts.push(Act::Send { key: key.clone(), channel: ch.id, text: url, reply_to, files: Vec::new() });
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
                        .fixed_pos(pos2(eb.rect.right() + 6.0, eb.rect.top() - 12.0))
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

impl IngleApp {
    /// An image or GIF in a message; click to see it full size.
    fn attachment(&mut self, ui: &mut egui::Ui, s: &ServerView, a: &Attachment) {
        let (w, h) = if a.width > 0 && a.height > 0 { (a.width as f32, a.height as f32) } else { (320.0, 200.0) };
        let max = vec2(400.0f32.min(ui.available_width().max(120.0)), 300.0);
        // Small images keep their size; big ones shrink to fit. Tiny ones get a minimum.
        let scale = (max.x / w).min(max.y / h).min(1.0);
        let size = vec2((w * scale).max(48.0), (h * scale).max(48.0));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let state = self.engine.file(&s.key, &a.id);
        let bytes = match &state {
            ingle_engine::files::FileState::Ready(b) => Some(b.clone()),
            _ => None,
        };
        let t = ui.input(|i| i.time);
        let tex = self.images.get(ui.ctx(), &a.id, bytes, crate::images::max_side_for_chat());
        match (tex.frame(t), &state) {
            (Some((id, isize, next)), _) => {
                let (fit, uv) = fit_in(rect, isize);
                ui.painter().add(
                    egui::epaint::RectShape::filled(fit, CornerRadius::same(6), Color32::WHITE).with_texture(id, uv),
                );
                if let Some(n) = next {
                    ui.ctx().request_repaint_after(n);
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
        if resp.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() {
            self.ui.viewer = Some((s.key.clone(), a.clone()));
        }
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

impl IngleApp {
    /// A GIF from GIPHY in a message, playing.
    fn gif_embed(&mut self, ui: &mut egui::Ui, url: &str) {
        let ctx = ui.ctx().clone();
        let dl = self.ui.gifs.get(url, move || ctx.request_repaint());
        let max = vec2(320.0, 240.0);
        match dl {
            crate::giphy::Dl::Ready(bytes) => {
                let t = ui.input(|i| i.time);
                let k = format!("gif:{url}");
                match self.images.get(ui.ctx(), &k, Some(bytes), 480).frame(t) {
                    Some((tex, size, next)) => {
                        let s = (max.x / size.x).min(max.y / size.y).min(2.0);
                        let (r, resp) = ui.allocate_exact_size(size * s, Sense::click());
                        ui.painter().add(egui::epaint::RectShape::filled(r, CornerRadius::same(8), Color32::WHITE).with_texture(
                            tex,
                            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        ));
                        if let Some(n) = next {
                            ui.ctx().request_repaint_after(n);
                        }
                        if resp.on_hover_text("GIF from GIPHY (click to open)").clicked() {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(url));
                        }
                    }
                    None => {
                        let (r, _) = ui.allocate_exact_size(vec2(200.0, 120.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(8), theme::ELEV);
                    }
                }
            }
            crate::giphy::Dl::Loading => {
                let (r, _) = ui.allocate_exact_size(vec2(200.0, 120.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(8), theme::ELEV);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "Loading GIF…", theme::font(12.5), theme::DIM);
            }
            crate::giphy::Dl::Failed => {}
        }
    }

    /// Opens the GIF picker with these search words, its search box taking the cursor.
    fn open_gif_picker(&mut self, words: String) {
        self.ui.gif_picker = true;
        self.ui.composer_emoji = false;
        self.ui.gif_search = words;
        self.ui.gif_focus = true;
    }

    /// The GIF picker: a GIPHY search box over what's trending or the results, like Discord's.
    /// Returns the GIF to send.
    fn gif_picker(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) -> Option<String> {
        const W: f32 = 460.0;
        ui.set_width(W);
        let mut send = None;
        let search_id = egui::Id::new("gif-search");
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
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.ui.gif_search).hint_text("Search GIPHY").desired_width(W - 130.0).id(search_id),
            );
            if self.ui.gif_focus || (!r.has_focus() && ui.ctx().memory(|m| m.focused().is_none())) {
                r.request_focus();
                self.ui.gif_focus = false;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new("Powered by GIPHY").font(theme::bold_font(10.5)).color(theme::MUTED));
            });
        });
        ui.add_space(8.0);
        let query = self.ui.gif_search.clone();
        let words = crate::giphy::normalize(&query);
        let title = if words.is_empty() { "TRENDING GIFS".to_string() } else { format!("GIFS FOR \u{201c}{}\u{201d}", words.to_uppercase()) };
        ui.label(RichText::new(title).font(theme::bold_font(11.0)).color(theme::DIM));
        ui.add_space(4.0);
        let Some(api_key) = crate::giphy::key(&st.settings.giphy_key) else {
            ui.label(
                RichText::new("GIF search isn't set up in this copy of Ingle. If you have a GIPHY API key, paste it here:")
                    .color(theme::MUTED),
            );
            let mut k = st.settings.giphy_key.clone();
            if ui.add(egui::TextEdit::singleline(&mut k).hint_text("GIPHY API key").desired_width(320.0)).changed() {
                let mut s = st.settings.clone();
                s.giphy_key = k.trim().to_string();
                acts.push(Act::Settings(s));
            }
            return None;
        };
        // One letter isn't worth a search (each one uses up some of the key's allowance).
        if words.chars().count() == 1 {
            ui.label(RichText::new("Keep typing to search GIPHY…").color(theme::MUTED));
            return None;
        }
        let ctx = ui.ctx().clone();
        self.ui.giphy.update(&query, &api_key, move || ctx.request_repaint());
        let gifs = match self.ui.giphy.current() {
            Some(crate::giphy::Search::Ready(v)) => v,
            Some(crate::giphy::Search::Failed(e)) => {
                ui.label(RichText::new(e).color(theme::LIVE));
                return None;
            }
            _ => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Searching GIPHY…").color(theme::MUTED));
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
        let mut sel = self.ui.giphy.sel.min(n - 1);
        if go_next {
            sel = (sel + 1) % n;
        }
        if go_prev {
            sel = (sel + n - 1) % n;
        }
        self.ui.giphy.sel = sel;
        if enter {
            send = Some(gifs[sel].url.clone());
        }
        // Two columns, each GIF at its own height (like Discord's picker), each going under the
        // shorter column.
        const GAP: f32 = 6.0;
        let col_w = (W - GAP) / 2.0;
        let mut col_h = [0.0f32; 2];
        let places: Vec<(usize, f32, f32)> = gifs
            .iter()
            .map(|g| {
                let h = (col_w * g.preview_h / g.preview_w.max(1.0)).clamp(60.0, 360.0);
                let c = usize::from(col_h[1] < col_h[0]);
                let y = col_h[c];
                col_h[c] += h + GAP;
                (c, y, h)
            })
            .collect();
        let total = (col_h[0].max(col_h[1]) - GAP).max(0.0);
        egui::ScrollArea::vertical().id_salt("gif-grid").max_height(340.0).auto_shrink([false, true]).show(ui, |ui| {
            let (area, _) = ui.allocate_exact_size(vec2(W, total), Sense::hover());
            for (i, (g, &(c, y, h))) in gifs.iter().zip(&places).enumerate() {
                let r = Rect::from_min_size(area.min + vec2(c as f32 * (col_w + GAP), y), vec2(col_w, h));
                let resp = ui.interact(r, ui.id().with(("gif", i)), Sense::click());
                if i == sel && (go_next || go_prev) {
                    ui.scroll_to_rect(r, None);
                }
                if !ui.is_rect_visible(r) {
                    continue;
                }
                ui.painter().rect_filled(r, CornerRadius::same(6), theme::ELEV);
                let ctx = ui.ctx().clone();
                if let crate::giphy::Dl::Ready(bytes) = self.ui.gifs.get(&g.preview, move || ctx.request_repaint()) {
                    let t = ui.input(|i| i.time);
                    if let Some((tex, _, next)) = self.images.get(ui.ctx(), &format!("gif:{}", g.preview), Some(bytes), 260).frame(t) {
                        ui.painter().add(
                            egui::epaint::RectShape::filled(r, CornerRadius::same(6), Color32::WHITE)
                                .with_texture(tex, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
                        );
                        if let Some(nx) = next {
                            ui.ctx().request_repaint_after(nx);
                        }
                    }
                }
                if i == sel || resp.hovered() {
                    ui.painter().rect_stroke(r, CornerRadius::same(6), Stroke::new(2.0f32, theme::EMBER), egui::StrokeKind::Inside);
                }
                let resp = if g.title.is_empty() { resp } else { resp.on_hover_text(&g.title) };
                if resp.clicked() {
                    send = Some(g.url.clone());
                }
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new("Click a GIF to send it  ·  ↑ ↓ to choose, Enter to send  ·  Esc to close").size(11.0).color(theme::DIM));
        send
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
    }
}
