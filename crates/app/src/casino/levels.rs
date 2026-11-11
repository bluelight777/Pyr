//! Levels and the Pyr VIP Club on screen (0.13.5), drawn as 0.13.28 drew them: the tier's crest
//! on the player card with its title and what the next level pays, today's three missions and
//! Ember Back under it, the VIP Club page (every tier, its crest and perks, where you are and
//! what's next), and the crest bursting onto the screen with the chips a level paid.
//!
//! (On a server from before levels, `RecordView::ember_bp` is 0: the old card stays.)

use std::time::Instant;

use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_poker::levels;
use pyr_engine::state::ServerView;
use pyr_poker::net::{MissionView, RecordView, ToServer};

use crate::app::PyrApp;
use crate::casino::pics;
use crate::poker::cards::chips_text;
use crate::theme;

/// Whether a server knows levels (it sends a tier's share of the house's edge).
pub fn has_levels(me: &RecordView) -> bool {
    me.ember_bp > 0
}

/// A tier's crest (0, Spark, to 9, Eternal Flame) in `r`.
pub fn crest(p: &egui::Painter, ctx: &egui::Context, tier: usize, r: Rect) -> bool {
    let i = tier.min(9) as f32;
    pics::part(p, ctx, "tier-crests", Rect::from_min_max(pos2(i / 10.0, 0.0), pos2((i + 1.0) / 10.0, 1.0)), r, 1920, Color32::WHITE)
}

/// A tier's color (its title, its ring).
pub fn tier_color(t: usize) -> Color32 {
    // (0.13.28's, read off its tier frames.)
    match t {
        0 => Color32::from_rgb(0xd0, 0x8a, 0x5a),
        1 => Color32::from_rgb(0xc8, 0x82, 0x4a),
        2 => Color32::from_rgb(0xc8, 0xd0, 0xdc),
        3 => Color32::from_rgb(0xff, 0xc8, 0x40),
        4 => Color32::from_rgb(0xff, 0x9a, 0x3a),
        5 => Color32::from_rgb(0x4a, 0xe0, 0x7a),
        6 => Color32::from_rgb(0xff, 0x4a, 0x3a),
        7 => Color32::from_rgb(0xff, 0xd8, 0x5a),
        8 => Color32::from_rgb(0x6a, 0xa8, 0xff),
        _ => Color32::from_rgb(0xc0, 0x7a, 0xff),
    }
}

const LOCKED: Color32 = Color32::from_rgb(0xa8, 0x99, 0x8c);
const ORANGE: Color32 = Color32::from_rgb(0xf0, 0x8a, 0x3a);

fn corner_color(i: usize) -> Color32 {
    match i {
        0 => Color32::from_rgb(0x3c, 0xc6, 0x6a),
        1 => Color32::from_rgb(0xe0, 0x4a, 0x5a),
        _ => Color32::from_rgb(0xf0, 0x8a, 0x2a),
    }
}

const CORNERS: [&str; 3] = ["TODAY · POKER", "TODAY · TABLES", "TODAY · SLOTS & ORIGINALS"];

/// A percent from hundredths of one ("7.5%", "5%").
pub fn percent(bp: u64) -> String {
    if bp % 100 == 0 { format!("{}%", bp / 100) } else { format!("{}%", (bp as f64 / 100.0)) }
}

/// What the line under my name says: my title, and the way to the next level (or star).
pub fn next_line(me: &RecordView) -> (String, String) {
    let title = levels::title(me.level);
    let to_go = me.level_to.saturating_sub(me.xp);
    if me.level >= levels::MAX_LEVEL {
        return (format!("{title} · ★ {}", me.stars), format!("{} XP to the next star (pays {})", chips_text(to_go), chips_text(me.next_reward)));
    }
    // ("almost there" in the last 15% of a level, as 0.13.28 said it.)
    let near = to_go * 100 <= (me.level_to - me.level_from).max(1) * 15;
    let lead = if near { "almost there: " } else { "" };
    (title, format!("{lead}{} XP to level {} (pays {})", chips_text(to_go), me.level + 1, chips_text(me.next_reward)))
}

