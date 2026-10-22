//! Reel Row's twelve mini games on screen (0.13.0), one for each machine, over its own scene
//! (Meshy's `rr-<id>-bonus`) with its own pieces (`rr-<id>-pieces`): the Heat Ladder, the Abyss
//! Dive, the Cargo Run, the Candy Grab, the Mine Shaft, the Neon Drop, the Curse of the Tomb,
//! Ice Fishing, the Potion Brew, the Treasure Match, the Cup Shuffle and the Champagne Tower.
//!
//! Each is played a step at a time (a bite, a dive, a grab, a pick, a cast…), each step
//! showing the next thing the round's random numbers drew, wherever I click: what's under
//! what isn't decided by the click, so no choice changes the odds (see `reelrow`). Spinning by
//! itself, the steps take themselves.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::Outcome;
use pyr_poker::casino::reels::{Pick, share};
use pyr_poker::casino::reelrow::{self as rw, Cup, Find, Mini, Rung, Tier};
use pyr_poker::net::RoundView;

use super::reel_themes::Theme;
use super::row_styles::Style;
use super::{col, ease_back, ease_out, glow_text, mix, pics, radial, vgrad_round};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// A mini game as it's played on screen.
pub struct MiniPlay {
    pub round: RoundView,
    /// When it starts (the reels with the scatters still show till then).
    pub started: Instant,
    /// Steps taken so far, and when the last was.
    pub step: usize,
    pub step_at: Option<Instant>,
    /// What I clicked at each step (a tile, a clam, a cup…), to show it there.
    pub picked: Vec<usize>,
    /// When it was over on screen, and whether its win has gone on the balance.
    pub over_at: Option<Instant>,
    pub collected: bool,
}

impl MiniPlay {
    pub fn new(round: RoundView, started: Instant) -> MiniPlay {
        MiniPlay { round, started, step: 0, step_at: None, picked: Vec::new(), over_at: None, collected: false }
    }
}

/// How many steps a game takes, all told.
fn steps(m: &Mini) -> usize {
    match m {
        Mini::Ladder { rungs, .. } => rungs.len(),
        Mini::Dive { depths, .. } => depths.len(),
        Mini::Tomb { chambers, .. } => chambers.len(),
        Mini::Cargo { .. } => 1,
        Mini::Claw { grabs } => grabs.len(),
        Mini::Mine { picks, .. } => picks.len(),
        Mini::Drop { balls } => balls.len(),
        Mini::Fishing { casts } => casts.len(),
        Mini::Potion { jars } => jars.len(),
        Mini::Match { opened, .. } => opened.len(),
        Mini::Cups { rounds, .. } => rounds.len(),
        Mini::Tower { tiers, .. } => tiers.len(),
    }
}

/// How long a step's show takes (seconds): the next waits for it.
fn step_secs(m: &Mini, turbo: bool) -> f32 {
    let k = if turbo { 0.55 } else { 1.0 };
    k * match m {
        Mini::Cargo { x100 } => cargo_secs(*x100),
        Mini::Drop { .. } => 1.9,
        Mini::Claw { .. } => 1.8,
        Mini::Cups { .. } => 0.9,
        Mini::Fishing { .. } => 1.2,
        Mini::Tower { .. } => 1.0,
        Mini::Ladder { .. } => 0.8,
        _ => 0.7,
    }
}

/// The Cargo Run's flight: longer the further it goes.
fn cargo_secs(x100: u64) -> f32 {
    2.0 + ((x100 as f32 / 100.0).ln()).max(0.0) * 0.9
}

/// The multiplier on the Cargo Run's dial `t` seconds into the flight (it climbs faster and
/// faster, reaching the cargo's as the rocket blows).
fn cargo_at(x100: u64, t: f32) -> f32 {
    let end = x100 as f32 / 100.0;
    let k = (t / cargo_secs(x100)).clamp(0.0, 1.0);
    (end.ln() * k * k).exp()
}

/// What's been won so far, `n` steps in (chips; before a Super's tripling).
fn won_after(m: &Mini, bet: u64, n: usize, total: u64) -> u64 {
    let done = n >= steps(m);
    let prize = |f: &Find| if let Find::Prize { chips, .. } = f { *chips } else { 0 };
    match m {
        Mini::Ladder { rungs, top } => rungs.iter().take(n).map(|r| if let Rung::Cleared { chips, .. } = r { *chips } else { 0 }).sum::<u64>() + if done && *top { share(bet, rw::PEPPER_X, 100) } else { 0 },
        Mini::Dive { depths, chest } => depths.iter().take(n).map(|t| prize(&t.found)).sum::<u64>() + if done && *chest { share(bet, rw::CHEST, 100) } else { 0 },
        Mini::Tomb { chambers, mask } => chambers.iter().take(n).map(|t| prize(&t.found)).sum::<u64>() + if done && *mask { share(bet, rw::MASK, 100) } else { 0 },
        Mini::Claw { grabs } | Mini::Fishing { casts: grabs } => grabs.iter().take(n).map(prize).sum(),
        Mini::Mine { picks, finale } => picks.iter().take(n).map(|p| match p { Pick::Chips { chips } | Pick::Gold { chips } => *chips, _ => 0 }).sum::<u64>() + if done && *finale { share(bet, rw::MINE_FINALE, 100) } else { 0 },
        Mini::Drop { balls } => balls.iter().take(n).map(|b| b.chips).sum(),
        Mini::Potion { jars } => {
            if done {
                total
            } else {
                jars.iter().take(n).map(|j| j.chips).sum()
            }
        }
        Mini::Cups { rounds, finale } => rounds.iter().take(n).map(|r| if let Cup::Found { chips } = r { *chips } else { 0 }).sum::<u64>() + if done && *finale { share(bet, rw::CUP_FINALE, 100) } else { 0 },
        Mini::Tower { tiers, .. } => {
            let v = tiers.iter().take(n).filter_map(|t| if let Tier::Poured { tower, .. } = t { Some(*tower) } else { None }).last().unwrap_or(rw::TOWER_BASE);
            if done { total } else { share(bet, v, 100) }
        }
        Mini::Cargo { .. } | Mini::Match { .. } => {
            if done {
                total
            } else {
                0
            }
        }
    }
}

/// A piece from the machine's sheet.
fn piece(p: &egui::Painter, ctx: &egui::Context, th: &Theme, i: usize, r: Rect, tint: Color32) {
    pics::cell(p, ctx, &format!("rr-{}-pieces", th.id), i, r, (r.width() * 3.0) as u32 + 1, tint);
}

