//! The funny machines' gimmicks, each its own (see `reel_themes::Humor`): Scoville Slots' heat
//! meter, Sugar Rush's sugar count, The Daily Grind's caffeine, and Frostbite Fortune's biggest
//! catches. The fun is in the pictures (the hosts, the steam, the fire): no joke lines (Blue:
//! "just make the images humorous"). All of it is for show: rounds are played by the house
//! the same as on any machine.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_poker::casino::reels::Outcome;

use super::reel_themes::{Humor, Theme};
use super::{col, pics, radial, vgrad_round};
use crate::poker::cards::chips_text;
use crate::theme;

const HEAT_NAMES: [&str; 6] = ["MILD", "MEDIUM", "HOT", "EXTRA HOT", "INFERNO", "SCOVILLED!"];

/// Grams of sugar each of Sugar Rush's symbols lands (with a straight face).
const GRAMS: [u64; 7] = [4, 11, 32, 47, 52, 118, 61];
/// Everything the four gimmicks keep between rounds (this session, while Pyr is open).
#[derive(Default)]
pub struct Fun {
    /// Scoville Slots: the heat (0 to 5) and as it's drawn (eased), when it boiled over, and
    /// what it last said.
    heat: u8,
    heat_shown: f32,
    boiled: Option<Instant>,
    /// Sugar Rush: grams of sugar landed.
    grams: u64,
    grams_shown: f64,
    /// The Daily Grind: caffeine (0 to 1) and as drawn.
    caffeine: Option<f32>,
    caffeine_shown: f32,
    espresso: Option<Instant>,
    /// Frostbite Fortune: the three biggest wins so far (times the bet ×100, chips won).
    tales: Vec<(u64, u64)>,
    slipped: Option<Instant>,
}

impl Fun {
    /// The Daily Grind's reels are slower when there's no caffeine left (it's just for show:
    /// where they stop is the house's).
    pub fn reel_speed(&self, humor: Option<Humor>) -> f32 {
        if humor == Some(Humor::Office) && self.caffeine.unwrap_or(1.0) < 0.15 { 0.62 } else { 1.0 }
    }

    /// A round's over on screen: each gimmick takes it in.
    pub fn round(&mut self, humor: Option<Humor>, o: &Outcome, _nonce: u64) {
        let Some(h) = humor else { return };
        let mult = pyr_poker::casino::multiple_x100(o.paid, o.wagered());
        match h {
            Humor::Heat => {
                if o.paid == 0 {
                    self.heat += 1;
                    if self.heat >= 5 {
                        self.boiled = Some(Instant::now());
                        self.heat = 2;
                    }
                } else if mult >= 1_000 {
                    self.heat = 0;
                } else {
                    self.heat = self.heat.saturating_sub(2);
                }
            }
            Humor::Dentist => {
                let landed: u64 = o.grid.map(|g| g.iter().flatten().filter(|&&s| (s as usize) < GRAMS.len()).map(|&s| GRAMS[s as usize]).sum()).unwrap_or(0);
                self.grams += if o.paid > 0 { landed * 2 } else { landed };
            }
            Humor::Office => {
                let c = self.caffeine.unwrap_or(1.0);
                let c = if mult >= 1_000 {
                    self.espresso = Some(Instant::now());
                    1.0
                } else if o.paid > 0 {
                    (c + 0.15).min(1.0)
                } else {
                    (c - 0.07).max(0.0)
                };
                self.caffeine = Some(c);
            }
            Humor::FishTales => {
                if o.paid == 0 {
                    self.slipped = Some(Instant::now());
                    return;
                }
                self.tales.push((mult, o.paid));
                self.tales.sort_by(|a, b| b.0.cmp(&a.0));
                self.tales.truncate(3);
            }
        }
    }

    /// The host's pose: pleased (false) or dismayed (true), for this machine now.
    fn dismayed(&self, h: Humor, now: Instant) -> bool {
        match h {
            Humor::Heat => self.heat >= 3 || self.boiled.is_some_and(|t| now.duration_since(t) < Duration::from_secs(3)),
            Humor::Dentist => self.grams >= 1_500,
            Humor::Office => self.caffeine.unwrap_or(1.0) < 0.35,
            Humor::FishTales => self.slipped.is_some_and(|t| now.duration_since(t) < Duration::from_millis(2_200)),
        }
    }