/// The crest in its ring (the level's progress around it) with the level on a badge under it.
pub fn crest_badge(p: &egui::Painter, ctx: &egui::Context, me: &RecordView, c: Pos2, radius: f32) {
    let t = me.tier as usize;
    p.circle_filled(c, radius, Color32::from_rgb(0x1a, 0x12, 0x10));
    p.circle_stroke(c, radius, Stroke::new(radius * 0.09, Color32::from_rgb(0x3a, 0x2a, 0x22)));
    let span = (me.level_to.saturating_sub(me.level_from)).max(1) as f32;
    let frac = (me.xp.saturating_sub(me.level_from) as f32 / span).clamp(0.0, 1.0);
    crate::poker::arc(p, c, radius, frac, Stroke::new(radius * 0.09, tier_color(t)));
    let s = radius * 1.3;
    crest(p, ctx, t, Rect::from_center_size(c, vec2(s, s)));
    let b = c + vec2(0.0, radius * 0.86);
    let br = (radius * 0.3).max(9.0);
    p.circle_filled(b, br, Color32::from_rgb(0x14, 0x10, 0x0e));
    p.circle_stroke(b, br, Stroke::new(1.5f32, Color32::from_rgb(0xe8, 0xd8, 0xc0)));
    p.text(b, Align2::CENTER_CENTER, me.level.to_string(), theme::bold_font(br * 1.15), Color32::WHITE);
}

/// A level reached, shown big for a few seconds: the crest bursting onto the screen with the
/// chips it paid (a whole new tier, bigger still).
pub struct Burst {
    pub level: u32,
    pub tier_up: bool,
    pub chips: u64,
    pub since: Instant,
}

const BURST_SECS: f32 = 3.6;