/// A big button under the game; true when it's clicked (or, spinning by itself, when it's
/// time).
fn big_button(ui: &mut egui::Ui, r: Rect, text: &str, font: egui::FontId, accent: Color32, enabled: bool) -> bool {
    let p = ui.painter();
    let resp = ui.interact(r, ui.id().with(("mini-btn", text)), Sense::click());
    let hover = resp.hovered() && enabled;
    p.rect_filled(r.translate(vec2(0.0, 4.0)), theme::r((r.height() / 2.0) as u8), Color32::from_black_alpha(140));
    vgrad_round(p, r, r.height() / 2.0, if !enabled { Color32::from_white_alpha(20) } else if hover { mix(accent, Color32::WHITE, 0.3) } else { accent }, if enabled { mix(accent, Color32::BLACK, 0.35) } else { Color32::from_white_alpha(10) });
    p.rect_stroke(r, theme::r((r.height() / 2.0) as u8), Stroke::new(2.0f32, if enabled { col::GOLD2 } else { Color32::from_white_alpha(30) }), StrokeKind::Inside);
    p.text(r.center(), Align2::CENTER_CENTER, text, font, if enabled { Color32::from_rgb(0x1a, 0x08, 0x04) } else { Color32::from_white_alpha(60) });
    enabled && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

/// A label over a dark pill.
fn tag(p: &egui::Painter, at: Pos2, text: &str, size: f32, color: Color32) {
    let g = p.layout_no_wrap(text.to_string(), theme::bold_font(size), color);
    let r = Rect::from_center_size(at, g.size() + vec2(14.0, 6.0));
    p.rect_filled(r, theme::r((r.height() / 2.0) as u8), Color32::from_black_alpha(190));
    p.galley(r.center() - g.size() / 2.0, g, color);
}

impl PyrApp {
    /// The machine's mini game over `r`: its scene, the game a step at a time, and Collect.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn row_mini(&mut self, ui: &mut egui::Ui, s: &ServerView, th: &Theme, st: &Style, r: Rect, now: Instant, time: f32) {
        let Some(mp) = self.poker.casino.row.mini.as_ref() else { return };
        let Outcome::Row(o) = mp.round.outcome.clone() else { return };
        let Some(bonus) = o.bonus.clone() else { return };
        let (step, step_at) = (mp.step, mp.step_at);
        let picked = mp.picked.clone();
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let turbo = self.poker.casino.row.turbo;
        let total_steps = steps(&bonus.game);
        let secs = step_secs(&bonus.game, turbo);
        // Where the last step's show has got to (0 to 1), and whether the next can be taken.
        let k = step_at.map(|t| (now.duration_since(t).as_secs_f32() / secs).min(1.0)).unwrap_or(1.0);
        let ready = k >= 1.0 && step < total_steps;
        let done = step >= total_steps && k >= 1.0;
        // The scene.
        p.rect_filled(r, theme::r(18), Color32::BLACK);
        let clip = p.with_clip_rect(r);
        pics::cover(&clip, &ctx, &format!("rr-{}-bonus", th.id), r, 1376, Color32::from_gray(95));
        super::vgrad(&clip, Rect::from_min_max(r.min, pos2(r.right(), r.top() + 110.0)), Color32::from_black_alpha(200), Color32::TRANSPARENT);
        super::vgrad(&clip, Rect::from_min_max(pos2(r.left(), r.bottom() - 120.0), r.max), Color32::TRANSPARENT, Color32::from_black_alpha(210));
        p.rect_stroke(r, theme::r(18), Stroke::new(2.0f32, th.accent.gamma_multiply(0.8)), StrokeKind::Inside);
        // Its name, what it's about, what's won so far.
        let (name, what) = rw::GAMES[(o.machine as usize).min(11)];
        let sup = bonus.times > 1;
        let title = if sup { format!("SUPER {}", name.to_uppercase()) } else { name.to_uppercase() };
        glow_text(&p, pos2(r.center().x, r.top() + 32.0), Align2::CENTER_CENTER, &title, theme::display_font(st.font, 34.0), col::GOLD2, th.accent.gamma_multiply(0.6));
        let g = p.layout(what.to_string(), theme::font(13.0), Color32::from_white_alpha(210), (r.width() - 80.0).min(760.0));
        p.galley(pos2(r.center().x - g.size().x / 2.0, r.top() + 56.0), g, Color32::WHITE);
        let so_far = won_after(&bonus.game, o.bet, step, bonus.won);
        let shown = if step_at.is_some() && k < 1.0 { won_after(&bonus.game, o.bet, step.saturating_sub(1), bonus.won) } else { so_far };
        let won_text = if sup { format!("WON {}  ×3 = {}", chips_text(shown), chips_text(shown * bonus.times)) } else { format!("WON {}", chips_text(shown)) };
        tag(&p, pos2(r.center().x, r.top() + 98.0), &won_text, 17.0, col::GOLD2);
        let board = Rect::from_min_max(pos2(r.left() + 30.0, r.top() + 120.0), pos2(r.right() - 30.0, r.bottom() - 74.0));
        let button = Rect::from_center_size(pos2(r.center().x, r.bottom() - 38.0), vec2(280.0, 50.0));
        let font = theme::display_font(st.font, 22.0);
        let auto = self.poker.casino.row.auto.on();
        let auto_go = auto && ready && step_at.is_none_or(|t| now.duration_since(t).as_secs_f32() > secs + 0.45) && now.duration_since(self.poker.casino.row.mini.as_ref().unwrap().started) > Duration::from_millis(900);
        // The game: what it draws, and the step a click takes (the thing clicked).
        let mut take: Option<usize> = None;
        let ctxs = Ctx { p: &p, ctx: &ctx, th, st, board, step, k, picked: &picked, time, bet: o.bet };
        match &bonus.game {
            Mini::Ladder { rungs, top } => {
                ladder(&ctxs, rungs, *top);
                if !done && big_button(ui, button, "TAKE A BITE", font.clone(), th.accent, ready) {
                    take = Some(0);
                }
            }
            Mini::Dive { depths, chest } => {
                take = trios(ui, &ctxs, depths, *chest, ready, true);
            }
            Mini::Tomb { chambers, mask } => {
                take = trios(ui, &ctxs, chambers, *mask, ready, false);
            }
            Mini::Cargo { x100 } => {
                cargo(&ctxs, *x100, step_at.map(|t| now.duration_since(t).as_secs_f32()));
                if step == 0 && big_button(ui, button, "LAUNCH", font.clone(), th.accent, ready) {
                    take = Some(0);
                }
            }
            Mini::Claw { grabs } => {
                claw(&ctxs, grabs);
                if !done && big_button(ui, button, "GRAB", font.clone(), th.accent, ready) {
                    take = Some(0);
                }
            }
            Mini::Mine { picks, finale } => {
                take = mine(ui, &ctxs, picks, *finale, ready);
            }
            Mini::Drop { balls } => {
                drop_board(&ctxs, balls);
                if !done && big_button(ui, button, &format!("DROP BALL {}", step + 1), font.clone(), th.accent, ready) {
                    take = Some(0);
                }
            }
            Mini::Fishing { casts } => {
                fishing(&ctxs, casts);
                if !done && big_button(ui, button, "CAST", font.clone(), th.accent, ready) {
                    take = Some(0);
                }
            }
            Mini::Potion { jars } => {
                take = potion(ui, &ctxs, jars, ready);
            }
            Mini::Match { opened, treasure, egg } => {
                take = treasure_match(ui, &ctxs, opened, *treasure, *egg, ready);
            }
            Mini::Cups { rounds, finale } => {
                take = cups(ui, &ctxs, rounds, *finale, ready);
            }
            Mini::Tower { tiers, tower } => {
                champagne(&ctxs, tiers, *tower);
                if !done && big_button(ui, button, "POUR", font.clone(), th.accent, ready) {
                    take = Some(0);
                }
            }
        }
        // Spinning by itself: the steps take themselves.
        if take.is_none() && auto_go {
            take = Some(auto_pick(&bonus.game, step, &picked));
        }
        if let Some(i) = take
            && ready
        {
            let mp = self.poker.casino.row.mini.as_mut().unwrap();
            mp.step += 1;
            mp.step_at = Some(now);
            mp.picked.push(i);
            let cue = step_cue(&bonus.game, step);
            self.poker.casino.sound_in(0.0, Cue::Flip, 0.0, 0.5);
            self.poker.casino.sound_in(secs * 0.6, cue, 0.0, 0.7);
        }
        // Over: what it all came to, and Collect.
        if done {
            let mp = self.poker.casino.row.mini.as_mut().unwrap();
            let over = *mp.over_at.get_or_insert(now);
            let paid = bonus.paid();
            if now.duration_since(over) < Duration::from_millis(30) {
                self.poker.casino.sound_in(0.0, if paid >= o.bet * 20 { Cue::WinBig } else { Cue::WinSmall }, 0.0, 0.8);
                self.row_burst(paid, o.bet, now);
                self.poker.casino.row.win = o.paid;
            }
            let a = (now.duration_since(over).as_secs_f32() / 0.4).min(1.0);
            let pr = Rect::from_center_size(pos2(r.center().x, r.bottom() - 100.0), vec2(360.0, 56.0) * (0.7 + 0.3 * ease_back(a)));
            p.rect_filled(pr, theme::r(20), Color32::from_black_alpha(220));
            p.rect_stroke(pr, theme::r(20), Stroke::new(2.0f32, th.accent), StrokeKind::Inside);
            glow_text(&p, pr.center(), Align2::CENTER_CENTER, &format!("{} {}", if sup { "SUPER TOTAL" } else { "TOTAL" }, chips_text(paid)), theme::display_font(st.font, 26.0), col::GOLD2, th.accent.gamma_multiply(0.5));
            let collect = big_button(ui, button, &format!("COLLECT {}", chips_text(paid)), font, col::GOLD, true);
            if collect || (auto && now.duration_since(over) > Duration::from_millis(1_800)) {
                let round = self.poker.casino.row.mini.as_ref().unwrap().round.clone();
                let ui_s = &mut self.poker.casino.row;
                ui_s.mini = None;
                ui_s.win = o.paid;
                self.row_finish(s, &round, now);
            }
        }
    }
}

/// What a step makes heard as its show lands.
fn step_cue(m: &Mini, step: usize) -> Cue {
    let bad = match m {
        Mini::Ladder { rungs, .. } => matches!(rungs.get(step), Some(Rung::Hot { saved: false })),
        Mini::Dive { depths: t, .. } | Mini::Tomb { chambers: t, .. } => matches!(t.get(step).map(|x| x.found), Some(Find::Danger { saved: false })),
        Mini::Fishing { casts } => matches!(casts.get(step), Some(Find::Danger { saved: false })),
        Mini::Mine { picks, .. } => matches!(picks.get(step), Some(Pick::Busted)),
        Mini::Cups { rounds, .. } => matches!(rounds.get(step), Some(Cup::Missed { saved: false, .. })),
        Mini::Tower { tiers, .. } => matches!(tiers.get(step), Some(Tier::Toppled { saved: false })),
        Mini::Cargo { .. } => return Cue::Boom,
        _ => false,
    };
    if bad { Cue::Fold } else { Cue::Coin }
}

/// The thing a step takes by itself, spinning by itself (any not taken yet).
fn auto_pick(m: &Mini, step: usize, picked: &[usize]) -> usize {
    match m {
        Mini::Mine { .. } => (0..15).find(|i| !picked.contains(i)).unwrap_or(0),
        Mini::Match { .. } => (0..13).find(|i| !picked.contains(i)).unwrap_or(0),
        Mini::Potion { .. } => [4, 1, 7][step.min(2)],
        _ => 1,
    }
}

