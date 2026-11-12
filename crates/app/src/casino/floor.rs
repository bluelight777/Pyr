//! The casino floor: the house's games, in chapters (0.13.2). The Bonfire and the Phoenix
//! Jackpot climbing, wins as they happen, a bar to jump to each chapter, then the chapters:
//! Slots (Pyr Sevens, Ember Dragon and Reel Row's twelve), Poker (its tables, Flash Pots, Pyr
//! Friday Night and tournaments), Table games and Pyr Originals. Each game is a tile like
//! Stake's (0.13.4): its name big in its own lettering, the studio under it, how many are
//! playing it right now and my best win under the tile, what it pays back on pointing at it.
//! All of it as 0.13.28 draws it, measured off its screen. The art is Meshy's (`casino::pics`).

use eframe::egui::{self, Align2, Color32, CornerRadius, Rect, RichText, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::state::ServerView;
use pyr_poker::casino::{Game, reels as rr};

use super::games_menu::{Chapter, Jump};
use super::reel_themes::THEMES;
use super::{col, glow_text, hgrad, pics, radial, vgrad_round_fn};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// "3d 4h", "2h 05m", "12m": how long until.
pub fn until(ms: u64) -> String {
    let now = pyr_proto::now_ms().max(0) as u64;
    let left = ms.saturating_sub(now) / 1000;
    let (d, h, m) = (left / 86_400, left % 86_400 / 3600, left % 3600 / 60);
    if d > 0 {
        format!("{d}d {h}h")
    } else if h > 0 {
        format!("{h}h {m:02}m")
    } else {
        format!("{m}m")
    }
}

/// What a tile on the floor opens.
#[derive(Clone, Copy)]
enum Opens {
    Game(Game),
    Machine(u8),
}

/// A game's tile: its picture (cut to a portrait about `focus`, a fraction across), its name as
/// lettered on the tile, the studio, its badges, what it pays back, and what it is (on
/// hovering).
struct Card {
    opens: Opens,
    /// Its statistics key (who's playing it, my best win).
    key: String,
    art: String,
    focus: f32,
    badges: Vec<(&'static str, Color32)>,
    rtp: String,
    pitch: String,
}

const WHEEL: Color32 = Color32::from_rgb(0x8a, 0x5a, 0xff);
const NEW: Color32 = Color32::from_rgb(0x3a, 0xd0, 0x8a);
/// The players line's dot: gray while nobody's playing, green while someone is.
const NOBODY: Color32 = Color32::from_rgb(0x5a, 0x5a, 0x5a);
const SOMEONE: Color32 = Color32::from_rgb(0x3a, 0xe0, 0x7a);

/// "RTP 99.00%": what a game pays back (a slot's by design; the others' with the best bets).
fn rtp_line(key: &str, g: Game) -> String {
    let bp = pyr_poker::casino::slot_rtp_bp(key).unwrap_or(10_000u32.saturating_sub(g.house_edge_bp() as u32));
    format!("RTP {}", super::info::percent_bp(bp))
}

fn game_card(g: Game, art: &str, focus: f32, badges: Vec<(&'static str, Color32)>) -> Card {
    Card { opens: Opens::Game(g), key: g.id().to_string(), art: art.into(), focus, badges, rtp: rtp_line(g.id(), g), pitch: g.pitch().into() }
}

/// A game's name on its tile, as 0.13.28 letters it: its lines, its display font (by its
/// picture's name), and the studio under it.
fn lettering(art: &str) -> (&'static [&'static str], &'static str, &'static str) {
    const PYR: &str = "PYR ORIGINALS";
    const CHIMBITA: &str = "CHIMBITA GAMES";
    match art {
        "tile-sevens" => (&["PYR", "SEVENS"], "barlow-black", PYR),
        "tile-dragon" => (&["EMBER", "DRAGON"], "anton", PYR),
        "reels-scoville-tile" => (&["SCOVILLE", "SLOTS"], "anton", CHIMBITA),
        "reels-deepsea-tile" => (&["DEEP SEA", "TREASURE"], "bebas", CHIMBITA),
        "reels-cosmic-tile" => (&["COSMIC", "JACKPOT"], "barlow-black", CHIMBITA),
        "reels-sugar-tile" => (&["SUGAR", "RUSH"], "squada", CHIMBITA),
        "reels-goldrush-tile" => (&["GOLD RUSH"], "fjalla", CHIMBITA),
        "reels-neon-tile" => (&["NEON", "NIGHTS"], "archivo-black", CHIMBITA),
        "reels-tomb-tile" => (&["TOMB", "OF RA"], "anton", CHIMBITA),
        "reels-frost-tile" => (&["FROSTBITE", "FORTUNE"], "bebas", CHIMBITA),
        "reels-cauldron-tile" => (&["MOONLIT", "CAULDRON"], "barlow-black", CHIMBITA),
        "reels-hoard-tile" => (&["DRAGON'S", "HOARD"], "squada", CHIMBITA),
        "reels-grind-tile" => (&["THE DAILY", "GRIND"], "fjalla", CHIMBITA),
        "reels-cabaret-tile" => (&["STARLIGHT", "CABARET"], "archivo-black", CHIMBITA),
        "tile-blackjack" => (&["BLACKJACK"], "bebas", PYR),
        "tile-roulette" => (&["ROULETTE"], "squada", PYR),
        "tile-baccarat" => (&["BACCARAT"], "barlow-black", PYR),
        "tile-craps" => (&["CRAPS"], "fjalla", PYR),
        "tile-plinko" => (&["PLINKO"], "anton", PYR),
        "tile-dice" => (&["DICE"], "fjalla", PYR),
        "tile-keno" => (&["KENO"], "fjalla", PYR),
        _ => (&[], "anton", PYR),
    }
}

impl PyrApp {
    pub fn casino_floor(&mut self, ui: &mut egui::Ui, s: &ServerView) {
        self.casino_open(s);
        // The casino's state again on coming back to the floor, and every 45 seconds on it
        // (who's playing what), as 0.13.28 asks.
        let now = ui.input(|i| i.time);
        let c = &mut self.poker.casino;
        if c.floor_drawn > 0.0 && (now - c.floor_drawn > 1.0 || now - c.floor_asked > 45.0) {
            self.engine.casino_open(&s.key, true);
            self.poker.casino.floor_asked = now;
        } else if c.floor_drawn == 0.0 {
            c.floor_asked = now;
        }
        self.poker.casino.floor_drawn = now;
        let w = (ui.available_width() - 56.0).max(320.0);
        // The Bonfire, biggest of all (on a server that has it).
        self.bonfire_keep(s);
        if s.poker.bonfire.is_some() {
            self.bonfire_hero(ui, s, w);
            ui.add_space(14.0);
        }
        self.jackpot_hero(ui, s, w);
        ui.add_space(12.0);
        self.live_wins(ui, s, w);
        ui.add_space(14.0);
        self.chapter_bar(ui);
        ui.add_space(22.0);
        // The slots: Pyr Sevens, then Ember Dragon and Reel Row's machines (on a server that has
        // them).
        let reel_row = s.has(pyr_proto::control::CASINO_012_FEATURE);
        let v13 = s.has(pyr_proto::control::CASINO_013_FEATURE);
        let remastered = s.has(pyr_proto::control::REMASTER_FEATURE);
        let mut slots = vec![game_card(Game::Slots, "tile-sevens", 0.5, vec![("JACKPOT", col::EMBER)])];
        if reel_row {
            slots.push(game_card(Game::Dragon, "tile-dragon", 0.5, if v13 { vec![("JACKPOTS", col::EMBER), ("BONUS BUY", NEW)] } else { vec![("JACKPOTS", col::EMBER), ("NEW", NEW)] }));
            for (m, th) in THEMES.iter().enumerate() {
                let mut badges = Vec::new();
                if remastered {
                    // Its feature and its reach (0.13.28's).
                    badges.push(super::rmshow::feature_badge(m as u8));
                    badges.push(top_badge(m as u8));
                } else if v13 {
                    // Its reach: how far its top prize goes, or its progressive jackpot.
                    badges.push(top_badge(m as u8));
                    badges.push(("BONUS BUY", NEW));
                } else if rr::bonus_kind(m as u8) == rr::BonusKind::Wheel {
                    badges.push(("WHEEL", WHEEL));
                }
                if badges.is_empty() {
                    badges.push(("NEW", NEW));
                }
                let key = pyr_poker::casino::reels_key(m as u8);
                slots.push(Card {
                    opens: Opens::Machine(m as u8),
                    art: format!("reels-{}-tile", th.id),
                    focus: if th.id == "goldrush" { 0.3 } else { 0.5 },
                    badges,
                    rtp: rtp_line(&key, Game::Reels),
                    key,
                    pitch: if remastered {
                        let (g, _) = pyr_poker::casino::reelrow::GAMES[m];
                        format!("{} {} Ante, a feature to buy, and its own mini game, the {g}. {}", th.tagline, super::rmshow::feature_words(m as u8).0, top_words(s, m as u8))
                    } else if v13 {
                        let (g, what) = pyr_poker::casino::reelrow::GAMES[m];
                        format!("{} 243 ways, and its own mini game, the {g}: {what} {}", th.tagline, top_words(s, m as u8))
                    } else {
                        format!("{} 243 ways, {}.", th.tagline, if rr::bonus_kind(m as u8) == rr::BonusKind::Wheel { "and a prize wheel" } else { "and a pick-and-dodge bonus" })
                    },
                });
            }
        }
        let note = (!reel_row).then_some("Twelve more machines come when this server's Pyr is updated to 0.12.");
        self.chapter(ui, s, w, Chapter::Slots, &format!("{} game{} · every one pays back about 99%", slots.len(), if slots.len() == 1 { "" } else { "s" }), &slots, note);
        ui.add_space(14.0);
        // Poker, like every other game here: its tables, Flash Pots and tournaments.
        self.chapter_head(ui, w, Chapter::Poker, "Cash tables, Flash Pot sit & go and tournaments, with friends and bots");
        ui.add_space(16.0);
        self.poker_sections(ui, s);
        ui.add_space(30.0);
        // (On a 0.12.0 server: tables with seats for people and bots; roulette's on 0.13.0.)
        let seats = |n: &'static str| if reel_row { vec![(n, col::GOLD_DARK)] } else { vec![] };
        let roulette_seats = if v13 { vec![("5 SEATS", col::GOLD_DARK), ("NEW", NEW)] } else { vec![] };
        let tables = vec![
            game_card(Game::Blackjack, "tile-blackjack", 0.38, seats("7 SEATS")),
            game_card(Game::Roulette, "tile-roulette", 0.5, roulette_seats),
            game_card(Game::Baccarat, "tile-baccarat", 0.5, seats("5 SEATS")),
            game_card(Game::Craps, "tile-craps", 0.5, seats("5 SEATS")),
        ];
        self.chapter(ui, s, w, Chapter::Tables, "Dealt and rolled by the house, checked by your Pyr", &tables, None);
        ui.add_space(14.0);
        let mut originals = Vec::new();
        // (Plinko, 0.13.5: where the server has it, first.)
        if s.has(pyr_proto::control::PLINKO_FEATURE) {
            originals.push(game_card(Game::Plinko, "tile-plinko", 0.5, vec![("NEW", NEW), ("10,000×", col::EMBER)]));
        }
        originals.extend([game_card(Game::Dice, "tile-dice", 0.5, vec![]), game_card(Game::Keno, "tile-keno", 0.5, vec![])]);
        self.chapter(ui, s, w, Chapter::Originals, "Quick games: pick your odds", &originals, None);
        // (Its words 64 under the tiles: the row's a button's height, the words in its middle.)
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            ui.label(RichText::new("Play chips only: they can't be bought or cashed out. Every round is checked by your own Pyr.").color(theme::DIM()).size(12.5));
            if ui.link(RichText::new("How?").size(12.5)).clicked() {
                self.poker.casino.fairness_open = true;
            }
        });
    }

    /// The bar over the chapters (0.13.2): a button for each, in its color, that jumps there.
    /// (As wide as the page less 92 allows, four of them 20 apart, 260 at the most: 0.13.28's
    /// run on 4 past the chapters' own right edge.)
    fn chapter_bar(&mut self, ui: &mut egui::Ui) {
        let page = ui.available_width();
        let (row, _) = ui.allocate_exact_size(vec2(page, 48.0), Sense::hover());
        let bw = ((page - 92.0) / 4.0).min(260.0);
        for (i, c) in Chapter::ALL.into_iter().enumerate() {
            let r = Rect::from_min_size(pos2(row.left() + 28.0 + i as f32 * (bw + 20.0), row.top()), vec2(bw, 48.0));
            let resp = ui.interact(r, ui.id().with(("chapter-bar", i)), Sense::click());
            let p = ui.painter();
            let color = c.color();
            p.rect_filled(r, theme::r(12), color.gamma_multiply(if resp.hovered() { 0.4 } else { 0.22 }));
            p.rect_filled(Rect::from_min_max(pos2(r.left(), r.top() + 8.0), pos2(r.left() + 5.0, r.bottom() - 8.0)), theme::r(2), color);
            p.rect_stroke(r, theme::r(12), Stroke::new(1.5f32, color), StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, c.name(), theme::bold_font(17.0), Color32::WHITE);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.poker.casino.jump = Some(Jump::Chapter(c));
            }
        }
    }

    /// A chapter's heading: its name on a band of its color fading out to the right, a bar of
    /// it at the left, a line of it under. (The bar over the chapters and the Games menu jump
    /// here.)
    fn chapter_head(&mut self, ui: &mut egui::Ui, w: f32, c: Chapter, sub: &str) {
        self.floor_anchor(ui, c);
        let (row, _) = ui.allocate_exact_size(vec2(w + 56.0, 62.0), Sense::hover());
        let band = Rect::from_min_size(pos2(row.left() + 28.0, row.top()), vec2(w, 56.0));
        let p = ui.painter();
        let color = c.color();
        hgrad(p, band, color.gamma_multiply(0.3), Color32::TRANSPARENT);
        p.rect_filled(Rect::from_min_size(band.min, vec2(6.0, band.height())), theme::r(3), color);
        let g = p.layout_no_wrap(c.name().to_string(), theme::bold_font(30.0), Color32::WHITE);
        let tw = g.size().x;
        p.galley(pos2(band.left() + 20.0, band.center().y - g.size().y / 2.0), g, Color32::WHITE);
        p.text(pos2(band.left() + 20.0 + tw + 16.0, band.center().y + 3.25), Align2::LEFT_CENTER, sub, theme::font(14.0), col::MUTED);
        p.rect_filled(Rect::from_min_size(pos2(band.left(), band.bottom() + 4.0), vec2(w, 2.0)), CornerRadius::ZERO, color.gamma_multiply(0.7));
    }

    /// The Phoenix Jackpot: the phoenix rising on the right, the pool climbing on the left.
    fn jackpot_hero(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32) {
        let c = &s.poker.casino;
        let t = ui.ctx().input(|i| i.time) as f32;
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let h = (w * 0.24).clamp(184.0, 236.0);
            let (r, resp) = ui.allocate_exact_size(vec2(w, h), Sense::click());
            let p = ui.painter().clone();
            let base = Color32::from_rgb(0x16, 0x07, 0x06);
            p.rect_filled(r.translate(vec2(0.0, 10.0)), theme::r(22), Color32::from_black_alpha(120));
            p.rect_filled(r, theme::r(20), base);
            let ph = Rect::from_min_max(pos2(r.right() - r.width() * 0.5, r.top()), r.max);
            pics::cover_rounded(&p, &ctx, "tile-phoenix", ph, 20.0, (0.5, 0.45), 1024, Color32::WHITE);
            // (The phoenix's edge melts into the dark.)
            hgrad(&p, Rect::from_min_max(ph.min, pos2(ph.left() + ph.width() * 0.45, ph.bottom())), base, base.gamma_multiply(0.0));
            radial(&p, pos2(r.left() + w * 0.18, r.center().y + 20.0), w * 0.32, col::EMBER.gamma_multiply(0.20 + 0.04 * (t * 1.7).sin()), Color32::TRANSPARENT);
            // Sparks rising.
            let clip = p.with_clip_rect(r.shrink(2.0));
            for i in 0..34 {
                let fi = i as f32;
                let speed = 0.08 + (fi * 0.37).fract() * 0.12;
                let k = (t * speed + (fi * 0.618).fract()) % 1.0;
                let x = r.left() + ((fi * 0.4142).fract() * 0.98 + 0.01) * r.width() + (t * 0.9 + fi).sin() * 8.0;
                let y = r.bottom() - k * (r.height() + 10.0);
                let a = (1.0 - k) * (k / 0.1).min(1.0);
                clip.circle_filled(pos2(x, y), 1.2 + (fi * 0.73).fract() * 1.8, col::GOLD2.gamma_multiply(0.75 * a));
            }
            let x = r.left() + 30.0;
            p.text(pos2(x, r.top() + 34.0), Align2::LEFT_CENTER, "PHOENIX JACKPOT", theme::bold_font(15.0), col::GOLD);
            let shown = self.poker.casino.slots.shown_jackpot(c.jackpot);
            let size = (h * 0.27).clamp(44.0, 62.0);
            glow_text(&p, pos2(x - 2.0, r.top() + 34.0 + size * 0.95), Align2::LEFT_CENTER, &chips_text(shown), theme::bold_font(size), col::GOLD2, col::EMBER.gamma_multiply(0.5));
            let g = p.layout("Five wild flames on a line of Pyr Sevens win it all. It grows with every spin.".into(), theme::font(13.5), col::TEXT, (w * 0.46).max(220.0));
            let gy = r.top() + 34.0 + size * 1.6;
            p.galley(pos2(x, gy), g.clone(), col::TEXT);
            let b = Rect::from_min_size(pos2(x, (gy + g.size().y + 14.0).min(r.bottom() - 54.0)), vec2(168.0, 42.0));
            let hover = resp.hovered();
            p.rect_filled(b.translate(vec2(0.0, 3.0)), theme::r(21), Color32::from_black_alpha(120));
            vgrad_round_fn(&p, b, 21.0, |k| super::mix(if hover { col::GOLD2 } else { col::GOLD }, col::GOLD_DARK, k * 0.6));
            p.text(b.center(), Align2::CENTER_CENTER, "Spin for it  ›", theme::bold_font(16.0), Color32::from_rgb(0x2a, 0x10, 0x04));
            p.rect_stroke(r, theme::r(20), Stroke::new(if hover { 2.0 } else { 1.5f32 }, if hover { col::GOLD } else { col::GOLD_DARK }), StrokeKind::Inside);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                self.poker.casino.game = Some(Game::Slots);
            }
        });
    }

    /// Wins as they happen, scrolling by (smoothly: see `text_exact`).
    fn live_wins(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32) {
        let c = &s.poker.casino;
        let time = ui.ctx().input(|i| i.time);
        let t = time as f32;
        ui.horizontal(|ui| {
            ui.add_space(28.0);
            let (r, _) = ui.allocate_exact_size(vec2(w, 44.0), Sense::hover());
            let p = ui.painter();
            p.rect_filled(r, theme::r(22), Color32::from_rgba_unmultiplied(0x0e, 0x0b, 0x12, 225));
            p.rect_stroke(r, theme::r(22), Stroke::new(1.0f32, col::GOLD_DARK.gamma_multiply(0.8)), StrokeKind::Inside);
            let tag = Rect::from_min_size(r.left_top() + vec2(6.0, 6.0), vec2(106.0, 32.0));
            p.rect_filled(tag, theme::r(16), Color32::from_rgb(0x3a, 0x0e, 0x0e));
            let pulse = 0.55 + 0.45 * (t * 3.0).sin().abs();
            p.circle_filled(tag.left_center() + vec2(16.0, 0.0), 4.5, col::LOSE.gamma_multiply(pulse));
            p.text(tag.left_center() + vec2(28.0, 0.0), Align2::LEFT_CENTER, "LIVE WINS", theme::bold_font(12.0), col::TEXT);
            let line = Rect::from_min_max(pos2(tag.right() + 12.0, r.top()), pos2(r.right() - 14.0, r.bottom()));
            let clip = p.with_clip_rect(line);
            if c.big_wins.is_empty() {
                clip.text(line.left_center(), Align2::LEFT_CENTER, "Nobody's hit big yet. Be the first on the board.", theme::font(13.0), col::MUTED);
            } else {
                let items: Vec<String> = c.big_wins.iter().take(12).map(|b| format!("{} won {} on {} ({})", b.name, chips_text(b.paid), b.game.name(), b.what)).collect();
                // (A star after each, the last too: where the line comes round again looks like
                // anywhere else.)
                const SEP: &str = "      ★      ";
                let text = items.join(SEP) + SEP;
                let galley = clip.layout_no_wrap(text, theme::bold_font(13.5), col::GOLD2);
                let span = galley.size().x.max(1.0);
                // Where it's got to, worked out in full (a long-open Pyr's clock is too big for
                // an f32 to keep the fractions of a pixel), and as many times round as it takes
                // to fill the line. Its height on a whole pixel: only across does it move.
                let x = line.left() - ((time * 55.0) % span as f64) as f32;
                let ppp = ui.ctx().pixels_per_point();
                let y = ((line.center().y - galley.size().y / 2.0) * ppp).round() / ppp;
                let times = (line.width() / span).ceil() as usize + 1;
                for k in 0..times {
                    super::text_exact(&clip, pos2(x + k as f32 * span, y), &galley, col::GOLD2);
                }
                // Every frame (in step with the screen), not every so often.
                ui.ctx().request_repaint();
            }
        });
    }

    /// A chapter of games: its heading, then their tiles, as many across as fit (each 178 wide
    /// at the least, 14 apart, a portrait), 42 between the rows (the line under each in it).
    #[allow(clippy::too_many_arguments)]
    fn chapter(&mut self, ui: &mut egui::Ui, s: &ServerView, w: f32, c: Chapter, sub: &str, cards: &[Card], note: Option<&str>) {
        self.chapter_head(ui, w, c, sub);
        ui.add_space(16.0);
        let gap = 14.0;
        let cols = (((w + gap) / (178.0 + gap)).floor() as usize).clamp(2, 8);
        let cw = (w - gap * (cols - 1) as f32) / cols as f32;
        let ch = cw * 4.0 / 3.0;
        for row in cards.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(28.0);
                for (k, card) in row.iter().enumerate() {
                    if k > 0 {
                        ui.add_space(gap);
                    }
                    self.floor_card(ui, s, card, vec2(cw, ch));
                }
            });
            ui.add_space(14.0);
        }
        if let Some(n) = note {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.label(RichText::new(n).color(col::MUTED).size(12.5));
            });
            ui.add_space(8.0);
        }
    }

    fn floor_card(&mut self, ui: &mut egui::Ui, s: &ServerView, card: &Card, size: egui::Vec2) {
        let ctx = ui.ctx().clone();
        // (The line under it is the tile's too, as 0.13.28's: pointing at it lifts the tile, and
        // the tile's words show under it.)
        let (whole, resp) = ui.allocate_exact_size(size + vec2(0.0, 22.0), Sense::click());
        let r = Rect::from_min_size(whole.min, size);
        let hover = resp.hovered();
        // (It lifts as it's hovered; the line under it stays.)
        let lift = ctx.animate_bool_with_time(resp.id.with("lift"), hover, 0.14);
        let foot = r.bottom();
        let r = r.translate(vec2(0.0, -5.0 * lift));
        let p = ui.painter().clone();
        let round = theme::rf(16.0);
        p.rect_filled(r.translate(vec2(0.0, 8.0 + 4.0 * lift)), theme::r(18), Color32::from_black_alpha((110.0 + 50.0 * lift) as u8));
        if lift > 0.01 {
            radial(&p, r.center(), r.width() * 0.9, col::GOLD.gamma_multiply(0.10 * lift), Color32::TRANSPARENT);
        }
        let tint = Color32::from_gray((215.0 + 40.0 * lift) as u8);
        if !pics::cover_rounded(&p, &ctx, &card.art, r, round, (card.focus, 0.5), (r.width() * 2.0) as u32, tint) {
            p.rect_filled(r, theme::r(16), Color32::from_rgb(0x1e, 0x1a, 0x28));
        }
        // Dark toward the foot, for its name.
        foot_shade(&p, r, round);
        // Badges.
        let mut bx = r.left() + 10.0;
        for (b, color) in &card.badges {
            let g = p.layout_no_wrap(b.to_string(), theme::bold_font(10.5), Color32::WHITE);
            let br = Rect::from_min_size(pos2(bx, r.top() + 10.0), vec2(g.size().x + 14.0, 20.0));
            p.rect_filled(br, theme::r(10), *color);
            p.galley(br.center() - g.size() / 2.0, g, Color32::WHITE);
            bx = br.right() + 6.0;
        }
        // Its name, big in its own lettering: a fifth of the tile's height on one line, 13% on
        // two (smaller, to fit, where a line would run past 85% of its width), the last line's
        // middle 22 and a bit off the foot, 0.9 of the size between the lines.
        let (lines, family, studio) = lettering(&card.art);
        if !lines.is_empty() {
            let base = r.height() * if lines.len() > 1 { 0.13 } else { 0.2 };
            let widest = lines.iter().map(|l| p.layout_no_wrap(l.to_string(), theme::display_font(family, base), Color32::PLACEHOLDER).size().x).fold(0.0f32, f32::max);
            let size = if widest > r.width() * 0.85 { base * r.width() * 0.85 / widest } else { base };
            let font = theme::display_font(family, size);
            let last = r.bottom() - 22.25 - 0.47 * size;
            for (i, line) in lines.iter().enumerate() {
                let y = last - 0.9 * size * (lines.len() - 1 - i) as f32;
                p.text(pos2(r.center().x, y), Align2::CENTER_CENTER, *line, font.clone(), Color32::WHITE);
            }
        }
        // The studio, tiny and spaced out under it.
        let ss = r.height() * 0.026;
        let mut job = egui::text::LayoutJob::default();
        job.append(studio, 0.0, egui::TextFormat { font_id: theme::display_font("barlow", ss), color: Color32::from_gray(200), extra_letter_spacing: ss * 0.16, ..Default::default() });
        let g = p.layout_job(job);
        p.galley(pos2(r.center().x - g.size().x / 2.0, r.bottom() - 14.5 - g.size().y / 2.0), g, Color32::from_gray(200));
        // Hovered: play, and what it pays back.
        if lift > 0.01 {
            let c = r.center() - vec2(0.0, r.height() * 0.08);
            let rad = 26.0 * (0.7 + 0.3 * lift);
            p.circle_filled(c, rad + 6.0, Color32::from_black_alpha((90.0 * lift) as u8));
            p.circle_filled(c, rad, col::GOLD.gamma_multiply(lift));
            let k = rad * 0.42;
            p.add(Shape::convex_polygon(vec![c + vec2(-k * 0.7, -k), c + vec2(k * 1.1, 0.0), c + vec2(-k * 0.7, k)], Color32::from_rgb(0x2a, 0x10, 0x04).gamma_multiply(lift), Stroke::NONE));
            p.text(c + vec2(0.0, 44.0), Align2::CENTER_CENTER, &card.rtp, theme::bold_font(11.5), col::GOLD.gamma_multiply(lift));
        }
        p.rect_stroke(r, theme::r(16), Stroke::new(1.0 + lift, super::mix(Color32::from_white_alpha(28), col::GOLD, lift)), StrokeKind::Inside);
        // Under it: how many are playing it now (from the server), and my best win at it.
        let n = s.poker.casino.playing.get(&card.key).copied().unwrap_or(0);
        let y = foot + 13.5;
        p.circle_filled(pos2(r.left() + 6.0, y), 3.5, if n > 0 { SOMEONE } else { NOBODY });
        p.text(pos2(r.left() + 15.0, y), Align2::LEFT_CENTER, format!("{n} playing"), theme::bold_font(12.0), if n > 0 { Color32::WHITE } else { col::MUTED });
        if let Some(st) = s.poker.casino.stats.get(&card.key).filter(|st| st.best_win > 0) {
            p.text(pos2(r.right() - 4.0, y), Align2::RIGHT_CENTER, format!("best +{}", crate::poker::cards::chips_short(st.best_win)), theme::font(11.0), col::WIN);
        }
        if resp.on_hover_text(&card.pitch).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            match card.opens {
                Opens::Game(g) => self.poker.casino.game = Some(g),
                Opens::Machine(m) => {
                    self.reels_open(m);
                    self.row_open(m);
                }
            }
        }
    }
}

