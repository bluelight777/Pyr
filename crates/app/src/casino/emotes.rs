//! Emotes at the tables: eighteen stickers made with Meshy (the Pyr flame's nine faces, and nine
//! casino ones: GG, ALL IN, aces, a crying chip, laughing dice, a cool chip, a bad beat, a bag of
//! chips, the crown), sent as `e:<name>` and shown big over the sender's seat: popping in, a
//! bounce, a float up, and gone. At poker, blackjack, craps and baccarat alike.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_poker::net::TableGame;

use super::{col, ease_back, pics, radial};
use crate::theme;

/// Each sticker: its name on the wire (after `e:`), and what it says (a tooltip).
pub const EMOTES: [(&str, &str); 18] = [
    ("laugh", "Crying laughing"),
    ("cry", "Crying"),
    ("cool", "Cool"),
    ("wow", "Wow"),
    ("angry", "Steaming"),
    ("think", "Hmm"),
    ("nice", "Nice!"),
    ("love", "Love it"),
    ("rich", "Rich"),
    ("gg", "GG"),
    ("allin", "All in"),
    ("aces", "Aces"),
    ("sob", "Bad chip day"),
    ("lol", "Dice LOL"),
    ("boss", "Boss chip"),
    ("badbeat", "Bad beat"),
    ("bag", "Bag of chips"),
    ("king", "King of the table"),
];

/// The ones on the bar (the rest are a click away).
pub const QUICK: [&str; 6] = ["gg", "nice", "laugh", "wow", "cry", "allin"];

/// How long one shows over a seat.
pub const SHOW_S: f32 = 3.2;

/// A sticker's place on the sheet, from what's sent (`e:gg`).
pub fn index(emote: &str) -> Option<usize> {
    let name = emote.strip_prefix("e:")?;
    EMOTES.iter().position(|(n, _)| *n == name)
}

pub fn wire(i: usize) -> String {
    format!("e:{}", EMOTES[i].0)
}

/// Sticker `i` drawn in `r`.
pub fn draw(p: &egui::Painter, ctx: &egui::Context, i: usize, r: Rect, tint: Color32) {
    let (cx, cy) = ((i % 6) as f32, (i / 6) as f32);
    let uv = Rect::from_min_max(pos2(cx / 6.0, cy / 3.0), pos2((cx + 1.0) / 6.0, (cy + 1.0) / 3.0));
    let px = ((r.width() * 6.0).max(384.0) as u32).min(1152);
    pics::part(p, ctx, "emotes", uv, r, px, tint);
}

/// An emote over a seat at `at`, `k` seconds after it came: a pop, a bounce, a float up, gone.
/// (Anything not a sticker is drawn as it came: an emoji or a word, as before.)
pub fn pop(p: &egui::Painter, ctx: &egui::Context, emote: &str, at: Pos2, k: f32, size: f32) {
    if !(0.0..SHOW_S).contains(&k) {
        return;
    }
    let fade = (1.0 - (k - (SHOW_S - 0.6)).max(0.0) / 0.6).clamp(0.0, 1.0);
    let lift = 26.0 * (k / SHOW_S);
    let c = at - vec2(0.0, lift);
    match index(emote) {
        Some(i) => {
            let grow = ease_back((k / 0.38).min(1.0));
            // (A little bounce after landing, then it breathes.)
            let bob = if k > 0.38 { 1.0 + 0.05 * ((k - 0.38) * 9.0).sin() * (-(k - 0.38) * 2.2).exp() } else { 1.0 };
            let s = size * grow.max(0.05) * bob;
            radial(p, c, s * 0.75, col::GOLD.gamma_multiply(0.28 * fade), Color32::TRANSPARENT);
            draw(p, ctx, i, Rect::from_center_size(c, vec2(s, s)), Color32::WHITE.gamma_multiply(fade));
        }
        None => {
            p.circle_filled(c, size * 0.3, Color32::from_black_alpha((160.0 * fade) as u8));
            crate::emoji::paint_text(p, c, Align2::CENTER_CENTER, emote, theme::bold_font(size * 0.26), Color32::WHITE.gamma_multiply(fade), None);
        }
    }
}