    /// Draws the gimmick: the host in `side` (beside the cabinet), its widget, and what's
    /// being said over `reels`.
    pub fn draw(&mut self, p: &egui::Painter, ctx: &egui::Context, t: &Theme, side: Rect, reels: Rect, now: Instant, time: f32) {
        let Some(h) = t.humor else { return };
        let dismayed = self.dismayed(h, now);
        // The widget first, at the top of the side.
        let wr = Rect::from_min_size(side.min, vec2(side.width(), (side.height() * 0.42).min(210.0)));
        match h {
            Humor::Heat => self.heat_meter(p, wr, reels, now, time),
            Humor::Dentist => self.dental_report(p, wr),
            Humor::Office => self.caffeine_meter(p, wr, reels, now, time),
            Humor::FishTales => self.trophy_wall(p, wr),
        }
        // The host under it, its two poses side by side in one picture.
        let hr = Rect::from_min_max(pos2(side.left(), wr.bottom() + 8.0), side.max);
        let s = hr.width().min(hr.height());
        let bob = (time * if dismayed { 9.0 } else { 2.2 }).sin() * if dismayed { 2.0 } else { 4.0 };
        let at = Rect::from_center_size(pos2(hr.center().x, hr.bottom() - s / 2.0 + bob), vec2(s, s));
        let uv = if dismayed { Rect::from_min_max(pos2(0.5, 0.0), pos2(1.0, 1.0)) } else { Rect::from_min_max(pos2(0.0, 0.0), pos2(0.5, 1.0)) };
        radial(p, at.center() + vec2(0.0, s * 0.18), s * 0.5, t.accent.gamma_multiply(0.16), Color32::TRANSPARENT);
        pics::part(p, ctx, &format!("host-{}", t.id), uv, at, 768, Color32::WHITE);
        // Scoville: breathing fire over the reels when it boils over.
        if h == Humor::Heat
            && let Some(b) = self.boiled
        {
            let k = now.duration_since(b).as_secs_f32();
            if k < 1.8 {
                self.fire_breath(p, at.center() + vec2(s * 0.1, -s * 0.18), reels, k, time);
            }
        }
    }

    /// Scoville Slots: a thermometer, steam off the reels when it's hot, the edges glowing.
    fn heat_meter(&mut self, p: &egui::Painter, r: Rect, reels: Rect, now: Instant, time: f32) {
        self.heat_shown += (self.heat as f32 - self.heat_shown) * 0.08;
        let level = self.heat_shown / 5.0;
        let body = Rect::from_center_size(pos2(r.left() + 26.0, r.center().y - 6.0), vec2(22.0, r.height() - 46.0));
        p.rect_filled(body, theme::r(11), Color32::from_rgb(0x1a, 0x10, 0x0e));
        p.rect_stroke(body, theme::r(11), Stroke::new(2.0f32, Color32::from_rgb(0xf4, 0xee, 0xe2)), StrokeKind::Outside);
        let bulb = pos2(body.center().x, body.bottom() + 8.0);
        let hot = super::mix(Color32::from_rgb(0xff, 0xc8, 0x3a), Color32::from_rgb(0xe8, 0x20, 0x10), level);
        let fill = Rect::from_min_max(pos2(body.left() + 5.0, body.bottom() - (body.height() - 10.0) * (0.12 + 0.88 * level)), pos2(body.right() - 5.0, body.bottom()));
        p.rect_filled(fill, theme::r(6), hot);
        p.circle_filled(bulb, 16.0, hot);
        p.circle_stroke(bulb, 16.0, Stroke::new(2.0f32, Color32::from_rgb(0xf4, 0xee, 0xe2)));
        for (k, name) in HEAT_NAMES.iter().enumerate() {
            let y = body.bottom() - 6.0 - (body.height() - 12.0) * (k as f32 / 5.0);
            let on = (self.heat as usize) >= k;
            p.line_segment([pos2(body.right() + 4.0, y), pos2(body.right() + 10.0, y)], Stroke::new(1.5f32, col::MUTED));
            p.text(pos2(body.right() + 14.0, y), Align2::LEFT_CENTER, *name, theme::bold_font(if on { 12.5 } else { 11.0 }), if on { hot } else { col::MUTED.gamma_multiply(0.6) });
        }
        // Hot: steam off the reels; hotter, the edges of the reels glow.
        if self.heat >= 3 {
            for i in 0..7 {
                let ph = (time * 0.6 + i as f32 * 0.37) % 1.0;
                let x = reels.left() + reels.width() * ((i as f32 * 0.153 + 0.08) % 1.0) + (time * 1.7 + i as f32).sin() * 8.0;
                let y = reels.top() + 10.0 - ph * 70.0;
                p.circle_filled(pos2(x, y), 6.0 + ph * 12.0, Color32::from_white_alpha((60.0 * (1.0 - ph)) as u8));
            }
        }
        if self.heat >= 4 {
            let a = (0.35 + 0.15 * (time * 4.0).sin()) * 255.0;
            p.rect_stroke(reels.expand(6.0), theme::r(16), Stroke::new(8.0f32, Color32::from_rgba_unmultiplied(0xff, 0x3a, 0x10, a as u8)), StrokeKind::Outside);
        }
        let _ = now;
    }

