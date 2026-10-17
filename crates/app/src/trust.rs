//! "Make sure it's them": the shields by people's names (made sure of, vouched for, or not the
//! person you made sure of under that name), the section in someone's profile that starts it,
//! the cards that carry it through (comparing words, asking something only they'd know), and
//! the dialog for writing a question. The engine does the work (`ingle_engine::verify`).

use eframe::egui::{self, Align, Align2, Color32, CornerRadius, Layout, Pos2, Rect, RichText, Sense, Shape, Stroke, pos2, vec2};
use ingle_engine::config::VerifiedHow;
use ingle_engine::state::{AppState, HandshakeKind, HandshakeStage, HandshakeView, ServerView, Trust};

use crate::app::{Act, Dialog, IngleApp, avatar};
use crate::theme;

/// Made sure of: a green shield.
pub const SURE: Color32 = Color32::from_rgb(0x3b, 0xa5, 0x5d);
/// Not the one you made sure of: red.
pub const WRONG: Color32 = Color32::from_rgb(0xe5, 0x48, 0x4d);

fn shield_shape(r: Rect) -> Vec<Pos2> {
    let (l, t, w, h) = (r.left(), r.top(), r.width(), r.height());
    vec![
        pos2(l + w * 0.5, t),
        pos2(l + w * 0.95, t + h * 0.16),
        pos2(l + w * 0.9, t + h * 0.55),
        pos2(l + w * 0.5, t + h),
        pos2(l + w * 0.1, t + h * 0.55),
        pos2(l + w * 0.05, t + h * 0.16),
    ]
}

fn tick(p: &egui::Painter, r: Rect, c: Color32) {
    let w = r.width();
    let pts = vec![pos2(r.left() + w * 0.28, r.top() + w * 0.5), pos2(r.left() + w * 0.45, r.top() + w * 0.66), pos2(r.left() + w * 0.74, r.top() + w * 0.34)];
    p.add(Shape::line(pts, Stroke::new(w * 0.12, c)));
}

/// The shield for how sure I can be of someone (nothing when there's nothing to go on).
pub fn shield(p: &egui::Painter, r: Rect, trust: &Trust) {
    match trust {
        Trust::Verified => {
            p.add(Shape::convex_polygon(shield_shape(r), SURE, Stroke::NONE));
            tick(p, r, Color32::WHITE);
        }
        Trust::Vouched(_) => {
            p.add(Shape::closed_line(shield_shape(r.shrink(0.5)), Stroke::new(1.4f32, SURE)));
            tick(p, r, SURE);
        }
        Trust::Impostor(_) => {
            p.add(Shape::convex_polygon(shield_shape(r), WRONG, Stroke::NONE));
            let c = r.center();
            p.line_segment([pos2(c.x, r.top() + r.height() * 0.24), pos2(c.x, r.top() + r.height() * 0.56)], Stroke::new(r.width() * 0.13, Color32::WHITE));
            p.circle_filled(pos2(c.x, r.top() + r.height() * 0.72), r.width() * 0.07, Color32::WHITE);
        }
        Trust::Unknown => {}
    }
}

/// A shield outline with a tick, for buttons that start making sure of someone.
pub fn shield_icon(p: &egui::Painter, r: Rect, c: Color32) {
    let r = Rect::from_center_size(r.center(), vec2(r.height() * 0.78, r.height()));
    p.add(Shape::closed_line(shield_shape(r.shrink(0.8)), Stroke::new(1.6f32, c)));
    tick(p, r, c);
}

/// What a shield means, for its hover text.
pub fn meaning(st: &AppState, trust: &Trust, name: &str) -> String {
    match trust {
        Trust::Verified => format!("You made sure it's really {name}"),
        Trust::Vouched(by) => format!("Vouched for by {}, who you made sure of", names(st, by)),
        Trust::Impostor(real) => format!("Not the {} you made sure of: someone else with a name like theirs", st.verified.get(real).map(|v| v.name.as_str()).unwrap_or(name)),
        Trust::Unknown => String::new(),
    }
}

fn names(st: &AppState, uids: &[String]) -> String {
    let mut n: Vec<String> = uids.iter().map(|u| st.verified.get(u).map(|v| v.name.clone()).unwrap_or_else(|| st.name_of(u))).collect();
    n.truncate(3);
    match n.len() {
        0 => "someone".into(),
        1 => n.remove(0),
        2 => format!("{} and {}", n[0], n[1]),
        _ => format!("{}, {} and {}", n[0], n[1], n[2]),
    }
}

