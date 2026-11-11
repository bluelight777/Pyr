//! Pyr Casino's menu bar (0.13.6), across the top of every casino page as 0.13.28 draws it:
//! the casino's mark and name, Lobby and Games (a menu of every game), then the VIP Club, the
//! leaderboard, your profile, and for the staff, Staff and the control room; Keys and Fairness
//! at the right. On a wide window each button has its name by it; on a narrower one it's the icon
//! alone, its name on hovering. Every size and color is 0.13.28's, measured off its screen (its
//! icons pixel by pixel), most of them 2 left of their buttons' middles, as there.

use eframe::egui::{self, Align2, Color32, CornerRadius, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use eframe::emath::GuiRounding;

use super::col;
use crate::icons;
use crate::theme;

/// The bar's height.
pub const BAR_H: f32 = 58.0;
/// A button: 44 by 40, 9 from the bar's top, 48 apart.
const BUTTON: egui::Vec2 = egui::vec2(44.0, 40.0);
const STEP: f32 = 48.0;
/// The icons when idle, and pointed at or open.
const IDLE: Color32 = Color32::from_gray(200);
const LIT: Color32 = Color32::WHITE;
/// Lobby, and the games menu's corner.
const ORANGE: Color32 = Color32::from_rgb(255, 122, 26);
/// The bar's own dark, where the house's door is.
const DOOR: Color32 = Color32::from_rgb(21, 12, 13);
/// Wider than this, the buttons have their names by them (0.13.28: from a bar 1151 wide).
const WIDE: f32 = 1150.0;
/// A named button: the first one's left edge (from the bar's), its icon's middle and its name
/// (from its own), and how much wider it is than its name; Games' arrow takes a little more.
const NAMED_FROM: f32 = 165.0;
const NAMED_ICON: f32 = 22.0;
const NAMED_TEXT: f32 = 36.0;
const NAMED_PAD: f32 = 39.8;
const ARROW_W: f32 = 18.8;

/// What a button on the bar is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    Lobby,
    Games,
    Vip,
    Leaderboard,
    Profile,
    Staff,
    Cms,
    /// (Its button comes with the keys for every game, 0.13.26.)
    #[allow(dead_code)]
    Keys,
    Fairness,
}

impl Item {
    pub fn tip(self) -> &'static str {
        match self {
            Item::Lobby => "Lobby",
            Item::Games => "Games",
            Item::Vip => "VIP Club",
            Item::Leaderboard => "Leaderboard",
            Item::Profile => "Profile",
            Item::Staff => "Staff",
            Item::Cms => "Control room",
            Item::Keys => "Keys",
            Item::Fairness => "Fairness",
        }
    }
}

/// What the bar did: the item clicked, if one was, and where the Games button is (its menu
/// hangs from it).
pub struct Bar {
    pub clicked: Option<Item>,
    pub games: Option<Rect>,
}

