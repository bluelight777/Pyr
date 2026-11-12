//! The Games menu (0.13.6): every game on one panel, hanging from the casino bar's Games button
//! as 0.13.28 draws it. The slots in two columns, then poker, the table games and Pyr's own,
//! each with its picture, a word on it and its badge; a game opens from its line, a column's
//! "See all" goes to its part of the lobby, "The whole lobby" to the lobby. Every size and color
//! is 0.13.28's, measured off its screen: the panel, its glows and shadow, the caret pointing at
//! the button, the columns (narrower when the window is), names cut short with "…" where they
//! don't fit.

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::casino::Game;

use super::reel_themes::THEMES;
use super::{col, pics, radial};
use crate::app::PyrApp;
use crate::theme;

/// The lobby's parts, the menu's columns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Chapter {
    Slots,
    Poker,
    Tables,
    Originals,
}

impl Chapter {
    /// Its color (the menu's column, the lobby's heading).
    pub fn color(self) -> Color32 {
        match self {
            Chapter::Slots => Color32::from_rgb(255, 138, 42),
            Chapter::Poker => Color32::from_rgb(46, 184, 106),
            Chapter::Tables => Color32::from_rgb(216, 58, 74),
            Chapter::Originals => Color32::from_rgb(154, 106, 240),
        }
    }

    /// The four, in the lobby's order.
    pub const ALL: [Chapter; 4] = [Chapter::Slots, Chapter::Poker, Chapter::Tables, Chapter::Originals];

    /// Its name on the lobby (its heading, its button on the bar over the chapters).
    pub fn name(self) -> &'static str {
        match self {
            Chapter::Slots => "Slots",
            Chapter::Poker => "Poker",
            Chapter::Tables => "Table games",
            Chapter::Originals => "Pyr Originals",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Chapter::Slots => "SLOTS",
            Chapter::Poker => "POKER",
            Chapter::Tables => "TABLE GAMES",
            Chapter::Originals => "PYR ORIGINALS",
        }
    }
}

/// Where the lobby's to go when it's next drawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Jump {
    Top,
    Chapter(Chapter),
}

/// What a line opens.
#[derive(Clone, Copy)]
enum Opens {
    Game(Game),
    Machine(u8),
    Chapter(Chapter),
}

