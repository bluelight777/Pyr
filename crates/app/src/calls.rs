//! Calls in direct messages, like Discord's: the card that rings when someone calls (Accept,
//! Decline), the other person's place in a call while it rings for them (and Ring again when
//! they don't pick up), the bar over a conversation whose call you could join, "Call" in the
//! menus on people's names, and which server a call goes through.

use std::time::Duration;

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Stroke, pos2, vec2};
use pyr_engine::state::{AppState, IncomingCall, RingState, ServerView};
use pyr_proto::control::{ChannelId, DM_CALLS_FEATURE, GROUPS_FEATURE, Group, MAX_GROUP};

use crate::app::{Act, Dialog, PyrApp, avatar};
use crate::dialogs::{field_label, input, title};
use crate::{icons, theme};

const GREEN: Color32 = Color32::from_rgb(0x23, 0xa5, 0x5a);
const GREEN_HOT: Color32 = Color32::from_rgb(0x2e, 0xc2, 0x6c);
const RED: Color32 = Color32::from_rgb(0xd8, 0x3b, 0x3b);
const RED_HOT: Color32 = Color32::from_rgb(0xee, 0x52, 0x52);
/// The bar over a conversation with a call going on.
const CALL_TINT: Color32 = Color32::from_rgb(0x1a, 0x2e, 0x21);

/// The server a call to someone goes through: one where you have a conversation with them
/// already, else `here`, else any other you share with them, as long as it can do calls.
pub(crate) fn call_server(st: &AppState, uid: &str, here: Option<&str>) -> Option<String> {
    let ok = |s: &&ServerView| s.has(DM_CALLS_FEATURE) && s.status.is_connected() && s.members.contains_key(uid);
    st.servers
        .iter()
        .filter(ok)
        .find(|s| s.dm_with(uid).is_some())
        .or_else(|| here.and_then(|k| st.server(k)).filter(ok))
        .or_else(|| st.servers.iter().find(ok))
        .map(|s| s.key.clone())
}

/// You're in a call with them now (in your conversation with them, or a group they're in).
pub(crate) fn in_call_with(st: &AppState, uid: &str) -> bool {
    st.call.as_ref().is_some_and(|c| st.server(&c.server_key).is_some_and(|s| s.others_in(c.channel_id, &st.my_uid).iter().any(|o| o == uid)))
}

/// A group's picture: two of its people, one over the other's corner (or the group icon).
pub(crate) fn group_picture(app: &mut PyrApp, p: &egui::Painter, ctx: &egui::Context, s: &ServerView, g: &Group, me: &str, center: egui::Pos2, radius: f32, bg: Color32) {
    let others: Vec<&String> = g.members.iter().filter(|m| *m != me).take(2).collect();
    match others.as_slice() {
        [a, b] => {
            let r = radius * 0.68;
            let (fa, fb) = (app.face(ctx, s, a), app.face(ctx, s, b));
            avatar(p, center + vec2(-radius * 0.32, -radius * 0.32), r, a, &s.member_name(a), false, bg, fa);
            p.circle_filled(center + vec2(radius * 0.32, radius * 0.32), r + 2.0, bg);
            avatar(p, center + vec2(radius * 0.32, radius * 0.32), r, b, &s.member_name(b), false, bg, fb);
        }
        _ => {
            p.circle_filled(center, radius, Color32::from_rgb(0x5b, 0x4a, 0x9e));
            icons::people(p, Rect::from_center_size(center, vec2(radius * 1.1, radius * 1.1)), Color32::WHITE);
        }
    }
}

/// "Call" in a menu on someone's name, like Discord's (`here`: the server the menu is in).
pub(crate) fn call_item(ui: &mut egui::Ui, st: &AppState, uid: &str, here: Option<&str>, acts: &mut Vec<Act>) -> bool {
    // (Bots don't take calls.)
    if uid == st.my_uid || st.blocked.contains(uid) || in_call_with(st, uid) || crate::app::is_bot(st, uid) {
        return false;
    }
    let Some(key) = call_server(st, uid, here) else { return false };
    if ui.button("Call").clicked() {
        acts.push(Act::CallPerson { key, uid: uid.to_string(), video: false });
        ui.close();
        return true;
    }
    false
}