/// What every game's drawing has to hand.
struct Ctx<'a> {
    p: &'a egui::Painter,
    ctx: &'a egui::Context,
    th: &'a Theme,
    st: &'a Style,
    board: Rect,
    /// Steps taken, and how far the last one's show has got (0 to 1).
    step: usize,
    k: f32,
    picked: &'a [usize],
    time: f32,
    bet: u64,
}

impl Ctx<'_> {
    /// Whether step `i`'s outcome shows yet (taken, and its show far enough along).
    fn shows(&self, i: usize, at: f32) -> bool {
        i + 1 < self.step || (i + 1 == self.step && self.k >= at)
    }
}

/// Heat Ladder: eight peppers up a ladder, the Pepper X on top, the heat rising.
fn ladder(c: &Ctx, rungs: &[Rung], top: bool) {
    let n = rw::RUNG.len();
    let b = c.board;
    let h = b.height() / (n as f32 + 1.4);
    let w = (h * 3.6).min(b.width() * 0.5);
    let x0 = b.center().x + b.width() * 0.12;
    // The rails.
    for dx in [-w / 2.0 - 8.0, w / 2.0 + 8.0] {
        c.p.line_segment([pos2(x0 + dx, b.bottom()), pos2(x0 + dx, b.top() + h * 0.4)], Stroke::new(6.0f32, Color32::from_rgb(0x6a, 0x3a, 0x1a)));
    }
    // Which pepper each rung is: hotter up the scale.
    const PEPPERS: [usize; 8] = [0, 0, 2, 2, 1, 1, 3, 3];
    for i in 0..n {
        let y = b.bottom() - (i as f32 + 0.6) * h;
        let rr = Rect::from_center_size(pos2(x0, y), vec2(w, h * 0.82));
        let shown = c.shows(i, 0.5);
        let here = i == c.step && c.step < rungs.len();
        let (fill, edge) = match (shown, rungs.get(i)) {
            (true, Some(Rung::Cleared { .. })) => (Color32::from_rgb(0x2a, 0x40, 0x14), col::WIN),
            (true, Some(Rung::Hot { saved: true })) => (Color32::from_rgb(0x4a, 0x30, 0x10), col::GOLD),
            (true, Some(Rung::Hot { saved: false })) => (Color32::from_rgb(0x6a, 0x0a, 0x06), col::LOSE),
            _ if here => (Color32::from_rgb(0x4a, 0x1a, 0x08), c.th.accent),
            _ => (Color32::from_black_alpha(170), Color32::from_white_alpha(40)),
        };
        c.p.rect_filled(rr, theme::r(10), fill);
        c.p.rect_stroke(rr, theme::r(10), Stroke::new(if here { 2.5f32 } else { 1.0 }, edge), StrokeKind::Inside);
        let pr = Rect::from_center_size(pos2(rr.left() + h * 0.5, rr.center().y), vec2(h * 0.8, h * 0.8));
        let wobble = if here { (c.time * 5.0).sin() * 0.06 } else { 0.0 };
        piece(c.p, c.ctx, c.th, PEPPERS[i], pr.translate(vec2(0.0, wobble * h)), if shown || here { Color32::WHITE } else { Color32::from_gray(110) });
        let prize = share(c.bet, rw::RUNG[i], 100);
        let label = match (shown, rungs.get(i)) {
            (true, Some(Rung::Cleared { chips, milk })) => format!("+{}{}", chips_text(*chips), if *milk { "  + milk" } else { "" }),
            (true, Some(Rung::Hot { saved: true })) => "TOO HOT! drank the milk".into(),
            (true, Some(Rung::Hot { saved: false })) => "TOO HOT!".into(),
            _ => format!("{}  ·  {}% too hot", chips_text(prize), rw::TOO_HOT[i]),
        };
        c.p.text(pos2(pr.right() + 10.0, rr.center().y), Align2::LEFT_CENTER, label, theme::bold_font((h * 0.26).clamp(11.0, 16.0)), if shown { edge } else { col::TEXT });
        if let (true, Some(Rung::Cleared { milk: true, .. })) = (shown, rungs.get(i)) {
            piece(c.p, c.ctx, c.th, 5, Rect::from_center_size(pos2(rr.right() - h * 0.45, rr.center().y), vec2(h * 0.7, h * 0.7)), Color32::WHITE);
        }
        // A burst of heat as a rung's too hot.
        if shown && i + 1 == c.step && matches!(rungs.get(i), Some(Rung::Hot { .. })) && c.k < 1.0 {
            radial(c.p, rr.center(), w * (0.4 + c.k), Color32::from_rgb(0xff, 0x40, 0x10).gamma_multiply(1.0 - c.k), Color32::TRANSPARENT);
        }
    }
    // The Pepper X on top.
    let px = pos2(x0, b.top() + h * 0.2);
    let reached = c.step >= rungs.len() && top && c.k >= 0.5;
    if reached {
        radial(c.p, px, h * 1.6, Color32::from_rgb(0xff, 0x6a, 0x10).gamma_multiply(0.5 + 0.3 * (c.time * 4.0).sin()), Color32::TRANSPARENT);
    }
    piece(c.p, c.ctx, c.th, 4, Rect::from_center_size(px, vec2(h * 1.2, h * 1.2)), if reached { Color32::WHITE } else { Color32::from_gray(90) });
    c.p.text(px + vec2(h * 0.8, 0.0), Align2::LEFT_CENTER, format!("PEPPER X  {}", chips_text(share(c.bet, rw::PEPPER_X, 100))), theme::display_font(c.st.font, (h * 0.34).clamp(14.0, 24.0)), if reached { col::GOLD2 } else { col::MUTED });
    // The heat: a thermometer up the left, and the milk in hand.
    let lives = rungs.iter().take(c.step).fold(rw::START_EXTRA as i32, |l, r| match r {
        Rung::Cleared { milk: true, .. } => l + 1,
        Rung::Hot { saved: true } => l - 1,
        _ => l,
    });
    let th_r = Rect::from_min_max(pos2(b.left() + b.width() * 0.12, b.top() + 10.0), pos2(b.left() + b.width() * 0.12 + 26.0, b.bottom() - 10.0));
    c.p.rect_filled(th_r, theme::r(13), Color32::from_black_alpha(170));
    let heat = (c.step as f32 / n as f32).min(1.0);
    let fillr = Rect::from_min_max(pos2(th_r.left() + 5.0, th_r.bottom() - 5.0 - (th_r.height() - 10.0) * heat), th_r.max - vec2(5.0, 5.0));
    c.p.rect_filled(fillr, theme::r(8), Color32::from_rgb(0xff, 0x40, 0x18));
    c.p.text(pos2(th_r.center().x, th_r.bottom() + 14.0), Align2::CENTER_CENTER, "HEAT", theme::bold_font(11.0), col::MUTED);
    for i in 0..lives.max(0) {
        piece(c.p, c.ctx, c.th, 5, Rect::from_center_size(pos2(th_r.right() + 30.0, th_r.bottom() - 30.0 - i as f32 * 46.0), vec2(42.0, 42.0)), Color32::WHITE);
    }
}

