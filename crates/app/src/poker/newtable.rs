//! A new table (0.13.4): its name, the game, the blinds (and the high rollers', 0.13.22), how
//! many seats, and a picture of it where each seat's bot is picked by who and how good; and the
//! Which bot? window, the same at a table's empty seats. As 0.13.28 has them, measured off it
//! (`measured/newtable.md`).

use std::f32::consts::{FRAC_PI_2, TAU};

use eframe::egui::{self, Align, Align2, Color32, Layout, Pos2, Rect, RichText, Sense, Stroke, UiBuilder, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::bots::{Level, persona};

use crate::theme;

/// A seat's bot: how good, and who (none: whichever bot isn't sitting yet).
#[derive(Clone, Debug, PartialEq)]
pub struct Pick {
    pub level: Level,
    pub who: Option<String>,
}

impl Pick {
    pub fn any(level: Level) -> Pick {
        Pick { level, who: None }
    }

    /// "Any · Medium", "Ace · Hard": under its seat in the picture.
    fn label(&self) -> String {
        format!("{} · {}", self.who.as_deref().unwrap_or("Any"), self.level.name())
    }
}

/// The Which bot? window, open for a seat: the level it shows (the seat's, or Medium).
#[derive(Clone)]
pub struct Picker {
    pub seat: usize,
    pub level: Level,
}

/// The blinds to pick from: 0.13.0's, then the high rollers' (0.13.22).
pub const STAKES: [(u64, u64); 12] = [
    (5, 10),
    (10, 20),
    (25, 50),
    (50, 100),
    (100, 200),
    (250, 500),
    (500, 1_000),
    (1_000, 2_000),
    (2_500, 5_000),
    (5_000, 10_000),
    (10_000, 20_000),
    (25_000, 50_000),
];

/// Where the high rollers' blinds start in [`STAKES`].
pub const HIGH: usize = 6;

/// "500", "1K", "2.5K": blinds as the New table window writes them.
fn k(n: u64) -> String {
    if n >= 1_000 {
        let k = n as f64 / 1_000.0;
        if k.fract() == 0.0 { format!("{k:.0}K") } else { format!("{k:.1}K") }
    } else {
        n.to_string()
    }
}

/// "10/20", "500/1K", "25K/50K".
pub fn blinds_text(sb: u64, bb: u64) -> String {
    format!("{}/{}", k(sb), k(bb))
}

/// The bots of a new table, seat by seat (seat 0 is whoever makes it): three Medium bots across
/// the far side, as 0.13.28 opens it.
pub fn first_bots(seats: usize) -> Vec<Option<Pick>> {
    let mut v = vec![None; seats];
    if seats == 6 {
        for s in [2, 3, 4] {
            v[s] = Some(Pick::any(Level::Regular));
        }
    }
    v
}

/// What a click on the picture asks for.
pub enum Click {
    /// The Which bot? window for this seat.
    Seat(usize),
    /// This seat's bot taken away.
    Remove(usize),
}

// The picture's measures (0.13.28's, at its size): the cloth's half-width and half-height, and
// the seats' oval round it.
const FELT_A: f32 = 190.0;
const FELT_B: f32 = 75.0;
const SEATS_A: f32 = 212.0;
const SEATS_B: f32 = 101.0;
const SEAT_R: f32 = 22.0;
const RIM: Color32 = Color32::from_rgb(0x4a, 0x2c, 0x1a);
const FELT: Color32 = Color32::from_rgb(0x8e, 0x16, 0x2a);
const BADGE: Color32 = Color32::from_gray(50);

/// Where seat `i` of `n` sits round the picture's middle `c` (seat 0 at the bottom, then on round
/// clockwise, as at the table).
fn seat_at(c: Pos2, i: usize, n: usize) -> Pos2 {
    let t = FRAC_PI_2 + i as f32 / n as f32 * TAU;
    c + vec2(SEATS_A * t.cos(), SEATS_B * t.sin())
}

/// The picture of the new table in `area` (the Bots line at its top): the bots in their seats
/// (× takes one away), the empty seats (+ puts one there) and me at the bottom.
pub fn picture(ui: &mut egui::Ui, area: Rect, bots: &[Option<Pick>], title: &str) -> Option<Click> {
    let p = ui.painter().clone();
    let c = pos2(area.center().x, area.top() + 163.0);
    // The table: its shadow, the rail, the cloth, the line round the middle, and its name.
    p.add(egui::epaint::EllipseShape::filled(c + vec2(0.0, 5.0), vec2(FELT_A + 14.0, FELT_B + 14.0), Color32::from_black_alpha(110)));
    p.add(egui::epaint::EllipseShape::filled(c, vec2(FELT_A + 12.0, FELT_B + 12.0), RIM));
    p.add(egui::epaint::EllipseShape::filled(c, vec2(FELT_A, FELT_B), FELT));
    p.add(egui::epaint::EllipseShape::stroke(c, vec2(FELT_A * 0.78, FELT_B * 0.7), Stroke::new(1.0f32, Color32::from_white_alpha(32))));
    p.text(c, Align2::CENTER_CENTER, title, theme::bold_font(12.0), Color32::from_white_alpha(120));
    let n = bots.len();
    let mut click = None;
    for i in 0..n {
        let at = seat_at(c, i, n);
        if i == 0 {
            // Me.
            p.circle_filled(at, SEAT_R, Color32::from_rgba_unmultiplied(0xff, 0x7a, 0x3d, 215));
            p.text(at, Align2::CENTER_CENTER, "You", theme::bold_font(12.0), Color32::WHITE);
            continue;
        }
        let r = Rect::from_center_size(at, vec2(SEAT_R * 2.0, SEAT_R * 2.0));
        let resp = ui.interact(r, ui.id().with(("new-table-seat", i)), Sense::click());
        match &bots[i] {
            None => {
                p.circle_filled(at, SEAT_R, Color32::from_black_alpha(90));
                super::table::dashed_circle(&p, at, SEAT_R, Stroke::new(1.5f32, theme::DIM()));
                crate::icons::plus(&p, Rect::from_center_size(at, vec2(14.0, 14.0)), theme::DIM().gamma_multiply(0.8), 2.0);
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    click = Some(Click::Seat(i));
                }
            }
            Some(b) => {
                super::table::bot_face(&p, at, SEAT_R, b.who.as_deref().unwrap_or("?"), b.level);
                // Its name and level under it.
                let g = p.layout_no_wrap(b.label(), theme::bold_font(10.5), Color32::WHITE);
                let lc = at + vec2(0.0, 31.0);
                p.rect_filled(Rect::from_center_size(lc, g.size() + vec2(10.0, 4.0)), theme::r(6), Color32::from_black_alpha(190));
                p.galley(lc - g.size() / 2.0, g, Color32::WHITE);
                // × at its shoulder.
                let x = at + vec2(18.0, -18.0);
                let xr = Rect::from_center_size(x, vec2(16.0, 16.0));
                let xresp = ui.interact(xr, ui.id().with(("new-table-remove", i)), Sense::click());
                p.circle_filled(x, 8.0, BADGE);
                crate::icons::close(&p, Rect::from_center_size(x, vec2(8.0, 8.0)), Color32::from_gray(210));
                if xresp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    click = Some(Click::Remove(i));
                } else if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    click = Some(Click::Seat(i));
                }
            }
        }
    }
    click
}