/// A shield in a row of text, right after a name, `size` points tall (with its meaning on hover).
/// Nothing when there's nothing to show.
pub fn badge(ui: &mut egui::Ui, st: &AppState, uid: &str, name: &str, size: f32) {
    if uid == st.my_uid {
        return;
    }
    let trust = st.trust(uid, name);
    if trust == Trust::Unknown {
        return;
    }
    let (r, resp) = ui.allocate_exact_size(vec2(size * 0.86, size), Sense::hover());
    shield(ui.painter(), r, &trust);
    resp.on_hover_text(meaning(st, &trust, name));
}

/// A shield painted at `at` (its left middle), for rows drawn by hand (hovering it says what it
/// means). Returns how wide it was (0 when there's none).
pub fn badge_at(ui: &egui::Ui, st: &AppState, uid: &str, name: &str, at: Pos2, size: f32) -> f32 {
    let w = paint_badge(ui.painter(), st, uid, name, at, size);
    if w > 0.0 {
        let r = Rect::from_min_size(pos2(at.x, at.y - size / 2.0), vec2(w, size));
        ui.interact(r, ui.id().with(("shield", uid)), Sense::hover()).on_hover_text(meaning(st, &st.trust(uid, name), name));
    }
    w
}

/// The shield alone (for rows whose own hover text says what it means). Returns how wide it
/// was (0 when there's none).
pub fn paint_badge(p: &egui::Painter, st: &AppState, uid: &str, name: &str, at: Pos2, size: f32) -> f32 {
    if uid == st.my_uid {
        return 0.0;
    }
    let trust = st.trust(uid, name);
    if trust == Trust::Unknown {
        return 0.0;
    }
    shield(p, Rect::from_min_size(pos2(at.x, at.y - size / 2.0), vec2(size * 0.86, size)), &trust);
    size * 0.86
}

fn how_text(how: VerifiedHow) -> &'static str {
    match how {
        VerifiedHow::Words => "You compared words",
        VerifiedHow::Question => "You asked something only they'd know",
    }
}

