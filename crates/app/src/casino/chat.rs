//! The casino's chat (0.13.0 servers): one room for everyone in Pyr Casino, there to type in
//! from the floor, any game or any table. A round button on the right edge (with how many new
//! lines have come in) opens it as a panel over the page; Enter sends. Lines from people I've
//! blocked aren't shown. Right-click a line to take it down (my own; anyone's for staff and
//! the server's admins: the server checks).

use eframe::egui::{self, Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::{AppState, ServerView};

use super::col;
use crate::app::PyrApp;
use crate::theme;

const PANEL_W: f32 = 340.0;
const LONGEST: usize = 300;

/// A name's colour in the chat, the same every time.
fn name_color(uid: &str) -> Color32 {
    const COLORS: [Color32; 8] = [
        Color32::from_rgb(0xff, 0x9a, 0x4a),
        Color32::from_rgb(0x6a, 0xc8, 0xff),
        Color32::from_rgb(0xc0, 0x8a, 0xff),
        Color32::from_rgb(0x7a, 0xe0, 0x8a),
        Color32::from_rgb(0xff, 0x7a, 0xa8),
        Color32::from_rgb(0xff, 0xd2, 0x4a),
        Color32::from_rgb(0x5a, 0xe0, 0xd0),
        Color32::from_rgb(0xff, 0xb0, 0x90),
    ];
    let h = uid.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3));
    COLORS[(h % COLORS.len() as u64) as usize]
}

/// How long ago, in a word or two.
fn ago(at_ms: u64) -> String {
    let now = pyr_proto::now_ms().max(0) as u64;
    let s = now.saturating_sub(at_ms) / 1000;
    match s {
        0..=59 => "just now".into(),
        60..=3_599 => format!("{} min ago", s / 60),
        3_600..=86_399 => format!("{} h ago", s / 3_600),
        _ => format!("{} d ago", s / 86_400),
    }
}