/// Abyss Dive and the Curse of the Tomb: rows of three to open, deeper and richer; a second
/// chance in some, a danger in others; the chest (or Ra's mask) past the last. The clicked one
/// opens (and shows what the step drew); the other two show what they held, a moment after.
fn trios(ui: &mut egui::Ui, c: &Ctx, rows: &[rw::Trio], last: bool, ready: bool, sea: bool) -> Option<usize> {
    let layout: &[(u8, u8, u8)] = if sea { &rw::DEPTHS } else { &rw::CHAMBERS };
    let prizes: &[u64] = if sea { &rw::PEARL } else { &rw::TREASURE };
    let n = layout.len();
    let b = c.board;
    let rh = b.height() / (n as f32 + 1.0);
    let cw = (rh * 1.25).min(b.width() / 5.0);
    let mut take = None;
    // Pieces: deep sea (pearl, black pearl, closed clam, open clam, shark, air tank, helmet,
    // chest, anchor); tomb (mummy, scarab, eye, ankh, mask, collar, canopic jar, coins, cobra).
    let prize_piece = |i: usize| if sea { if i < 3 { 0 } else { 1 } } else { [1, 2, 5, 7][i.min(3)] };
    let (extra_piece, danger_piece) = if sea { (5, 4) } else { (3, 0) };
    let current = c.step.min(n);
    for (i, &(np, ne, nd)) in layout.iter().enumerate() {
        let y = b.top() + (i as f32 + 0.5) * rh;
        let row = Rect::from_center_size(pos2(b.center().x, y), vec2(cw * 3.0 + 60.0 + 220.0, rh * 0.92));
        let here = i == c.step && i < rows.len() && ready;
        c.p.rect_filled(row, theme::r(14), Color32::from_black_alpha(if here { 150 } else { 110 }));
        if here {
            c.p.rect_stroke(row, theme::r(14), Stroke::new(2.0f32, c.th.accent), StrokeKind::Inside);
        }
        let label = format!("{} {}  ·  {}", if sea { "DEPTH" } else { "CHAMBER" }, i + 1, chips_text(share(c.bet, prizes[i], 100)));
        c.p.text(pos2(row.left() + 14.0, y - 9.0), Align2::LEFT_CENTER, label, theme::bold_font(13.0), col::TEXT);
        let odds = format!("{} {} · {} {} · {} {}", np, if sea { "pearl" } else { "treasure" }, ne, if sea { "tank" } else { "amulet" }, nd, if sea { "shark" } else { "mummy" });
        c.p.text(pos2(row.left() + 14.0, y + 10.0), Align2::LEFT_CENTER, odds, theme::font(11.0), col::MUTED);
        for j in 0..3 {
            let cr = Rect::from_center_size(pos2(row.left() + 220.0 + (j as f32 + 0.5) * (cw + 20.0), y), vec2(cw, rh * 0.86));
            let taken = c.picked.get(i).copied();
            let opened = i < c.step;
            let resp = ui.interact(cr, ui.id().with(("trio", i, j)), Sense::click());
            let hover = here && resp.hovered();
            if !opened {
                // Closed: a clam, or a sarcophagus.
                let g = if hover { 1.0 + 0.05 * (c.time * 8.0).sin() } else { 1.0 };
                let r2 = Rect::from_center_size(cr.center(), cr.size() * g);
                if sea {
                    piece(c.p, c.ctx, c.th, 2, r2, if here { Color32::WHITE } else { Color32::from_gray(120) });
                } else {
                    sarcophagus(c.p, r2, if here { col::GOLD } else { Color32::from_rgb(0x8a, 0x6a, 0x2a) });
                }
                if here && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    take = Some(j);
                }
                continue;
            }
            let t = &rows[i];
            let is_mine = taken == Some(j);
            let at = if is_mine { 0.35 } else { 0.85 };
            if !c.shows(i, at) {
                if sea { piece(c.p, c.ctx, c.th, 2, cr, Color32::WHITE) } else { sarcophagus(c.p, cr, col::GOLD) }
                continue;
            }
            let kind = if is_mine {
                match t.found {
                    Find::Prize { .. } => 0,
                    Find::Extra => 1,
                    Find::Danger { .. } => 2,
                }
            } else {
                let others: Vec<usize> = (0..3).filter(|x| Some(*x) != taken).collect();
                let k = others.iter().position(|x| *x == j).unwrap_or(0);
                t.others.get(k).copied().unwrap_or(0) as usize
            };
            let idx = match kind {
                0 => prize_piece(i),
                1 => extra_piece,
                _ => danger_piece,
            };
            let tint = if is_mine { Color32::WHITE } else { Color32::from_white_alpha(110) };
            if is_mine {
                let glow = match t.found {
                    Find::Danger { saved: false } => col::LOSE,
                    Find::Danger { saved: true } => col::GOLD,
                    _ => c.th.accent,
                };
                radial(c.p, cr.center(), cw * 0.75, glow.gamma_multiply(0.45), Color32::TRANSPARENT);
            }
            if sea && kind == 0 {
                piece(c.p, c.ctx, c.th, 3, cr, tint);
            }
            let sz = if sea && kind == 0 { 0.5 } else { 0.85 };
            piece(c.p, c.ctx, c.th, idx, Rect::from_center_size(cr.center() - vec2(0.0, if sea && kind == 0 { cr.height() * 0.06 } else { 0.0 }), cr.size() * sz), tint);
            if is_mine {
                let text = match t.found {
                    Find::Prize { chips, .. } => format!("+{}", chips_text(chips)),
                    Find::Extra => if sea { "AIR TANK".into() } else { "AMULET".into() },
                    Find::Danger { saved: true } => "SAVED".into(),
                    Find::Danger { saved: false } => if sea { "SHARK!".into() } else { "CURSED!".into() },
                };
                tag(c.p, pos2(cr.center().x, cr.bottom() - 6.0), &text, 12.0, match t.found {
                    Find::Danger { saved: false } => col::LOSE,
                    _ => col::GOLD2,
                });
            }
        }
    }
    // The prize past the last row.
    let y = b.bottom() - rh * 0.4;
    let reached = c.step >= rows.len() && last && c.k >= 0.6;
    let lr = Rect::from_center_size(pos2(b.center().x, y), vec2(rh * 1.0, rh * 0.9));
    if reached {
        radial(c.p, lr.center(), rh * 1.4, col::GOLD.gamma_multiply(0.5), Color32::TRANSPARENT);
    }
    piece(c.p, c.ctx, c.th, if sea { 7 } else { 4 }, lr, if reached { Color32::WHITE } else { Color32::from_gray(80) });
    c.p.text(pos2(lr.right() + 12.0, y), Align2::LEFT_CENTER, format!("{}  {}", if sea { "THE CHEST" } else { "RA'S MASK" }, chips_text(share(c.bet, if sea { rw::CHEST } else { rw::MASK }, 100))), theme::display_font(c.st.font, 20.0), if reached { col::GOLD2 } else { col::MUTED });
    // Second chances in hand.
    let held = rows.iter().take(c.step).fold(rw::START_EXTRA as i32, |h, t| match t.found {
        Find::Extra => h + 1,
        Find::Danger { saved: true } => h - 1,
        _ => h,
    });
    for i in 0..held.max(0) {
        piece(c.p, c.ctx, c.th, extra_piece, Rect::from_center_size(pos2(b.right() - 30.0 - i as f32 * 50.0, b.top() + 24.0), vec2(44.0, 44.0)), Color32::WHITE);
    }
    let _ = current;
    take
}

/// A sarcophagus, closed: gold, tapering, a face at the top.
fn sarcophagus(p: &egui::Painter, r: Rect, gold: Color32) {
    let c = r.center();
    let (w, h) = (r.width() * 0.36, r.height() * 0.46);
    let pts = vec![c + vec2(-w * 0.7, -h), c + vec2(w * 0.7, -h), c + vec2(w, -h * 0.4), c + vec2(w * 0.6, h), c + vec2(-w * 0.6, h), c + vec2(-w, -h * 0.4)];
    p.add(Shape::convex_polygon(pts, mix(gold, Color32::BLACK, 0.25), Stroke::new(2.0f32, gold)));
    p.circle_filled(c + vec2(0.0, -h * 0.55), w * 0.42, mix(gold, Color32::WHITE, 0.15));
    p.line_segment([c + vec2(-w * 0.4, -h * 0.1), c + vec2(w * 0.4, -h * 0.1)], Stroke::new(2.0f32, Color32::from_rgb(0x2a, 0x6a, 0xa0)));
    p.line_segment([c + vec2(-w * 0.35, h * 0.25), c + vec2(w * 0.35, h * 0.25)], Stroke::new(2.0f32, Color32::from_rgb(0x2a, 0x6a, 0xa0)));
}

/// Cargo Run: the rocket climbs, its cargo's multiple counting up, till it blows.
fn cargo(c: &Ctx, x100: u64, since: Option<f32>) {
    let b = c.board;
    let dial = Rect::from_center_size(pos2(b.center().x, b.top() + b.height() * 0.3), vec2(320.0, 110.0));
    let t = since.unwrap_or(0.0);
    let secs = cargo_secs(x100);
    let flying = since.is_some() && t < secs;
    let blown = since.is_some() && t >= secs;
    let m = if since.is_some() { cargo_at(x100, t) } else { 1.0 };
    // The flight path: a curve up and across, the rocket along it.
    let k = (t / secs).clamp(0.0, 1.0);
    let path = |u: f32| pos2(b.left() + 60.0 + u * (b.width() - 140.0), b.bottom() - 30.0 - (u.powf(1.6)) * (b.height() - 80.0));
    let pts: Vec<Pos2> = (0..=40).map(|i| path(i as f32 / 40.0 * k)).collect();
    if pts.len() > 1 {
        c.p.add(Shape::line(pts, Stroke::new(3.0f32, c.st.second.gamma_multiply(0.6))));
    }
    let at = path(k);
    if !blown {
        let ang = {
            let a = path((k + 0.01).min(1.0));
            (a - at).angle()
        };
        let rr = Rect::from_center_size(at, vec2(96.0, 96.0));
        // (The rocket's art points up and right: turned along its path.)
        let _ = ang;
        radial(c.p, at - vec2(30.0, -24.0), 40.0, Color32::from_rgb(0xff, 0xa0, 0x40).gamma_multiply(if flying { 0.6 } else { 0.0 }), Color32::TRANSPARENT);
        piece(c.p, c.ctx, c.th, 0, rr, Color32::WHITE);
        piece(c.p, c.ctx, c.th, 2, Rect::from_center_size(at + vec2(-46.0, 26.0), vec2(48.0, 48.0)), Color32::WHITE);
    } else {
        let u = ((t - secs) / 0.6).min(1.0);
        radial(c.p, at, 40.0 + 140.0 * u, Color32::from_rgb(0xff, 0xc0, 0x40).gamma_multiply(1.0 - u * 0.7), Color32::TRANSPARENT);
        radial(c.p, at, 20.0 + 60.0 * u, Color32::WHITE.gamma_multiply(1.0 - u), Color32::TRANSPARENT);
        piece(c.p, c.ctx, c.th, 1, Rect::from_center_size(at + vec2(0.0, 30.0 * u), vec2(70.0, 70.0)), Color32::WHITE);
    }
    // The multiple, big.
    c.p.rect_filled(dial, theme::r(18), Color32::from_black_alpha(190));
    c.p.rect_stroke(dial, theme::r(18), Stroke::new(2.0f32, c.st.second), StrokeKind::Inside);
    let color = if blown { col::GOLD2 } else { Color32::WHITE };
    glow_text(c.p, dial.center() - vec2(0.0, 10.0), Align2::CENTER_CENTER, &format!("{m:.2}×"), theme::display_font(c.st.font, 46.0), color, c.st.second.gamma_multiply(0.6));
    let sub = if blown { format!("CARGO DELIVERED: {}", chips_text(share(c.bet, x100, 100))) } else if flying { format!("{} and climbing", chips_text((m * c.bet as f32) as u64)) } else { format!("{:.2}× to {}×", rw::CARGO_A as f32 / 100.0, rw::CARGO_CAP / 100) };
    c.p.text(dial.center() + vec2(0.0, 34.0), Align2::CENTER_CENTER, sub, theme::bold_font(13.0), col::MUTED);
}