/// Pulsing rings around a picture: `t` in seconds, `col` their color.
fn pulse_rings(p: &egui::Painter, center: egui::Pos2, radius: f32, t: f32, col: Color32) {
    for k in 0..2 {
        let phase = ((t / 1.6) + k as f32 * 0.5).fract();
        let r = radius + 2.0 + phase * radius * 0.55;
        let a = (1.0 - phase).powf(1.6) * 0.55;
        p.circle_stroke(center, r, Stroke::new(2.5_f32, col.gamma_multiply(a)));
    }
}

/// A round button with an icon on it. True when clicked.
fn round_button(ui: &mut egui::Ui, center: egui::Pos2, radius: f32, fill: Color32, hot: Color32, tip: &str, draw: impl Fn(&egui::Painter, Rect, Color32)) -> bool {
    let rect = Rect::from_center_size(center, vec2(radius * 2.0, radius * 2.0));
    let resp = ui.interact(rect, ui.id().with(("round", tip)), Sense::click());
    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    let p = ui.painter();
    p.circle_filled(center, radius + h * 1.5, lerp(fill, hot, h));
    draw(p, Rect::from_center_size(center, vec2(radius, radius)), Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked()
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()), m(a.a(), b.a()))
}

impl PyrApp {
    /// A call ringing here: a card at the top of the window, wherever you are in Pyr, with
    /// Decline and Accept (and, for a video call, Accept with your camera on).
    pub(crate) fn incoming_call_card(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        // The newest one (more than one at once is rare).
        let Some(c) = st.incoming_calls.last().cloned() else { return };
        let Some(s) = st.server(&c.server_key) else { return };
        let name = s.member_name(&c.from);
        // (Calling: a moving picture moves.)
        let face = self.face_moving(ctx, s, &c.from, true);
        let screen = ctx.content_rect();
        let w = 340.0;
        // Slides in from above as it starts ringing.
        let age = c.since.elapsed().as_secs_f32();
        let ease = 1.0 - (1.0 - (age / 0.28).clamp(0.0, 1.0)).powi(3);
        let top = screen.top() + if crate::titlebar::enabled() { crate::titlebar::HEIGHT } else { 0.0 } + 10.0 - (1.0 - ease) * 24.0;
        let pos = pos2(screen.center().x - w / 2.0, top);
        let id = egui::Id::new(("incoming-call", &c.server_key, c.channel_id));
        let card = egui::Area::new(id).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
            ui.set_opacity(ease);
            egui::Frame::new()
                .fill(theme::ELEV())
                .stroke(Stroke::new(1.0_f32, theme::LINE()))
                .corner_radius(16)
                .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(150) })
                .inner_margin(egui::Margin { left: 18, right: 18, top: 16, bottom: 16 })
                .show(ui, |ui| {
                    ui.set_width(w - 36.0);
                    self.incoming_call_body(ui, st, s, &c, &name, face, acts);
                });
        });
        // (A program open inside Pyr keeps clear of it.)
        self.ui.overlay_rects.push(card.response.rect);
        ctx.request_repaint_after(Duration::from_millis(33));
    }

    #[allow(clippy::too_many_arguments)]
    fn incoming_call_body(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, c: &IncomingCall, name: &str, face: Option<egui::TextureId>, acts: &mut Vec<Act>) {
        let t = ui.input(|i| i.time) as f32;
        let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 64.0), Sense::click());
        let center = pos2(row.left() + 32.0, row.center().y);
        let p = ui.painter();
        pulse_rings(p, center, 28.0, t, theme::ONLINE());
        avatar(p, center, 28.0, &c.from, name, false, theme::ELEV(), face);
        let x = row.left() + 76.0;
        let room = row.right() - x;
        crate::emoji::paint_text(p, pos2(x, row.center().y - 11.0), Align2::LEFT_CENTER, name, theme::bold_font(17.0), theme::TEXT(), Some(room));
        let what = if c.video { "Incoming video call…" } else { "Incoming call…" };
        p.text(pos2(x, row.center().y + 10.0), Align2::LEFT_CENTER, what, theme::font(13.0), theme::MUTED());
        // In a group: which; and which server it came through, when there's more than one.
        let mut under = Vec::new();
        if let Some(g) = s.group(c.channel_id) {
            under.push(s.group_name(g, &st.my_uid));
        }
        if st.servers.len() > 1 {
            under.push(s.name.clone());
        }
        if !under.is_empty() {
            crate::emoji::paint_text(p, pos2(x, row.center().y + 27.0), Align2::LEFT_CENTER, &under.join(" · "), theme::font(11.5), theme::DIM(), Some(room));
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("Open your conversation with {name}")).clicked() {
            acts.push(Act::ShowDm { key: c.server_key.clone(), channel: c.channel_id });
        }
        ui.add_space(12.0);
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), Sense::hover());
        let r = 22.0;
        let (key, channel) = (c.server_key.clone(), c.channel_id);
        // Decline on the left, Accept on the right (and Accept with video beside it).
        if round_button(ui, pos2(bar.left() + 70.0, bar.center().y), r, RED, RED_HOT, "Decline", |p, r, c| icons::hangup(p, r, c)) {
            acts.push(Act::DeclineCall { key: key.clone(), channel });
        }
        let accept_x = bar.right() - 70.0;
        if c.video && round_button(ui, pos2(accept_x - 58.0, bar.center().y), r, GREEN, GREEN_HOT, "Answer with your camera on", |p, r, c| icons::camera(p, r, c, false)) {
            acts.push(Act::AnswerCall { key: key.clone(), channel, video: true });
        }
        if round_button(ui, pos2(accept_x, bar.center().y), r, GREEN, GREEN_HOT, "Answer", |p, r, c| icons::phone(p, r, c)) {
            acts.push(Act::AnswerCall { key, channel, video: false });
        }
    }

    /// The other person's place in a call we're in, while they aren't in it: ringing (and
    /// whether they said no or didn't answer, with Ring again), or just not there.
    pub(crate) fn ringing_tile(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, uid: &str, rect: Rect, acts: &mut Vec<Act>) {
        let call = st.call.as_ref();
        let ringing = st.outgoing_call.as_ref().filter(|o| call.is_some_and(|c| c.server_key == o.server_key && c.channel_id == o.channel_id) && o.to.iter().any(|t| t == uid));
        // (In a group, each says no for themselves.)
        let state = ringing.map(|o| if o.declined.iter().any(|d| d == uid) { RingState::Declined } else { o.state });
        let name = s.member_name(uid);
        let face = self.face(ui.ctx(), s, uid);
        let p = ui.painter_at(rect);
        // (In their own color, like their tile once they're here.)
        let bg = match self.tile_tint(s, uid) {
            Some((top, bottom)) => {
                crate::stage::paint_tint(&p, rect, top, bottom);
                theme::mix(top, bottom, 0.5)
            }
            None => {
                p.rect_filled(rect, crate::theme::r(12), theme::TILE());
                theme::TILE()
            }
        };
        let radius = (rect.width().min(rect.height()) * 0.17).clamp(26.0, 64.0);
        let center = pos2(rect.center().x, rect.center().y - 18.0);
        let t = ui.input(|i| i.time) as f32;
        if state == Some(RingState::Ringing) {
            pulse_rings(&p, center, radius, t, theme::ONLINE());
            ui.ctx().request_repaint_after(Duration::from_millis(33));
        }
        avatar(&p, center, radius, uid, &name, false, bg, face);
        // Not here (yet): a little faded.
        p.circle_filled(center, radius, bg.gamma_multiply(0.45));
        let ny = center.y + radius + 18.0;
        crate::emoji::paint_text(&p, pos2(rect.center().x, ny), Align2::CENTER_CENTER, &name, theme::bold_font(15.0), theme::TEXT(), Some(rect.width() - 24.0));
        let (status, col) = match state {
            Some(RingState::Ringing) => {
                // "Calling", and its dots coming and going.
                let dots = ".".repeat(((t * 2.0) as usize % 3) + 1);
                (format!("Calling{dots}"), theme::MUTED())
            }
            Some(RingState::Declined) => ("Declined".to_string(), theme::LIVE()),
            Some(RingState::NoAnswer) => ("Didn't answer".to_string(), theme::MUTED()),
            None => ("Not in the call".to_string(), theme::DIM()),
        };
        p.text(pos2(rect.center().x, ny + 20.0), Align2::CENTER_CENTER, status, theme::font(13.0), col);
        if state != Some(RingState::Ringing) && rect.height() > 180.0 {
            let b = Rect::from_center_size(pos2(rect.center().x, ny + 52.0), vec2(118.0, 30.0));
            let resp = ui.interact(b, ui.id().with(("ring-again", uid)), Sense::click());
            let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
            ui.painter().rect_filled(b, crate::theme::r(15), lerp(GREEN, GREEN_HOT, h));
            icons::phone(ui.painter(), Rect::from_center_size(pos2(b.left() + 20.0, b.center().y), vec2(14.0, 14.0)), Color32::WHITE);
            ui.painter().text(pos2(b.left() + 34.0, b.center().y), Align2::LEFT_CENTER, if state.is_some() { "Ring again" } else { "Ring" }, theme::bold_font(13.0), Color32::WHITE);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("Ring {name}")).clicked() {
                match state {
                    Some(_) => acts.push(Act::RingAgain),
                    None => {
                        if let Some(c) = call {
                            acts.push(Act::CallDm { key: c.server_key.clone(), channel: c.channel_id, video: false });
                        }
                    }
                }
            }
        }
    }

    /// Over a conversation (or group) whose call others are in and you aren't: who, and Join.
    pub(crate) fn dm_call_banner(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, channel: ChannelId, acts: &mut Vec<Act>) {
        let in_it = st.call.as_ref().is_some_and(|c| c.server_key == s.key && c.channel_id == channel);
        let there: Vec<String> = s.voice_in(channel).iter().map(|p| p.user_id.clone()).filter(|u| *u != st.my_uid).collect();
        if in_it || there.is_empty() || !s.has(DM_CALLS_FEATURE) {
            return;
        }
        let ringing = st.incoming_calls.iter().find(|c| c.server_key == s.key && c.channel_id == channel).map(|c| c.from.clone());
        // Who to show: the caller, or the first one there.
        let with = ringing.clone().unwrap_or_else(|| there[0].clone());
        let name = s.member_name(&with);
        egui::TopBottomPanel::top(ui.id().with("dm-call-banner"))
            .exact_height(54.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(CALL_TINT).inner_margin(egui::Margin::symmetric(16, 0)))
            .show_inside(ui, |ui| {
                let r = ui.max_rect();
                let t = ui.input(|i| i.time) as f32;
                let center = pos2(r.left() + 16.0, r.center().y);
                let face = self.face(ui.ctx(), s, &with);
                pulse_rings(ui.painter(), center, 14.0, t, theme::ONLINE());
                avatar(ui.painter(), center, 14.0, &with, &name, false, CALL_TINT, face);
                let text = match (&ringing, there.len()) {
                    (Some(_), _) => format!("{name} is calling you…"),
                    (None, 1) => format!("{name} is in a call, waiting for you"),
                    (None, n) => format!("{n} people are in a call: {name} and {}", if n == 2 { "one more".to_string() } else { format!("{} more", n - 1) }),
                };
                crate::emoji::paint_text(ui.painter(), pos2(r.left() + 42.0, r.center().y), Align2::LEFT_CENTER, &text, theme::bold_font(14.0), theme::TEXT(), Some(r.width() - 260.0));
                let mut x = r.right();
                let key = s.key.clone();
                for (label, video, fill, hot) in [("Join with video", true, theme::ELEV2(), theme::LINE()), ("Join call", false, GREEN, GREEN_HOT)] {
                    let galley = ui.painter().layout_no_wrap(label.to_string(), theme::bold_font(13.0), Color32::WHITE);
                    let bw = galley.size().x + 26.0;
                    let b = Rect::from_min_size(pos2(x - bw, r.center().y - 15.0), vec2(bw, 30.0));
                    x -= bw + 8.0;
                    let resp = ui.interact(b, ui.id().with(("join-dm-call", video)), Sense::click());
                    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
                    ui.painter().rect_filled(b, crate::theme::r(15), lerp(fill, hot, h));
                    ui.painter().galley(pos2(b.center().x - galley.size().x / 2.0, b.center().y - galley.size().y / 2.0), galley, Color32::WHITE);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        acts.push(Act::AnswerCall { key: key.clone(), channel, video });
                    }
                }
                ui.ctx().request_repaint_after(Duration::from_millis(50));
            });
    }
}