/// A tile darkened toward its foot, for its name: black from nothing 11% of the way down to
/// 80% at the foot, and over the last 13% another, to 52% (0.13.28's two, read off its tiles
/// against their pictures), in bands that follow the rounded corners.
fn foot_shade(p: &egui::Painter, r: Rect, radius: f32) {
    let rad = radius.min(r.width() / 2.0).min(r.height() / 2.0).max(0.0);
    let alpha = |y: f32| {
        let t = ((y - r.top()) / r.height()).clamp(0.0, 1.0);
        let a = 0.9 * (t - 0.11).max(0.0);
        let b = 4.0 * (t - 0.87).max(0.0);
        1.0 - (1.0 - a) * (1.0 - b)
    };
    let inset = |y: f32| {
        let d = y - (r.bottom() - rad);
        if d <= 0.0 { 0.0 } else { rad - (rad * rad - d * d).max(0.0).sqrt() }
    };
    let mut ys: Vec<f32> = [0.11, 0.87, 0.89, 0.91, 0.93].into_iter().map(|t| r.top() + r.height() * t).filter(|&y| y < r.bottom() - rad).collect();
    let n = (rad.ceil() as usize * 2).max(1);
    ys.extend((0..=n).map(|i| r.bottom() - rad + rad * i as f32 / n as f32));
    let mut m = egui::Mesh::default();
    for (k, &y) in ys.iter().enumerate() {
        let c = Color32::from_black_alpha((alpha(y) * 255.0).round() as u8);
        let i = inset(y);
        m.colored_vertex(pos2(r.left() + i, y), c);
        m.colored_vertex(pos2(r.right() - i, y), c);
        if k > 0 {
            let b = (2 * k) as u32;
            m.add_triangle(b - 2, b - 1, b);
            m.add_triangle(b - 1, b + 1, b);
        }
    }
    p.add(Shape::mesh(m));
}