impl PyrApp {
    /// Under the player card on the casino's home: today's three missions and Ember Back.
    pub(crate) fn levels_row(&mut self, ui: &mut egui::Ui, s: &ServerView, me: &RecordView) {
        if !has_levels(me) {
            return;
        }
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(600.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 76.0), Sense::hover());
            let gap = 10.0;
            let cw = (w - 3.0 * gap) / 4.0;
            let p = ui.painter().clone();
            for i in 0..3 {
                let c = Rect::from_min_size(pos2(r.left() + i as f32 * (cw + gap), r.top()), vec2(cw, r.height()));
                mission_card(&p, c, i, me.missions.get(i));
            }
            // Ember Back.
            let c = Rect::from_min_size(pos2(r.left() + 3.0 * (cw + gap), r.top()), vec2(cw, r.height()));
            p.rect_filled(c, theme::r(10), Color32::from_rgba_unmultiplied(0x30, 0x1a, 0x12, 225));
            let tc = tier_color(me.tier as usize);
            p.rect_stroke(c, theme::r(10), Stroke::new(1.5f32, tc.gamma_multiply(0.85)), StrokeKind::Inside);
            let x = c.left() + 14.0;
            p.text(pos2(x, c.top() + 14.0), Align2::LEFT_CENTER, format!("EMBER BACK · {} AT {}", percent(me.ember_bp), levels::TIERS[(me.tier as usize).min(9)].0.to_uppercase()), theme::bold_font(10.0), tc);
            p.text(pos2(x, c.top() + 40.0), Align2::LEFT_CENTER, format!("{} chips", chips_text(me.ember_back)), theme::bold_font(20.0), Color32::WHITE);
            let under = p.layout_no_wrap("a share of the house's edge on your play".into(), theme::font(10.5), crate::casino::col::MUTED);
            // (Kept inside the card: 0.13.28's ran past its edge.)
            let k = ((c.width() - 24.0) / under.size().x).min(1.0);
            let under = if k < 1.0 { p.layout_no_wrap("a share of the house's edge on your play".into(), theme::font(10.5 * k), crate::casino::col::MUTED) } else { under };
            p.galley(pos2(x, c.bottom() - 13.0 - under.size().y / 2.0), under, crate::casino::col::MUTED);
            let b = Rect::from_min_size(pos2(c.right() - 88.0, c.top() + 24.0), vec2(74.0, 32.0));
            let ready = me.ember_back > 0;
            let resp = ui.put(
                b,
                egui::Button::new(RichText::new("Claim").font(theme::bold_font(13.5)).color(if ready { Color32::WHITE } else { crate::casino::col::MUTED }))
                    .fill(if ready { crate::casino::col::EMBER } else { Color32::from_rgb(0x3a, 0x30, 0x2a) })
                    .corner_radius(theme::r(8)),
            );
            if resp.on_hover_text("Ember Back pays you back a share of the house's edge on everything you play (and of the rake you pay at poker). Claim it any time.").clicked() && ready {
                self.engine.poker(&s.key, ToServer::ClaimEmber);
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        });
    }

    /// The Pyr VIP Club: where I am, its perks, the weekly bonus, and every tier.
    pub(crate) fn vip_page(&mut self, ui: &mut egui::Ui, s: &ServerView, me: &RecordView) {
        let ctx = ui.ctx().clone();
        // (Kept fresh: the lobby, and my record with it, is asked for every few seconds.)
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        let t = (me.tier as usize).min(9);
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(600.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 168.0), Sense::hover());
            let p = ui.painter().clone();
            crate::casino::vgrad_round(&p, r, 16.0, Color32::from_rgb(0x34, 0x1a, 0x18), Color32::from_rgb(0x1c, 0x10, 0x10));
            p.rect_stroke(r, theme::r(16), Stroke::new(1.5f32, ORANGE.gamma_multiply(0.7)), StrokeKind::Inside);
            crest_badge(&p, &ctx, me, pos2(r.left() + 110.0, r.center().y), 62.0);
            let x = r.left() + 210.0;
            p.text(pos2(x, r.top() + 30.0), Align2::LEFT_CENTER, "PYR VIP CLUB", theme::bold_font(12.0), ORANGE);
            crate::casino::glow_text(&p, pos2(x, r.top() + 63.0), Align2::LEFT_CENTER, &levels::title(me.level), theme::bold_font(30.0), Color32::from_rgb(0xff, 0xe6, 0xd0), ORANGE.gamma_multiply(0.35));
            let next = if t + 1 < levels::TIERS.len() {
                let (first, _) = levels::tier_levels(t + 1);
                let n = first - me.level;
                format!("Level {} · {} more level{} to {}", me.level, n, if n == 1 { "" } else { "s" }, levels::TIERS[t + 1].0)
            } else {
                format!("Level {} · ★ {}", me.level, me.stars)
            };
            p.text(pos2(x, r.top() + 98.0), Align2::LEFT_CENTER, next, theme::font(14.5), Color32::from_rgb(0xd8, 0xcc, 0xc0));
            p.text(
                pos2(x, r.top() + 123.0),
                Align2::LEFT_CENTER,
                format!("Ember Back {} of the house's edge · daily chips ×{:.2}", percent(me.ember_bp), levels::daily_x100(t) as f64 / 100.0),
                theme::font(12.5),
                crate::casino::col::MUTED,
            );
            // The weekly VIP bonus.
            let wb = Rect::from_min_size(pos2(r.right() - 250.0, r.top() + 39.0), vec2(226.0, 90.0));
            p.rect_filled(wb, theme::r(10), Color32::from_rgb(0x1a, 0x10, 0x0e));
            p.rect_stroke(wb, theme::r(10), Stroke::new(1.0f32, Color32::from_rgb(0x6a, 0x4a, 0x38)), StrokeKind::Inside);
            p.text(pos2(wb.left() + 14.0, wb.top() + 16.0), Align2::LEFT_CENTER, "WEEKLY VIP BONUS", theme::bold_font(10.0), ORANGE);
            p.text(pos2(wb.left() + 14.0, wb.top() + 42.0), Align2::LEFT_CENTER, format!("{} chips", chips_text(me.weekly_ready)), theme::bold_font(20.0), Color32::WHITE);
            let under = if t == 0 { "from Ember tier up".to_string() } else { format!("this week's so far: {}", chips_text(me.weekly_building)) };
            p.text(pos2(wb.left() + 14.0, wb.bottom() - 16.0), Align2::LEFT_CENTER, under, theme::font(11.0), Color32::from_rgb(0xd0, 0xc4, 0xb8));
            let b = Rect::from_min_size(pos2(wb.right() - 80.0, wb.top() + 26.0), vec2(66.0, 30.0));
            let ready = me.weekly_ready > 0;
            let resp = ui.put(
                b,
                egui::Button::new(RichText::new("Claim").font(theme::bold_font(13.5)).color(if ready { Color32::WHITE } else { crate::casino::col::MUTED }))
                    .fill(if ready { crate::casino::col::EMBER } else { Color32::from_rgb(0x3a, 0x30, 0x2a) })
                    .corner_radius(theme::r(8)),
            );
            if resp.on_hover_text("From Ember tier up: 20% on top of everything your Ember Back earned last week, ready when the week turns (Mondays).").clicked() && ready {
                self.engine.poker(&s.key, ToServer::ClaimWeekly);
            }
        });
        ui.add_space(24.0);
        // Every tier.
        let cols = 4;
        for row in 0..levels::TIERS.len().div_ceil(cols) {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                let w = (ui.available_width() - 28.0).max(600.0);
                let gap = 12.0;
                let cw = (w - gap * (cols as f32 - 1.0)) / cols as f32;
                // (Tall enough for every perk: 0.13.28's ran out past the card's bottom.)
                let (r, _) = ui.allocate_exact_size(vec2(w, 284.0), Sense::hover());
                let p = ui.painter().clone();
                for c in 0..cols {
                    let i = row * cols + c;
                    if i >= levels::TIERS.len() {
                        break;
                    }
                    tier_card(&p, &ctx, Rect::from_min_size(pos2(r.left() + c as f32 * (cw + gap), r.top()), vec2(cw, r.height())), i, t);
                }
            });
            ui.add_space(12.0);
        }
    }

    /// The level-up burst, over the casino (as 0.13.28's: the crest with "LEVEL 13" and the
    /// level's title, the chips it paid counting up; a new tier bigger, "EMBER!").
    pub(crate) fn level_burst(&mut self, ctx: &egui::Context, area: Rect) {
        let Some(b) = &self.poker.burst else { return };
        let t = b.since.elapsed().as_secs_f32();
        let life = if b.tier_up { BURST_SECS + 1.0 } else { BURST_SECS };
        if t > life {
            self.poker.burst = None;
            return;
        }
        let tier = levels::tier(b.level);
        let col = tier_color(tier);
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::new("level-burst")));
        let fade = if t < 0.2 { t / 0.2 } else if t > life - 0.6 { (life - t) / 0.6 } else { 1.0 };
        let c = area.center() - vec2(0.0, if b.tier_up { 60.0 } else { 40.0 });
        // The room darkens around it (more for a tier), and rays turn slowly behind.
        let dark = if b.tier_up { 170.0 } else { 110.0 };
        p.rect_filled(area, 0.0, Color32::from_black_alpha((dark * fade) as u8));
        let rays = 24;
        let reach = area.width().max(area.height());
        for k in 0..rays {
            let a0 = (k as f32 / rays as f32) * std::f32::consts::TAU + t * 0.15;
            let a1 = a0 + std::f32::consts::TAU / rays as f32 * 0.5;
            let pts = vec![c, c + vec2(a0.cos(), a0.sin()) * reach, c + vec2(a1.cos(), a1.sin()) * reach];
            p.add(egui::Shape::convex_polygon(pts, Color32::from_white_alpha((if b.tier_up { 9.0 } else { 6.0 } * fade) as u8), Stroke::NONE));
        }
        // A ring goes out from it as it lands.
        let ring_t = (t / 0.9).min(1.0);
        if ring_t < 1.0 {
            p.circle_stroke(c, 60.0 + ring_t * 260.0, Stroke::new(2.0f32, col.gamma_multiply((1.0 - ring_t) * 0.8 * fade)));
        }
        // The crest pops in (over-shooting a little).
        let k = (t / 0.45).min(1.0);
        let pop = 1.0 + 0.16 * (k * std::f32::consts::PI).sin() * (1.0 - k * 0.5);
        let grow = (t / 0.25).min(1.0);
        let size = if b.tier_up { 250.0 } else { 112.0 } * grow * pop;
        p.circle_filled(c, size * 0.45, col.gamma_multiply(0.16 * fade));
        // (Embers rising round a new tier's crest.)
        if b.tier_up {
            for i in 0..14 {
                let seed = i as f32 * 1.618;
                let life_i = (t * 0.6 + seed).fract();
                let x = c.x + ((seed * 7.3).sin()) * size * 0.55;
                let y = c.y + size * 0.4 - life_i * size * 1.1;
                p.circle_filled(pos2(x, y), 2.5, Color32::from_rgb(0xff, 0x9a, 0x3a).gamma_multiply((1.0 - life_i) * fade));
            }
        }
        crest(&p, ctx, tier, Rect::from_center_size(c, vec2(size, size)));
        let white = Color32::WHITE.gamma_multiply(fade);
        let outline = Color32::from_black_alpha((200.0 * fade) as u8);
        let (big, big_size) = if b.tier_up { (format!("{}!", levels::TIERS[tier].0.to_uppercase()), 64.0) } else { (format!("LEVEL {}", b.level), 44.0) };
        let y = c.y + size * 0.5 + if b.tier_up { 40.0 } else { 26.0 };
        outlined(&p, pos2(c.x, y), &big, theme::bold_font(big_size), white, outline);
        let sub = if b.tier_up {
            let felt = matches!(tier, 3 | 5 | 8);
            format!("Level {} · a new tier: its frame{} yours", b.level, if felt { " and felt are" } else { " is" })
        } else {
            levels::title(b.level)
        };
        let sub_y = y + big_size * 0.5 + 16.0;
        p.text(pos2(c.x, sub_y), Align2::CENTER_CENTER, sub, theme::bold_font(if b.tier_up { 15.0 } else { 13.0 }), col.gamma_multiply(fade));
        // The chips it paid, counting up.
        if b.chips > 0 && t > 0.6 {
            let k = ((t - 0.6) / 1.0).min(1.0);
            let shown = (b.chips as f64 * (1.0 - (1.0 - k as f64).powi(3))).round() as u64;
            let size = if b.tier_up { 40.0 } else { 28.0 };
            outlined(&p, pos2(c.x, sub_y + 22.0 + size * 0.5), &format!("+{} chips", chips_text(shown)), theme::bold_font(size), Color32::from_rgb(0xff, 0xe8, 0xb8).gamma_multiply(fade * (0.6 + 0.4 * k)), outline);
        }
        ctx.request_repaint();
    }
}

