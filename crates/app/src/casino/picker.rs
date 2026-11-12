//! The tables to pick from, along the top of a table game (0.13.4): "TABLES pick one", then a
//! card a table, with its name, YOU'RE HERE or how many seats are open, its bets and a dot a
//! seat (green where someone sits). As 0.13.28 draws them, measured off its screen; better than
//! 0.13.28 in one thing: a table's bets never run over its seats (Crown Room's did), they're said
//! shorter where they'd have to.

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};

use super::{col, vgrad_round};
use crate::poker::cards::{chips_short, chips_text};
use crate::theme;

/// The strip's height (the table below starts under it).
pub const STRIP_H: f32 = 72.0;
const CARD_W: f32 = 234.0;
const CARD_H: f32 = 56.0;
const GAP: f32 = 10.0;
/// The "+ Another table" button's width (blackjack's).
pub const ANOTHER_W: f32 = 130.0;

/// A table on the strip.
pub struct Pick {
    pub id: u64,
    pub name: String,
    pub min: u64,
    pub max: u64,
    /// People and bots sitting.
    pub seated: usize,
    /// Something to say after its bets (a craps table's point: "point 4").
    pub note: Option<String>,
}

/// Draws the strip's words and cards (`seats` a table), leaving `keep` at the strip's right for
/// a button of the game's own. Returns the table clicked, never the one I'm at.
pub fn cards(ui: &mut egui::Ui, strip: Rect, tables: &[Pick], seats: usize, at: Option<u64>, salt: &str, keep: f32) -> Option<u64> {
    let p = ui.painter().clone();
    p.text(pos2(strip.left() + 16.0, strip.top() + 21.0), Align2::LEFT_TOP, "TABLES", theme::bold_font(12.0), col::GOLD2);
    p.text(pos2(strip.left() + 16.0, strip.top() + 39.0), Align2::LEFT_TOP, "pick one", theme::font(11.0), col::MUTED);
    let mut clicked = None;
    let mut x = strip.left() + 86.0;
    for t in tables {
        let r = Rect::from_min_size(pos2(x, strip.top() + 8.0), vec2(CARD_W, CARD_H));
        x += CARD_W + GAP;
        if r.right() > strip.right() - keep {
            break;
        }
        let on = Some(t.id) == at;
        let resp = ui.interact(r, ui.id().with((salt, t.id)), Sense::click());
        if on {
            vgrad_round(&p, r, 14.0, col::GOLD.gamma_multiply(0.40), col::GOLD.gamma_multiply(0.17));
            p.rect_stroke(r, theme::r(14), Stroke::new(2.0f32, col::GOLD2), StrokeKind::Inside);
        } else {
            p.rect_filled(r, theme::r(14), Color32::from_white_alpha(if resp.hovered() { 22 } else { 11 }));
            p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, Color32::from_white_alpha(50)), StrokeKind::Inside);
        }
        p.text(r.left_top() + vec2(14.0, 9.0), Align2::LEFT_TOP, &t.name, theme::bold_font(16.0), if on { col::GOLD2 } else { Color32::WHITE });
        // YOU'RE HERE, or the seats open (FULL when none).
        let open = seats.saturating_sub(t.seated);
        let (words, fill, ink) = if on {
            ("YOU'RE HERE".to_string(), col::GOLD2, Color32::from_rgb(0x2a, 0x10, 0x04))
        } else if open == 0 {
            ("FULL".to_string(), Color32::from_rgb(0x8a, 0x24, 0x1c), Color32::WHITE)
        } else {
            (format!("{open} OPEN"), Color32::from_rgb(0x1e, 0x6a, 0x3e), Color32::WHITE)
        };
        let g = p.layout_no_wrap(words, theme::bold_font(10.0), ink);
        let pill = Rect::from_min_size(pos2(r.right() - 12.0 - g.size().x - 14.0, r.top() + 9.0), vec2(g.size().x + 14.0, 17.0));
        p.rect_filled(pill, egui::CornerRadius::same(9), fill);
        p.galley(pos2(pill.left() + 7.0, r.top() + 12.0), g, ink);
        // A dot a seat, the taken ones green.
        let dot = |i: usize| pos2(r.right() - 14.0 - (seats - 1 - i) as f32 * 13.0, r.top() + 40.0);
        for i in 0..seats {
            // (A circle's stroke goes round its outside: 4 here rings at 4.5, as 0.13.28's.)
            if i < t.seated {
                p.circle_filled(dot(i), 4.5, Color32::from_rgb(0x3a, 0xe0, 0x7a));
            } else {
                p.circle_stroke(dot(i), 4.0, Stroke::new(1.0f32, Color32::from_white_alpha(110)));
            }
        }
        // Its bets, as short as they must be to stay clear of the dots.
        let room = dot(0).x - 4.5 - 6.0 - (r.left() + 14.0);
        let color = if on { Color32::from_rgb(0xff, 0xe6, 0xa8) } else { Color32::from_rgb(0xdf, 0xb2, 0x4c) };
        // (A craps table's point after them, as 0.13.28 says it: "Bets 10–1,000  ·  point 4".)
        let note = t.note.as_ref().map(|n| format!("  ·  {n}")).unwrap_or_default();
        let bets = [
            format!("Bets {}–{}{note}", chips_text(t.min), chips_text(t.max)),
            format!("{}–{}{note}", chips_text(t.min), chips_text(t.max)),
            format!("{}–{}{note}", chips_short(t.min), chips_short(t.max)),
            format!("{}–{}", chips_short(t.min), chips_short(t.max)),
        ];
        let g = bets
            .iter()
            .map(|b| p.layout_no_wrap(b.clone(), theme::bold_font(12.5), color))
            .find(|g| g.size().x <= room)
            .unwrap_or_else(|| p.layout_no_wrap(bets[3].clone(), theme::bold_font(12.5), color));
        p.galley(r.left_top() + vec2(14.0, 33.0), g, color);
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !on {
            clicked = Some(t.id);
        }
    }
    clicked
}

/// Blackjack's "+ Another table", at the strip's right: whether it was clicked.
pub fn another(ui: &mut egui::Ui, strip: Rect, tip: &str) -> bool {
    let p = ui.painter().clone();
    let r = Rect::from_min_size(Pos2::new(strip.right() - 14.0 - ANOTHER_W, strip.top() + 8.0), vec2(ANOTHER_W, CARD_H));
    let resp = ui.interact(r, ui.id().with("table-another"), Sense::click());
    p.rect_filled(r, theme::r(14), Color32::from_white_alpha(if resp.hovered() { 22 } else { 11 }));
    p.text(r.center(), Align2::CENTER_CENTER, "+ Another table", theme::bold_font(13.0), col::TEXT);
    resp.on_hover_text(tip).clicked()
}

#[cfg(test)]
mod tests {
    /// The cards sit where 0.13.28's do: the first at 86 from the strip's left, 244 apart.
    #[test]
    fn cards_where_0_13_28_has_them() {
        assert_eq!(super::CARD_W + super::GAP, 244.0);
        assert_eq!(super::STRIP_H - super::CARD_H, 16.0);
    }
}