impl IngleApp {
    /// In someone's profile: how sure you can be it's them, and the ways to make sure. True when
    /// one of them was chosen.
    pub(crate) fn trust_section(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, uid: &str, acts: &mut Vec<Act>) -> bool {
        if uid == st.my_uid {
            return false;
        }
        let name = s.member_name(uid);
        let trust = st.trust(uid, &name);
        ui.label(RichText::new("MAKE SURE IT'S THEM").font(theme::bold_font(11.0)).color(theme::DIM()));
        ui.add_space(2.0);
        match &trust {
            Trust::Verified => {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                    shield(ui.painter(), r, &trust);
                    ui.label(RichText::new(format!("It's really {name}")).font(theme::bold_font(13.5)).color(SURE));
                });
                if let Some(v) = st.verified.get(uid) {
                    let when = chrono::DateTime::from_timestamp_millis(v.at_ms).map(|d| d.with_timezone(&chrono::Local).format("%b %-d, %Y").to_string()).unwrap_or_default();
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(format!("{} on {when}.", how_text(v.how))).size(12.0).color(theme::MUTED()));
                        if ui.add(egui::Button::new(RichText::new("Forget").size(12.0).color(theme::DIM())).frame(false)).on_hover_text("Stop counting them as made sure of").clicked() {
                            acts.push(Act::ForgetVerified(uid.to_string()));
                        }
                    });
                }
                return false;
            }
            Trust::Impostor(real) => {
                let real_name = st.verified.get(real).map(|v| v.name.clone()).unwrap_or_default();
                crate::dialogs::notice(ui, &format!("Not the {real_name} you made sure of: someone else, with a name like theirs. Be careful what you tell them."), true);
                ui.add_space(4.0);
            }
            Trust::Vouched(by) => {
                ui.horizontal_wrapped(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                    shield(ui.painter(), r, &trust);
                    ui.label(RichText::new(format!("Vouched for by {}, who you made sure of.", names(st, by))).size(12.5).color(theme::MUTED()));
                });
                ui.add_space(2.0);
            }
            Trust::Unknown => {
                ui.label(RichText::new("Nothing says yet that it's really them. Take ten seconds:").size(12.5).color(theme::MUTED()));
            }
        }
        let can = s.has(ingle_proto::control::HANDSHAKE_FEATURE);
        let online = s.members.get(uid).is_some_and(|m| m.online) && s.status.is_connected();
        let mut chose = false;
        ui.add_enabled_ui(can && online, |ui| {
            ui.horizontal(|ui| {
                if ui.add(theme::primary_button("Compare words")).on_hover_text("Both of you see six words: say them to each other (in a call, on the phone or in person)").clicked() {
                    acts.push(Act::CompareWords { key: s.key.clone(), uid: uid.to_string() });
                    chose = true;
                }
                if ui.button("Ask something…").on_hover_text("Ask something only they'd know: when you can't talk right now").clicked() {
                    self.ui.dialog = Some(Dialog::AskThem { key: s.key.clone(), uid: uid.to_string(), question: String::new(), answer: String::new() });
                    chose = true;
                }
            });
        });
        if !can {
            ui.label(RichText::new("This server needs updating (Ingle 0.7.6) first.").size(11.5).color(theme::DIM()));
        } else if !online {
            ui.label(RichText::new(format!("When {name} is online.")).size(11.5).color(theme::DIM()));
        }
        chose
    }

    /// "Make sure it's them" in the menu from right-clicking someone: compare words, or ask
    /// something only they'd know (nothing for yourself or someone you made sure of already).
    /// True when one was chosen.
    pub(crate) fn make_sure_menu(&mut self, ui: &mut egui::Ui, st: &AppState, s: &ServerView, uid: &str, acts: &mut Vec<Act>) -> bool {
        // (A bot isn't a person to make sure of.)
        if uid == st.my_uid || st.verified.contains_key(uid) || s.is_bot(uid) {
            return false;
        }
        let name = s.member_name(uid);
        let why_not = if !s.has(ingle_proto::control::HANDSHAKE_FEATURE) {
            Some(format!("{} needs updating to Ingle 0.7.6 first", s.name))
        } else if !s.status.is_connected() || !s.members.get(uid).is_some_and(|m| m.online) {
            Some(format!("When {name} is online"))
        } else {
            None
        };
        let mut chose = false;
        let menu = ui.add_enabled_ui(why_not.is_none(), |ui| {
            ui.menu_button("Make sure it's them", |ui| {
                if ui.button("Compare words").on_hover_text("Both of you see six words: say them to each other (in a call, on the phone or in person)").clicked() {
                    acts.push(Act::CompareWords { key: s.key.clone(), uid: uid.to_string() });
                    chose = true;
                }
                if ui.button("Ask something only they'd know…").on_hover_text("When you can't talk right now").clicked() {
                    self.ui.dialog = Some(Dialog::AskThem { key: s.key.clone(), uid: uid.to_string(), question: String::new(), answer: String::new() });
                    chose = true;
                }
                if chose {
                    ui.close();
                }
            })
            .response
        });
        if let Some(why) = why_not {
            menu.inner.on_disabled_hover_text(why);
        }
        chose
    }

    /// Writing the question (and the answer you expect).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn ask_dialog(&mut self, ui: &mut egui::Ui, st: &AppState, key: String, uid: String, mut question: String, mut answer: String, acts: &mut Vec<Act>, close: &mut bool) {
        ui.set_width(440.0);
        let name = st.server(&key).map(|s| s.member_name(&uid)).unwrap_or_else(|| st.name_of(&uid));
        crate::dialogs::title(ui, &format!("Make sure it's really {name}"));
        ui.label(
            RichText::new(format!(
                "Ask something only {name} would know, and write the answer you expect. {name} types theirs: Ingle tells you both whether they match, and never sends either answer anywhere."
            ))
            .color(theme::MUTED()),
        );
        crate::dialogs::field_label(ui, "Your question");
        ui.add(egui::TextEdit::singleline(&mut question).hint_text("Where did we first meet?").char_limit(ingle_proto::handshake::QUESTION_MAX).desired_width(f32::INFINITY));
        crate::dialogs::field_label(ui, "The answer you expect");
        ui.add(egui::TextEdit::singleline(&mut answer).hint_text("Only you two know it").char_limit(ingle_proto::handshake::ANSWER_MAX).password(true).desired_width(f32::INFINITY));
        ui.label(RichText::new("Capitals, spaces and punctuation don't matter. Not something anyone could guess or look up (and not yes or no). The server sees the question, never the answer.").size(12.0).color(theme::DIM()));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let ready = !question.trim().is_empty() && !ingle_proto::handshake::normalize(&answer).is_empty();
            if ui.add_enabled(ready, theme::primary_button("Ask")).clicked() {
                acts.push(Act::AskThem { key: key.clone(), uid: uid.clone(), question: question.trim().to_string(), answer: std::mem::take(&mut answer) });
                *close = true;
            }
            if ui.button("Cancel").clicked() {
                *close = true;
            }
        });
        if !*close {
            self.ui.dialog = Some(Dialog::AskThem { key, uid, question, answer });
        }
    }

    /// The cards for making sure of people, top right: asked, waiting, the words, the outcome.
    pub(crate) fn handshake_cards(&mut self, ctx: &egui::Context, st: &AppState, acts: &mut Vec<Act>) {
        if st.handshakes.is_empty() {
            return;
        }
        let screen = ctx.content_rect();
        // (Under the header: over it, they'd cover its buttons.)
        let top = screen.top() + if crate::titlebar::enabled() { crate::titlebar::HEIGHT } else { 0.0 } + 56.0;
        let w = 360.0;
        let shown = egui::Area::new(egui::Id::new("handshake-cards")).order(egui::Order::Foreground).fixed_pos(pos2(screen.right() - w - 16.0, top)).show(ctx, |ui| {
            ui.set_width(w);
            for v in st.handshakes.iter().rev().take(3) {
                egui::Frame::new()
                    .fill(theme::ELEV())
                    .stroke(Stroke::new(1.0_f32, if matches!(v.stage, HandshakeStage::Sure) { SURE } else { theme::LINE() }))
                    .corner_radius(14)
                    .shadow(egui::epaint::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
                    .inner_margin(egui::Margin::same(14))
                    .show(ui, |ui| {
                        ui.set_width(w - 28.0);
                        self.handshake_card(ui, st, v, acts);
                    });
                ui.add_space(8.0);
            }
        });
        self.ui.overlay_rects.push(shown.response.rect);
    }

    fn handshake_card(&mut self, ui: &mut egui::Ui, st: &AppState, v: &HandshakeView, acts: &mut Vec<Act>) {
        let Some(s) = st.server(&v.server) else { return };
        let name = s.member_name(&v.with);
        let sid = v.sid.clone();
        // Who, and what this is.
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
            let face = self.face(ui.ctx(), s, &v.with);
            avatar(ui.painter(), r.center(), 16.0, &v.with, &name, false, theme::ELEV(), face);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).font(theme::bold_font(14.5)).color(theme::TEXT()));
                    badge(ui, st, &v.with, &name, 14.0);
                });
                let what = match (v.kind, v.mine) {
                    (HandshakeKind::Words, _) => "Comparing words",
                    (HandshakeKind::Ask, true) => "You asked something only they'd know",
                    (HandshakeKind::Ask, false) => "Asks something only you'd know",
                };
                ui.label(RichText::new(what).size(12.0).color(theme::MUTED()));
            });
        });
        ui.add_space(8.0);
        match &v.stage {
            HandshakeStage::Asked => match v.kind {
                HandshakeKind::Words => {
                    ui.label(RichText::new(format!("{name} wants to make sure you're both who you say, by comparing six words. Best when you can talk: in a call, on the phone, or in person.")).color(theme::TEXT()));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button("Compare words")).clicked() {
                            acts.push(Act::AcceptWords(sid.clone()));
                        }
                        if ui.button("Not now").clicked() {
                            acts.push(Act::DeclineHandshake(sid.clone()));
                        }
                    });
                }
                HandshakeKind::Ask => {
                    ui.label(RichText::new(format!("{name} wants to make sure it's you, and asks:")).color(theme::TEXT()));
                    egui::Frame::new().fill(theme::ELEV2()).corner_radius(8).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.add(egui::Label::new(RichText::new(&v.question).italics().color(theme::TEXT())).wrap());
                    });
                    let answer = self.ui.handshake_answers.entry(sid.clone()).or_default();
                    let field = ui.add(egui::TextEdit::singleline(answer).hint_text("Your answer").char_limit(ingle_proto::handshake::ANSWER_MAX).password(true).desired_width(f32::INFINITY));
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.label(RichText::new("Your answer never leaves this PC: only whether it matches theirs.").size(11.5).color(theme::DIM()));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let ready = !ingle_proto::handshake::normalize(answer).is_empty();
                        if (ui.add_enabled(ready, theme::primary_button("Answer")).clicked() || (enter && ready)) && ready {
                            acts.push(Act::AnswerQuestion { sid: sid.clone(), answer: std::mem::take(answer) });
                        }
                        if ui.button("Not now").clicked() {
                            acts.push(Act::DeclineHandshake(sid.clone()));
                        }
                    });
                }
            },
            HandshakeStage::Waiting => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    let text = match (v.kind, v.mine) {
                        (HandshakeKind::Words, true) => format!("Waiting for {name} to say yes…"),
                        (HandshakeKind::Ask, true) => format!("Waiting for {name}'s answer…"),
                        _ => format!("Waiting for {name}'s Ingle…"),
                    };
                    ui.label(RichText::new(text).color(theme::MUTED()));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Cancel").clicked() {
                            acts.push(Act::DeclineHandshake(sid.clone()));
                        }
                    });
                });
            }
            HandshakeStage::Compare { words, they_said } => {
                ui.label(RichText::new(format!("Say these to {name}, or listen to them say theirs. They should be the same, in the same order:")).color(theme::TEXT()));
                ui.add_space(6.0);
                word_grid(ui, words);
                ui.add_space(6.0);
                match they_said {
                    Some(true) => {
                        ui.label(RichText::new(format!("{name} says they match.")).color(SURE));
                    }
                    Some(false) => {
                        ui.label(RichText::new(format!("{name} says they don't match.")).color(WRONG));
                    }
                    None => {}
                }
                ui.horizontal(|ui| {
                    let yes = egui::Button::new(RichText::new("They match").color(Color32::WHITE).font(theme::bold_font(14.0))).fill(SURE).corner_radius(CornerRadius::same(6));
                    if ui.add(yes).clicked() {
                        acts.push(Act::WordsSaid { sid: sid.clone(), matched: true });
                    }
                    if ui.button("They don't").clicked() {
                        acts.push(Act::WordsSaid { sid: sid.clone(), matched: false });
                    }
                });
            }
            HandshakeStage::Sure => {
                ui.horizontal_wrapped(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(16.0, 18.0), Sense::hover());
                    shield(ui.painter(), r, &Trust::Verified);
                    ui.label(RichText::new(format!("It's really {name}.")).font(theme::bold_font(14.0)).color(SURE));
                });
                ui.label(RichText::new("Ingle remembers: this shield shows by their name from now on.").size(12.0).color(theme::MUTED()));
                ui.add_space(4.0);
                if ui.button("Done").clicked() {
                    acts.push(Act::DismissHandshake(sid.clone()));
                }
            }
            HandshakeStage::AnswersMatched => {
                ui.label(RichText::new(format!("Your answer matched the one {name} expected. If only {name} could have known it, it's really them.")).color(theme::TEXT()));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let yes = egui::Button::new(RichText::new(format!("It's {name}")).color(Color32::WHITE).font(theme::bold_font(14.0))).fill(SURE).corner_radius(CornerRadius::same(6));
                    if ui.add(yes).clicked() {
                        acts.push(Act::ItIsThem(sid.clone()));
                    }
                    if ui.button("Not sure").on_hover_text("Anyone could have known that").clicked() {
                        acts.push(Act::DismissHandshake(sid.clone()));
                    }
                });
            }
            HandshakeStage::Failed(why) => {
                ui.add(egui::Label::new(RichText::new(why).color(WRONG)).wrap());
                ui.add_space(4.0);
                if ui.button("OK").clicked() {
                    acts.push(Act::DismissHandshake(sid.clone()));
                }
            }
        }
    }
}

/// The six words, three to a row, each with its picture over it.
fn word_grid(ui: &mut egui::Ui, words: &[(String, String)]) {
    let cols = 3;
    let cell = vec2((ui.available_width() - 8.0 * (cols as f32 - 1.0)) / cols as f32, 58.0);
    for row in words.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            for (emoji, word) in row {
                let (r, _) = ui.allocate_exact_size(cell, Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(8), theme::ELEV2());
                crate::emoji::paint_text(ui.painter(), pos2(r.center().x, r.top() + 20.0), Align2::CENTER_CENTER, emoji, theme::font(24.0), theme::TEXT(), None);
                ui.painter().text(pos2(r.center().x, r.bottom() - 13.0), Align2::CENTER_CENTER, word, theme::bold_font(13.5), theme::TEXT());
            }
        });
    }
}