    fn fire_breath(&self, p: &egui::Painter, mouth: Pos2, reels: Rect, k: f32, time: f32) {
        // A jet of fire from the chili's mouth across the reels, growing and then burning out.
        let reach = (k / 0.5).min(1.0);
        let fade = ((1.8 - k) / 0.6).clamp(0.0, 1.0);
        let target = pos2(reels.right() - 20.0, reels.center().y);
        for i in 0..40 {
            let f = i as f32 / 40.0;
            if f > reach {
                break;
            }
            let at = mouth + (target - mouth) * f + vec2(0.0, (time * 20.0 + i as f32 * 1.7).sin() * 18.0 * f);
            let r = 8.0 + 46.0 * f;
            let c = super::mix(Color32::from_rgb(0xff, 0xf0, 0x80), Color32::from_rgb(0xff, 0x30, 0x10), f);
            p.circle_filled(at, r, c.gamma_multiply(0.18 * fade));
            p.circle_filled(at, r * 0.5, c.gamma_multiply(0.35 * fade));
        }
    }

    /// Sugar Rush: the dentist's clipboard.
    fn dental_report(&mut self, p: &egui::Painter, r: Rect) {
        self.grams_shown += (self.grams as f64 - self.grams_shown) * 0.1;
        let board = r.shrink2(vec2(6.0, 4.0));
        p.rect_filled(board.translate(vec2(0.0, 4.0)), theme::r(10), Color32::from_black_alpha(120));
        p.rect_filled(board, theme::r(10), Color32::from_rgb(0xb8, 0x82, 0x50));
        let paper = board.shrink2(vec2(10.0, 12.0)).translate(vec2(0.0, 6.0));
        p.rect_filled(paper, theme::r(4), Color32::from_rgb(0xfd, 0xfa, 0xf2));
        let clip = Rect::from_center_size(pos2(board.center().x, board.top() + 6.0), vec2(56.0, 16.0));
        p.rect_filled(clip, theme::r(4), Color32::from_rgb(0x9a, 0x9a, 0xa4));
        let ink = Color32::from_rgb(0x2a, 0x2a, 0x3a);
        p.text(paper.left_top() + vec2(10.0, 10.0), Align2::LEFT_TOP, "SUGAR", theme::bold_font(13.0), Color32::from_rgb(0xc8, 0x2a, 0x5a));
        let grams = self.grams_shown.round() as u64;
        p.text(paper.left_top() + vec2(10.0, 34.0), Align2::LEFT_TOP, format!("{} g", chips_text(grams)), theme::bold_font(22.0), ink);
    }