/// Candy Grab: the claw drops, closes, and comes up with what it caught.
fn claw(c: &Ctx, grabs: &[Find]) {
    let b = c.board;
    let left = rw::GRABS as i32 + grabs.iter().take(c.step).filter(|g| matches!(g, Find::Extra)).count() as i32 - grabs.iter().take(c.step).filter(|g| matches!(g, Find::Prize { .. })).count() as i32;
    let col_x = b.center().x;
    // The claw: across, down, closed, up again (the last step's show).
    let anim = c.step > 0 && c.k < 1.0;
    let k = if anim { c.k } else { 0.0 };
    let drop = if k < 0.45 { ease_out(k / 0.45) } else { 1.0 - ease_out(((k - 0.45) / 0.55).min(1.0)) };
    let sway = if anim { ((c.picked.len() as f32 * 1.7).sin()) * b.width() * 0.18 * (k * 3.0).min(1.0) } else { (c.time * 0.8).sin() * 30.0 };
    let top = b.top() + 10.0;
    let y = top + drop * (b.height() * 0.62);
    let x = col_x + sway;
    c.p.line_segment([pos2(x, top - 20.0), pos2(x, y)], Stroke::new(3.0f32, Color32::from_rgb(0xc0, 0xc0, 0xc8)));
    piece(c.p, c.ctx, c.th, 0, Rect::from_center_size(pos2(x, y + 40.0), vec2(110.0, 110.0)), Color32::WHITE);
    // What it's holding on the way up.
    if anim && k > 0.45
        && let Some(g) = grabs.get(c.step - 1)
    {
        let idx = match g {
            Find::Prize { kind, .. } => candy_piece(*kind),
            Find::Extra => Some(6),
            _ => None,
        };
        if let Some(i) = idx {
            piece(c.p, c.ctx, c.th, i, Rect::from_center_size(pos2(x, y + 92.0), vec2(70.0, 70.0)), Color32::WHITE);
        }
    }
    // The haul so far, along the bottom.
    let shown: Vec<&Find> = grabs.iter().take(if anim { c.step - 1 } else { c.step }).collect();
    let n = shown.len().max(1);
    let w = ((b.width() - 40.0) / n as f32).min(110.0);
    for (i, g) in shown.iter().enumerate() {
        let cc = pos2(b.center().x - (n as f32 - 1.0) * w / 2.0 + i as f32 * w, b.bottom() - 36.0);
        let (idx, text) = match g {
            Find::Prize { kind, chips } => (candy_piece(*kind), if *chips > 0 { format!("+{}", chips_text(*chips)) } else { "nothing".into() }),
            Find::Extra => (Some(6), "ANOTHER GO".into()),
            Find::Danger { .. } => (None, String::new()),
        };
        if let Some(ix) = idx {
            piece(c.p, c.ctx, c.th, ix, Rect::from_center_size(cc - vec2(0.0, 14.0), vec2(w * 0.7, w * 0.7)), Color32::WHITE);
        } else {
            let (m, d) = (cc - vec2(0.0, 14.0), 10.0);
            let xs = Stroke::new(3.0f32, col::MUTED);
            c.p.line_segment([m + vec2(-d, -d), m + vec2(d, d)], xs);
            c.p.line_segment([m + vec2(-d, d), m + vec2(d, -d)], xs);
        }
        tag(c.p, cc + vec2(0.0, 26.0), &text, 11.5, if matches!(g, Find::Extra) { c.st.second } else { col::GOLD2 });
    }
    tag(c.p, pos2(b.right() - 70.0, b.top() + 20.0), &format!("GOES LEFT: {}", left.max(0)), 14.0, Color32::WHITE);
}

/// Candy Grab's kinds on the sheet (0: nothing).
fn candy_piece(kind: u8) -> Option<usize> {
    match kind {
        1 => Some(1),
        2 => Some(2),
        3 => Some(3),
        4 => Some(4),
        5 => Some(8),
        _ => None,
    }
}

/// Mine Shaft: fifteen rocks to break; under them nuggets, a lantern, the mother lode, cave-ins.
fn mine(ui: &mut egui::Ui, c: &Ctx, picks: &[Pick], finale: bool, ready: bool) -> Option<usize> {
    let b = c.board;
    let gap = 12.0;
    let tw = ((b.width() - 4.0 * gap) / 5.0).min((b.height() - 2.0 * gap) / 3.0 * 1.3);
    let th_ = tw / 1.3;
    let gw = tw * 5.0 + gap * 4.0;
    let x0 = b.center().x - gw / 2.0;
    let y0 = b.top() + (b.height() - (th_ * 3.0 + gap * 2.0)) / 2.0;
    let mut take = None;
    for i in 0..15 {
        let tr = Rect::from_min_size(pos2(x0 + (i % 5) as f32 * (tw + gap), y0 + (i / 5) as f32 * (th_ + gap)), vec2(tw, th_));
        let at = c.picked.iter().position(|t| *t == i);
        match at.and_then(|k| picks.get(k).map(|x| (k, x))) {
            Some((k, pick)) if c.shows(k, 0.3) => {
                let (idx, text, color) = match pick {
                    Pick::Chips { chips } => (0, format!("+{}", chips_text(*chips)), col::GOLD2),
                    Pick::Gold { chips } => (7, format!("MOTHER LODE +{}", chips_text(*chips)), col::GOLD2),
                    Pick::Life => (6, "LANTERN · +1 LIFE".into(), c.st.second),
                    Pick::Saved => (5, "CAVE-IN · SAVED".into(), col::GOLD),
                    Pick::Busted => (4, "CAVE-IN!".into(), col::LOSE),
                };
                c.p.rect_filled(tr, theme::r(10), Color32::from_black_alpha(170));
                c.p.rect_stroke(tr, theme::r(10), Stroke::new(1.5f32, color.gamma_multiply(0.7)), StrokeKind::Inside);
                piece(c.p, c.ctx, c.th, idx, Rect::from_center_size(tr.center() - vec2(0.0, th_ * 0.1), vec2(th_ * 0.72, th_ * 0.72)), Color32::WHITE);
                tag(c.p, pos2(tr.center().x, tr.bottom() - 12.0), &text, 11.0, color);
            }
            Some(_) => {
                // Breaking open.
                let shake = (c.time * 60.0).sin() * 3.0;
                piece(c.p, c.ctx, c.th, 8, tr.translate(vec2(shake, 0.0)).shrink(6.0), Color32::WHITE);
            }
            None => {
                let resp = ui.interact(tr, ui.id().with(("mine", i)), Sense::click());
                let hover = ready && resp.hovered();
                piece(c.p, c.ctx, c.th, 8, if hover { tr.shrink(2.0) } else { tr.shrink(6.0) }, if hover { Color32::WHITE } else { Color32::from_gray(170) });
                if hover {
                    piece(c.p, c.ctx, c.th, 2, Rect::from_center_size(tr.right_top() + vec2(-tw * 0.18, th_ * 0.22), vec2(th_ * 0.4, th_ * 0.4)), Color32::WHITE);
                }
                if ready && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    take = Some(i);
                }
            }
        }
    }
    let lives = 1 + picks.iter().take(c.step).filter(|x| matches!(x, Pick::Life)).count() as i32 - picks.iter().take(c.step).filter(|x| matches!(x, Pick::Saved)).count() as i32;
    tag(c.p, pos2(b.right() - 60.0, b.top() + 6.0), &format!("LIVES {}", lives.max(0)), 13.0, Color32::WHITE);
    if c.step >= picks.len() && finale && c.k >= 0.5 {
        glow_text(c.p, b.center(), Align2::CENTER_CENTER, &format!("SHAFT CLEARED +{}", chips_text(share(c.bet, rw::MINE_FINALE, 100))), theme::display_font(c.st.font, 34.0), col::GOLD2, Color32::from_black_alpha(200));
    }
    take
}

