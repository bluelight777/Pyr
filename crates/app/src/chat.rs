//! Text chat: message list (grouping, replies, reactions, edits) and the composer.

use std::time::{Duration, Instant};

use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{self, Align, Color32, CornerRadius, Key, KeyboardShortcut, Layout, Modifiers, Rect, RichText, Sense, Stroke, pos2, vec2};
use ingle_engine::state::{AppState, ServerView};
use ingle_proto::control::{ChannelId, ChannelKind, Message};

use crate::app::{Act, IngleApp, avatar, icon_button, role_color};
use crate::{icons, markdown, theme};

pub const QUICK_REACTIONS: &[&str] = &["👍", "❤", "😂", "😮", "😢", "🔥", "🎉", "👀", "💯", "✅", "🙏", "😎", "⭐", "👏", "😅", "🚀"];
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
        let Some(ch) = s.channel(channel).cloned() else { return };
        let key = s.key.clone();
        let my_name = s.me.as_ref().map(|m| m.name.clone()).unwrap_or_else(|| st.my_name.clone());

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
        if s.unread.contains(&channel) || s.mentions.contains_key(&channel) || self.ui.channel.get(&key) != Some(&channel) && !in_call_panel {
            acts.push(Act::SetActive { key: key.clone(), channel });
        }
        if !in_call_panel {
            self.ui.channel.insert(key.clone(), channel);
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
                    let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                    if ch.kind == ChannelKind::Voice {
                        icons::chat(ui.painter(), r, theme::DIM);
                    } else {
                        icons::hash(ui.painter(), r, theme::DIM);
                    }
                    ui.label(RichText::new(&ch.name).font(theme::bold_font(15.5)).color(theme::TEXT));
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
                        } else {
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
            .show_inside(ui, |ui| self.composer(ui, st, s, &ch, acts));

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
                        self.channel_intro(ui, &ch);
                    } else if log.loading {
                        ui.vertical_centered(|ui| ui.label(RichText::new("Loading older messages…").color(theme::DIM)));
                    }
                    let mut prev: Option<&Message> = None;
                    for m in &log.messages {
                        let new_day = prev.map(|p| local(p.created_at).date_naive() != local(m.created_at).date_naive()).unwrap_or(true);
                        if new_day {
                            day_divider(ui, &day_label(m.created_at));
                        }
                        let grouped = !new_day
                            && m.reply_to.is_none()
                            && prev.map(|p| p.author == m.author && m.created_at - p.created_at < GROUP_WINDOW_MS).unwrap_or(false);
                        self.message(ui, st, s, log, m, grouped, &my_name, acts);
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
        my_name: &str,
        acts: &mut Vec<Act>,
    ) {
        let key = s.key.clone();
        let mine = m.author == st.my_uid;
        let author = s.members.get(&m.author);
        let name = s.member_name(&m.author);
        let mention = !mine && {
            let l = m.content.to_lowercase();
            (!my_name.is_empty() && l.contains(&format!("@{}", my_name.to_lowercase()))) || l.contains("@everyone") || l.contains("@here")
        };
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
                    avatar(ui.painter(), ar.center(), 8.0, u, &s.member_name(u), false, theme::MAIN);
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
                avatar(ui.painter(), r.center(), 20.0, &m.author, &name, false, theme::MAIN);
            }
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width() - 16.0);
                if !grouped {
                    ui.horizontal(|ui| {
                        let col = author.map(|a| role_color(a.role)).unwrap_or(theme::TEXT);
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
                        markdown::show(ui, &m.content, theme::BODY, my_name);
                        if m.edited_at.is_some() {
                            ui.label(RichText::new("(edited)").size(10.5).color(theme::DIM));
                        }
                    }
                }
                if !m.reactions.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        for r in &m.reactions {
                            let me_in = r.users.iter().any(|u| u == &st.my_uid);
                            let label = RichText::new(format!("{} {}", r.emoji, r.users.len())).size(13.0).color(if me_in { theme::EMBER2 } else { theme::MUTED });
                            let b = egui::Button::new(label)
                                .fill(if me_in { Color32::from_rgb(0x3d, 0x26, 0x18) } else { theme::ELEV })
                                .stroke(if me_in { Stroke::new(1.0f32, theme::EMBER) } else { Stroke::NONE })
                                .corner_radius(CornerRadius::same(8))
                                .min_size(vec2(0.0, 24.0));
                            let names: Vec<String> = r.users.iter().map(|u| s.member_name(u)).collect();
                            if ui.add(b).on_hover_text(format!("{} reacted with {}", names.join(", "), r.emoji)).clicked() {
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
            let admin = s.is_admin();
            let n = 2 + mine as usize + (mine || admin) as usize;
            let w = n as f32 * 30.0 + 8.0;
            let tools = Rect::from_min_size(pos2(rect.right() - w - 16.0, rect.top() - 10.0), vec2(w, 32.0));
            ui.painter().rect_filled(tools, CornerRadius::same(6), theme::ELEV);
            ui.painter().rect_stroke(tools, CornerRadius::same(6), Stroke::new(1.0f32, theme::LINE), egui::StrokeKind::Inside);
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(tools.shrink2(vec2(4.0, 2.0))).layout(Layout::left_to_right(Align::Center)));
            child.spacing_mut().item_spacing.x = 2.0;
            if icon_button(&mut child, 28.0, "Add reaction", self.ui.react_picker == Some(m.id), |p, r, c| icons::smile(p, r, c)).clicked() {
                self.ui.react_picker = if self.ui.react_picker == Some(m.id) { None } else { Some(m.id) };
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
                    .fixed_pos(pos2(tools.right() - 8.0 * 34.0 - 12.0, tools.bottom() + 4.0))
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).fill(theme::SIDE).show(ui, |ui| {
                            egui::Grid::new(("react-grid", m.id)).spacing(vec2(2.0, 2.0)).show(ui, |ui| {
                                for (i, e) in QUICK_REACTIONS.iter().enumerate() {
                                    if ui.add(egui::Button::new(RichText::new(*e).size(18.0)).frame(false).min_size(vec2(32.0, 32.0))).clicked() {
                                        let already = m.reactions.iter().any(|r| r.emoji == *e && r.users.iter().any(|u| u == &st.my_uid));
                                        acts.push(Act::React { key: key.clone(), id: m.id, emoji: e.to_string(), add: !already });
                                        self.ui.react_picker = None;
                                    }
                                    if i % 8 == 7 {
                                        ui.end_row();
                                    }
                                }
                            });
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
                markdown::show(ui, &p.content, col, &st.my_name);
                if let Some(err) = &p.failed {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("Not sent: {err}")).size(11.5).color(theme::LIVE));
                        if ui.small_button("Retry").clicked() {
                            acts.push(Act::DiscardPending { key: key.to_string(), channel, nonce: p.nonce.clone() });
                            acts.push(Act::Send { key: key.to_string(), channel, text: p.content.clone(), reply_to: p.reply_to });
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
        egui::Frame::new().fill(theme::ELEV).corner_radius(8).inner_margin(egui::Margin::symmetric(12, 6)).show(ui, |ui| {
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
            ui.horizontal(|ui| {
                let hint = if !connected {
                    "Reconnecting…".to_string()
                } else if ch.kind == ChannelKind::Voice {
                    format!("Message {}", ch.name)
                } else {
                    format!("Message #{}", ch.name)
                };
                let width = ui.available_width() - 40.0;
                let resp = ui.add_enabled(
                    connected,
                    egui::TextEdit::multiline(&mut draft)
                        .id(ui.id().with(("composer", &key, ch.id)))
                        .hint_text(RichText::new(hint).color(theme::DIM))
                        .desired_rows(1)
                        .desired_width(width)
                        .frame(false)
                        .font(theme::font(14.5))
                        .return_key(KeyboardShortcut::new(Modifiers::SHIFT, Key::Enter)),
                );
                if self.ui.editing.is_none() && self.ui.dialog.is_none() && !resp.has_focus() && ui.ctx().memory(|m| m.focused().is_none()) {
                    resp.request_focus();
                }
                let send = resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift);
                if send && !draft.trim().is_empty() {
                    let reply_to = self.ui.reply.take().map(|(id, _)| id);
                    acts.push(Act::Send { key: key.clone(), channel: ch.id, text: draft.trim().to_string(), reply_to });
                    draft.clear();
                    resp.request_focus();
                }
                if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.ui.reply = None;
                }
                if icon_button(ui, 30.0, "Emoji", self.ui.composer_emoji, |p, r, c| icons::smile(p, r, c)).clicked() {
                    self.ui.composer_emoji = !self.ui.composer_emoji;
                }
            });
            if self.ui.composer_emoji {
                ui.horizontal_wrapped(|ui| {
                    for e in QUICK_REACTIONS {
                        if ui.add(egui::Button::new(RichText::new(*e).size(18.0)).frame(false)).clicked() {
                            draft.push_str(e);
                        }
                    }
                });
            }
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

fn day_divider(ui: &mut egui::Ui, label: &str) {
    ui.add_space(12.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 16.0), Sense::hover());
    let tw = ui.fonts_mut(|f| f.layout_no_wrap(label.to_string(), theme::bold_font(11.5), Color32::WHITE).size().x);
    let c = r.center();
    ui.painter().line_segment([pos2(r.left() + 16.0, c.y), pos2(c.x - tw / 2.0 - 8.0, c.y)], Stroke::new(1.0f32, theme::LINE));
    ui.painter().line_segment([pos2(c.x + tw / 2.0 + 8.0, c.y), pos2(r.right() - 16.0, c.y)], Stroke::new(1.0f32, theme::LINE));
    ui.painter().text(c, egui::Align2::CENTER_CENTER, label, theme::bold_font(11.5), theme::DIM);
}