/// The emote bar: the quick ones, and More (all eighteen). What was picked, if anything (at
/// most one every so often, so nobody floods the table).
pub fn bar(ui: &mut egui::Ui, at: Pos2, size: f32, id: &str, last: &mut Option<Instant>) -> Option<String> {
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    let ready = ready(last);
    let mut picked = None;
    let mut x = at.x;
    for name in QUICK {
        let Some(i) = index(&format!("e:{name}")) else { continue };
        let r = Rect::from_min_size(pos2(x, at.y), vec2(size, size));
        x += size + 6.0;
        let resp = ui.interact(r, ui.id().with((id, name)), Sense::click());
        let hot = resp.hovered() && ready;
        p.rect_filled(r, theme::r(12), if hot { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(12) });
        let grow = if hot { 1.08 } else { 0.92 };
        draw(&p, &ctx, i, Rect::from_center_size(r.center(), r.size() * grow), if ready { Color32::WHITE } else { Color32::from_white_alpha(110) });
        if resp.on_hover_text(EMOTES[i].1).clicked() && ready {
            picked = Some(wire(i));
        }
    }
    // More: all of them.
    let r = Rect::from_min_size(pos2(x, at.y), vec2(size, size));
    let resp = ui.interact(r, ui.id().with((id, "more")), Sense::click());
    p.rect_filled(r, theme::r(12), if resp.hovered() { Color32::from_white_alpha(34) } else { Color32::from_white_alpha(12) });
    p.rect_stroke(r, theme::r(12), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, "+", theme::bold_font(size * 0.5), col::GOLD2);
    let resp = resp.on_hover_text("All the emotes");
    if let Some(i) = grid(&resp, id, ready, egui::RectAlign::TOP_START) {
        picked = Some(wire(i));
    }
    if picked.is_some() {
        *last = Some(Instant::now());
    }
    picked
}

/// One button for them all (where a table's controls leave no room for the bar): a sticker that
/// opens all eighteen.
pub fn button(ui: &mut egui::Ui, r: Rect, id: &str, last: &mut Option<Instant>) -> Option<String> {
    let ctx = ui.ctx().clone();
    let p = ui.painter().clone();
    let ready = ready(last);
    let resp = ui.interact(r, ui.id().with((id, "emotes")), Sense::click());
    let hot = resp.hovered();
    p.rect_filled(r, theme::r(14), if hot { Color32::from_white_alpha(36) } else { Color32::from_white_alpha(16) });
    p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, if hot { col::GOLD } else { col::GOLD_DARK }), StrokeKind::Inside);
    let s = r.height().min(r.width()) * if hot { 0.98 } else { 0.86 };
    draw(&p, &ctx, 0, Rect::from_center_size(r.center(), vec2(s, s)), if ready { Color32::WHITE } else { Color32::from_white_alpha(110) });
    let resp = resp.on_hover_text("Emotes: send one to the table");
    let picked = grid(&resp, id, ready, egui::RectAlign::TOP_END).map(wire);
    if picked.is_some() {
        *last = Some(Instant::now());
    }
    picked
}

/// Whether one can be sent yet.
fn ready(last: &Option<Instant>) -> bool {
    last.is_none_or(|t| t.elapsed() > Duration::from_millis(1_200))
}

/// All eighteen, big, in a menu opened from `resp`; the one clicked.
fn grid(resp: &egui::Response, id: &str, ready: bool, align: egui::RectAlign) -> Option<usize> {
    let ctx = resp.ctx.clone();
    let mut picked = None;
    egui::Popup::menu(resp).align(align).show(|ui| {
        crate::menu::style(ui);
        ui.label(egui::RichText::new("Emotes").strong().color(col::GOLD2));
        egui::Grid::new((id, "emote-grid")).spacing(vec2(6.0, 6.0)).show(ui, |ui| {
            for (i, (_, words)) in EMOTES.iter().enumerate() {
                let (cell, resp) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::click());
                let hot = resp.hovered();
                ui.painter().rect_filled(cell, theme::r(10), if hot { Color32::from_white_alpha(30) } else { Color32::from_white_alpha(8) });
                draw(ui.painter(), &ctx, i, cell.shrink(if hot { 1.0 } else { 5.0 }), if ready { Color32::WHITE } else { Color32::from_white_alpha(120) });
                if resp.on_hover_text(*words).clicked() && ready {
                    picked = Some(i);
                    ui.close();
                }
                if i % 6 == 5 {
                    ui.end_row();
                }
            }
        });
    });
    picked
}

/// The emotes lately at a blackjack, craps or baccarat table, each over the seat it came from
/// (`seat_at`).
pub fn pops(p: &egui::Painter, ctx: &egui::Context, list: &[(TableGame, u64, u8, String, Instant)], game: TableGame, table: u64, size: f32, seat_at: impl Fn(u8) -> Pos2) {
    for (g, t, seat, e, at) in list {
        if *g == game && *t == table {
            pop(p, ctx, e, seat_at(*seat), at.elapsed().as_secs_f32(), size);
        }
    }
    if list.iter().any(|(g, t, .., at)| *g == game && *t == table && at.elapsed().as_secs_f32() < SHOW_S) {
        ctx.request_repaint();
    }
}
