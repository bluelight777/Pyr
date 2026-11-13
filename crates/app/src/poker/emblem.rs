//! Staff titles at the tables (0.13.4/0.13.6), as 0.13.28 shows them: over the seat's picture
//! the Owner of the Game's winged shield with a crown, a Game Master's with a gem, an
//! Ambassador's small silver shield, each in its own glow with light turning behind it and
//! sparkles coming and going; at the plate's top a pill naming it (GAME MASTER for both the
//! Owner and Game Masters, AMBASSADOR). Hidden when they've hidden their title (`hide_title`).

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Stroke, StrokeKind, epaint, vec2};
use pyr_poker::net::Badge;

use crate::theme;

mod shapes {
    use super::epaint;
    include!("emblem_shapes.rs");
}

pub use shapes::EmblemKind;

/// The emblem over a seat whose picture's middle is `at`, the table at scale `k`.
pub fn paint(p: &egui::Painter, at: Pos2, k: f32, badge: Badge) {
    let kind = match badge {
        Badge::Owner => EmblemKind::Owner,
        Badge::GameMaster => EmblemKind::GameMaster,
        Badge::Ambassador => EmblemKind::Ambassador,
    };
    // (Seconds that keep counting while the app runs: the glow's beat, the rays, the sparkles.)
    let t = p.ctx().input(|i| i.time) as f32;
    p.extend(shapes::emblem(kind, at, k, t));
    p.ctx().request_repaint();
}

/// The title's pill, its middle at `mid` (where a bot's level goes: the plate's top).
pub fn label(p: &egui::Painter, mid: Pos2, k: f32, badge: Badge) {
    let (words, fill) = match badge {
        Badge::Ambassador => ("AMBASSADOR", Color32::from_rgb(42, 106, 200)),
        Badge::Owner | Badge::GameMaster => ("GAME MASTER", Color32::from_rgb(176, 122, 20)),
    };
    let r = Rect::from_center_size(mid, vec2(12.0 + 6.6 * words.len() as f32, 15.0) * k);
    let round = egui::CornerRadius::same((r.height() / 2.0).round() as u8);
    p.rect_filled(r, round, fill);
    p.rect_stroke(r, round, Stroke::new(1.0f32, Color32::from_rgb(255, 224, 138)), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, words, theme::bold_font(9.5 * k), Color32::WHITE);
}