impl PyrApp {
    /// Pick people: for a new group (`channel` none), to add to a group, or to make a group
    /// with the other person in a direct message (moving its call there, if you're in it).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_people_dialog(
        &mut self,
        ui: &mut egui::Ui,
        st: &AppState,
        key: String,
        channel: Option<ChannelId>,
        mut picked: Vec<String>,
        mut search: String,
        acts: &mut Vec<Act>,
        close: &mut bool,
    ) {
        ui.set_width(440.0);
        let Some(s) = st.server(&key) else {
            *close = true;
            return;
        };
        let me = st.my_uid.clone();
        let group = channel.and_then(|c| s.group(c)).cloned();
        let dm_with = channel.and_then(|c| s.dm(c)).map(|d| d.with.clone());
        // Who's in it already (and so isn't offered), and how many more fit.
        let already: Vec<String> = match (&group, &dm_with) {
            (Some(g), _) => g.members.clone(),
            (None, Some(w)) => vec![me.clone(), w.clone()],
            _ => vec![me.clone()],
        };
        let room = MAX_GROUP.saturating_sub(already.len());
        let heading = match (&group, &dm_with) {
            (Some(g), _) => format!("Add people to {}", s.group_name(g, &me)),
            (None, Some(w)) => format!("Add people to your chat with {}", s.member_name(w)),
            _ => "New group".to_string(),
        };
        title(ui, &heading);
        let in_call = channel.is_some_and(|c| st.call.as_ref().is_some_and(|x| x.server_key == key && x.channel_id == c));
        let note = match (&group, &dm_with) {
            (Some(_), _) if in_call => "They're added to the group (they can read what's been said in it, too) and rung into its call.".to_string(),
            (Some(_), _) => "They're added to the group, and can read what's been said in it, too.".to_string(),
            (None, Some(_)) if in_call => "This makes a new group with the three (or more) of you, and moves your call there. Your chat here stays just yours.".to_string(),
            (None, Some(_)) => "This makes a new group with the three (or more) of you. Your chat here stays just yours.".to_string(),
            _ => format!("A group of up to {MAX_GROUP}, with its own chat and calls. Only the people in it see it."),
        };
        ui.label(RichText::new(note).color(theme::MUTED()));
        ui.add_space(8.0);
        let r = input(ui, &mut search, "Find someone");
        if ui.memory(|m| m.focused().is_none()) {
            r.request_focus();
        }
        ui.add_space(6.0);
        let q = search.trim().to_lowercase();
        let mut people: Vec<(String, String, crate::app::Presence)> = s
            .members
            .values()
            .filter(|m| !already.contains(&m.user_id) && !st.blocked.contains(&m.user_id))
            .map(|m| (m.user_id.clone(), s.member_name(&m.user_id), crate::app::Presence::of(m)))
            .filter(|(_, n, _)| q.is_empty() || n.to_lowercase().contains(&q))
            .collect();
        // Friends first, then who's online, then by name.
        people.sort_by_key(|(u, n, on)| (st.relation(u) != Some(pyr_proto::control::RelationKind::Friend), !on.here(), n.to_lowercase()));
        let left = room.saturating_sub(picked.len());
        ui.label(RichText::new(if left == 0 { "That's as many as a group can have.".to_string() } else { format!("You can add {left} more.") }).size(12.0).color(theme::DIM()));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
            if people.is_empty() {
                ui.label(RichText::new("Nobody else to add here.").color(theme::DIM()));
            }
            for (uid, name, presence) in &people {
                let on = picked.contains(uid);
                let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                if resp.hovered() || on {
                    if on {
                        theme::selected(ui.painter(), row, 6, theme::ELEV2());
                    } else {
                        ui.painter().rect_filled(row, crate::theme::r(6), theme::HOVER());
                    }
                }
                let face = self.face(ui.ctx(), s, uid);
                avatar(ui.painter(), pos2(row.left() + 22.0, row.center().y), 14.0, uid, name, false, theme::ELEV(), face);
                if presence.here() {
                    crate::app::status_dot(ui.painter(), pos2(row.left() + 32.0, row.center().y + 10.0), *presence, theme::ELEV());
                }
                crate::emoji::paint_text(ui.painter(), pos2(row.left() + 46.0, row.center().y), Align2::LEFT_CENTER, name, theme::font(14.5), theme::TEXT(), Some(row.width() - 90.0));
                // A tick box on the right.
                let b = Rect::from_center_size(pos2(row.right() - 20.0, row.center().y), vec2(20.0, 20.0));
                ui.painter().rect_filled(b, crate::theme::r(5), if on { theme::EMBER() } else { theme::MAIN() });
                ui.painter().rect_stroke(b, crate::theme::r(5), Stroke::new(1.0_f32, if on { theme::EMBER() } else { theme::LINE() }), egui::StrokeKind::Inside);
                if on {
                    icons::tick(ui.painter(), b.shrink(3.0), theme::RAIL());
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    if on {
                        picked.retain(|p| p != uid);
                    } else if picked.len() < room {
                        picked.push(uid.clone());
                    }
                }
            }
        });
        ui.add_space(12.0);
        let need = if group.is_none() && dm_with.is_none() { 2 } else { 1 };
        let ready = picked.len() >= need;
        let label = match (&group, &dm_with) {
            (Some(_), _) => "Add",
            _ => "Create group",
        };
        ui.horizontal(|ui| {
            if ui.add_enabled(ready, theme::primary_button(label).min_size(vec2(120.0, 34.0))).clicked() {
                match (&group, &dm_with) {
                    (Some(g), _) => acts.push(Act::AddToGroup { key: key.clone(), channel: g.id, members: picked.clone(), ring: in_call }),
                    (None, Some(w)) => {
                        let mut members = vec![w.clone()];
                        members.extend(picked.iter().cloned());
                        let call = in_call.then(|| st.call.as_ref().is_some_and(|c| c.camera_on));
                        acts.push(Act::CreateGroup { key: key.clone(), members, call, from: channel.filter(|_| in_call) });
                    }
                    _ => acts.push(Act::CreateGroup { key: key.clone(), members: picked.clone(), call: None, from: None }),
                }
                *close = true;
            }
            if ui.button("Cancel").clicked() {
                *close = true;
            }
            if !ready {
                let why = if need == 2 { "Pick at least two people" } else { "Pick someone" };
                ui.label(RichText::new(why).size(12.0).color(theme::DIM()));
            }
        });
        if !*close {
            self.ui.dialog = Some(Dialog::AddPeople { key, channel, picked, search });
        }
    }

    pub(crate) fn rename_group_dialog(&mut self, ui: &mut egui::Ui, key: String, channel: ChannelId, mut name: String, acts: &mut Vec<Act>, close: &mut bool) {
        ui.set_width(400.0);
        title(ui, "Name the group");
        field_label(ui, "Name");
        let r = input(ui, &mut name, "Leave empty to name it after who's in it");
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        if !enter {
            r.request_focus();
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button("Save").min_size(vec2(100.0, 34.0))).clicked() || enter {
                acts.push(Act::RenameGroup { key: key.clone(), channel, name: name.trim().to_string() });
                *close = true;
            }
            if ui.button("Cancel").clicked() {
                *close = true;
            }
        });
        if !*close {
            self.ui.dialog = Some(Dialog::RenameGroup { key, channel, name });
        }
    }

    pub(crate) fn leave_group_dialog(&mut self, ui: &mut egui::Ui, key: String, channel: ChannelId, name: &str, acts: &mut Vec<Act>, close: &mut bool) {
        ui.set_width(420.0);
        title(ui, &format!("Leave {name}?"));
        ui.label(RichText::new("You won't see what's said in it any more, and can only come back if someone in it adds you.").color(theme::MUTED()));
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            if ui.add(theme::danger_button("Leave group").min_size(vec2(120.0, 34.0))).clicked() {
                acts.push(Act::LeaveGroup { key: key.clone(), channel });
                *close = true;
            }
            if ui.button("Cancel").clicked() {
                *close = true;
            }
        });
    }

    /// A group in the conversation list: its picture, name and how many are in it.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn group_row(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, g: &Group, selected: bool, show_server: bool, acts: &mut Vec<Act>) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 16.0, 44.0), Sense::click());
        let r = r.translate(vec2(8.0, 0.0));
        if selected || resp.hovered() {
            if selected {
                theme::selected(ui.painter(), r, 6, theme::ELEV2());
            } else {
                ui.painter().rect_filled(r, crate::theme::r(6), theme::HOVER());
            }
        }
        let bg = if selected { theme::ELEV2() } else if resp.hovered() { theme::HOVER() } else { theme::SIDE() };
        let p = ui.painter().clone();
        group_picture(self, &p, ui.ctx(), s, g, &st.my_uid, pos2(r.left() + 22.0, r.center().y), 16.0, bg);
        let name = s.group_name(g, &st.my_uid);
        let unread = s.mentions.get(&g.id).copied().unwrap_or(0);
        let col = if selected || unread > 0 { theme::TEXT() } else { theme::MUTED() };
        let room = r.width() - 48.0 - 34.0;
        let members = if g.members.len() == 1 { "1 member".to_string() } else { format!("{} members", g.members.len()) };
        let sub = if show_server { format!("{members} · {}", s.name) } else { members };
        crate::emoji::paint_text(&p, pos2(r.left() + 48.0, r.center().y - 7.0), Align2::LEFT_CENTER, &name, theme::bold_font(14.0), col, Some(room));
        p.text(pos2(r.left() + 48.0, r.center().y + 9.0), Align2::LEFT_CENTER, sub, theme::font(11.5), theme::DIM());
        if unread > 0 {
            let b = pos2(r.right() - 16.0, r.center().y);
            p.circle_filled(b, 8.5, theme::LIVE());
            p.text(b, Align2::CENTER_CENTER, unread.min(99).to_string(), theme::bold_font(10.5), Color32::WHITE);
        }
        if !s.voice_in(g.id).is_empty() {
            let x = if unread > 0 { r.right() - 40.0 } else { r.right() - 16.0 };
            icons::phone(&p, Rect::from_center_size(pos2(x, r.center().y), vec2(15.0, 15.0)), theme::ONLINE());
        }
        if resp.clicked() {
            self.ui.dm = Some((s.key.clone(), g.id));
        }
        let (key, id) = (s.key.clone(), g.id);
        let muted = st.is_muted(&key, id);
        resp.context_menu(|ui| {
            crate::menu::style(ui);
            if s.has(DM_CALLS_FEATURE) && ui.button("Call").clicked() {
                acts.push(Act::CallDm { key: key.clone(), channel: id, video: false });
                ui.close();
            }
            if ui.button("Add people").clicked() {
                self.ui.dialog = Some(Dialog::AddPeople { key: key.clone(), channel: Some(id), picked: Vec::new(), search: String::new() });
                ui.close();
            }
            if ui.button("Name the group").clicked() {
                self.ui.dialog = Some(Dialog::RenameGroup { key: key.clone(), channel: id, name: g.name.clone() });
                ui.close();
            }
            if ui.button(if muted { "Unmute group" } else { "Mute group" }).clicked() {
                acts.push(Act::MuteChat { key: key.clone(), channel: Some(id), muted: !muted });
                ui.close();
            }
            ui.separator();
            if crate::menu::danger(ui, "Leave group").clicked() {
                self.ui.dialog = Some(Dialog::LeaveGroup { key: key.clone(), channel: id, name: name.clone() });
                ui.close();
            }
        });
    }

    /// A group's people, down the right of its chat (like Discord's), with Add people and
    /// Leave group.
    pub(crate) fn group_members_panel(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, g: &Group, acts: &mut Vec<Act>) {
        // (Scrolls: under a call there's less room.)
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.group_members_list(ui, st, s, g, acts));
    }

    fn group_members_list(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, g: &Group, acts: &mut Vec<Act>) {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(14.0);
            ui.label(RichText::new(format!("MEMBERS — {}", g.members.len())).font(theme::bold_font(11.5)).color(theme::DIM()));
        });
        ui.add_space(6.0);
        let in_call: Vec<String> = s.voice_in(g.id).iter().map(|p| p.user_id.clone()).collect();
        for uid in &g.members {
            let name = s.member_name(uid);
            let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 16.0, 40.0), Sense::click());
            let row = row.translate(vec2(8.0, 0.0));
            if resp.hovered() {
                ui.painter().rect_filled(row, crate::theme::r(6), theme::HOVER());
            }
            let face = self.face(ui.ctx(), s, uid);
            let c = pos2(row.left() + 20.0, row.center().y);
            avatar(ui.painter(), c, 14.0, uid, &name, false, theme::SIDE(), face);
            let presence = if *uid == st.my_uid { crate::app::Presence::mine(st) } else { crate::app::Presence::in_server(s, uid) };
            if presence.here() {
                crate::app::status_dot(ui.painter(), c + vec2(10.0, 10.0), presence, theme::SIDE());
            }
            let mut x = row.left() + 42.0;
            let named = crate::emoji::paint_text(ui.painter(), pos2(x, row.center().y), Align2::LEFT_CENTER, &name, theme::font(14.0), theme::MUTED(), Some(row.width() - 80.0));
            x = named.right() + 6.0;
            if *uid == g.owner {
                icons::crown(ui.painter(), Rect::from_center_size(pos2(x + 7.0, row.center().y), vec2(14.0, 14.0)), theme::OWNER());
                x += 18.0;
            }
            crate::trust::badge_at(ui, st, uid, &name, pos2(x, row.center().y), 13.0);
            if in_call.contains(uid) {
                icons::phone(ui.painter(), Rect::from_center_size(pos2(row.right() - 14.0, row.center().y), vec2(14.0, 14.0)), theme::ONLINE());
            }
            if *uid != st.my_uid {
                let uid2 = uid.clone();
                resp.context_menu(|ui| {
                    crate::menu::style(ui);
                    call_item(ui, st, &uid2, Some(&s.key), acts);
                    if ui.button("Message").clicked() {
                        acts.push(Act::OpenDm { key: s.key.clone(), uid: uid2.clone() });
                        ui.close();
                    }
                    self.make_sure_menu(ui, st, s, &uid2, acts);
                    crate::app::person_menu(ui, st, &uid2, acts);
                });
            }
        }
        // Then (under a line) adding people and leaving, as rows like the people's.
        ui.add_space(6.0);
        let (line, _) = ui.allocate_exact_size(vec2(ui.available_width(), 9.0), Sense::hover());
        ui.painter().line_segment([pos2(line.left() + 16.0, line.center().y), pos2(line.right() - 16.0, line.center().y)], Stroke::new(1.0f32, theme::LINE()));
        if g.members.len() < MAX_GROUP
            && can_group(s)
            && panel_row(ui, "Add people", theme::MUTED(), theme::TEXT(), |p, r, c| icons::add_person(p, r, c))
        {
            self.ui.dialog = Some(Dialog::AddPeople { key: s.key.clone(), channel: Some(g.id), picked: Vec::new(), search: String::new() });
        }
        if panel_row(ui, "Leave group", theme::LIVE().gamma_multiply(0.85), theme::LIVE(), |p, r, c| icons::leave(p, r, c)) {
            self.ui.dialog = Some(Dialog::LeaveGroup { key: s.key.clone(), channel: g.id, name: s.group_name(g, &st.my_uid) });
        }
        ui.add_space(10.0);
    }
}