/// Neon Drop: ten rows of pegs, eleven bins, a ball bouncing down to its bin.
fn drop_board(c: &Ctx, balls: &[rw::Ball]) {
    let b = c.board;
    let rows = rw::PEG_ROWS;
    let h = b.height() - 60.0;
    let sp = (h / (rows as f32 + 1.0)).min(b.width() / 13.0);
    let top = pos2(b.center().x, b.top() + 10.0);
    let peg = |r: usize, i: usize| pos2(top.x + (i as f32 - r as f32 / 2.0) * sp, top.y + (r as f32 + 1.0) * sp);
    for r in 0..rows {
        for i in 0..=r + 1 {
            let q = peg(r, i);
            c.p.circle_filled(q, 3.5, c.st.second.gamma_multiply(0.85));
            c.p.circle_filled(q, 7.0, c.st.second.gamma_multiply(0.12));
        }
    }
    // The bins and what they pay.
    let by = top.y + (rows as f32 + 1.0) * sp + 18.0;
    let counts: Vec<usize> = (0..=rows).map(|k| balls.iter().take(c.step.saturating_sub(if c.k < 1.0 { 1 } else { 0 })).filter(|x| x.bin as usize == k).count()).collect();
    for k in 0..=rows {
        let x = top.x + (k as f32 - rows as f32 / 2.0) * sp;
        let br = Rect::from_center_size(pos2(x, by), vec2(sp * 0.92, 34.0));
        let hot = counts[k] > 0;
        let col_ = if k == 0 || k == rows { c.th.accent } else if k == 1 || k == rows - 1 { c.st.second } else { Color32::from_rgb(0xff, 0xe0, 0x6a) };
        c.p.rect_filled(br, theme::r(6), col_.gamma_multiply(if hot { 0.55 } else { 0.22 }));
        c.p.rect_stroke(br, theme::r(6), Stroke::new(1.5f32, col_), StrokeKind::Inside);
        let x100 = rw::BINS[k];
        let label = if x100 % 100 == 0 { format!("{}×", x100 / 100) } else { format!("{:.1}×", x100 as f32 / 100.0) };
        c.p.text(br.center(), Align2::CENTER_CENTER, label, theme::bold_font((sp * 0.24).clamp(9.0, 13.0)), Color32::WHITE);
        if hot {
            c.p.text(br.center_bottom() + vec2(0.0, 10.0), Align2::CENTER_CENTER, format!("×{}", counts[k]), theme::bold_font(11.0), col_);
        }
    }
    // The ball on its way.
    if c.step > 0 && c.k < 1.0
        && let Some(ball) = balls.get(c.step - 1)
    {
        let u = c.k * (rows as f32 + 1.0);
        let r = u.floor() as usize;
        let f = u - u.floor();
        let mut col_i = 0usize;
        for rr in 0..r.min(rows) {
            col_i += ((ball.path >> rr) & 1) as usize;
        }
        let from = if r == 0 { top } else { peg(r - 1, col_i) - vec2(0.0, 10.0) };
        let to_i = if r < rows { col_i + ((ball.path >> r) & 1) as usize } else { col_i };
        let to = if r < rows { peg(r, to_i) - vec2(0.0, 10.0) } else { pos2(top.x + (ball.bin as f32 - rows as f32 / 2.0) * sp, by) };
        let q = from + (to - from) * f - vec2(0.0, (f * std::f32::consts::PI).sin() * sp * 0.35);
        piece(c.p, c.ctx, c.th, 0, Rect::from_center_size(q, vec2(sp * 0.6, sp * 0.6)), Color32::WHITE);
    }
    tag(c.p, pos2(b.right() - 70.0, b.top() + 10.0), &format!("BALLS LEFT {}", rw::BALLS - c.step.min(rw::BALLS)), 13.0, Color32::WHITE);
}

/// Ice Fishing: the hole in the ice, each cast's catch out of it, the catches on the board.
fn fishing(c: &Ctx, casts: &[Find]) {
    let b = c.board;
    let hole = pos2(b.center().x - b.width() * 0.12, b.center().y + 20.0);
    c.p.add(egui::epaint::EllipseShape::filled(hole, vec2(110.0, 40.0), Color32::from_rgb(0x06, 0x22, 0x40)));
    c.p.add(egui::epaint::EllipseShape::stroke(hole, vec2(110.0, 40.0), Stroke::new(4.0f32, Color32::from_rgb(0xd8, 0xf0, 0xff))));
    // The cast: the line down, the catch up.
    if c.step > 0
        && let Some(f) = casts.get(c.step - 1)
    {
        let k = c.k;
        let up = if k < 0.5 { 0.0 } else { ease_back(((k - 0.5) / 0.5).min(1.0)) };
        let at = hole - vec2(0.0, up * 120.0);
        if k < 0.5 {
            let bob = (c.time * 10.0).sin() * 4.0;
            c.p.line_segment([pos2(hole.x, b.top()), hole + vec2(0.0, bob)], Stroke::new(1.5f32, Color32::from_white_alpha(200)));
        } else {
            let idx = fish_piece(f);
            piece(c.p, c.ctx, c.th, idx, Rect::from_center_size(at, vec2(120.0, 120.0)), Color32::WHITE);
            let text = match f {
                Find::Prize { chips: 0, .. } => "A BOOT".to_string(),
                Find::Prize { chips, .. } => format!("+{}", chips_text(*chips)),
                Find::Extra => "HOT COCOA · a life".into(),
                Find::Danger { saved: true } => "THIN ICE · SAVED".into(),
                Find::Danger { saved: false } => "THIN ICE!".into(),
            };
            tag(c.p, at + vec2(0.0, 70.0), &text, 14.0, match f {
                Find::Danger { saved: false } => col::LOSE,
                _ => col::GOLD2,
            });
        }
    }
    // The trophy board.
    let board = Rect::from_min_max(pos2(b.center().x + b.width() * 0.12, b.top()), b.max - vec2(0.0, 10.0));
    c.p.rect_filled(board, theme::r(12), Color32::from_black_alpha(170));
    c.p.text(pos2(board.center().x, board.top() + 18.0), Align2::CENTER_CENTER, "THE CATCH", theme::display_font(c.st.font, 20.0), col::GOLD2);
    let shown = c.step.saturating_sub(if c.k < 1.0 { 1 } else { 0 });
    let rows = rw::CASTS as usize;
    let rh = ((board.height() - 40.0) / rows as f32).min(54.0);
    for (i, f) in casts.iter().take(shown).enumerate() {
        let y = board.top() + 40.0 + (i as f32 + 0.5) * rh;
        piece(c.p, c.ctx, c.th, fish_piece(f), Rect::from_center_size(pos2(board.left() + 34.0, y), vec2(rh * 0.9, rh * 0.9)), Color32::WHITE);
        let t = match f {
            Find::Prize { chips: 0, .. } => "a boot".to_string(),
            Find::Prize { kind, chips } => format!("{}  +{}", ["", "a perch", "a trout", "a salmon", "a big salmon", "THE GOLDEN FISH"][(*kind as usize).min(5)], chips_text(*chips)),
            Find::Extra => "hot cocoa".into(),
            Find::Danger { saved } => if *saved { "thin ice (cocoa drunk)".into() } else { "thin ice: in you go".into() },
        };
        c.p.text(pos2(board.left() + 64.0, y), Align2::LEFT_CENTER, t, theme::bold_font(13.0), col::TEXT);
    }
    tag(c.p, pos2(hole.x, b.top() + 12.0), &format!("CASTS LEFT {}", (rw::CASTS as usize).saturating_sub(c.step)), 13.0, Color32::WHITE);
    let cocoa = casts.iter().take(shown).fold(rw::START_EXTRA as i32, |n, f| match f {
        Find::Extra => n + 1,
        Find::Danger { saved: true } => n - 1,
        _ => n,
    });
    tag(c.p, pos2(hole.x, b.top() + 34.0), &format!("COCOA IN HAND {}", cocoa.max(0)), 13.0, col::GOLD2);
}

fn fish_piece(f: &Find) -> usize {
    match f {
        Find::Prize { chips: 0, .. } => 3,
        Find::Prize { kind, .. } => [0, 0, 0, 1, 7, 2][(*kind as usize).min(5)],
        Find::Extra => 4,
        Find::Danger { .. } => 5,
    }
}