    /// The Daily Grind: the caffeine level, the reels nodding off without it.
    fn caffeine_meter(&mut self, p: &egui::Painter, r: Rect, reels: Rect, now: Instant, time: f32) {
        let c = self.caffeine.unwrap_or(1.0);
        self.caffeine_shown += (c - self.caffeine_shown) * 0.08;
        let mug = Rect::from_center_size(pos2(r.left() + 52.0, r.center().y), vec2(70.0, (r.height() - 40.0).min(120.0)));
        p.rect_filled(mug, theme::r(10), Color32::from_rgb(0xf4, 0xee, 0xe2));
        let coffee = Rect::from_min_max(pos2(mug.left() + 6.0, mug.bottom() - 6.0 - (mug.height() - 12.0) * self.caffeine_shown), mug.max - vec2(6.0, 6.0));
        p.rect_filled(coffee, theme::r(6), Color32::from_rgb(0x5a, 0x32, 0x16));
        // An espresso shot: the mug glows and steams for a moment.
        if let Some(t) = self.espresso {
            let k = now.duration_since(t).as_secs_f32();
            if k < 2.0 {
                let a = 1.0 - k / 2.0;
                radial(p, mug.center(), mug.width() * 1.1, col::GOLD.gamma_multiply(0.45 * a), Color32::TRANSPARENT);
                for i in 0..3 {
                    let ph = (k * 0.8 + i as f32 / 3.0) % 1.0;
                    p.circle_filled(pos2(mug.center().x + (i as f32 - 1.0) * 14.0, mug.top() - 8.0 - ph * 40.0), 5.0 + ph * 8.0, Color32::from_white_alpha((90.0 * a * (1.0 - ph)) as u8));
                }
            }
        }
        p.circle_stroke(mug.right_center() + vec2(10.0, 0.0), 14.0, Stroke::new(6.0f32, Color32::from_rgb(0xf4, 0xee, 0xe2)));
        p.text(pos2(mug.right() + 34.0, mug.top() + 8.0), Align2::LEFT_TOP, "CAFFEINE", theme::bold_font(12.0), col::MUTED);
        p.text(pos2(mug.right() + 34.0, mug.top() + 26.0), Align2::LEFT_TOP, format!("{:.0}%", c * 100.0), theme::bold_font(22.0), if c < 0.15 { Color32::from_rgb(0xc8, 0x6a, 0x4a) } else { col::GOLD2 });
        // No caffeine: Zzz off the reels.
        if c < 0.15 {
            for i in 0..3 {
                let ph = (time * 0.5 + i as f32 / 3.0) % 1.0;
                let at = pos2(reels.right() - 40.0 + ph * 30.0, reels.top() + 20.0 - ph * 50.0);
                p.text(at, Align2::CENTER_CENTER, "Z", theme::bold_font(14.0 + ph * 16.0), Color32::from_white_alpha((200.0 * (1.0 - ph)) as u8));
            }
        }
    }

    /// Frostbite Fortune: the trophy wall of the tallest tales.
    fn trophy_wall(&self, p: &egui::Painter, r: Rect) {
        let board = r.shrink2(vec2(4.0, 4.0));
        vgrad_round(p, board, 10.0, Color32::from_rgb(0x6a, 0x42, 0x22), Color32::from_rgb(0x3a, 0x22, 0x10));
        p.rect_stroke(board, theme::r(10), Stroke::new(2.0f32, Color32::from_rgb(0x2a, 0x16, 0x08)), StrokeKind::Inside);
        p.text(pos2(board.center().x, board.top() + 16.0), Align2::CENTER_CENTER, "BIGGEST CATCHES", theme::bold_font(13.0), Color32::from_rgb(0xff, 0xe2, 0x9a));
        if self.tales.is_empty() {
            let g = p.layout("No wins yet.".into(), theme::font(12.0), Color32::from_rgb(0xe8, 0xd8, 0xc0), board.width() - 24.0);
            p.galley(pos2(board.left() + 12.0, board.top() + 36.0), g, Color32::WHITE);
            return;
        }
        for (i, (mult, chips)) in self.tales.iter().enumerate() {
            let tale = format!("{} · ×{:.1}", chips_text(*chips), *mult as f64 / 100.0);
            let plaque = Rect::from_min_size(pos2(board.left() + 10.0, board.top() + 34.0 + i as f32 * 44.0), vec2(board.width() - 20.0, 38.0));
            if plaque.bottom() > board.bottom() - 4.0 {
                break;
            }
            p.rect_filled(plaque, theme::r(6), Color32::from_rgb(0xd8, 0xb0, 0x5a));
            p.add(Shape::circle_filled(plaque.left_center() + vec2(14.0, 0.0), 9.0, Color32::from_rgb(0x5a, 0x7a, 0x9a)));
            let g = p.layout(tale.clone(), theme::bold_font(11.5), Color32::from_rgb(0x2a, 0x1a, 0x08), plaque.width() - 36.0);
            p.galley(pos2(plaque.left() + 28.0, plaque.center().y - g.size().y / 2.0), g, Color32::BLACK);
        }
    }
}

