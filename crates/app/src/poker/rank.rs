//! Poker's MMR at a glance: each tier's color and emblem (a cut gem in a hexagon, in the
//! tier's colors, crowned at the top two), and the ladder (everyone rated here, bots too).

use eframe::egui::{self, Align2, Color32, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::net::ToServer;
use pyr_poker::rating::Tier;

use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

pub fn color(t: Tier) -> Color32 {
    match t {
        Tier::Bronze => Color32::from_rgb(0xc0, 0x7a, 0x48),
        Tier::Silver => Color32::from_rgb(0xc4, 0xcc, 0xd8),
        Tier::Gold => Color32::from_rgb(0xf5, 0xc4, 0x51),
        Tier::Platinum => Color32::from_rgb(0x5f, 0xd8, 0xc8),
        Tier::Diamond => Color32::from_rgb(0x6a, 0xa8, 0xff),
        Tier::Master => Color32::from_rgb(0xc2, 0x7a, 0xff),
        Tier::Grandmaster => Color32::from_rgb(0xff, 0x5a, 0x5a),
    }
}

fn mix(a: Color32, b: Color32, k: f32) -> Color32 {
    crate::casino::mix(a, b, k)
}

/// A tier's emblem, `r` across from its middle to a corner.
pub fn emblem(p: &egui::Painter, c: Pos2, r: f32, t: Tier) {
    let base = color(t);
    let hex = |rad: f32| -> Vec<Pos2> {
        (0..6)
            .map(|k| {
                let a = (k as f32 * 60.0 - 90.0).to_radians();
                c + vec2(a.cos(), a.sin()) * rad
            })
            .collect()
    };
    p.add(Shape::convex_polygon(hex(r + 1.6), Color32::from_black_alpha(170), Stroke::NONE));
    p.add(Shape::convex_polygon(hex(r), mix(base, Color32::BLACK, 0.45), Stroke::new((r * 0.09).max(1.0), mix(base, Color32::WHITE, 0.3))));
    // The gem: a lit top, a shadowed bottom.
    let g = hex(r * 0.62);
    p.add(Shape::convex_polygon(g.clone(), base, Stroke::NONE));
    p.add(Shape::convex_polygon(vec![g[5], g[0], g[1], c], mix(base, Color32::WHITE, 0.45), Stroke::NONE));
    p.add(Shape::convex_polygon(vec![g[2], g[3], g[4], c], mix(base, Color32::BLACK, 0.25), Stroke::NONE));
    if t >= Tier::Master {
        // A crown over it.
        crate::icons::crown(p, Rect::from_center_size(c - vec2(0.0, r * 1.18), vec2(r * 1.1, r * 0.7)), Color32::from_rgb(0xff, 0xd7, 0x6a));
    }
}

/// A bot's way of playing, in a word or two.
pub fn kind_name(k: pyr_poker::bots::persona::Kind) -> &'static str {
    use pyr_poker::bots::persona::Kind;
    match k {
        Kind::Pro => "a pro",
        Kind::Maniac => "a maniac",
        Kind::Lag => "loose and aggressive",
        Kind::Nit => "a nit",
        Kind::Rock => "a rock",
        Kind::Trapper => "a trapper",
        Kind::Station => "a calling station",
        Kind::Gambler => "a gambler",
        Kind::Grinder => "a grinder",
        Kind::Bully => "a bully",
        Kind::Wildcard => "a wildcard",
    }
}

/// "Gold · 1,180", or "Placing · 1,000" while it's finding its level.
pub fn label(mmr: u32, placing: bool) -> String {
    let t = Tier::of(mmr as f64);
    if placing { format!("{} · {} MMR (placing)", t.name(), chips_text(mmr as u64)) } else { format!("{} · {} MMR", t.name(), chips_text(mmr as u64)) }
}