/// Potion Brew: nine jars on the shelf, three picked, their colours and herbs, the brew.
fn potion(ui: &mut egui::Ui, c: &Ctx, jars: &[rw::Jar], ready: bool) -> Option<usize> {
    let b = c.board;
    let mut take = None;
    let n = 9;
    let jw = ((b.width() - 80.0) / n as f32).min(110.0);
    let y = b.top() + jw * 0.7;
    for i in 0..n {
        let jr = Rect::from_center_size(pos2(b.center().x + (i as f32 - 4.0) * jw, y), vec2(jw * 0.9, jw * 1.1));
        let at = c.picked.iter().position(|t| *t == i);
        match at {
            Some(k) if c.shows(k, 0.4) => {
                let j = jars[k];
                piece(c.p, c.ctx, c.th, j.colour as usize, jr, Color32::WHITE);
                tag(c.p, pos2(jr.center().x, jr.bottom() + 8.0), &format!("+{}", chips_text(j.chips)), 11.5, col::GOLD2);
            }
            Some(_) => {
                let lift = c.k * 30.0;
                piece(c.p, c.ctx, c.th, 7, jr.translate(vec2(0.0, -lift)), Color32::WHITE);
            }
            None => {
                let resp = ui.interact(jr, ui.id().with(("jar", i)), Sense::click());
                let hover = ready && resp.hovered();
                piece(c.p, c.ctx, c.th, 7, if hover { jr.translate(vec2(0.0, -6.0)) } else { jr }, if c.step < 3 { Color32::WHITE } else { Color32::from_gray(90) });
                if ready && c.step < 3 && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    take = Some(i);
                }
            }
        }
    }
    // The cauldron, and the brew once three are in.
    let cc = pos2(b.center().x, b.bottom() - 80.0);
    let brewed = c.step >= 3 && c.k >= 1.0;
    let x = if brewed { rw::brew_x(jars) } else { 0 };
    let glow = match x {
        4 => c.th.accent,
        2 => c.st.second,
        1 => Color32::from_rgb(0x8a, 0x8a, 0x6a),
        _ => Color32::from_rgb(0x30, 0x90, 0x40),
    };
    radial(c.p, cc, 150.0, glow.gamma_multiply(0.45 + 0.15 * (c.time * 3.0).sin()), Color32::TRANSPARENT);
    let picked: Vec<String> = jars.iter().take(c.step).enumerate().filter(|(k, _)| c.shows(*k, 0.4)).map(|(_, j)| ["RED", "GREEN", "BLUE"][j.colour as usize].to_string()).collect();
    c.p.text(cc - vec2(0.0, 30.0), Align2::CENTER_CENTER, if picked.is_empty() { "PICK THREE JARS".to_string() } else { picked.join(" + ") }, theme::display_font(c.st.font, 24.0), Color32::WHITE);
    let rule = format!("all one colour ×{}  ·  all different ×{}  ·  else ×1", rw::SAME_X, rw::RAINBOW_X);
    c.p.text(cc + vec2(0.0, 2.0), Align2::CENTER_CENTER, rule, theme::font(12.5), col::MUTED);
    if brewed {
        let what = match x {
            4 => "ONE COLOUR: ×4!",
            2 => "A RAINBOW: ×2!",
            _ => "A MUDDY BREW: ×1",
        };
        piece(c.p, c.ctx, c.th, if x == 4 { 6 } else if x == 2 { 6 } else { 8 }, Rect::from_center_size(cc + vec2(-180.0, 10.0), vec2(90.0, 90.0)), Color32::WHITE);
        glow_text(c.p, cc + vec2(0.0, 40.0), Align2::CENTER_CENTER, what, theme::display_font(c.st.font, 30.0), col::GOLD2, glow.gamma_multiply(0.6));
    }
    take
}

/// Treasure Match: thirteen chests (four treasures three times over, and the egg); open till
/// three match.
fn treasure_match(ui: &mut egui::Ui, c: &Ctx, opened: &[u8], treasure: u8, egg: bool, ready: bool) -> Option<usize> {
    let b = c.board;
    let mut take = None;
    let rows = [5usize, 4, 4];
    let cw = ((b.width() - 300.0) / 5.0).min((b.height() - 20.0) / 3.0 * 1.1).min(130.0);
    let ch = cw / 1.1;
    let mut i = 0;
    for (ri, n) in rows.iter().enumerate() {
        for k in 0..*n {
            let x = b.center().x - 120.0 + (k as f32 - (*n as f32 - 1.0) / 2.0) * (cw + 10.0);
            let r = Rect::from_center_size(pos2(x, b.top() + (ri as f32 + 0.5) * (ch + 10.0)), vec2(cw, ch));
            let at = c.picked.iter().position(|t| *t == i);
            match at {
                Some(s) if c.shows(s, 0.4) => {
                    let what = opened[s];
                    piece(c.p, c.ctx, c.th, 6, r, Color32::from_gray(150));
                    let win = what == treasure && c.step >= opened.len() && c.k >= 1.0;
                    if win || what == 4 {
                        radial(c.p, r.center(), cw * 0.7, col::GOLD.gamma_multiply(0.5), Color32::TRANSPARENT);
                    }
                    piece(c.p, c.ctx, c.th, what as usize, Rect::from_center_size(r.center() - vec2(0.0, ch * 0.1), vec2(cw * 0.55, cw * 0.55)), Color32::WHITE);
                }
                Some(_) => {
                    let shake = (c.time * 50.0).sin() * 2.0;
                    piece(c.p, c.ctx, c.th, 5, r.translate(vec2(shake, 0.0)), Color32::WHITE);
                }
                None => {
                    let resp = ui.interact(r, ui.id().with(("chest", i)), Sense::click());
                    let hover = ready && resp.hovered();
                    piece(c.p, c.ctx, c.th, 5, if hover { r.expand(4.0) } else { r }, if ready { Color32::WHITE } else { Color32::from_gray(120) });
                    if ready && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        take = Some(i);
                    }
                }
            }
            i += 1;
        }
    }
    // The tally: each treasure's count and its prize; the egg.
    let tally = Rect::from_min_max(pos2(b.right() - 250.0, b.top()), b.max);
    c.p.rect_filled(tally, theme::r(12), Color32::from_black_alpha(170));
    let seen = |t: u8| opened.iter().take(c.step).enumerate().filter(|(s, x)| **x == t && c.shows(*s, 0.4)).count();
    for t in 0..4u8 {
        let y = tally.top() + 30.0 + t as f32 * 54.0;
        piece(c.p, c.ctx, c.th, t as usize, Rect::from_center_size(pos2(tally.left() + 32.0, y), vec2(44.0, 44.0)), Color32::WHITE);
        let n = seen(t);
        for d in 0..3 {
            let dc = pos2(tally.left() + 70.0 + d as f32 * 20.0, y);
            c.p.circle_filled(dc, 7.0, if d < n { col::GOLD } else { Color32::from_white_alpha(30) });
        }
        c.p.text(pos2(tally.right() - 12.0, y), Align2::RIGHT_CENTER, chips_text(share(c.bet, rw::TREASURES[t as usize], 100)), theme::bold_font(13.0), col::GOLD2);
    }
    let egg_seen = seen(4) > 0;
    let y = tally.top() + 30.0 + 4.0 * 54.0;
    piece(c.p, c.ctx, c.th, 4, Rect::from_center_size(pos2(tally.left() + 32.0, y), vec2(44.0, 44.0)), if egg_seen { Color32::WHITE } else { Color32::from_gray(80) });
    c.p.text(pos2(tally.left() + 64.0, y), Align2::LEFT_CENTER, if egg_seen { "THE EGG: ×2!" } else { "the egg doubles it" }, theme::bold_font(13.0), if egg_seen { col::GOLD2 } else { col::MUTED });
    if c.step >= opened.len() && c.k >= 1.0 {
        let names = ["JADE", "RUBIES", "GOLD", "PEARLS"];
        glow_text(c.p, pos2(b.center().x - 120.0, b.bottom() - 16.0), Align2::CENTER_CENTER, &format!("THREE {}{}", names[treasure as usize], if egg { "  ×2" } else { "" }), theme::display_font(c.st.font, 28.0), col::GOLD2, Color32::from_black_alpha(200));
    }
    take
}