/// The bar, across `bar` (`BAR_H` tall): `left` are the items from Lobby on, `right` the ones at
/// the right end, `on` the page showing (its button lit, a gold line under it), `open` an item
/// whose menu is open (warmed, an orange line under it).
pub fn bar(ui: &mut egui::Ui, bar: Rect, left: &[Item], right: &[Item], on: Option<Item>, open: Option<Item>) -> Bar {
    let p = ui.painter().clone();
    // Its dark, down from (28,16,16) to (14,8,10).
    super::vgrad(&p, bar, Color32::from_rgb(28, 16, 16), Color32::from_rgb(14, 8, 10));
    // The mark and the name.
    let (l, t) = (bar.left(), bar.top());
    p.circle_filled(pos2(l + 30.5, t + 28.5), 16.5, Color32::from_rgb(34, 25, 26));
    crate::poker::mark::icon(&p, pos2(l + 30.0, t + 29.0), 22.0, col::GOLD2);
    p.text(pos2(l + 54.0, t + 28.5), Align2::LEFT_CENTER, "PYR CASINO", theme::display_font("anton", 21.0), col::GOLD2);
    let wide = bar.width() > WIDE;
    let name_font = theme::bold_font(13.5);
    // (Laid out uncolored: each is drawn in its state's color.)
    let named = |item: Item| p.layout_no_wrap(item.tip().to_string(), name_font.clone(), Color32::PLACEHOLDER);
    let width = |item: Item| {
        if wide {
            named(item).size().x + NAMED_PAD + if item == Item::Games { ARROW_W } else { 0.0 }
        } else {
            BUTTON.x
        }
    };
    let mut out = Bar { clicked: None, games: None };
    let mut button = |ui: &mut egui::Ui, item: Item, r: Rect| {
        let id = ui.id().with(("casino-bar", item as u8));
        let mut resp = ui.interact(r, id, Sense::click());
        if !wide {
            resp = resp.on_hover_text(item.tip());
        }
        let lit = on == Some(item);
        let opened = open == Some(item);
        let hot = resp.hovered() || opened;
        if lit {
            // (On whole pixels, as a plain rectangle would be.)
            super::vgrad_round(&p, r.round_to_pixels(ui.ctx().pixels_per_point()), 9.0, Color32::from_rgb(67, 53, 40), Color32::from_rgb(59, 48, 37));
        } else if opened {
            p.rect_filled(r, CornerRadius::same(9), ORANGE.gamma_multiply(0.18));
        } else if hot {
            p.rect_filled(r, CornerRadius::same(10), Color32::from_rgba_unmultiplied(255, 255, 255, 12));
        }
        if lit || opened {
            // The line under it: gold for the page showing, orange for a menu open.
            let line = Rect::from_center_size(pos2(r.center().x, t + 55.5), vec2(r.width() - 20.0, 3.0));
            p.rect_filled(line, CornerRadius::same(1), if lit { col::GOLD2 } else { ORANGE });
        }
        let ink = if lit || hot { LIT } else { IDLE };
        let at = if wide { pos2(r.left() + NAMED_ICON + named_nudge(item), r.center().y) } else { r.center() };
        icon(&p, item, at, ink);
        if wide {
            let name = named(item);
            let x = r.left() + NAMED_TEXT;
            let end = x + name.size().x;
            p.galley(pos2(x, t + 29.0 - name.size().y / 2.0), name, ink);
            if item == Item::Games {
                // An arrow: down, or up while its menu is open.
                let c = end + 10.35;
                let (a, b) = if opened { (t + 31.0, t + 27.0) } else { (t + 27.2, t + 31.0) };
                p.add(Shape::line(vec![pos2(c - 3.8, a), pos2(c, b), pos2(c + 3.8, a)], Stroke::new(1.8f32, ink)));
            }
        }
        if item == Item::Games {
            out.games = Some(r);
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            out.clicked = Some(item);
        }
    };
    if wide {
        // Named: one after another, 4 apart (24 after Games, a line between).
        let mut x = l + NAMED_FROM;
        for (i, &item) in left.iter().enumerate() {
            let r = Rect::from_min_size(pos2(x, t + 9.0), vec2(width(item), BUTTON.y));
            button(ui, item, r);
            x = r.right() + 4.0;
            if i == 1 {
                let dx = r.right() + 12.0;
                p.line_segment([pos2(dx, t + 16.0), pos2(dx, t + 42.0)], Stroke::new(1.0f32, Color32::from_rgb(48, 40, 40)));
                x += 20.0;
            }
        }
        // From the right, the last one 14 in from the edge.
        let mut x = bar.right() - 14.0;
        for &item in right.iter().rev() {
            let w = width(item);
            let r = Rect::from_min_size(pos2(x - w, t + 9.0), vec2(w, BUTTON.y));
            button(ui, item, r);
            x = r.left() - 4.0;
        }
    } else {
        // From the left: Lobby's middle 187 in, Games, a line, then the rest.
        let mut x = l + 187.0;
        for (i, &item) in left.iter().enumerate() {
            button(ui, item, Rect::from_center_size(pos2(x, t + 29.0), BUTTON));
            x += STEP;
            if i == 1 {
                p.line_segment([pos2(l + 269.5, t + 16.0), pos2(l + 269.5, t + 42.0)], Stroke::new(1.0f32, Color32::from_rgb(48, 40, 40)));
                x += 20.0;
            }
        }
        // From the right: the last one's middle 36 in from the edge.
        let mut x = bar.right() - 36.0 - STEP * (right.len().saturating_sub(1)) as f32;
        for &item in right {
            button(ui, item, Rect::from_center_size(pos2(x, t + 29.0), BUTTON));
            x += STEP;
        }
    }
    out
}

/// How far right (left, if less than nothing) of where it'd be on its own an icon sits in a named
/// button (0.13.28's, measured).
fn named_nudge(item: Item) -> f32 {
    match item {
        Item::Vip => -0.45,
        Item::Fairness => -0.19,
        _ => 0.0,
    }
}