impl PyrApp {
    /// The MMR ladder (asked for every ten seconds while it shows).
    pub(super) fn ladder(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        if !s.has(pyr_proto::control::CASINO_012_FEATURE) {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new("Ratings come when this server's Pyr is updated to 0.12.").color(theme::MUTED()));
            });
            return;
        }
        if self.poker.ratings_asked.is_none_or(|t| t.elapsed().as_secs() >= 10) {
            self.poker.ratings_asked = Some(std::time::Instant::now());
            self.engine.poker(&s.key, ToServer::Ratings);
        }
        let Some(rows) = s.poker.ratings.clone() else {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new("Loading…").color(theme::MUTED()));
            });
            return;
        };
        if rows.is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Nobody's rated yet").font(theme::bold_font(18.0)).color(theme::TEXT()));
                ui.label(RichText::new("Play a cash hand or finish a tournament: everyone at the table gets a rating, bots too.").color(theme::MUTED()));
            });
            return;
        }
        let me = s.me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
        for (i, row) in rows.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                let w = (ui.available_width() - 28.0).max(300.0);
                let (r, _) = ui.allocate_exact_size(vec2(w, 50.0), Sense::hover());
                let p = ui.painter();
                let mine = row.id == me;
                p.rect_filled(r, theme::r(10), if mine { Color32::from_rgba_unmultiplied(0xf5, 0xc4, 0x51, 30) } else if i % 2 == 0 { Color32::from_white_alpha(8) } else { Color32::TRANSPARENT });
                if mine {
                    p.rect_stroke(r, theme::r(10), Stroke::new(1.0f32, crate::casino::col::GOLD_DARK), StrokeKind::Inside);
                }
                let c = r.left_center() + vec2(22.0, 0.0);
                let medal = [Color32::from_rgb(0xff, 0xd2, 0x4a), Color32::from_rgb(0xd6, 0xdc, 0xe4), Color32::from_rgb(0xd0, 0x8a, 0x4c)].get(i).copied();
                match medal {
                    Some(m) => {
                        p.circle_filled(c, 12.0, m);
                        p.text(c, Align2::CENTER_CENTER, (i + 1).to_string(), theme::bold_font(12.0), Color32::from_rgb(0x22, 0x1a, 0x10));
                    }
                    None => {
                        p.text(c, Align2::CENTER_CENTER, (i + 1).to_string(), theme::bold_font(13.0), theme::MUTED());
                    }
                }
                let tier = Tier::of(row.mmr as f64);
                emblem(p, r.left_center() + vec2(62.0, 1.0), 14.0, tier);
                let name = p.layout_no_wrap(row.name.clone(), theme::bold_font(15.0), theme::TEXT());
                let nw = name.size().x;
                p.galley(pos2(r.left() + 88.0, r.center().y - 10.0 - name.size().y / 2.0), name, theme::TEXT());
                if let Some(level) = row.bot {
                    let (text, col) = match level {
                        pyr_poker::bots::Level::Genius => ("GENIUS BOT", Color32::from_rgb(0x7a, 0x4a, 0xe0)),
                        pyr_poker::bots::Level::Shark => ("SHARK BOT", Color32::from_rgb(0xc4, 0x34, 0x3e)),
                        pyr_poker::bots::Level::Regular => ("BOT", Color32::from_rgb(0x3d, 0x5f, 0xc4)),
                        pyr_poker::bots::Level::Rookie => ("ROOKIE BOT", Color32::from_rgb(0x2f, 0x8a, 0x52)),
                    };
                    let tag = Rect::from_min_size(pos2(r.left() + 96.0 + nw, r.center().y - 18.0), vec2(10.0 + 6.4 * text.len() as f32, 16.0));
                    p.rect_filled(tag, theme::r(8), col);
                    p.text(tag.center(), Align2::CENTER_CENTER, text, theme::bold_font(9.5), Color32::WHITE);
                }
                // (A bot: how it plays, too.)
                let kind = row.bot.map(|_| format!(" · {}", kind_name(pyr_poker::bots::persona::of(&row.name).kind))).unwrap_or_default();
                let sub = format!("{}{}{kind} · {} · {}", tier.name(), if row.placing { " (placing)" } else { "" }, crate::poker::cards::count(row.tourneys as u64, "tournament", "tournaments"), crate::poker::cards::count(row.hands as u64, "hand", "hands"));
                p.text(pos2(r.left() + 88.0, r.center().y + 11.0), Align2::LEFT_CENTER, sub, theme::font(12.0), theme::MUTED());
                p.text(r.right_center() - vec2(16.0, 6.0), Align2::RIGHT_CENTER, chips_text(row.mmr as u64), theme::bold_font(18.0), color(tier));
                p.text(r.right_center() - vec2(16.0, -12.0), Align2::RIGHT_CENTER, format!("best {}", chips_text(row.peak as u64)), theme::font(11.0), theme::DIM());
            });
            ui.add_space(4.0);
        }
    }

    /// My rating, big: the emblem, the tier and MMR, my best, and how far to the next tier.
    pub(super) fn rank_card(&mut self, ui: &mut egui::Ui, me: &pyr_poker::net::RecordView) {
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let w = (ui.available_width() - 28.0).max(300.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 108.0), Sense::hover());
            let p = ui.painter();
            let tier = Tier::of(me.mmr as f64);
            let col = color(tier);
            crate::casino::vgrad_round(p, r, 14.0, mix(col, Color32::BLACK, 0.78), Color32::from_rgb(0x10, 0x0e, 0x14));
            p.rect_stroke(r, theme::r(14), Stroke::new(1.0f32, mix(col, Color32::BLACK, 0.4)), StrokeKind::Inside);
            emblem(p, r.left_center() + vec2(58.0, 4.0), 34.0, tier);
            let x = r.left() + 112.0;
            p.text(pos2(x, r.top() + 26.0), Align2::LEFT_CENTER, "POKER RANK", theme::bold_font(11.5), theme::MUTED());
            p.text(pos2(x, r.top() + 52.0), Align2::LEFT_CENTER, format!("{} · {} MMR", tier.name(), chips_text(me.mmr as u64)), theme::bold_font(22.0), col);
            let note = if me.mmr_placing { "Placing: your first games move it faster while it finds your level.".to_string() } else { format!("Best yet {}", chips_text(me.mmr_peak as u64)) };
            p.text(pos2(x, r.top() + 80.0), Align2::LEFT_CENTER, note, theme::font(12.5), theme::MUTED());
            // How far to the next tier.
            if let Some(next) = Tier::ALL.iter().copied().find(|t| *t > tier) {
                let from = tier.floor();
                let to = next.floor();
                let k = ((me.mmr as f64 - from) / (to - from)).clamp(0.0, 1.0) as f32;
                let bar = Rect::from_min_size(pos2(r.right() - 260.0, r.center().y - 4.0), vec2(230.0, 8.0));
                p.rect_filled(bar, theme::r(4), Color32::from_white_alpha(20));
                p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * k, bar.height())), theme::r(4), col);
                p.text(bar.left_top() - vec2(0.0, 12.0), Align2::LEFT_CENTER, format!("{} to {}", chips_text((to - me.mmr as f64).max(0.0) as u64), next.name()), theme::font(12.0), theme::MUTED());
            }
        });
    }
}