/// The Bots line over the picture: how many, what to do, and Fill the seats / Clear (true, true
/// when clicked).
pub fn bots_line(ui: &mut egui::Ui, area: Rect, count: usize) -> (bool, bool) {
    // (Two labels on one line, their feet level.)
    let p = ui.painter();
    let g = p.layout_no_wrap(format!("Bots: {count}"), theme::font(14.5), theme::MUTED());
    let hint = area.left_top() + vec2(g.size().x + 8.0, 14.0);
    p.galley(area.left_top() + vec2(0.0, 11.0), g, theme::MUTED());
    p.text(hint, Align2::LEFT_TOP, "click a seat to put a bot there, and pick which", theme::font(12.0), theme::DIM());
    let row = Rect::from_min_max(area.left_top() + vec2(0.0, 13.0), pos2(area.right(), area.top() + 43.0));
    // (A child of its own: the picture's room is taken already.)
    let mut ui = ui.new_child(UiBuilder::new().max_rect(row).layout(Layout::right_to_left(Align::Min)));
    let clear = ui.small_button("Clear").clicked();
    let fill = ui.small_button("Fill the seats").clicked();
    (fill, clear)
}

/// The levels the Which bot? window offers: Hard where the server has it (0.13.7), Shark (once
/// Genius) where it knows it.
fn levels(s: &ServerView) -> Vec<Level> {
    Level::ALL
        .into_iter()
        .filter(|l| match l {
            Level::Hard => s.has(pyr_proto::control::HARD_BOTS_FEATURE),
            Level::Genius => s.has(pyr_proto::control::CASINO_012_FEATURE),
            _ => true,
        })
        .collect()
}