/// Text with a dark edge round it (big words over a busy game).
fn outlined(p: &egui::Painter, at: Pos2, text: &str, font: egui::FontId, color: Color32, edge: Color32) {
    for (dx, dy) in [(-2.0, 0.0), (2.0, 0.0), (0.0, -2.0), (0.0, 2.0), (-1.5, -1.5), (1.5, 1.5), (-1.5, 1.5), (1.5, -1.5)] {
        p.text(at + vec2(dx, dy), Align2::CENTER_CENTER, text, font.clone(), edge);
    }
    p.text(at, Align2::CENTER_CENTER, text, font, color);
}

fn mission_card(p: &egui::Painter, c: Rect, i: usize, m: Option<&MissionView>) {
    let col = corner_color(i);
    let done = m.is_some_and(|m| m.done);
    if done {
        // Done: the card in its color, edged in it, the bar full.
        p.rect_filled(c, theme::r(10), col.gamma_multiply(0.22));
        p.rect_stroke(c, theme::r(10), Stroke::new(1.5f32, col.gamma_multiply(0.9)), StrokeKind::Inside);
    } else {
        // To do: dark, with its color down the left edge.
        p.rect_filled(c, theme::r(10), Color32::from_rgba_unmultiplied(0x2c, 0x26, 0x22, 230));
        p.rect_stroke(c, theme::r(10), Stroke::new(1.0f32, Color32::from_rgb(0x3a, 0x32, 0x2c)), StrokeKind::Inside);
        let edge = Rect::from_min_max(pos2(c.left() + 1.0, c.top() + 8.0), pos2(c.left() + 4.0, c.bottom() - 8.0));
        p.rect_filled(edge, theme::r(2), col);
    }
    let x = c.left() + 14.0;
    p.text(pos2(x, c.top() + 14.0), Align2::LEFT_CENTER, CORNERS[i], theme::bold_font(10.0), col);
    let Some(m) = m else { return };
    let text = p.layout_no_wrap(m.text.clone(), theme::bold_font(13.5), Color32::WHITE);
    p.galley(pos2(x, c.top() + 33.0 - text.size().y / 2.0), text, Color32::WHITE);
    let bar = Rect::from_min_size(pos2(x, c.top() + 50.0), vec2(c.width() - 28.0, 5.0));
    p.rect_filled(bar, theme::r(3), Color32::from_rgb(0x1c, 0x18, 0x16));
    let frac = if m.target > 0 { (m.progress as f32 / m.target as f32).clamp(0.0, 1.0) } else { 0.0 };
    if frac > 0.0 {
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * frac, bar.height())), theme::r(3), col);
    }
    let left = if m.done { "Done".to_string() } else { format!("{} / {}", m.progress, m.target) };
    p.text(pos2(x, c.bottom() - 11.0), Align2::LEFT_CENTER, left, theme::font(11.5), if m.done { Color32::WHITE } else { crate::casino::col::MUTED });
    p.text(pos2(c.right() - 14.0, c.bottom() - 11.0), Align2::RIGHT_CENTER, format!("+{} XP · +{}", m.xp, chips_text(m.chips)), theme::bold_font(11.5), Color32::WHITE);
}