/// A line: its name, picture, the word under it, its badge.
struct Entry {
    name: &'static str,
    art: String,
    /// Where its picture's cut about (a fraction across).
    focus: f32,
    sub: &'static str,
    pill: Option<(&'static str, Color32)>,
    opens: Opens,
}

const PANEL_W: f32 = 1198.0;
const PAD: f32 = 26.0;
/// A line's height, and from one to the next.
const LINE: f32 = 38.0;
const PITCH: f32 = 52.0;
/// The panel's dark, down it; the caret's fill.
const TOP: Color32 = Color32::from_rgb(34, 18, 16);
const FOOT: Color32 = Color32::from_rgb(12, 7, 9);
/// A name, and a name pointed at.
const NAME: Color32 = Color32::from_gray(225);
const NEW: Color32 = Color32::from_rgb(0x3a, 0xd0, 0x8a);
/// The lobby button's dark, and its words when pointed at.
const PILL: Color32 = Color32::from_rgb(45, 31, 29);
const INK: Color32 = Color32::from_rgb(0x2a, 0x10, 0x04);

fn entry(g: Game, art: &str, pill: Option<(&'static str, Color32)>) -> Entry {
    Entry { name: g.name(), art: art.into(), focus: 0.5, sub: "Play now", pill, opens: Opens::Game(g) }
}

/// The games this server has, by column.
fn columns(s: &ServerView) -> [(Chapter, Vec<Entry>); 4] {
    let reel_row = s.has(pyr_proto::control::CASINO_012_FEATURE);
    let mut slots = vec![entry(Game::Slots, "tile-sevens", Some(("JACKPOT", col::EMBER)))];
    if reel_row {
        slots.push(entry(Game::Dragon, "tile-dragon", None));
        for (m, th) in THEMES.iter().enumerate() {
            let focus = if th.id == "goldrush" { 0.3 } else { 0.5 };
            slots.push(Entry { name: th.name, art: format!("reels-{}-tile", th.id), focus, sub: "Play now", pill: None, opens: Opens::Machine(m as u8) });
        }
    }
    let poker = |name: &'static str, art: &str| Entry { name, art: art.into(), focus: 0.5, sub: "With friends and bots", pill: None, opens: Opens::Chapter(Chapter::Poker) };
    let poker = vec![poker("Cash tables", "table-poker"), poker("Bomb Pot", "tile-bombpot"), poker("Tournaments", "tile-tourney")];
    let tables = vec![entry(Game::Blackjack, "tile-blackjack", None), entry(Game::Roulette, "tile-roulette", None), entry(Game::Baccarat, "tile-baccarat", None), entry(Game::Craps, "tile-craps", None)];
    let mut originals = Vec::new();
    if s.has(pyr_proto::control::PLINKO_FEATURE) {
        originals.push(entry(Game::Plinko, "tile-plinko", Some(("NEW", NEW))));
    }
    originals.extend([entry(Game::Dice, "tile-dice", None), entry(Game::Keno, "tile-keno", None)]);
    [(Chapter::Slots, slots), (Chapter::Poker, poker), (Chapter::Tables, tables), (Chapter::Originals, originals)]
}

impl PyrApp {
    /// Where a chapter of the lobby starts: if the Games menu asked to go there, the page goes
    /// so it's at the top, 18 under the bar (as 0.13.28's does: its heading's band at 76 in a
    /// window 800 tall; egui keeps what it scrolls to 6 under the page's top, so 12 over it).
    pub fn floor_anchor(&mut self, ui: &mut egui::Ui, c: Chapter) {
        if self.poker.casino.jump == Some(Jump::Chapter(c)) {
            self.poker.casino.jump = None;
            let at = ui.cursor().min;
            ui.scroll_to_rect(Rect::from_min_size(pos2(at.x, at.y - 12.0), vec2(1.0, 1.0)), Some(egui::Align::TOP));
        }
    }

    /// The menu, open under the Games button (`button`, on the bar `bar`).
    pub fn games_menu(&mut self, ui: &mut egui::Ui, s: &ServerView, bar: Rect, button: Rect) {
        let ctx = ui.ctx().clone();
        let screen = ctx.content_rect();
        let cols = columns(s);
        let slot_rows = cols[0].1.len().div_ceil(2);
        let rows = cols.iter().skip(1).map(|c| c.1.len()).max().unwrap_or(0).max(slot_rows).max(1);
        // 1198 wide (less in a narrower window, 16 in from its left), 16 in from the bar's right,
        // its top a pixel up the bar.
        let right = bar.right() - 16.0;
        let w = PANEL_W.min(right - screen.left() - 16.0).max(400.0);
        let panel = Rect::from_min_size(pos2(right - w, bar.bottom() - 1.0), vec2(w, 152.0 + PITCH * rows as f32));
        let mut close = false;
        let mut go: Option<Opens> = None;
        let mut lobby = false;
        egui::Area::new(egui::Id::new("casino-games-menu")).order(egui::Order::Foreground).fixed_pos(panel.min).show(&ctx, |ui| {
            let p = ui.painter().clone();
            let (l, t, r, b) = (panel.left(), panel.top(), panel.right(), panel.bottom());
            // Its shadow, down from it.
            p.add(egui::epaint::RectShape::filled(panel.translate(vec2(0.0, 16.0)), CornerRadius::same(16), Color32::from_black_alpha(216)).with_blur_width(54.0));
            super::vgrad_round(&p, panel, 16.0, TOP, FOOT);
            // An ember's glow from its top left (over the bar too), a violet one at its foot.
            radial(&p, pos2(l + 262.0, t + 5.0), 538.0, col::EMBER.gamma_multiply(0.13), Color32::TRANSPARENT);
            radial(&p, pos2(r - 218.0, b - 4.0), 488.0, Chapter::Originals.color().gamma_multiply(0.086), Color32::TRANSPARENT);
            p.rect_stroke(panel, CornerRadius::same(16), Stroke::new(1.5f32, col::GOLD_DARK), StrokeKind::Inside);
            p.line_segment([pos2(l + 40.0, t + 1.0), pos2(r - 40.0, t + 1.0)], Stroke::new(2.0f32, col::GOLD.gamma_multiply(0.42)));
            // The caret, pointing up at the button.
            let cx = button.center().x;
            let caret = vec![pos2(cx - 9.0, t + 0.5), pos2(cx, t - 8.5), pos2(cx + 9.0, t + 0.5)];
            p.add(Shape::convex_polygon(caret.clone(), TOP, Stroke::NONE));
            p.add(Shape::line(caret, Stroke::new(1.5f32, col::GOLD_DARK)));
            // The panel takes the pointer (what's under it doesn't).
            let _ = ui.interact(panel, ui.id().with("panel"), Sense::click());
            // The heading, the count, the way to the lobby.
            let title = p.layout_no_wrap("ALL GAMES".into(), theme::display_font("anton", 24.0), col::GOLD2);
            let tw = title.size().x;
            p.galley(pos2(l + PAD, t + 18.0), title, col::GOLD2);
            let n: usize = cols.iter().map(|c| c.1.len()).sum();
            p.text(pos2(l + PAD + tw + 12.0, t + 33.0), Align2::LEFT_TOP, format!("{n} games · every one checked by your own Pyr"), theme::font(12.5), col::MUTED);
            let words = p.layout_no_wrap("The whole lobby".into(), theme::bold_font(12.5), Color32::PLACEHOLDER);
            let arrow = p.layout_no_wrap("›".into(), theme::bold_font(12.5), Color32::PLACEHOLDER);
            let pw = 10.0 + words.size().x + 7.5 + arrow.size().x + 10.0;
            let pill = Rect::from_min_size(pos2(r - PAD - pw, t + 18.0), vec2(pw, 28.0));
            let resp = ui.interact(pill, ui.id().with("lobby"), Sense::click());
            let hot = resp.hovered();
            p.rect_filled(pill, CornerRadius::same(14), if hot { col::GOLD2 } else { PILL });
            let ink = if hot { INK } else { col::GOLD2 };
            let end = pill.left() + 10.0 + words.size().x;
            p.galley(pos2(pill.left() + 10.0, pill.top() + 7.0), words, ink);
            p.galley(pos2(end + 7.5, pill.top() + 7.0), arrow, ink);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                lobby = true;
            }
            // The columns: the slots two wide, then poker, the table games, Pyr's own.
            let x0 = l + PAD;
            let u = ((w - 2.0 * PAD) - 76.0) / 5.0;
            let mut x = x0;
            for (chapter, entries) in &cols {
                let (chapter, two) = (*chapter, *chapter == Chapter::Slots);
                let cw = if two { 2.0 * u + 10.0 } else { u };
                let colr = Rect::from_min_size(pos2(x, t), vec2(cw, panel.height()));
                if self.games_column(ui, &p, colr, chapter, entries, u, &mut go) {
                    go = Some(Opens::Chapter(chapter));
                }
                x += cw + 22.0;
            }
            let last = t + 112.0 + PITCH * (rows - 1) as f32 + LINE;
            p.text(pos2(x0, last + 21.0), Align2::LEFT_TOP, "Play chips only: they can't be bought or cashed out.", theme::font(12.0), col::MUTED);
        });
        // A click anywhere else closes it (and does what it does there).
        if ctx.input(|i| i.pointer.any_click()) && let Some(at) = ctx.input(|i| i.pointer.interact_pos()) {
            if !panel.contains(at) && !button.contains(at) {
                close = true;
            }
        }
        if lobby {
            self.poker.casino.jump = Some(Jump::Top);
            go = None;
            close = true;
            self.poker.tab = crate::poker::Tab::Floor;
        }
        if let Some(o) = go {
            close = true;
            match o {
                Opens::Game(g) => self.poker.casino.game = Some(g),
                Opens::Machine(m) => {
                    self.reels_open(m);
                    self.row_open(m);
                }
                Opens::Chapter(c) => {
                    self.poker.tab = crate::poker::Tab::Floor;
                    self.poker.casino.jump = Some(Jump::Chapter(c));
                }
            }
        }
        if close {
            self.poker.casino.games_menu = false;
        }
    }

    /// A column: its heading (icon, name, how many, "See all ›" — true if that's clicked), a
    /// line under it in its color, then its games (the slots' in two columns `u` wide).
    #[allow(clippy::too_many_arguments)]
    fn games_column(&mut self, ui: &mut egui::Ui, p: &egui::Painter, colr: Rect, chapter: Chapter, entries: &[Entry], u: f32, go: &mut Option<Opens>) -> bool {
        let color = chapter.color();
        let (l, t) = (colr.left(), colr.top());
        let hy = t + 84.5;
        heading_icon(p, chapter, l, hy);
        let title = p.layout_no_wrap(chapter.title().into(), theme::display_font("bebas", 19.0), Color32::WHITE);
        let tw = title.size().x;
        p.galley(pos2(l + 28.0, hy - title.size().y / 2.0 + 1.0), title, Color32::WHITE);
        p.text(pos2(l + 28.0 + tw + 6.0, hy + 2.3), Align2::LEFT_CENTER, entries.len().to_string(), theme::bold_font(11.0), color);
        let all = p.layout_no_wrap("See all ›".into(), theme::bold_font(11.5), Color32::PLACEHOLDER);
        let all_r = Rect::from_min_size(pos2(colr.right() - all.size().x, hy - all.size().y / 2.0), all.size());
        let resp = ui.interact(all_r, ui.id().with(("see-all", chapter as u8)), Sense::click());
        p.galley(all_r.min, all, if resp.hovered() { Color32::WHITE } else { color });
        let see_all = resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        p.line_segment([pos2(l, t + 102.5), pos2(colr.right(), t + 102.5)], Stroke::new(1.0f32, color.gamma_multiply(0.45)));
        let per = if chapter == Chapter::Slots { entries.len().div_ceil(2) } else { entries.len() };
        for (i, e) in entries.iter().enumerate() {
            let (c, k) = (i / per.max(1), i % per.max(1));
            let item = Rect::from_min_size(pos2(l + c as f32 * (u + 10.0), t + 108.0 + PITCH * k as f32), vec2(u, LINE + 8.0));
            if self.games_line(ui, p, item, chapter, e) {
                *go = Some(e.opens);
            }
        }
        see_all
    }

    /// A game's line in the menu (`r`: 4 above and below its picture). True if it's clicked.
    fn games_line(&mut self, ui: &mut egui::Ui, p: &egui::Painter, r: Rect, chapter: Chapter, e: &Entry) -> bool {
        let color = chapter.color();
        let resp = ui.interact(r, ui.id().with(("games-line", e.name)), Sense::click());
        let hot = resp.hovered();
        if hot {
            p.rect_filled(r, CornerRadius::same(8), Color32::from_rgba_unmultiplied(255, 255, 255, 16));
            p.rect_filled(Rect::from_center_size(pos2(r.left() + 1.5, r.center().y), vec2(3.0, 28.6)), CornerRadius::same(1), color);
        }
        let thumb = Rect::from_min_size(pos2(r.left() + 6.0, r.top() + 4.0), vec2(56.0, LINE));
        // (A little dimmed, full bright when pointed at.)
        if !pics::cover_rounded(p, ui.ctx(), &e.art, thumb, 7.0, (e.focus, 0.5), 112, Color32::from_gray(if hot { 255 } else { 215 })) {
            p.rect_filled(thumb, CornerRadius::same(7), Color32::from_rgb(0x1e, 0x1a, 0x28));
        }
        p.rect_stroke(thumb, CornerRadius::same(7), Stroke::new(1.0f32, if hot { color } else { Color32::from_white_alpha(30) }), StrokeKind::Inside);
        // Its badge at the right, 6 in; its name up to 4 before that (or 10 from the end), cut
        // short with "…" if it has to be.
        let x = r.left() + 72.0;
        let mut edge = r.right() - 6.0;
        if let Some((word, fill)) = e.pill {
            let g = p.layout_no_wrap(word.to_string(), theme::bold_font(9.0), Color32::WHITE);
            let pw = if word == "NEW" { 31.0 } else { g.size().x + 12.0 };
            let pill = Rect::from_min_size(pos2(edge - pw, r.top() + 8.0), vec2(pw, 15.0));
            p.rect_filled(pill, CornerRadius::same(8), fill);
            p.galley(pill.center() - g.size() / 2.0, g, Color32::WHITE);
            edge = pill.left();
        }
        let mut job = egui::text::LayoutJob::simple_singleline(e.name.to_string(), theme::bold_font(13.5), if hot { Color32::WHITE } else { NAME });
        job.wrap = egui::text::TextWrapping { max_width: (edge - 4.0 - x).max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        let name = ui.fonts_mut(|f| f.layout_job(job));
        p.galley(pos2(x, r.top() + 7.0), name, NAME);
        p.text(pos2(x, r.top() + 26.0), Align2::LEFT_TOP, e.sub, theme::font(11.5), col::MUTED);
        resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
    }
}

/// A column's icon, its middle about (`l` + 11, `y`): a 7 in a box for the slots, a spade,
/// two dice, Pyr's campfire.
fn heading_icon(p: &egui::Painter, chapter: Chapter, l: f32, y: f32) {
    match chapter {
        Chapter::Slots => {
            let c = pos2(l + 11.0, y + 0.5);
            p.rect_stroke(Rect::from_center_size(c, vec2(18.0, 18.0)), CornerRadius::same(5), Stroke::new(2.0f32, chapter.color()), StrokeKind::Inside);
            let at = |x: f32, dy: f32| pos2(c.x + x, c.y + dy);
            p.add(Shape::line(vec![at(-4.2, -3.7), at(2.6, -3.7), at(-2.0, 5.0)], Stroke::new(1.6f32, Color32::WHITE)));
        }
        Chapter::Poker => {
            // Its point, the two lobes, a stem widening to its foot.
            let c = pos2(l + 11.0, y + 1.0);
            let at = |x: f32, dy: f32| pos2(c.x + x, c.y + dy);
            p.add(Shape::convex_polygon(vec![at(0.0, -9.3), at(8.1, -1.0), at(3.75, 0.7), at(0.0, 3.4), at(-3.75, 0.7), at(-8.1, -1.0)], Color32::WHITE, Stroke::NONE));
            p.circle_filled(at(-3.75, 0.7), 4.6, Color32::WHITE);
            p.circle_filled(at(3.75, 0.7), 4.6, Color32::WHITE);
            p.add(Shape::convex_polygon(vec![at(-0.9, 4.6), at(0.9, 4.6), at(2.6, 8.0), at(-2.6, 8.0)], Color32::WHITE, Stroke::NONE));
        }
        Chapter::Tables => {
            // A red die, a white one over it showing two.
            let c = pos2(l + 11.5, y + 1.5);
            let red = Rect::from_center_size(c + vec2(-3.5, -3.0), vec2(10.0, 10.0));
            p.rect_filled(red, CornerRadius::same(3), chapter.color());
            p.circle_filled(red.center() + vec2(-0.4, -0.5), 1.5, Color32::WHITE);
            let white = Rect::from_center_size(c + vec2(3.5, 3.0), vec2(10.0, 10.0));
            p.rect_filled(white, CornerRadius::same(3), Color32::WHITE);
            for d in [vec2(-2.3, -2.4), vec2(2.6, 2.6)] {
                p.circle_filled(white.center() + d, 1.3, Color32::from_rgb(20, 12, 12));
            }
        }
        Chapter::Originals => crate::icons::flame(p, Rect::from_center_size(pos2(l + 11.5, y + 0.8), vec2(18.0, 18.0))),
    }
}