/// The Which bot? window for `picker`'s seat: how good, and who (the bots that aren't `taken`;
/// a server that doesn't pick bots by name gets an "Add a … bot" button instead). The pick, or
/// `Some(None)` once it's closed without one.
pub fn which_bot(ctx: &egui::Context, picker: &mut Picker, taken: &[String], s: &ServerView) -> Option<Option<Pick>> {
    let mut open = true;
    let mut picked = None;
    let by_name = s.has(pyr_proto::control::POKER_PICK_BOT_FEATURE);
    // (In front of whatever opened it: that one keeps its title lit, as in 0.13.28.)
    egui::Window::new("Which bot?").order(egui::Order::Foreground).collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).open(&mut open).show(ctx, |ui| {
        if by_name {
            ui.set_width(600.0);
        }
        ui.label(RichText::new("How good").color(theme::MUTED()));
        ui.horizontal(|ui| {
            for l in levels(s) {
                ui.selectable_value(&mut picker.level, l, l.name());
            }
        });
        ui.add_space(8.0);
        if !by_name {
            if ui.add(theme::primary_button(&format!("Add a {} bot", picker.level.name()))).clicked() {
                picked = Some(Pick::any(picker.level));
            }
            return;
        }
        ui.label(RichText::new("Who: each plays their own way").color(theme::MUTED()));
        ui.add_space(4.0);
        egui::ScrollArea::vertical().max_height(324.0).auto_shrink([false, true]).show(ui, |ui| {
            // (10 between the rows, and under the last.)
            ui.spacing_mut().item_spacing.y = 0.0;
            let w = (ui.available_width() - 10.0) / 2.0;
            let mut who: Vec<Option<&persona::Persona>> = vec![None];
            who.extend(persona::ALL.iter().filter(|p| p.name != "Wren").map(Some));
            for row in who.chunks(2) {
                ui.horizontal(|ui| {
                    for p in row {
                        if card(ui, w, *p, taken) {
                            picked = Some(Pick { level: picker.level, who: p.map(|p| p.name.to_string()) });
                        }
                    }
                });
                ui.add_space(10.0);
            }
        });
    });
    if picked.is_some() {
        Some(picked)
    } else if !open {
        Some(None)
    } else {
        None
    }
}

/// One bot's card in the Which bot? window (none: Anyone); true when it's picked.
fn card(ui: &mut egui::Ui, w: f32, who: Option<&persona::Persona>, taken: &[String]) -> bool {
    let sitting = who.is_some_and(|p| taken.iter().any(|t| t.split_whitespace().next() == Some(p.name)));
    let (r, resp) = ui.allocate_exact_size(vec2(w, 56.0), if sitting { Sense::hover() } else { Sense::click() });
    let p = ui.painter();
    let fill = if resp.hovered() && !sitting { theme::ELEV2() } else { theme::ELEV() };
    p.rect_filled(r, theme::r(8), fill);
    let at = r.left_top() + vec2(30.0, 28.0);
    let fade = if sitting { 0.35 } else { 1.0 };
    let (name, line) = match who {
        Some(b) => {
            super::table::bot_face(p, at, 20.0, b.name, Level::Regular);
            if sitting {
                p.circle_filled(at, 20.0, Color32::from_black_alpha(150));
            }
            let line = if sitting { format!("{}'s at this table already", b.name) } else { b.style.to_string() };
            (b.name, line)
        }
        None => {
            p.circle_filled(at, 20.0, theme::ELEV2());
            p.text(at, Align2::CENTER_CENTER, "?", theme::bold_font(20.0), theme::TEXT());
            ("Anyone", "Whichever bot isn't sitting yet".to_string())
        }
    };
    p.text(r.left_top() + vec2(60.0, 8.0), Align2::LEFT_TOP, name, theme::bold_font(14.5), theme::TEXT().gamma_multiply(fade));
    let g = p.layout(line, theme::font(11.5), theme::MUTED().gamma_multiply(fade), w - 70.0);
    p.galley(r.left_top() + vec2(60.0, 27.0), g, theme::MUTED());
    resp.on_hover_cursor(if sitting { egui::CursorIcon::Default } else { egui::CursorIcon::PointingHand }).clicked()
}