impl PyrApp {
    /// The chat's button, and the panel when it's open, over `area` (the whole casino page).
    pub(crate) fn casino_chat_ui(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, area: Rect) {
        if !s.has(pyr_proto::control::CASINO_013_FEATURE) {
            return;
        }
        let ctx = ui.ctx().clone();
        let open = self.poker.chat_open;
        let unread = if open { 0 } else { s.poker.chat_unread };
        // The button: a speech bubble on the right edge, a little above the middle.
        let c = pos2(area.right() - 30.0, area.top() + area.height() * 0.42);
        let mut toggle = false;
        egui::Area::new(egui::Id::new("casino-chat-button")).order(egui::Order::Foreground).fixed_pos(c - vec2(24.0, 24.0)).show(&ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
            let p = ui.painter();
            let fill = if open { theme::EMBER() } else if resp.hovered() { Color32::from_rgb(0x3a, 0x2c, 0x24) } else { Color32::from_rgb(0x24, 0x1c, 0x18) };
            p.circle_filled(r.center(), 24.0, Color32::from_black_alpha(120));
            p.circle_filled(r.center() - vec2(0.0, 1.0), 23.0, fill);
            p.circle_stroke(r.center() - vec2(0.0, 1.0), 23.0, Stroke::new(1.5f32, col::GOLD.gamma_multiply(0.7)));
            // A bubble with a tail, and three dots.
            let b = Rect::from_center_size(r.center() - vec2(0.0, 3.0), vec2(24.0, 17.0));
            let ink = if open { theme::on_ember() } else { Color32::WHITE };
            p.rect_filled(b, theme::r(6), ink);
            p.add(egui::Shape::convex_polygon(vec![b.left_bottom() + vec2(5.0, -1.0), b.left_bottom() + vec2(12.0, -1.0), b.left_bottom() + vec2(4.0, 6.0)], ink, Stroke::NONE));
            for i in 0..3 {
                p.circle_filled(b.center() + vec2((i as f32 - 1.0) * 6.0, 0.0), 1.8, fill);
            }
            if unread > 0 {
                let at = r.right_top() + vec2(-6.0, 6.0);
                p.circle_filled(at, 10.0, theme::LIVE());
                p.text(at, Align2::CENTER_CENTER, if unread > 99 { "99+".to_string() } else { unread.to_string() }, theme::bold_font(10.5), Color32::WHITE);
            }
            if resp.on_hover_text(if open { "Close the casino's chat" } else { "The casino's chat: everyone in Pyr Casino" }).clicked() {
                toggle = true;
            }
        });
        if toggle {
            self.poker.chat_open = !open;
            self.poker.chat_focus = !open;
        }
        if !self.poker.chat_open {
            return;
        }
        if !s.poker.chat_loaded {
            self.engine.casino_chat_log(&s.key);
        }
        if s.poker.chat_unread > 0 {
            self.engine.casino_chat_seen(&s.key);
        }
        let h = (area.height() - 120.0).clamp(260.0, 560.0);
        let panel = Rect::from_min_size(pos2(c.x - 34.0 - PANEL_W, (c.y - h * 0.45).max(area.top() + 56.0)), vec2(PANEL_W, h));
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        let staff = s.is_admin() || s.poker.me.as_ref().is_some_and(|m| m.role.is_some());
        let mut send: Option<String> = None;
        let mut take_down: Option<u64> = None;
        let mut close = false;
        egui::Area::new(egui::Id::new("casino-chat-panel")).order(egui::Order::Foreground).fixed_pos(panel.min).show(&ctx, |ui| {
            let (r, _) = ui.allocate_exact_size(panel.size(), Sense::hover());
            let p = ui.painter().clone();
            p.rect_filled(r.translate(vec2(0.0, 6.0)), theme::r(16), Color32::from_black_alpha(110));
            p.rect_filled(r, theme::r(16), Color32::from_rgb(0x16, 0x11, 0x0f));
            p.rect_stroke(r, theme::r(16), Stroke::new(1.0f32, col::GOLD.gamma_multiply(0.45)), StrokeKind::Inside);
            p.text(r.left_top() + vec2(16.0, 20.0), Align2::LEFT_CENTER, "Casino chat", theme::bold_font(16.0), col::GOLD2);
            p.text(r.left_top() + vec2(16.0, 40.0), Align2::LEFT_CENTER, "Everyone in Pyr Casino on this server", theme::font(11.5), col::MUTED);
            let x = Rect::from_center_size(pos2(r.right() - 22.0, r.top() + 22.0), vec2(26.0, 26.0));
            let xr = ui.interact(x, ui.id().with("chat-close"), Sense::click());
            if xr.hovered() {
                p.circle_filled(x.center(), 13.0, Color32::from_white_alpha(24));
            }
            let d = 5.0;
            p.line_segment([x.center() + vec2(-d, -d), x.center() + vec2(d, d)], Stroke::new(2.0f32, col::MUTED));
            p.line_segment([x.center() + vec2(-d, d), x.center() + vec2(d, -d)], Stroke::new(2.0f32, col::MUTED));
            if xr.clicked() {
                close = true;
            }
            // The lines.
            let list = Rect::from_min_max(r.left_top() + vec2(10.0, 56.0), r.right_bottom() - vec2(10.0, 54.0));
            let mut lui = ui.new_child(egui::UiBuilder::new().max_rect(list));
            egui::ScrollArea::vertical().id_salt("casino-chat-lines").stick_to_bottom(true).auto_shrink([false, false]).show(&mut lui, |ui| {
                ui.set_width(list.width() - 12.0);
                let shown: Vec<_> = s.poker.chat.iter().filter(|l| !st.blocked.contains(&l.uid)).collect();
                if shown.is_empty() {
                    ui.add_space(12.0);
                    ui.label(RichText::new(if s.poker.chat_loaded { "Nobody's said anything yet. Say hello." } else { "Loading…" }).color(col::MUTED).size(13.0));
                }
                for l in shown {
                    let resp = ui
                        .horizontal_wrapped(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(RichText::new(&l.name).strong().size(13.5).color(name_color(&l.uid)));
                            ui.label(RichText::new(&l.text).size(13.5).color(col::TEXT));
                        })
                        .response
                        .on_hover_text(ago(l.at_ms));
                    if l.uid == me || staff {
                        resp.context_menu(|ui| {
                            crate::menu::style(ui);
                            if ui.button(if l.uid == me { "Take down my line" } else { "Take this line down" }).clicked() {
                                take_down = Some(l.id);
                                ui.close();
                            }
                        });
                    }
                    ui.add_space(4.0);
                }
            });
            // Typing.
            let input = Rect::from_min_max(pos2(r.left() + 12.0, r.bottom() - 44.0), pos2(r.right() - 12.0, r.bottom() - 12.0));
            let mut iui = ui.new_child(egui::UiBuilder::new().max_rect(input));
            let te = egui::TextEdit::singleline(&mut self.poker.chat_text).hint_text("Say something to the casino…").char_limit(LONGEST).desired_width(input.width()).font(theme::font(14.0));
            let resp = iui.add(te);
            if std::mem::take(&mut self.poker.chat_focus) {
                resp.request_focus();
            }
            if resp.lost_focus() && iui.input(|i| i.key_pressed(egui::Key::Enter)) {
                let text = self.poker.chat_text.trim().to_string();
                if !text.is_empty() {
                    send = Some(text);
                    self.poker.chat_text.clear();
                }
                resp.request_focus();
            }
        });
        if close || ctx.input(|i| i.key_pressed(egui::Key::Escape)) && self.poker.chat_open {
            self.poker.chat_open = false;
        }
        if let Some(text) = send {
            self.engine.casino_chat(&s.key, &text);
        }
        if let Some(id) = take_down {
            self.engine.casino_chat_delete(&s.key, id);
        }
    }
}