/// One tier on the VIP Club page: its crest, its levels, its perks.
fn tier_card(p: &egui::Painter, ctx: &egui::Context, r: Rect, i: usize, mine: usize) {
    let (first, last) = levels::tier_levels(i);
    let here = i == mine;
    let next = i == mine + 1;
    let reached = i <= mine;
    let open = reached || next;
    p.rect_filled(r, theme::r(12), if here { Color32::from_rgb(0x3a, 0x26, 0x1c) } else { Color32::from_rgb(0x2a, 0x22, 0x1e) });
    let stroke = if here {
        Stroke::new(2.0f32, ORANGE)
    } else if next {
        Stroke::new(1.5f32, Color32::from_rgb(0xe8, 0xe0, 0xd8))
    } else {
        Stroke::new(1.0f32, Color32::from_rgb(0x3a, 0x30, 0x2a))
    };
    p.rect_stroke(r, theme::r(12), stroke, StrokeKind::Inside);
    let tint = if open { Color32::WHITE } else { Color32::from_white_alpha(150) };
    if let Some(t) = pics::tex(ctx, "tier-crests", 1920) {
        let fi = i as f32;
        p.image(t.id(), Rect::from_center_size(pos2(r.center().x, r.top() + 56.0), vec2(84.0, 84.0)), Rect::from_min_max(pos2(fi / 10.0, 0.0), pos2((fi + 1.0) / 10.0, 1.0)), tint);
    }
    let name_col = if open { tier_color(i) } else { LOCKED };
    p.text(pos2(r.center().x, r.top() + 113.0), Align2::CENTER_CENTER, levels::TIERS[i].0, theme::bold_font(17.0), name_col);
    let (label, label_col) = if here {
        ("YOU ARE HERE".to_string(), Color32::WHITE)
    } else if next {
        ("NEXT".to_string(), crate::casino::col::MUTED)
    } else if first == last {
        (format!("LEVEL {first}"), crate::casino::col::MUTED)
    } else {
        (format!("LEVELS {first}–{last}"), crate::casino::col::MUTED)
    };
    p.text(pos2(r.center().x, r.top() + 132.0), Align2::CENTER_CENTER, label, theme::bold_font(10.0), label_col);
    let mut perks = vec![format!("Ember Back {} of the house's edge", percent(levels::ember_bp(i))), format!("Daily chips ×{:.2}", levels::daily_x100(i) as f64 / 100.0)];
    if i >= 1 {
        perks.push("Weekly bonus: +20% of your Ember Back".into());
        let felt = matches!(i, 3 | 5 | 8);
        perks.push(format!("The {} frame{}", levels::TIERS[i].0, if felt { " and felt" } else { "" }));
        perks.push("A 100,000-chip chest on arrival".into());
    }
    let text_col = if reached { Color32::from_rgb(0xec, 0xe4, 0xd8) } else { Color32::from_rgb(0xb8, 0xac, 0xa0) };
    let mut y = r.top() + 152.0;
    for perk in perks {
        let g = p.layout(format!("• {perk}"), theme::font(12.0), text_col, r.width() - 28.0);
        let h = g.size().y;
        p.galley(pos2(r.left() + 14.0, y), g, text_col);
        y += h + 3.0;
    }
}