/// Cup Shuffle: three cups shuffled, one picked, the bean under it or not.
fn cups(ui: &mut egui::Ui, c: &Ctx, rounds: &[Cup], finale: bool, ready: bool) -> Option<usize> {
    let b = c.board;
    let mut take = None;
    let cy = b.center().y + 30.0;
    let sp = (b.width() / 4.0).min(220.0);
    let shuffling = ready && c.step < rounds.len();
    // Cups dance about before each pick (slower each round's start), then settle.
    let st = if shuffling { (c.time * 4.0 + c.step as f32).sin() } else { 0.0 };
    let last = if c.step > 0 { rounds.get(c.step - 1).copied() } else { None };
    let lifting = c.step > 0 && c.k < 1.0;
    for j in 0..3 {
        let base = pos2(b.center().x + (j as f32 - 1.0) * sp, cy);
        let wob = if shuffling { vec2(st * 18.0 * (j as f32 - 1.0), (c.time * 7.0 + j as f32).sin().abs() * -8.0) } else { vec2(0.0, 0.0) };
        let picked_j = c.picked.last().copied();
        let lift = if (lifting || (c.step > 0 && !shuffling)) && last.is_some() {
            let shows = |cupi: usize| match last {
                Some(Cup::Found { .. }) => Some(cupi) == picked_j,
                Some(Cup::Missed { other, .. }) => {
                    let others: Vec<usize> = (0..3).filter(|x| Some(*x) != picked_j).collect();
                    Some(cupi) == picked_j || others.get(other as usize) == Some(&cupi)
                }
                None => false,
            };
            if shows(j) { 70.0 * ease_out(c.k.min(1.0) * 1.5).min(1.0) } else { 0.0 }
        } else {
            0.0
        };
        let cr = Rect::from_center_size(base + wob - vec2(0.0, lift), vec2(sp * 0.7, sp * 0.7));
        // What's under it, once it's lifted.
        if lift > 20.0 {
            let bean_here = match last {
                Some(Cup::Found { .. }) => Some(j) == picked_j,
                Some(Cup::Missed { other, .. }) => {
                    let others: Vec<usize> = (0..3).filter(|x| Some(*x) != picked_j).collect();
                    others.get(other as usize) == Some(&j)
                }
                None => false,
            };
            if bean_here {
                piece(c.p, c.ctx, c.th, if c.step == rounds.len() { 1 } else { 0 }, Rect::from_center_size(base + vec2(0.0, sp * 0.18), vec2(sp * 0.3, sp * 0.3)), Color32::WHITE);
            }
        }
        let resp = ui.interact(cr, ui.id().with(("cup", j)), Sense::click());
        let hover = ready && resp.hovered();
        piece(c.p, c.ctx, c.th, 2, if hover { cr.expand(5.0) } else { cr }, Color32::WHITE);
        if ready && c.step < rounds.len() && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            take = Some(j);
        }
    }
    // The rounds: their prizes, found, missed (a napkin), spilled.
    for (i, prize) in rw::CUP_ROUND.iter().enumerate() {
        let x = b.center().x + (i as f32 - 2.0) * 120.0;
        let r = Rect::from_center_size(pos2(x, b.top() + 20.0), vec2(110.0, 30.0));
        let shown = c.shows(i, 0.6);
        let (fill, text) = match (shown, rounds.get(i)) {
            (true, Some(Cup::Found { chips })) => (col::WIN.gamma_multiply(0.5), format!("+{}", chips_text(*chips))),
            (true, Some(Cup::Missed { saved: true, .. })) => (col::GOLD.gamma_multiply(0.4), "napkin".into()),
            (true, Some(Cup::Missed { saved: false, .. })) => (col::LOSE.gamma_multiply(0.5), "spilled".into()),
            _ => (Color32::from_black_alpha(170), chips_text(share(c.bet, *prize, 100))),
        };
        c.p.rect_filled(r, theme::r(15), fill);
        c.p.text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(13.0), Color32::WHITE);
    }
    let napkins = rw::NAPKINS as i32 - rounds.iter().take(c.step).filter(|r| matches!(r, Cup::Missed { saved: true, .. })).count() as i32;
    for i in 0..napkins.max(0) {
        piece(c.p, c.ctx, c.th, 4, Rect::from_center_size(pos2(b.right() - 40.0 - i as f32 * 50.0, b.top() + 64.0), vec2(48.0, 48.0)), Color32::WHITE);
    }
    if c.step >= rounds.len() && c.k >= 1.0 {
        let t = if finale { format!("BARISTA OF THE YEAR +{}", chips_text(share(c.bet, rw::CUP_FINALE, 100))) } else { "SPILLED!".into() };
        glow_text(c.p, pos2(b.center().x, b.bottom() - 18.0), Align2::CENTER_CENTER, &t, theme::display_font(c.st.font, 28.0), if finale { col::GOLD2 } else { col::LOSE }, Color32::from_black_alpha(200));
    }
    take
}

/// Champagne Tower: six tiers of glasses, each poured one multiplying the tower.
fn champagne(c: &Ctx, tiers: &[Tier], tower: u64) {
    let b = c.board;
    let n = rw::GROW_TENTHS.len();
    let gw = ((b.width() * 0.5) / n as f32).min((b.height() - 40.0) / n as f32 * 1.1);
    let base_y = b.bottom() - gw * 0.5;
    let poured_to = tiers.iter().take(c.step).filter(|t| matches!(t, Tier::Poured { .. })).count();
    for tier in 0..n {
        let count = n - tier;
        let y = base_y - tier as f32 * gw * 0.88;
        // Which step poured this tier, if one did.
        let mut k = 0;
        let mut step_of = None;
        for (s, t) in tiers.iter().enumerate() {
            if matches!(t, Tier::Poured { .. }) {
                if k == tier {
                    step_of = Some(s);
                    break;
                }
                k += 1;
            }
        }
        let full = step_of.is_some_and(|s| c.shows(s, 0.6));
        for g in 0..count {
            let x = b.center().x - gw * 1.6 + (g as f32 - (count as f32 - 1.0) / 2.0) * gw * 0.9;
            let r = Rect::from_center_size(pos2(x, y), vec2(gw, gw));
            if full {
                radial(c.p, r.center(), gw * 0.6, Color32::from_rgb(0xff, 0xe0, 0x8a).gamma_multiply(0.25), Color32::TRANSPARENT);
            }
            piece(c.p, c.ctx, c.th, 0, r, if full { Color32::WHITE } else { Color32::from_white_alpha(90) });
        }
    }
    // The bottle pouring, and a topple.
    if c.step > 0 && c.k < 1.0
        && let Some(t) = tiers.get(c.step - 1)
    {
        let y = base_y - poured_to.saturating_sub(1) as f32 * gw * 0.88 - gw;
        match t {
            Tier::Poured { .. } => piece(c.p, c.ctx, c.th, 1, Rect::from_center_size(pos2(b.center().x - gw * 1.6 + gw * 0.6, y - gw * 0.4), vec2(gw * 1.2, gw * 1.2)), Color32::WHITE),
            Tier::Toppled { .. } => {
                let shake = (c.time * 40.0).sin() * 6.0 * (1.0 - c.k);
                piece(c.p, c.ctx, c.th, 2, Rect::from_center_size(pos2(b.center().x - gw * 1.6 + shake, y + gw * 0.4), vec2(gw * 1.4, gw * 1.4)), Color32::WHITE);
            }
        }
    }
    // The tower's worth, the encores, the tiers.
    let side = Rect::from_min_max(pos2(b.center().x + gw * 1.2, b.top()), b.max - vec2(10.0, 10.0));
    let value = tiers.iter().take(c.step).enumerate().filter(|(s, _)| c.shows(*s, 0.6)).filter_map(|(_, t)| if let Tier::Poured { tower, .. } = t { Some(*tower) } else { None }).last().unwrap_or(rw::TOWER_BASE);
    glow_text(c.p, pos2(side.center().x, side.top() + 30.0), Align2::CENTER_CENTER, &format!("{:.2}×", value as f32 / 100.0), theme::display_font(c.st.font, 44.0), col::GOLD2, c.th.accent.gamma_multiply(0.5));
    c.p.text(pos2(side.center().x, side.top() + 66.0), Align2::CENTER_CENTER, format!("the tower: {}", chips_text(share(c.bet, value, 100))), theme::bold_font(14.0), Color32::WHITE);
    for (t, (g, p_)) in rw::GROW_TENTHS.iter().zip(rw::TOPPLE.iter()).enumerate() {
        let y = side.top() + 100.0 + t as f32 * 30.0;
        let done_t = t < poured_to;
        c.p.text(pos2(side.left() + 10.0, y), Align2::LEFT_CENTER, format!("tier {}: ×{:.1}, {}% topple", t + 1, *g as f32 / 10.0, p_), theme::font(13.0), if done_t { col::WIN } else { col::MUTED });
    }
    let enc = tiers.iter().take(c.step).fold(rw::START_EXTRA as i32, |e, t| match t {
        Tier::Poured { encore: true, .. } => e + 1,
        Tier::Toppled { saved: true } => e - 1,
        _ => e,
    });
    for i in 0..enc.max(0) {
        piece(c.p, c.ctx, c.th, 6, Rect::from_center_size(pos2(side.right() - 30.0 - i as f32 * 46.0, side.bottom() - 30.0), vec2(42.0, 42.0)), Color32::WHITE);
    }
    if c.step >= tiers.len() && c.k >= 1.0 {
        let standing = tiers.len() == n && tiers.iter().all(|t| matches!(t, Tier::Poured { .. }));
        let t = if standing { "THE TOWER STANDS!".to_string() } else { format!("CURTAIN: {:.2}×", tower as f32 / 100.0) };
        glow_text(c.p, pos2(b.center().x - gw * 1.6, b.top() + 10.0), Align2::CENTER_CENTER, &t, theme::display_font(c.st.font, 28.0), col::GOLD2, Color32::from_black_alpha(200));
    }
}