/// An item's icon, about `c` (its button's middle, on a whole pixel). Each as 0.13.28 draws it,
/// read off its screen pixel by pixel: most sit 2 left of the middle.
fn icon(p: &egui::Painter, item: Item, c: Pos2, ink: Color32) {
    let at = |x: f32, y: f32| pos2(c.x + x, c.y + y);
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(at(x0, y0), at(x1, y1));
    match item {
        Item::Lobby => {
            // A house: the roof, the walls (rounded at the foot), and a door of the bar's dark.
            p.add(Shape::convex_polygon(vec![at(-1.8, -9.9), at(7.3, -0.9), at(-11.0, -0.9)], ink, Stroke::NONE));
            p.rect_filled(rect(-8.0, -1.0, 5.0, 8.0), CornerRadius { nw: 0, ne: 0, sw: 2, se: 2 }, ink);
            p.rect_filled(rect(-4.0, 2.0, 1.0, 8.0), CornerRadius::ZERO, DOOR);
        }
        Item::Games => {
            // Four tiles, the first lit.
            for (i, (x, y)) in [(-11.0, -9.0), (-1.0, -9.0), (-11.0, 1.0), (-1.0, 1.0)].into_iter().enumerate() {
                p.rect_filled(Rect::from_min_size(at(x, y), vec2(8.0, 8.0)), CornerRadius::same(2), if i == 0 { ORANGE } else { ink });
            }
        }
        Item::Vip => {
            // A gem: gold, edged in the icon's color.
            let gem = vec![at(-7.2, -8.5), at(3.6, -8.5), at(7.5, -3.1), at(-1.7, 9.6), at(-11.1, -3.1)];
            p.add(Shape::convex_polygon(gem, col::GOLD2, Stroke::new(1.0f32, ink)));
        }
        Item::Leaderboard => {
            // A podium, its middle the highest, a star over it: three steps 6¾ wide, 1 apart.
            // (The quarters show only where a step's edge falls between pixels: in a named button.)
            for (x0, x1, y0) in [(-13.0, -6.25, 0.0), (-5.25, 1.5, -3.0), (2.5, 9.25, 3.0)] {
                p.rect_filled(rect(x0, y0, x1, 10.0), CornerRadius::same(1), ink);
            }
            icons::star(p, Rect::from_center_size(at(-1.875, -7.42), vec2(7.5, 7.5)), ink, true);
        }
        Item::Profile => {
            // A head, and shoulders: half an oval.
            p.circle_filled(at(-1.8, -4.0), 4.3, ink);
            let shoulders: Vec<Pos2> = (0..=20)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 20.0);
                    at(-1.8 + 8.25 * a.cos(), 10.3 + 7.9 * a.sin())
                })
                .collect();
            p.add(Shape::convex_polygon(shoulders, ink, Stroke::NONE));
        }
        Item::Staff => icons::star(p, Rect::from_center_size(at(-0.5, 0.7), vec2(22.0, 22.0)), ink, true),
        Item::Cms => {
            // A gear: a ring with eight teeth.
            let g = at(-0.5, 0.0);
            p.circle_stroke(g, 6.3, Stroke::new(2.6f32, ink));
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::FRAC_PI_4;
                let d = vec2(a.cos(), a.sin());
                p.line_segment([g + d * 7.0, g + d * 9.3], Stroke::new(3.0f32, ink));
            }
        }
        Item::Keys => {
            // A keyboard: its edge, two rows of four keys (the first gold), the space bar.
            p.rect_stroke(rect(-11.2, -6.2, 7.2, 6.2), CornerRadius::same(3), Stroke::new(1.6f32, ink), egui::StrokeKind::Middle);
            let key = ink.gamma_multiply(0.925);
            for row in 0..2 {
                for i in 0..4 {
                    let r = Rect::from_min_size(at(-9.0 + 4.0 * i as f32, -4.0 + 3.0 * row as f32), vec2(2.0, 2.0));
                    p.rect_filled(r, CornerRadius::ZERO, if row == 0 && i == 0 { col::GOLD2 } else { key });
                }
            }
            p.rect_filled(rect(-7.0, 3.0, 3.0, 5.0), CornerRadius::ZERO, key);
        }
        Item::Fairness => {
            // A padlock: its body, the shackle over it.
            p.rect_filled(rect(-8.75, -1.0, 4.75, 8.0), CornerRadius::same(1), ink);
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                    at(-2.0 + 4.15 * a.cos(), -1.0 + 4.15 * a.sin())
                })
                .collect();
            p.add(Shape::line(pts, Stroke::new(2.5f32, ink)));
        }
    }
}