/// A row in a side panel that does something: an icon in a circle where the people's pictures
/// go, and what it does.
fn panel_row(ui: &mut egui::Ui, label: &str, color: Color32, hot: Color32, icon: impl Fn(&egui::Painter, Rect, Color32)) -> bool {
    let (row, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 16.0, 40.0), Sense::click());
    let row = row.translate(vec2(8.0, 0.0));
    let h = ui.ctx().animate_bool_with_time(resp.id, resp.hovered(), 0.12);
    if h > 0.0 {
        ui.painter().rect_filled(row, crate::theme::r(6), theme::HOVER().gamma_multiply(h));
    }
    let col = lerp(color, hot, h);
    let c = pos2(row.left() + 20.0, row.center().y);
    ui.painter().circle_filled(c, 14.0, theme::ELEV2());
    icon(ui.painter(), Rect::from_center_size(c, vec2(15.0, 15.0)), col);
    ui.painter().text(pos2(row.left() + 42.0, row.center().y), Align2::LEFT_CENTER, label, theme::font(14.0), col);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// Groups can be made on this server.
pub(crate) fn can_group(s: &ServerView) -> bool {
    s.has(GROUPS_FEATURE) && s.status.is_connected()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_engine::state::ConnStatus;
    use pyr_proto::control::{Dm, Member};

    fn server(key: &str, calls: bool, members: &[&str], dm_with: Option<&str>) -> ServerView {
        let mut s = ServerView::new(key.into(), key.into(), "127.0.0.1:1".into());
        s.status = ConnStatus::Connected;
        if calls {
            s.features.push(DM_CALLS_FEATURE.to_string());
        }
        for m in members {
            s.members.insert(
                m.to_string(),
                Member { user_id: m.to_string(), name: m.to_string(), online: true, ..Default::default() },
            );
        }
        if let Some(w) = dm_with {
            s.upsert_dm(Dm { id: 5, with: w.into(), last_message: None, last_from: None });
        }
        s
    }

    #[test]
    fn calls_go_where_you_already_talk_then_here() {
        let mut st = AppState { my_uid: "me".into(), ..Default::default() };
        st.servers = vec![
            server("a", true, &["me", "bo"], None),
            server("b", false, &["me", "bo"], Some("bo")),
            server("c", true, &["me", "bo"], Some("bo")),
        ];
        // "b" has the conversation but can't do calls: "c" has both.
        assert_eq!(call_server(&st, "bo", Some("a")).as_deref(), Some("c"));
        st.servers[2].dms.clear();
        assert_eq!(call_server(&st, "bo", Some("c")).as_deref(), Some("c"), "here");
        assert_eq!(call_server(&st, "bo", None).as_deref(), Some("a"), "any other");
        assert_eq!(call_server(&st, "nobody", None), None, "not someone you share a server with");
    }
}
