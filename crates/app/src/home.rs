//! The home screen, like Discord's: direct message conversations down the side, and the
//! friends list (everyone you share a server with) or the open conversation in the middle.

use std::collections::BTreeMap;

use eframe::egui::{self, Align, Color32, CornerRadius, Layout, Rect, RichText, Sense, Stroke, pos2, vec2};
use ingle_engine::state::{AppState, ServerView};
use ingle_proto::control::{ChannelId, DMS_FEATURE};

use crate::app::{Act, IngleApp, avatar, status_dot};
use crate::theme;

/// Someone you share at least one server with.
struct Friend {
    uid: String,
    name: String,
    online: bool,
    /// Online, but idle everywhere they're online (a moon).
    idle: bool,
    /// Do Not Disturb, on a server you share.
    dnd: bool,
    /// Servers you share (keys), where a conversation could happen.
    servers: Vec<String>,
}

impl Friend {
    fn presence(&self) -> crate::app::Presence {
        match (self.online, self.dnd, self.idle) {
            (false, _, _) => crate::app::Presence::Offline,
            (true, true, _) => crate::app::Presence::Dnd,
            (true, false, true) => crate::app::Presence::Idle,
            (true, false, false) => crate::app::Presence::Online,
        }
    }
}

fn friends(st: &AppState) -> Vec<Friend> {
    let mut by_uid: BTreeMap<String, Friend> = BTreeMap::new();
    for s in &st.servers {
        for m in s.members.values() {
            if m.user_id == st.my_uid {
                continue;
            }
            let f = by_uid.entry(m.user_id.clone()).or_insert_with(|| Friend {
                uid: m.user_id.clone(),
                name: s.member_name(&m.user_id),
                online: false,
                idle: false,
                dnd: false,
                servers: Vec::new(),
            });
            if m.online && s.status.is_connected() {
                f.idle = if f.online { f.idle && m.idle } else { m.idle };
                f.dnd |= m.dnd;
                f.online = true;
            }
            f.servers.push(s.key.clone());
        }
    }
    let mut v: Vec<Friend> = by_uid.into_values().collect();
    v.sort_by(|a, b| b.online.cmp(&a.online).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    v
}

/// Where to talk to someone: an existing conversation, else the first connected server you
/// share that has direct messages.
fn dm_server(st: &AppState, uid: &str) -> Option<String> {
    st.servers
        .iter()
        .find(|s| s.dm_with(uid).is_some() && s.status.is_connected())
        .or_else(|| st.servers.iter().find(|s| s.has(DMS_FEATURE) && s.status.is_connected() && s.members.contains_key(uid)))
        .map(|s| s.key.clone())
}

impl IngleApp {
    pub(crate) fn home_ui(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        // The open conversation might be gone (left the server).
        let open = self.ui.dm.clone().and_then(|(k, id)| st.server(&k).filter(|s| s.is_private(id)).map(|s| (s.clone(), id)));
        if open.is_none() {
            self.ui.dm = None;
        }
        egui::SidePanel::left("home-side")
            .exact_width(240.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::panel(theme::SIDE(), false)))
            .show(ctx, |ui| self.home_sidebar(ui, st, acts));
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::panel(theme::MAIN(), true))).show(ctx, |ui| match open {
            // In this conversation's call: the call on top (like Discord's), the chat under it.
            Some((server, id)) if st.call.as_ref().is_some_and(|c| c.server_key == server.key && c.channel_id == id) => {
                let full = ui.max_rect();
                let h = (full.height() * 0.55).clamp(300.0_f32.min(full.height() - 160.0), (full.height() - 240.0).max(300.0));
                let top = Rect::from_min_size(full.min, vec2(full.width(), h));
                let mut call_ui = ui.new_child(egui::UiBuilder::new().max_rect(top).id_salt("dm-call"));
                self.stage(&mut call_ui, ctx, st, &server, acts);
                let rest = Rect::from_min_max(pos2(full.left(), top.bottom()), full.max);
                let mut chat_ui = ui.new_child(egui::UiBuilder::new().max_rect(rest).id_salt("dm-chat"));
                self.chat_view(&mut chat_ui, st, &server, id, false, acts);
            }
            Some((server, id)) => self.chat_view(ui, st, &server, id, false, acts),
            None => self.friends_page(ui, st, acts),
        });
    }

    fn home_sidebar(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        egui::TopBottomPanel::bottom("home-user-panel")
            .exact_height(56.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::panel(theme::SIDE_DARK(), false)).inner_margin(egui::Margin::symmetric(8, 0)))
            .show_inside(ui, |ui| self.user_panel(ui, st, acts));
        if let Some(call) = st.call.as_ref() {
            egui::TopBottomPanel::bottom("home-voice-panel")
                .exact_height(crate::app::voice_panel_height(st, call))
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(theme::panel(theme::SIDE_DARK(), false)).inner_margin(egui::Margin::symmetric(10, 8)))
                .show_inside(ui, |ui| self.voice_panel(ui, st, call, acts));
        } else if st.voice_rejoin.is_some() {
            egui::TopBottomPanel::bottom("home-voice-panel")
                .exact_height(56.0)
                .show_separator_line(false)
                .frame(egui::Frame::new().fill(theme::panel(theme::SIDE_DARK(), false)).inner_margin(egui::Margin::symmetric(10, 8)))
                .show_inside(ui, |ui| self.rejoin_panel(ui, st, acts));
        }
        egui::TopBottomPanel::top("home-header")
            .exact_height(48.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::inner(theme::SIDE(), false)).inner_margin(egui::Margin::symmetric(16, 0)))
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("Direct Messages").font(theme::bold_font(15.5)).color(theme::TEXT()));
                });
                let r = ui.max_rect();
                ui.painter().line_segment([pos2(r.left() - 16.0, r.bottom()), pos2(r.right() + 16.0, r.bottom())], Stroke::new(1.0f32, theme::SHADOW_LINE()));
            });
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_space(8.0);
            // Friends
            let on_friends = self.ui.dm.is_none();
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 16.0, 42.0), Sense::click());
            let r = r.translate(vec2(8.0, 0.0));
            if on_friends || resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(6), if on_friends { theme::ELEV2() } else { theme::HOVER() });
            }
            crate::icons::people(ui.painter(), Rect::from_center_size(pos2(r.left() + 22.0, r.center().y), vec2(20.0, 20.0)), theme::TEXT());
            ui.painter().text(pos2(r.left() + 48.0, r.center().y), egui::Align2::LEFT_CENTER, "Friends", theme::bold_font(14.5), theme::TEXT());
            let pending = st.relations().iter().filter(|r| r.1 == ingle_proto::control::RelationKind::Incoming).count();
            if pending > 0 {
                let b = pos2(r.right() - 16.0, r.center().y);
                ui.painter().circle_filled(b, 8.5, theme::LIVE());
                ui.painter().text(b, egui::Align2::CENTER_CENTER, pending.min(99).to_string(), theme::bold_font(10.5), Color32::WHITE);
            }
            if resp.clicked() {
                self.ui.dm = None;
                acts.push(Act::ClearActive);
            }
            ui.add_space(12.0);
            // A server where groups can be made: the one with the conversation open, else the
            // first that can.
            let group_server = self
                .ui
                .dm
                .as_ref()
                .and_then(|(k, _)| st.server(k))
                .filter(|s| crate::calls::can_group(s))
                .or_else(|| st.servers.iter().find(|s| crate::calls::can_group(s)))
                .map(|s| s.key.clone());
            ui.horizontal(|ui| {
                ui.add_space(16.0);
                ui.label(RichText::new("DIRECT MESSAGES").font(theme::bold_font(11.5)).color(theme::DIM()));
                if let Some(key) = group_server {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.add_space(12.0);
                        if crate::app::icon_button(ui, 20.0, "New group", false, |p, r, c| crate::icons::plus(p, r, c, 1.8)).clicked() {
                            self.ui.dialog = Some(crate::app::Dialog::AddPeople { key, channel: None, picked: Vec::new(), search: String::new() });
                        }
                    });
                }
            });
            ui.add_space(4.0);
            let several = st.servers.iter().filter(|s| !s.dms.is_empty() || !s.groups.is_empty()).count() > 1;
            let mut any = false;
            for s in &st.servers {
                // Conversations and groups together, newest first.
                let mut list: Vec<(Option<i64>, ChannelId, bool)> = s.dms.iter().map(|d| (d.last_message, d.id, false)).collect();
                list.extend(s.groups.iter().map(|g| (g.last_message, g.id, true)));
                list.sort_by_key(|x| std::cmp::Reverse((x.0, x.1)));
                for (last, id, is_group) in list {
                    let selected = self.ui.dm.as_ref() == Some(&(s.key.clone(), id));
                    // (An empty conversation shows once it's open; a new group always does.)
                    if last.is_none() && !selected && !is_group {
                        continue;
                    }
                    any = true;
                    if is_group {
                        if let Some(g) = s.group(id).cloned() {
                            self.group_row(ui, st, s, &g, selected, several, acts);
                        }
                    } else if let Some(with) = s.dm(id).map(|d| d.with.clone()) {
                        self.dm_row(ui, st, s, id, &with, selected, several, acts);
                    }
                }
            }
            if !any {
                ui.horizontal(|ui| {
                    ui.add_space(16.0);
                    ui.add(egui::Label::new(RichText::new("Message someone from Friends or a server's member list.").size(12.5).color(theme::DIM())).wrap());
                });
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn dm_row(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        s: &ServerView,
        id: ChannelId,
        with: &str,
        selected: bool,
        show_server: bool,
        acts: &mut Vec<Act>,
    ) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 16.0, 44.0), Sense::click());
        let r = r.translate(vec2(8.0, 0.0));
        if selected || resp.hovered() {
            ui.painter().rect_filled(r, CornerRadius::same(6), if selected { theme::ELEV2() } else { theme::HOVER() });
        }
        let name = s.member_name(with);
        let presence = crate::app::Presence::in_server(s, with);
        let c = pos2(r.left() + 22.0, r.center().y);
        let face = self.face(ui.ctx(), s, with);
        avatar(ui.painter(), c, 16.0, with, &name, false, theme::SIDE(), face);
        if presence.here() {
            status_dot(ui.painter(), c + vec2(12.0, 12.0), presence, theme::SIDE());
        }
        let unread = s.mentions.get(&id).copied().unwrap_or(0);
        let col = if selected || unread > 0 { theme::TEXT() } else { theme::MUTED() };
        let named = if show_server {
            let n = ui.painter().text(pos2(r.left() + 48.0, r.center().y - 7.0), egui::Align2::LEFT_CENTER, &name, theme::bold_font(14.0), col);
            ui.painter().text(pos2(r.left() + 48.0, r.center().y + 9.0), egui::Align2::LEFT_CENTER, &s.name, theme::font(11.5), theme::DIM());
            n
        } else {
            ui.painter().text(pos2(r.left() + 48.0, r.center().y), egui::Align2::LEFT_CENTER, &name, theme::bold_font(14.0), col)
        };
        crate::trust::badge_at(ui, st, with, &name, pos2(named.right() + 5.0, named.center().y), 13.0);
        if unread > 0 {
            let b = pos2(r.right() - 16.0, r.center().y);
            ui.painter().circle_filled(b, 8.5, theme::LIVE());
            ui.painter().text(b, egui::Align2::CENTER_CENTER, unread.min(99).to_string(), theme::bold_font(10.5), Color32::WHITE);
        }
        // A call going on in it: a green phone (left of the unread count).
        if !s.voice_in(id).is_empty() {
            let x = if unread > 0 { r.right() - 40.0 } else { r.right() - 16.0 };
            crate::icons::phone(ui.painter(), Rect::from_center_size(pos2(x, r.center().y), vec2(15.0, 15.0)), theme::ONLINE());
        }
        if resp.clicked() {
            self.ui.dm = Some((s.key.clone(), id));
        }
        let muted = st.is_muted(&s.key, id);
        let key = s.key.clone();
        resp.context_menu(|ui| {
            crate::menu::style(ui);
            crate::calls::call_item(ui, st, with, Some(&key), acts);
            self.make_sure_menu(ui, st, s, with, acts);
            if ui.button(if muted { "Unmute conversation" } else { "Mute conversation" }).clicked() {
                acts.push(Act::MuteChat { key: key.clone(), channel: Some(id), muted: !muted });
                ui.close();
            }
            crate::app::person_menu(ui, st, with, acts);
        });
    }

    fn friends_page(&mut self, ui: &mut egui::Ui, st: &AppState, acts: &mut Vec<Act>) {
        use ingle_proto::control::RelationKind;
        if self.ui.active_view.is_some() {
            acts.push(Act::ClearActive);
        }
        let people = friends(st);
        let rels = st.relations();
        let pending = rels.iter().filter(|r| r.1 == RelationKind::Incoming).count();
        egui::TopBottomPanel::top("friends-header")
            .exact_height(48.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(theme::inner(theme::MAIN(), true)).inner_margin(egui::Margin::symmetric(16, 0)))
            .show_inside(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    crate::icons::people(ui.painter(), r, theme::DIM());
                    ui.label(RichText::new("Friends").font(theme::bold_font(15.5)).color(theme::TEXT()));
                    ui.add_space(12.0);
                    for (label, tab) in [("Online", 0u8), ("All", 1), ("Pending", 2), ("Blocked", 3)] {
                        let on = self.ui.friends_tab == tab;
                        let b = egui::Button::new(RichText::new(label).color(if on { theme::TEXT() } else { theme::MUTED() }))
                            .fill(if on { theme::ELEV2() } else { Color32::TRANSPARENT })
                            .corner_radius(4);
                        let r = ui.add(b);
                        if tab == 2 && pending > 0 {
                            let c = pos2(r.rect.right() + 2.0, r.rect.center().y);
                            ui.add_space(14.0);
                            ui.painter().circle_filled(c + vec2(6.0, 0.0), 8.0, theme::LIVE());
                            ui.painter().text(c + vec2(6.0, 0.0), egui::Align2::CENTER_CENTER, pending.min(99).to_string(), theme::bold_font(10.5), Color32::WHITE);
                        }
                        if r.clicked() {
                            self.ui.friends_tab = tab;
                        }
                    }
                    let on = self.ui.friends_tab == 4;
                    let add = egui::Button::new(RichText::new("Add Friend").color(if on { theme::ONLINE() } else { Color32::WHITE }))
                        .fill(if on { Color32::TRANSPARENT } else { Color32::from_rgb(0x24, 0x80, 0x46) })
                        .corner_radius(4);
                    if ui.add(add).clicked() {
                        self.ui.friends_tab = 4;
                    }
                });
                let r = ui.max_rect();
                ui.painter().line_segment([pos2(r.left() - 16.0, r.bottom()), pos2(r.right() + 16.0, r.bottom())], Stroke::new(1.0f32, theme::SHADOW_LINE()));
            });
        let tab = self.ui.friends_tab;
        egui::CentralPanel::default().frame(egui::Frame::new().fill(theme::inner(theme::MAIN(), true)).inner_margin(egui::Margin::symmetric(24, 16))).show_inside(ui, |ui| {
            let online = |uid: &str| people.iter().any(|f| f.uid == uid && f.online);
            if tab == 4 {
                ui.label(RichText::new("ADD FRIEND").font(theme::bold_font(14.0)).color(theme::TEXT()));
                ui.label(RichText::new("You can add anyone you share a server with. Search by their name.").color(theme::MUTED()));
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.ui.friend_search)
                        .hint_text("Type a name")
                        .desired_width(ui.available_width())
                        .margin(egui::Margin::symmetric(12, 10))
                        .font(theme::font(15.0)),
                );
                ui.add_space(10.0);
                let q = self.ui.friend_search.trim().to_lowercase();
                let found: Vec<&Friend> = people.iter().filter(|f| q.is_empty() || f.name.to_lowercase().contains(&q)).collect();
                if found.is_empty() {
                    ui.label(RichText::new("Nobody by that name in your servers.").color(theme::DIM()));
                    return;
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    for f in found {
                        let sub = f.presence().label();
                        let rel = st.relation(&f.uid);
                        self.person_row(ui, st, f, sub, acts, |ui, acts| match rel {
                            Some(RelationKind::Friend) => {
                                ui.label(RichText::new("Friends").color(theme::ONLINE()));
                            }
                            Some(RelationKind::Outgoing) => {
                                ui.label(RichText::new("Request sent").color(theme::DIM()));
                            }
                            Some(RelationKind::Incoming) => {
                                if ui.add(theme::primary_button("Accept")).clicked() {
                                    acts.push(Act::AcceptFriend(f.uid.clone()));
                                }
                            }
                            Some(RelationKind::Blocked) => {
                                ui.label(RichText::new("Blocked").color(theme::LIVE()));
                            }
                            None => {
                                if ui.add(theme::primary_button("Send Friend Request")).clicked() {
                                    acts.push(Act::AddFriend(f.uid.clone()));
                                }
                            }
                        });
                    }
                });
                return;
            }
            let rows: Vec<(String, RelationKind)> = rels
                .iter()
                .filter(|(uid, k)| match tab {
                    0 => *k == RelationKind::Friend && online(uid),
                    1 => *k == RelationKind::Friend,
                    2 => matches!(k, RelationKind::Incoming | RelationKind::Outgoing),
                    _ => *k == RelationKind::Blocked,
                })
                .cloned()
                .collect();
            let label = ["ONLINE", "ALL FRIENDS", "PENDING", "BLOCKED"][tab.min(3) as usize];
            ui.label(RichText::new(format!("{label} — {}", rows.len())).font(theme::bold_font(11.5)).color(theme::DIM()));
            ui.add_space(8.0);
            if rows.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    let msg = match tab {
                        0 if rels.iter().any(|r| r.1 == RelationKind::Friend) => "None of your friends are online right now.",
                        0 | 1 => "No friends yet. Add someone you share a server with: click Add Friend, or right-click them in a member list.",
                        2 => "There are no pending friend requests.",
                        _ => "You haven't blocked anyone.",
                    };
                    ui.label(RichText::new(msg).color(theme::MUTED()));
                    if tab < 2 && ui.add(theme::primary_button("Add Friend")).clicked() {
                        self.ui.friends_tab = 4;
                    }
                });
                return;
            }
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for (uid, kind) in rows {
                    let f = people.iter().find(|f| f.uid == uid);
                    let fallback = Friend { uid: uid.clone(), name: st.name_of(&uid), online: false, idle: false, dnd: false, servers: Vec::new() };
                    let f = f.unwrap_or(&fallback);
                    let sub = match kind {
                        RelationKind::Incoming => "Incoming Friend Request",
                        RelationKind::Outgoing => "Outgoing Friend Request",
                        RelationKind::Blocked => "Blocked",
                        _ => f.presence().label(),
                    };
                    let target = dm_server(st, &uid);
                    self.person_row(ui, st, f, sub, acts, |ui, acts| match kind {
                        RelationKind::Friend => {
                            ui.menu_button("More", |ui| {
                                crate::menu::style(ui);
                                crate::calls::call_item(ui, st, &uid, None, acts);
                                crate::app::person_menu(ui, st, &uid, acts)
                            });
                            ui.add_enabled_ui(target.is_some(), |ui| {
                                if ui.add(theme::primary_button("Message")).clicked() {
                                    if let Some(key) = &target {
                                        acts.push(Act::OpenDm { key: key.clone(), uid: uid.clone() });
                                    }
                                }
                            });
                        }
                        RelationKind::Incoming => {
                            if ui.add(egui::Button::new("Ignore").min_size(vec2(64.0, 32.0))).clicked() {
                                acts.push(Act::RemoveFriend(uid.clone()));
                            }
                            if ui.add(theme::primary_button("Accept")).clicked() {
                                acts.push(Act::AcceptFriend(uid.clone()));
                            }
                        }
                        RelationKind::Outgoing => {
                            if ui.add(egui::Button::new("Cancel").min_size(vec2(64.0, 32.0))).clicked() {
                                acts.push(Act::RemoveFriend(uid.clone()));
                            }
                        }
                        RelationKind::Blocked => {
                            if ui.add(egui::Button::new("Unblock").min_size(vec2(72.0, 32.0))).clicked() {
                                acts.push(Act::Unblock(uid.clone()));
                            }
                        }
                    });
                }
            });
        });
    }

    /// One person in a friends list: picture, name, a line under it, buttons on the right
    /// (laid out right to left), and the friend/block menu on right-click.
    fn person_row(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        f: &Friend,
        sub: &str,
        acts: &mut Vec<Act>,
        buttons: impl FnOnce(&mut egui::Ui, &mut Vec<Act>),
    ) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 58.0), Sense::click());
        ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0f32, theme::LINE()));
        if resp.hovered() {
            ui.painter().rect_filled(r.shrink2(vec2(0.0, 2.0)), CornerRadius::same(8), theme::HOVER());
        }
        let s = f.servers.first().and_then(|k| st.server(k));
        let face = s.and_then(|s| self.face(ui.ctx(), s, &f.uid));
        let c = pos2(r.left() + 26.0, r.center().y);
        avatar(ui.painter(), c, 18.0, &f.uid, &f.name, false, theme::MAIN(), face);
        if f.online {
            status_dot(ui.painter(), c + vec2(13.0, 13.0), f.presence(), theme::MAIN());
        }
        let named = ui.painter().text(pos2(r.left() + 56.0, r.center().y - 8.0), egui::Align2::LEFT_CENTER, &f.name, theme::bold_font(15.0), theme::TEXT());
        crate::trust::badge_at(ui, st, &f.uid, &f.name, pos2(named.right() + 5.0, named.center().y), 14.0);
        ui.painter().text(pos2(r.left() + 56.0, r.center().y + 10.0), egui::Align2::LEFT_CENTER, sub, theme::font(12.5), theme::MUTED());
        let br = Rect::from_min_max(pos2(r.center().x, r.top() + 10.0), pos2(r.right() - 12.0, r.bottom() - 10.0));
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(br).layout(Layout::right_to_left(Align::Center)));
        buttons(&mut child, acts);
        let uid = f.uid.clone();
        // Making sure of them goes through a server you share where they're online.
        let through = f
            .servers
            .iter()
            .filter_map(|k| st.server(k))
            .find(|s| s.has(ingle_proto::control::HANDSHAKE_FEATURE) && s.members.get(&uid).is_some_and(|m| m.online))
            .or(s)
            .map(|s| s.key.clone());
        resp.context_menu(|ui| {
            crate::menu::style(ui);
            crate::calls::call_item(ui, st, &uid, None, acts);
            if let Some(s) = through.as_deref().and_then(|k| st.server(k)) {
                self.make_sure_menu(ui, st, s, &uid, acts);
            }
            crate::app::person_menu(ui, st, &uid, acts)
        });
    }
}