/// A Reel Row machine's reach, as a badge: its top prize's size, or its progressive jackpot.
fn top_badge(machine: u8) -> (&'static str, Color32) {
    use pyr_poker::casino::reelrow::{TOPS, Top};
    let gold = Color32::from_rgb(0xb8, 0x7a, 0x10);
    match TOPS[machine as usize].top {
        Top::Progressive => ("PROGRESSIVE", col::EMBER),
        Top::Fixed(500_000) => ("500,000×", Color32::from_rgb(0x8a, 0x2a, 0xd0)),
        Top::Fixed(50_000) => ("50,000×", gold),
        Top::Fixed(25_000) => ("25,000×", gold),
        Top::Fixed(20_000) => ("20,000×", gold),
        Top::Fixed(10_000) => ("10,000×", gold),
        Top::Fixed(7_500) => ("7,500×", gold),
        Top::Fixed(_) => ("5,000×", gold),
    }
}

/// A Reel Row machine's top prize, in words (its progressive jackpot's pool as it stands).
pub(super) fn top_words(s: &ServerView, machine: u8) -> String {
    use pyr_poker::casino::reelrow::{TOPS, Top};
    let t = &TOPS[machine as usize];
    let odds = 1_000_000_000 / t.per_billion.max(1) as u64;
    match t.top {
        Top::Fixed(x) => format!("Its top prize, {}: {}× the bet, at the end of 1 bonus in {}.", t.name, crate::poker::cards::chips_text(x), crate::poker::cards::chips_text(odds)),
        Top::Progressive => {
            let pool = s.poker.casino.row_pools.get(&machine).copied().unwrap_or(pyr_poker::casino::reelrow::POOL_SEED);
            format!(
                "Its progressive jackpot, {}: {} chips now (all of it at the biggest bet, its share at smaller ones), at the end of 1 bonus in {}; every round here adds to it.",
                t.name,
                crate::poker::cards::chips_text(pool),
                crate::poker::cards::chips_text(odds)
            )
        }
    }
}
