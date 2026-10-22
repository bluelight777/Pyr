//! Ember Dragon at the app: the dragon's lair (Meshy's render), lava glowing and embers rising,
//! the four jackpots across the top, the dragon on its hoard growing from an egg to an Elder as
//! eggs land, the reels (Meshy's symbols), and Ember Respins: the fireballs held on the board,
//! the rest spinning again, each new one sticking and giving three more spins, the board's total
//! climbing, the jackpots going off. The rounds are the house's (`pyr_poker::casino::dragon`),
//! checked by this Pyr like every other.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use pyr_engine::poker_sfx::Cue;
use pyr_engine::state::ServerView;
use pyr_poker::casino::dragon::{self as dg, AGES, BETS, Fire, Jackpot, REELS, ROWS};
use pyr_poker::casino::feature::Feature;
use pyr_poker::casino::{Bet, Outcome};
use pyr_poker::net::RoundView;

use super::stake::{Pick, SlotKind, StakeUi};

use super::spin::{Reel, SPEED};
use super::{col, ease_back, ease_out, glow_text, pics, radial, vgrad, vgrad_round};
use crate::app::PyrApp;
use crate::poker::cards::chips_text;
use crate::theme;

/// Filler under the reels while they spin: this many symbols round each strip.
const STRIP: usize = 24;
/// Each symbol's cell on Meshy's sheet (crown, chalice, sword / dragon coin, chest, egg /
/// fireball, ruby, sapphire).
const CELL: [usize; 9] = [8, 7, 2, 1, 0, 4, 3, 6, 5];
const NAMES: [&str; 9] = ["Sapphire", "Ruby", "Sword", "Chalice", "Crown", "Chest", "Dragon coin", "Fireball", "Egg"];
/// A respin on the board, and the pause before the next.
const RESPIN_S: f32 = 1.15;
const EMBER: Color32 = Color32::from_rgb(0xff, 0x6a, 0x1a);
const LAVA: Color32 = Color32::from_rgb(0xff, 0x3a, 0x08);

fn jackpot_color(j: Jackpot) -> Color32 {
    match j {
        Jackpot::Mini => Color32::from_rgb(0x4a, 0xc8, 0x6a),
        Jackpot::Minor => Color32::from_rgb(0x4a, 0x9a, 0xff),
        Jackpot::Major => Color32::from_rgb(0xc2, 0x6a, 0xff),
        Jackpot::Grand => Color32::from_rgb(0xff, 0xc8, 0x3a),
    }
}

/// Ember Respins as they play out on screen.
struct RespinPlay {
    round: RoundView,
    started: Instant,
    /// When it was over on screen, and whether its win has gone on the balance.
    over_at: Option<Instant>,
    collected: bool,
    /// Respins already shown (for their sounds).
    sounded: usize,
}

pub struct DragonUi {
    pub bet: u64,
    reels: [Reel; REELS],
    strips: [[u8; STRIP]; REELS],
    showing: Option<RoundView>,
    stopped_at: Option<Instant>,
    counted: Option<Instant>,
    waiting: Option<Instant>,
    respins: Option<RespinPlay>,
    pub win: u64,
    pub auto: super::auto::Auto,
    pub turbo: bool,
    next_at: Option<Instant>,
    /// The dragon's growth as shown (it catches up as the eggs fly to it), and a growth spurt
    /// being celebrated (its new age, since when).
    shown_growth: Option<(u64, u32)>,
    grew: Option<(usize, Instant)>,
    /// Eggs flying from the reels to the dragon (from, when they set off).
    eggs: Vec<(Pos2, Instant)>,
    /// When the pools were last asked for.
    asked: Option<Instant>,
    /// Play Feature and Fairness (0.13.0 servers); all or nothing being revealed.
    pub stake: StakeUi,
    all_in: Option<(RoundView, Instant, bool)>,
}

impl Default for DragonUi {
    fn default() -> Self {
        DragonUi {
            bet: 100,
            reels: std::array::from_fn(|r| Reel::still((r * 5) as f32)),
            strips: std::array::from_fn(|r| filler(r as u64 * 7_919 + 11)),
            showing: None,
            stopped_at: None,
            counted: None,
            waiting: None,
            respins: None,
            win: 0,
            auto: Default::default(),
            turbo: false,
            next_at: None,
            shown_growth: None,
            grew: None,
            eggs: Vec::new(),
            asked: None,
            stake: StakeUi::default(),
            all_in: None,
        }
    }
}

/// Symbols to fill a strip with while it spins (by the game's own odds, roughly).
fn filler(seed: u64) -> [u8; STRIP] {
    let mut x = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
    std::array::from_fn(|_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        match (x >> 11) % 1000 {
            0..250 => 0,
            250..435 => 1,
            435..575 => 2,
            575..680 => 3,
            680..755 => 4,
            755..805 => 5,
            805..845 => dg::WILD,
            845..965 => dg::FIRE,
            _ => dg::EGG,
        }
    })
}

/// A fireball's words: its prize (chips at the bet) or its jackpot.
fn fire_label(f: Fire, bet: u64) -> (String, Color32) {
    match f {
        Fire::Prize { x } => (chips_text(x * bet), Color32::WHITE),
        Fire::Jackpot { which } => (which.name().to_uppercase(), jackpot_color(which)),
    }
}

impl DragonUi {
    /// Spinning, landing, counting, or in Ember Respins.
    fn running(&self, now: Instant) -> bool {
        self.waiting.is_some() || self.stopped_at.is_some_and(|e| now < e) || self.respins.as_ref().is_some_and(|b| !b.collected) || self.all_in.is_some()
    }
}

impl PyrApp {
    /// My dragon's growth at a bet, as the house last said.
    fn dragon_growth(s: &ServerView, bet: u64) -> u32 {
        s.poker.casino.dragon.as_ref().and_then(|d| d.growth.get(&bet).copied()).unwrap_or(0)
    }

    /// A round came back: the reels land on it.
    pub(super) fn dragon_round(&mut self, round: &RoundView) {
        let Outcome::Dragon(o) = &round.outcome else { return };
        let now = Instant::now();
        let ui = &mut self.poker.casino.dragon;
        ui.waiting = None;
        ui.win = 0;
        ui.counted = None;
        if o.feature == Feature::AllIn {
            ui.showing = None;
            for r in &mut ui.reels {
                *r = Reel::still(r.pos(now).round());
            }
            ui.all_in = Some((round.clone(), now, false));
            self.poker.casino.sound_in(0.0, Cue::Fuse, 0.0, 0.7);
            return;
        }
        ui.showing = Some(round.clone());
        ui.shown_growth = Some((o.bet, o.growth));
        // Each strip: the round's three symbols at its top, filler after them.
        for (r, strip) in ui.strips.iter_mut().enumerate() {
            let mut s = filler(round.nonce.wrapping_mul(31).wrapping_add(r as u64));
            s[..ROWS].copy_from_slice(&o.grid[r]);
            *strip = s;
        }
        for reel in &mut ui.reels {
            if reel.speed <= 0.0 || reel.landed(now) {
                reel.start(now, SPEED);
            }
        }
        let min = if ui.turbo { 0.25 } else { 0.7 };
        let ends = super::spin::land(&mut ui.reels, &[STRIP; REELS], &[0; REELS], now + Duration::from_secs_f32(min), ui.turbo);
        let last = ends.iter().max().copied().unwrap_or(now) + Duration::from_secs_f32(super::spin::land_secs(ui.turbo) * 0.45);
        ui.stopped_at = Some(last);
        // (Five fireballs landed: the last reel slows right down, teasing the sixth.)
        for (i, e) in ends.iter().enumerate() {
            self.poker.casino.sound_in(e.saturating_duration_since(now).as_secs_f32(), Cue::ReelStop, (i as f32 - 2.0) * 0.3, 0.7);
        }
    }

    fn dragon_spin(&mut self, s: &ServerView) {
        let f = self.poker.casino.dragon.stake.armed.unwrap_or(Feature::Spin);
        self.dragon_spin_with(s, f);
    }

    /// A round: a spin (with Play Feature's choice, on a 0.13.0 server), from my seed for the
    /// slot there.
    fn dragon_spin_with(&mut self, s: &ServerView, feature: Feature) {
        let Some(commit) = s.poker.casino.commit else { return };
        let now = Instant::now();
        let v13 = s.has(pyr_proto::control::CASINO_013_FEATURE);
        let feature = if v13 { feature } else { Feature::Spin };
        let ui = &self.poker.casino.dragon;
        let cost = pyr_poker::casino::feature::cost(ui.bet, dg::price(feature).unwrap_or(100));
        if cost > super::chips_now(s) {
            self.poker.casino.dragon.auto.stop();
            return;
        }
        if self.poker.casino.busy() || ui.running(now) {
            return;
        }
        let bet = ui.bet;
        let growth = Self::dragon_growth(s, bet);
        self.poker.casino.sound_in(0.0, Cue::ReelSpin, 0.0, 0.6);
        let ui = &mut self.poker.casino.dragon;
        for r in &mut ui.reels {
            r.start(now, SPEED);
        }
        ui.waiting = Some(now);
        ui.showing = None;
        ui.respins = None;
        ui.counted = None;
        ui.stopped_at = None;
        ui.win = 0;
        ui.eggs.clear();
        ui.all_in = None;
        self.poker.casino.celebration = None;
        self.poker.casino.pending = Some(now);
        if v13 {
            let b = if feature == Feature::Spin { Bet::Dragon { bet, growth } } else { Bet::DragonPlus { bet, growth, feature } };
            self.play_seeded(s, SlotKind::Dragon, b);
        } else {
            self.engine.casino_play(&s.key, commit, Bet::Dragon { bet, growth });
        }
    }

    pub(super) fn dragon_page(&mut self, ui: &mut egui::Ui, s: &ServerView, area: Rect) {
        let now = Instant::now();
        let ctx = ui.ctx().clone();
        let p = ui.painter().clone();
        let time = ctx.input(|i| i.time) as f32;
        // The pools climb with everyone's spins: asked for now and then.
        if self.poker.casino.dragon.asked.is_none_or(|t| t.elapsed() > Duration::from_secs(8)) {
            self.poker.casino.dragon.asked = Some(now);
            self.engine.casino_open(&s.key, true);
        }
        // The lair: the cave, lava glowing at its foot, embers rising.
        p.rect_filled(area, 0, Color32::from_rgb(0x0c, 0x05, 0x04));
        pics::cover(&p, &ctx, "dragon-lair", area, 2048, Color32::from_gray(150));
        vgrad(&p, area, Color32::from_black_alpha(110), Color32::from_black_alpha(40));
        let throb = 0.75 + 0.25 * (time * 1.3).sin();
        vgrad(&p, Rect::from_min_max(pos2(area.left(), area.bottom() - area.height() * 0.35), area.max), Color32::TRANSPARENT, LAVA.gamma_multiply(0.22 * throb));
        for i in 0..46u32 {
            let k = ((time * (0.05 + (i % 7) as f32 * 0.012)) + i as f32 * 0.137).fract();
            let x = area.left() + area.width() * ((i as f32 * 0.6180).fract() + (time * 0.1 + i as f32).sin() * 0.01);
            let y = area.bottom() - k * area.height() * 1.05;
            let a = (1.0 - k) * (k * 8.0).min(1.0);
            p.circle_filled(pos2(x, y), 1.4 + (i % 3) as f32 * 0.7, Color32::from_rgba_unmultiplied(0xff, 0x9a + (i % 5) as u8 * 12, 0x30, (210.0 * a) as u8));
        }

        // A refused spin (not enough chips, say): the reels come to rest.
        if let Some(since) = self.poker.casino.dragon.waiting
            && self.poker.casino.pending.is_none()
            && since.elapsed() > Duration::from_millis(300)
        {
            let r = &mut self.poker.casino.dragon;
            r.waiting = None;
            for reel in &mut r.reels {
                *reel = Reel::still(reel.pos(now).round());
            }
            r.auto.stop();
        }

        // Layout: the jackpots on top, the dragon on the left, the reels, the controls.
        let controls_h = 104.0;
        let jack_h = (area.height() * 0.13).clamp(70.0, 104.0);
        let room = Rect::from_min_max(pos2(area.left() + 16.0, area.top() + 12.0 + jack_h + 10.0), pos2(area.right() - 16.0, area.bottom() - controls_h - 10.0));
        let dragon_w = (room.width() * 0.26).clamp(170.0, 330.0);
        let cell = ((room.height() - 30.0) / 3.0).min((room.width() - dragon_w - 60.0) / 5.0).clamp(56.0, 160.0);
        let gap = (cell * 0.06).max(4.0);
        let reels_w = cell * 5.0 + gap * 4.0;
        let block_w = dragon_w + 24.0 + reels_w;
        let left = room.center().x - block_w / 2.0;
        let reels = Rect::from_min_size(pos2(left + dragon_w + 24.0, room.center().y - cell * 1.5), vec2(reels_w, cell * 3.0));
        let den = Rect::from_min_size(pos2(left, reels.top() - 12.0), vec2(dragon_w, reels.height() + 24.0));
        let jack = Rect::from_min_size(pos2(area.center().x - (block_w.max(560.0)) / 2.0, area.top() + 12.0), vec2(block_w.max(560.0), jack_h));
        self.dragon_jackpots(&p, s, jack, time);
        self.dragon_den(&p, &ctx, s, den, now, time);
        let reel_rects: Vec<Rect> = (0..REELS).map(|i| Rect::from_min_size(pos2(reels.left() + i as f32 * (cell + gap), reels.top()), vec2(cell, cell * 3.0))).collect();
        let in_respins = self.poker.casino.dragon.respins.as_ref().is_some_and(|r| now >= r.started);
        if in_respins {
            self.dragon_respins(ui, s, reels, &reel_rects, cell, now, time);
        } else {
            self.dragon_reels(&p, &ctx, s, &reel_rects, reels, cell, now, time);
        }
        self.dragon_after_stop(s, &reel_rects, cell, den, now);
        self.dragon_eggs(&p, &ctx, den, cell, now);
        // All or nothing, revealed over the reels.
        if let Some((round, since, told)) = self.poker.casino.dragon.all_in.clone()
            && let Outcome::Dragon(o) = &round.outcome
        {
            let t = (now - since).as_secs_f32();
            let landed = super::stake::all_in_dial(&p, reels.expand(14.0), EMBER, "cinzel-deco", o.lucky, o.paid, &format!("1 in 51 for {}", chips_text(o.bet * dg::ALL_IN_X)), t);
            if landed && !told {
                self.poker.casino.dragon.all_in.as_mut().unwrap().2 = true;
                self.poker.casino.dragon.win = o.paid;
                self.poker.casino.sound_in(0.0, if o.lucky { Cue::Jackpot } else { Cue::Fold }, 0.0, 0.8);
            }
            if landed && t > 5.4 {
                self.poker.casino.dragon.all_in = None;
                self.dragon_finish(s, &round, now);
            }
        }
        // The controls.
        let bar = Rect::from_min_size(pos2(area.center().x - block_w.max(620.0) / 2.0, area.bottom() - controls_h + 4.0), vec2(block_w.max(620.0), controls_h - 18.0));
        self.dragon_controls(ui, s, bar, now);
        // Play Feature and Fairness (0.13.0 servers), over the dragon's den.
        if s.has(pyr_proto::control::CASINO_013_FEATURE) {
            let kind = SlotKind::Dragon;
            let pf = Rect::from_min_size(pos2(den.left(), (den.bottom() + 8.0).min(bar.top() - 52.0)), vec2(den.width().min(220.0), 44.0));
            let busy = self.poker.casino.busy() || self.poker.casino.dragon.running(now);
            self.play_feature_button(ui, kind, pf, EMBER, !busy && !self.poker.casino.dragon.auto.on());
            let fr = Rect::from_min_size(pos2(bar.right() - 112.0, bar.top() - 40.0), vec2(108.0, 30.0));
            self.fairness_button(ui, kind, fr);
            let bet = self.poker.casino.dragon.bet;
            if let Some(pick) = self.play_feature_menu(ui, kind, area, bet, bet, EMBER, super::chips_now(s)) {
                match pick {
                    Pick::Arm(f) => self.poker.casino.dragon.stake.armed = f,
                    Pick::Buy(f) => self.dragon_spin_with(s, f),
                }
            }
            self.fairness_panel(&ctx, s, kind);
        }
        // Autoplay spins again.
        let r = &self.poker.casino.dragon;
        if r.auto.on() && r.next_at.is_some_and(|t| now >= t) && !self.poker.casino.busy() && !r.running(now) {
            self.poker.casino.dragon.next_at = None;
            self.dragon_spin(s);
        }
        ctx.request_repaint_after(Duration::from_millis(16));
    }

    /// The four jackpots: the Grand in the middle, big; each at what it's worth at the bet now.
    fn dragon_jackpots(&self, p: &egui::Painter, s: &ServerView, r: Rect, time: f32) {
        let bet = self.poker.casino.dragon.bet;
        let pools = s.poker.casino.dragon.as_ref().map(|d| d.pools).unwrap_or_else(dg::seed_pools);
        let order = [Jackpot::Mini, Jackpot::Minor, Jackpot::Grand, Jackpot::Major];
        let weights = [0.8, 0.9, 1.5, 1.0];
        let total: f32 = weights.iter().sum::<f32>() + 0.06 * 3.0;
        let mut x = r.left();
        for (j, w) in order.iter().zip(weights) {
            let pw = r.width() * w / total;
            let grand = *j == Jackpot::Grand;
            let pr = Rect::from_min_size(pos2(x, r.top() + if grand { 0.0 } else { r.height() * 0.12 }), vec2(pw, if grand { r.height() } else { r.height() * 0.8 }));
            x += pw + r.width() * 0.06 / total;
            let c = jackpot_color(*j);
            p.rect_filled(pr.translate(vec2(0.0, 4.0)), theme::r(14), Color32::from_black_alpha(150));
            vgrad_round(p, pr, 14.0, super::mix(c, Color32::BLACK, 0.55), Color32::from_rgb(0x12, 0x06, 0x04));
            let glow = if grand { 0.7 + 0.3 * (time * 2.2).sin().abs() } else { 0.75 };
            p.rect_stroke(pr, theme::r(14), Stroke::new(if grand { 2.5f32 } else { 1.5 }, c.gamma_multiply(glow)), StrokeKind::Inside);
            let fs = if grand { pr.height() * 0.2 } else { pr.height() * 0.22 };
            p.text(pr.center_top() + vec2(0.0, pr.height() * 0.24), Align2::CENTER_CENTER, j.name().to_uppercase(), theme::bold_font(fs.clamp(10.0, 18.0)), c);
            let value = dg::jackpot_value(*j, pools[j.index()], bet);
            let vf = if grand { pr.height() * 0.38 } else { pr.height() * 0.34 };
            glow_text(p, pr.center() + vec2(0.0, pr.height() * 0.14), Align2::CENTER_CENTER, &chips_text(value), theme::bold_font(vf.clamp(13.0, 36.0)), col::GOLD2, c.gamma_multiply(0.5));
        }
    }

    /// The dragon on its hoard: its age, what it multiplies wins by, and how far to the next.
    fn dragon_den(&mut self, p: &egui::Painter, ctx: &egui::Context, s: &ServerView, r: Rect, now: Instant, time: f32) {
        let bet = self.poker.casino.dragon.bet;
        // (While a spin plays out, what it was as the spin began; once its eggs land, after.)
        let growth = match self.poker.casino.dragon.shown_growth {
            Some((b, g)) if b == bet => g,
            _ => Self::dragon_growth(s, bet),
        };
        let age = dg::age(growth);
        p.rect_filled(r.translate(vec2(0.0, 6.0)), theme::r(20), Color32::from_black_alpha(140));
        vgrad_round(p, r, 20.0, Color32::from_rgb(0x2a, 0x0c, 0x06), Color32::from_rgb(0x0e, 0x04, 0x02));
        p.rect_stroke(r, theme::r(20), Stroke::new(1.5f32, EMBER.gamma_multiply(0.6)), StrokeKind::Inside);
        // A growth spurt: a flash and its new name, big.
        let spurt = self.poker.casino.dragon.grew.filter(|(_, t)| now.duration_since(*t) < Duration::from_millis(2_600));
        let pic = Rect::from_center_size(pos2(r.center().x, r.top() + r.height() * 0.42), vec2(r.width() * 0.92, r.width() * 0.92).min(vec2(r.height() * 0.62, r.height() * 0.62)));
        let breathe = 1.0 + 0.015 * (time * 1.6).sin();
        radial(p, pic.center(), pic.width() * 0.6, LAVA.gamma_multiply(0.35 + 0.1 * (time * 2.0).sin()), Color32::TRANSPARENT);
        if let Some((_, t)) = spurt {
            let k = now.duration_since(t).as_secs_f32();
            radial(p, pic.center(), pic.width() * (0.4 + k * 0.5), Color32::from_rgba_unmultiplied(0xff, 0xe0, 0x90, (220.0 * (1.0 - k / 2.6).max(0.0)) as u8), Color32::TRANSPARENT);
        }
        let art = format!("dragon-age{}", age + 1);
        pics::part(p, ctx, &art, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Rect::from_center_size(pic.center(), pic.size() * breathe), (pic.width() * 1.2) as u32, Color32::WHITE);
        let (_, name, mult) = AGES[age];
        let ny = pic.bottom() + 6.0;
        glow_text(p, pos2(r.center().x, ny + 12.0), Align2::CENTER_CENTER, &name.to_uppercase(), theme::bold_font((r.width() * 0.1).clamp(15.0, 24.0)), col::GOLD2, EMBER.gamma_multiply(0.6));
        p.text(pos2(r.center().x, ny + 36.0), Align2::CENTER_CENTER, format!("Reel wins ×{mult}"), theme::bold_font(14.0), if mult > 1 { col::WIN } else { col::MUTED });
        // How far to the next age: a bar of eggs.
        let bar = Rect::from_min_size(pos2(r.left() + 18.0, ny + 54.0), vec2(r.width() - 36.0, 10.0));
        p.rect_filled(bar, theme::r(5), Color32::from_white_alpha(18));
        let (from, to) = (AGES[age].0, AGES.get(age + 1).map(|a| a.0));
        let k = to.map(|to| (growth - from) as f32 / (to - from) as f32).unwrap_or(1.0);
        p.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * k.clamp(0.0, 1.0), bar.height())), theme::r(5), EMBER);
        let words = match to {
            Some(to) => format!("{} egg{} to {}", to - growth, if to - growth == 1 { "" } else { "s" }, AGES[age + 1].1),
            None => "Fully grown: wins ×5 until its fire's spent".into(),
        };
        p.text(pos2(r.center().x, bar.bottom() + 14.0), Align2::CENTER_CENTER, words, theme::font(12.0), col::MUTED);
        if let Some((a, t)) = spurt {
            let k = now.duration_since(t).as_secs_f32();
            let pop = ease_back((k / 0.45).min(1.0));
            let fade = (1.0 - (k - 2.0).max(0.0) / 0.6).clamp(0.0, 1.0);
            glow_text(p, pic.center(), Align2::CENTER_CENTER, &format!("{}!", AGES[a].1.to_uppercase()), theme::bold_font(r.width() * 0.16 * pop.max(0.05)), Color32::WHITE.gamma_multiply(fade), EMBER.gamma_multiply(fade));
        }
    }

    /// The reels, spinning or still (the wins lit while they're shown).
    #[allow(clippy::too_many_arguments)]
    fn dragon_reels(&mut self, p: &egui::Painter, ctx: &egui::Context, s: &ServerView, reel_rects: &[Rect], reels: Rect, cell: f32, now: Instant, time: f32) {
        let _ = s;
        let r = &self.poker.casino.dragon;
        let px = (cell * 3.0) as u32;
        let frame = reels.expand(10.0);
        p.rect_filled(frame.translate(vec2(0.0, 8.0)), theme::r(18), Color32::from_black_alpha(160));
        p.rect_filled(frame, theme::r(16), Color32::from_black_alpha(215));
        for rr_ in reel_rects {
            vgrad(p, *rr_, Color32::from_rgb(0x1c, 0x08, 0x06), Color32::from_rgb(0x30, 0x10, 0x08));
        }
        let (lit, msg) = self.dragon_lit(now);
        let o = r.showing.as_ref().and_then(|rv| if let Outcome::Dragon(o) = &rv.outcome { Some(o.clone()) } else { None });
        let still = r.stopped_at.is_some_and(|e| now >= e);
        for (i, rr_) in reel_rects.iter().enumerate() {
            let clip = p.with_clip_rect(*rr_);
            let reel = r.reels[i];
            let pos = reel.pos(now);
            let base = pos.floor() as i32;
            let frac = pos - pos.floor();
            let blurred = reel.blurred(now);
            for row in -1..=3 {
                let idx = (base + row).rem_euclid(STRIP as i32) as usize;
                let sym = r.strips[i][idx];
                let y = rr_.top() + (row as f32 - frac) * cell;
                let sr = Rect::from_min_size(pos2(rr_.left(), y), vec2(cell, cell)).shrink(cell * 0.06);
                let on = lit.iter().any(|(rl, rw)| *rl == i && (row == *rw as i32)) && !blurred;
                if on {
                    let pulse = 0.5 + 0.5 * (time * 6.0).sin();
                    radial(&clip, sr.center(), cell * 0.55, EMBER.gamma_multiply(0.35 + 0.2 * pulse), Color32::TRANSPARENT);
                }
                // A fireball glows, its prize on it (once it's landed).
                if sym == dg::FIRE && !blurred {
                    radial(&clip, sr.center(), cell * 0.6, Color32::from_rgba_unmultiplied(0xff, 0x70, 0x10, 90), Color32::TRANSPARENT);
                }
                let grow = if on { 1.0 + 0.05 * (time * 6.0).sin().abs() } else { 1.0 };
                let sr = Rect::from_center_size(sr.center(), sr.size() * grow);
                if blurred {
                    pics::cell(&clip, ctx, "dragon-symbols", CELL[sym as usize], sr.translate(vec2(0.0, -cell * 0.14)), px, Color32::from_white_alpha(70));
                    pics::cell(&clip, ctx, "dragon-symbols", CELL[sym as usize], sr, px, Color32::from_white_alpha(190));
                } else {
                    let dim = if lit.is_empty() || on || sym == dg::FIRE { Color32::WHITE } else { Color32::from_gray(140) };
                    pics::cell(&clip, ctx, "dragon-symbols", CELL[sym as usize], sr, px, dim);
                }
                // The fireball's prize, once the reel's still.
                if sym == dg::FIRE
                    && (0..ROWS as i32).contains(&row)
                    && still
                    && let Some(o) = &o
                    && let Some((_, f)) = o.fires.iter().find(|(sp, _)| *sp == dg::spot(i, row as usize))
                {
                    let (text, c) = fire_label(*f, o.bet);
                    let pill = Rect::from_center_size(sr.center() + vec2(0.0, cell * 0.24), vec2(cell * 0.8, cell * 0.26));
                    clip.rect_filled(pill, theme::r(8), Color32::from_black_alpha(190));
                    clip.text(pill.center(), Align2::CENTER_CENTER, text, theme::bold_font((cell * 0.15).clamp(10.0, 20.0)), c);
                }
                if on {
                    clip.rect_stroke(sr.expand(cell * 0.03), theme::r(12), Stroke::new(2.5f32, EMBER), StrokeKind::Inside);
                }
            }
            vgrad(&clip, Rect::from_min_size(rr_.min, vec2(cell, cell * 0.3)), Color32::from_black_alpha(140), Color32::TRANSPARENT);
            vgrad(&clip, Rect::from_min_max(pos2(rr_.left(), rr_.bottom() - cell * 0.3), rr_.max), Color32::TRANSPARENT, Color32::from_black_alpha(140));
        }
        p.rect_stroke(frame, theme::r(16), Stroke::new(3.0f32, col::GOLD_DARK), StrokeKind::Inside);
        if let Some(m) = msg {
            let g = p.layout_no_wrap(m, theme::bold_font(17.0), col::GOLD2);
            let mr = Rect::from_center_size(pos2(reels.center().x, reels.bottom() - 26.0), g.size() + vec2(30.0, 14.0));
            p.rect_filled(mr, theme::r(17), Color32::from_black_alpha(215));
            p.rect_stroke(mr, theme::r(17), Stroke::new(1.0f32, EMBER.gamma_multiply(0.7)), StrokeKind::Inside);
            p.galley(mr.center() - g.size() / 2.0, g, col::GOLD2);
        }
    }

    /// The cells lit (reel, row) and the words about the win shown now.
    fn dragon_lit(&self, now: Instant) -> (Vec<(usize, usize)>, Option<String>) {
        let r = &self.poker.casino.dragon;
        let (Some(round), Some(since)) = (&r.showing, r.counted) else { return (Vec::new(), None) };
        let Outcome::Dragon(o) = &round.outcome else { return (Vec::new(), None) };
        if o.wins.is_empty() {
            return (Vec::new(), None);
        }
        let k = now.duration_since(since).as_secs_f32();
        let which = if k < 1.2 { None } else { Some(((k - 1.2) / 1.4) as usize % o.wins.len()) };
        let mut lit = Vec::new();
        for (i, w) in o.wins.iter().enumerate() {
            if which.is_some_and(|x| x != i) {
                continue;
            }
            for (reel, col_) in o.grid.iter().enumerate().take(w.count as usize) {
                for (row, s) in col_.iter().enumerate() {
                    if *s == w.symbol || *s == dg::WILD {
                        lit.push((reel, row));
                    }
                }
            }
        }
        let mult = dg::age_mult(o.growth);
        let msg = match which {
            Some(i) => {
                let w = &o.wins[i];
                let chips = (w.points * dg::POINT_TENTHOUSANDTHS * o.bet * mult + 5_000) / 10_000;
                Some(format!("{} × {} · {} way{}{} · {}", NAMES[w.symbol as usize], w.count, w.ways, if w.ways == 1 { "" } else { "s" }, if mult > 1 { format!(" · dragon ×{mult}") } else { String::new() }, chips_text(chips)))
            }
            None => Some(if mult > 1 { format!("WIN {}  (×{mult})", chips_text(o.reels_paid)) } else { format!("WIN {}", chips_text(o.reels_paid)) }),
        };
        (lit, msg)
    }

    /// Once the reels are still: the wins counted, the eggs flown to the dragon, then Ember
    /// Respins or the round's end.
    fn dragon_after_stop(&mut self, s: &ServerView, reel_rects: &[Rect], cell: f32, den: Rect, now: Instant) {
        let r = &self.poker.casino.dragon;
        let Some(end) = r.stopped_at else { return };
        if now < end || r.counted.is_some() {
            return;
        }
        let Some(round) = r.showing.clone() else { return };
        let Outcome::Dragon(o) = &round.outcome else { return };
        let ui = &mut self.poker.casino.dragon;
        ui.counted = Some(now);
        ui.win = o.reels_paid;
        // The eggs fly to the dragon, one after another.
        for (reel, col_) in o.grid.iter().enumerate() {
            for (row, sym) in col_.iter().enumerate() {
                if *sym == dg::EGG && reel < reel_rects.len() {
                    let at = reel_rects[reel].min + vec2(cell / 2.0, cell * (row as f32 + 0.5));
                    let n = ui.eggs.len() as u64;
                    ui.eggs.push((at, now + Duration::from_millis(250 + n * 120)));
                }
            }
        }
        let _ = den;
        if o.reels_paid > 0 {
            let big = o.reels_paid >= o.bet * 10;
            self.poker.casino.sound_in(0.0, if big { Cue::WinBig } else { Cue::WinSmall }, 0.0, 0.7);
        }
        match &o.respins {
            Some(rs) => {
                self.poker.casino.announce("EMBER RESPINS!", format!("{} fireballs", o.fires.len()), EMBER, (0.5, 0.42), 0.3);
                self.poker.casino.sound_in(0.3, Cue::Jackpot, 0.0, 0.7);
                let _ = rs;
                self.poker.casino.dragon.respins = Some(RespinPlay { round: round.clone(), started: now + Duration::from_millis(2_000), over_at: None, collected: false, sounded: 0 });
            }
            None => self.dragon_finish(s, &round, now),
        }
    }

    /// The eggs flying to the dragon; as the last lands, its growth shown (and a growth spurt).
    fn dragon_eggs(&mut self, p: &egui::Painter, ctx: &egui::Context, den: Rect, cell: f32, now: Instant) {
        let to = pos2(den.center().x, den.top() + den.height() * 0.42);
        let ui = &mut self.poker.casino.dragon;
        let mut landed = 0;
        for (from, at) in &ui.eggs {
            let k = now.saturating_duration_since(*at).as_secs_f32() / 0.7;
            if now < *at {
                continue;
            }
            if k >= 1.0 {
                landed += 1;
                continue;
            }
            let e = ease_out(k);
            let mid = pos2((from.x + to.x) / 2.0, from.y.min(to.y) - cell * 1.2);
            // (Along a curve: from the reel, up, and down onto the hoard.)
            let a = *from + (mid - *from) * e;
            let b = mid + (to - mid) * e;
            let q = a + (b - a) * e;
            let size = cell * (0.6 - 0.25 * e);
            pics::cell(p, ctx, "dragon-symbols", CELL[dg::EGG as usize], Rect::from_center_size(q, vec2(size, size)), (cell * 3.0) as u32, Color32::WHITE);
        }
        if !ui.eggs.is_empty() && landed == ui.eggs.len() {
            ui.eggs.clear();
            if let Some(round) = &ui.showing
                && let Outcome::Dragon(o) = &round.outcome
                && o.respins.is_none()
            {
                let before = dg::age(o.growth);
                ui.shown_growth = Some((o.bet, o.growth_after));
                let after = dg::age(o.growth_after);
                if after > before {
                    ui.grew = Some((after, now));
                    self.poker.casino.sound_in(0.0, Cue::WinBig, 0.0, 0.8);
                }
            }
        }
    }

    /// Ember Respins on the board: the held fireballs, the rest spinning, the respins left, the
    /// board's total, then the win to collect.
    #[allow(clippy::too_many_arguments)]
    fn dragon_respins(&mut self, ui: &mut egui::Ui, s: &ServerView, reels: Rect, reel_rects: &[Rect], cell: f32, now: Instant, time: f32) {
        let p = ui.painter().clone();
        let ctx = ui.ctx().clone();
        let Some(play) = self.poker.casino.dragon.respins.as_ref() else { return };
        let Outcome::Dragon(o) = play.round.outcome.clone() else { return };
        let Some(r) = o.respins.clone() else { return };
        let k = now.duration_since(play.started).as_secs_f32();
        // How far it's got: respins shown, and the board then.
        let shown = ((k / RESPIN_S) as usize).min(r.spins.len());
        let step_k = (k / RESPIN_S).fract();
        let done = shown >= r.spins.len();
        let mut held: Vec<(u8, Fire, Option<f32>)> = o.fires.iter().map(|(sp, f)| (*sp, *f, None)).collect();
        let mut left = dg::RESPINS;
        for (n, caught) in r.spins.iter().enumerate().take(shown) {
            if caught.is_empty() {
                left = left.saturating_sub(1);
            } else {
                left = dg::RESPINS;
                // (How long ago it landed, for its burst.)
                let ago = k - (n as f32 + 1.0) * RESPIN_S;
                held.extend(caught.iter().map(|(sp, f)| (*sp, *f, Some(ago))));
            }
        }
        // Sounds: a thump for each fireball that lands, a tick for a respin gone.
        let sounded = play.sounded;
        if shown > sounded {
            for caught in &r.spins[sounded..shown] {
                self.poker.casino.sound_in(0.0, if caught.is_empty() { Cue::Tick } else { Cue::Coin }, 0.0, 0.7);
            }
            if let Some(pl) = self.poker.casino.dragon.respins.as_mut() {
                pl.sounded = shown;
            }
        }
        let board: u64 = held.iter().map(|(_, f, _)| f.x()).sum::<u64>() * o.bet * if o.super_respins { dg::SUPER_X } else { 1 };
        // The board.
        let frame = reels.expand(10.0);
        p.rect_filled(frame.translate(vec2(0.0, 8.0)), theme::r(18), Color32::from_black_alpha(160));
        vgrad_round(&p, frame, 16.0, Color32::from_rgb(0x30, 0x08, 0x04), Color32::from_rgb(0x12, 0x03, 0x02));
        p.rect_stroke(frame, theme::r(16), Stroke::new(3.0f32, EMBER.gamma_multiply(0.8 + 0.2 * (time * 3.0).sin())), StrokeKind::Inside);
        let px = (cell * 3.0) as u32;
        for (reel, rr_) in reel_rects.iter().enumerate() {
            for row in 0..ROWS {
                let sp = dg::spot(reel, row);
                let sr = Rect::from_min_size(pos2(rr_.left(), rr_.top() + row as f32 * cell), vec2(cell, cell)).shrink(cell * 0.05);
                match held.iter().find(|h| h.0 == sp) {
                    Some((_, f, ago)) => {
                        let pop = ago.map(|a| ease_back((a / 0.35).clamp(0.0, 1.0))).unwrap_or(1.0);
                        radial(&p, sr.center(), cell * 0.62, Color32::from_rgba_unmultiplied(0xff, 0x80, 0x18, 120), Color32::TRANSPARENT);
                        if let Some(a) = ago
                            && *a < 0.6
                        {
                            radial(&p, sr.center(), cell * (0.5 + a * 1.5), Color32::from_rgba_unmultiplied(0xff, 0xe0, 0x90, (200.0 * (1.0 - a / 0.6)) as u8), Color32::TRANSPARENT);
                        }
                        let fr = Rect::from_center_size(sr.center(), sr.size() * pop.max(0.05));
                        pics::cell(&p, &ctx, "dragon-symbols", CELL[dg::FIRE as usize], fr, px, Color32::WHITE);
                        let (text, c) = fire_label(*f, if o.super_respins { o.bet * dg::SUPER_X } else { o.bet });
                        let pill = Rect::from_center_size(sr.center() + vec2(0.0, cell * 0.24), vec2(cell * 0.86, cell * 0.28));
                        p.rect_filled(pill, theme::r(8), Color32::from_black_alpha(200));
                        p.text(pill.center(), Align2::CENTER_CENTER, text, theme::bold_font((cell * 0.16).clamp(10.0, 22.0)), c);
                    }
                    None => {
                        // An empty spot: dark, embers flickering while it spins.
                        p.rect_filled(sr, theme::r(10), Color32::from_rgb(0x16, 0x05, 0x03));
                        p.rect_stroke(sr, theme::r(10), Stroke::new(1.0f32, EMBER.gamma_multiply(0.25)), StrokeKind::Inside);
                        if !done {
                            let flick = ((time * 17.0 + sp as f32 * 1.7).sin() * 0.5 + 0.5) * (1.0 - step_k * 0.4);
                            radial(&p, sr.center(), cell * 0.3, Color32::from_rgba_unmultiplied(0xff, 0x60, 0x10, (70.0 * flick) as u8), Color32::TRANSPARENT);
                        }
                    }
                }
            }
        }
        // Over the board: the respins left, the board's total, the spots filled.
        let top = frame.top() - 4.0;
        let hdr = Rect::from_center_size(pos2(frame.center().x, top - 22.0), vec2(frame.width().min(560.0), 40.0));
        p.rect_filled(hdr, theme::r(20), Color32::from_black_alpha(220));
        p.rect_stroke(hdr, theme::r(20), Stroke::new(1.5f32, EMBER), StrokeKind::Inside);
        p.text(hdr.left_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, if o.super_respins { "SUPER ×3" } else { "RESPINS" }, theme::bold_font(12.5), if o.super_respins { col::GOLD2 } else { col::MUTED });
        for i in 0..dg::RESPINS {
            let c = hdr.left_center() + vec2(90.0 + i as f32 * 22.0, 0.0);
            let on = i < left;
            p.circle_filled(c, 7.5, if on { EMBER } else { Color32::from_white_alpha(25) });
        }
        glow_text(&p, hdr.center() + vec2(30.0, 0.0), Align2::CENTER_CENTER, &format!("BOARD {}", chips_text(board)), theme::bold_font(18.0), col::GOLD2, EMBER.gamma_multiply(0.5));
        p.text(hdr.right_center() - vec2(16.0, 0.0), Align2::RIGHT_CENTER, format!("SPOTS {}/15", held.len()), theme::bold_font(14.0), Color32::WHITE);
        if done {
            self.dragon_collect(ui, s, &o, &r, reels, now);
        }
    }

    /// Respins over: the jackpots called out, the total, Collect.
    fn dragon_collect(&mut self, ui: &mut egui::Ui, s: &ServerView, o: &dg::Outcome, r: &dg::Respins, reels: Rect, now: Instant) {
        let p = ui.painter().clone();
        let Some(play) = self.poker.casino.dragon.respins.as_mut() else { return };
        let first = play.over_at.is_none();
        let over_at = *play.over_at.get_or_insert(now);
        if play.collected {
            return;
        }
        if first {
            let jackpots = o.jackpots();
            let best = jackpots.iter().max().copied();
            let (title, color) = match (r.full, best) {
                (true, _) => ("GRAND JACKPOT!".to_string(), jackpot_color(Jackpot::Grand)),
                (false, Some(j)) => (format!("{} JACKPOT!", j.name().to_uppercase()), jackpot_color(j)),
                (false, None) => ("EMBER RESPINS".to_string(), EMBER),
            };
            self.poker.casino.announce(title, format!("{} fireballs · {}", r.board.len(), chips_text(r.paid)), color, (0.5, 0.42), 0.2);
            self.poker.casino.sound_in(0.2, if best.is_some() || r.full { Cue::Jackpot } else { Cue::WinBig }, 0.0, 0.9);
        }
        let k = now.duration_since(over_at).as_secs_f32();
        let total = o.reels_paid + r.paid;
        let shown = (total as f64 * ease_out((k / 1.6).min(1.0)) as f64) as u64;
        let br = Rect::from_center_size(pos2(reels.center().x, reels.bottom() + 6.0), vec2(300.0, 52.0));
        let resp = ui.interact(br, ui.id().with("dragon-collect"), Sense::click());
        p.rect_filled(br.translate(vec2(0.0, 4.0)), theme::r(26), Color32::from_black_alpha(140));
        p.rect_filled(br, theme::r(26), if resp.hovered() { col::GOLD2 } else { col::GOLD });
        p.text(br.center(), Align2::CENTER_CENTER, format!("COLLECT {}", chips_text(shown)), theme::bold_font(19.0), Color32::from_rgb(0x2a, 0x10, 0x04));
        let auto = self.poker.casino.dragon.auto.on();
        if resp.clicked() || (auto && k > 2.4) {
            let round = self.poker.casino.dragon.respins.as_ref().map(|pl| pl.round.clone());
            let d = &mut self.poker.casino.dragon;
            d.respins = None;
            d.win = total;
            // (Its fire's spent: back to an egg.)
            d.shown_growth = Some((o.bet, o.growth_after));
            if let Some(round) = round {
                self.dragon_finish(s, &round, now);
            }
        }
    }

    /// The round's over on screen: its win on the balance, a celebration if it's big, and the
    /// next spin if it's spinning by itself.
    fn dragon_finish(&mut self, _s: &ServerView, round: &RoundView, now: Instant) {
        let Outcome::Dragon(o) = &round.outcome else { return };
        self.poker.casino.settle_balance();
        self.poker.casino.celebrate(o.paid, o.wagered(), !o.jackpots().is_empty(), 0.2);
        let big = o.paid >= o.wagered() * 10;
        let ui = &mut self.poker.casino.dragon;
        if ui.auto.spun(big) {
            let pause = if o.paid > 0 { 1.4 } else { 0.5 };
            ui.next_at = Some(now + Duration::from_secs_f32(pause * if ui.turbo { 0.5 } else { 1.0 }));
        }
    }

    fn dragon_controls(&mut self, ui: &mut egui::Ui, s: &ServerView, bar: Rect, now: Instant) {
        let p = ui.painter().clone();
        vgrad_round(&p, bar, 18.0, Color32::from_rgb(0x1e, 0x0a, 0x06), Color32::from_rgb(0x0c, 0x04, 0x02));
        p.rect_stroke(bar, theme::r(18), Stroke::new(1.0f32, col::GOLD_DARK), StrokeKind::Inside);
        let cy = bar.center().y;
        let busy = self.poker.casino.busy() || self.poker.casino.dragon.running(now);
        let bet = self.poker.casino.dragon.bet;
        p.text(pos2(bar.left() + 22.0, bar.top() + 18.0), Align2::LEFT_CENTER, "BET", theme::bold_font(11.5), col::MUTED);
        let minus = Rect::from_center_size(pos2(bar.left() + 36.0, cy + 10.0), vec2(32.0, 32.0));
        let plus = Rect::from_center_size(pos2(bar.left() + 150.0, cy + 10.0), vec2(32.0, 32.0));
        for (r, label, dir) in [(minus, "−", -1i32), (plus, "+", 1)] {
            let resp = ui.interact(r, ui.id().with(("dbet", label)), Sense::click());
            p.circle_filled(r.center(), 16.0, if resp.hovered() && !busy { col::GOLD } else { col::GOLD_DARK });
            p.text(r.center(), Align2::CENTER_CENTER, label, theme::bold_font(19.0), Color32::BLACK);
            if resp.clicked() && !busy {
                let i = BETS.iter().position(|x| *x == bet).unwrap_or(2) as i32;
                self.poker.casino.dragon.bet = BETS[(i + dir).clamp(0, BETS.len() as i32 - 1) as usize];
                self.poker.casino.dragon.shown_growth = None;
            }
        }
        let amount = Rect::from_center_size(pos2(bar.left() + 93.0, cy + 10.0), vec2(76.0, 30.0));
        let aresp = ui.interact(amount, ui.id().with("dbet-pick"), Sense::click()).on_hover_text("Choose the bet (each bet has its own dragon)");
        p.text(amount.center(), Align2::CENTER_CENTER, chips_text(bet), theme::bold_font(20.0), col::GOLD2);
        if !busy {
            egui::Popup::menu(&aresp).align(egui::RectAlign::TOP_START).show(|ui| {
                crate::menu::style(ui);
                for b in BETS {
                    let g = Self::dragon_growth(s, b);
                    if ui.button(format!("{} chips  ·  {} dragon", chips_text(b), AGES[dg::age(g)].1)).clicked() {
                        self.poker.casino.dragon.bet = b;
                        self.poker.casino.dragon.shown_growth = None;
                        ui.close();
                    }
                }
            });
        }
        let win = self.poker.casino.dragon.win;
        p.text(pos2(bar.left() + 190.0, bar.top() + 18.0), Align2::LEFT_CENTER, "WIN", theme::bold_font(11.5), col::MUTED);
        p.text(pos2(bar.left() + 190.0, cy + 10.0), Align2::LEFT_CENTER, if win > 0 { chips_text(win) } else { "–".into() }, theme::bold_font(22.0), if win > 0 { col::WIN } else { col::MUTED });
        let auto_now = self.poker.casino.dragon.auto;
        let auto_text = if auto_now.on() { format!("Auto {}", auto_now.left_text()) } else { "Auto ⌄".into() };
        let buttons_w: f32 = ["Turbo".to_string(), auto_text, "Info".into()].iter().map(|t| 20.0 + t.chars().count() as f32 * 7.6 + 8.0).sum();
        let left_of_buttons = bar.right() - 16.0 - buttons_w;
        let spin_c = pos2(((bar.left() + 310.0 + left_of_buttons) / 2.0).max(bar.left() + 350.0), cy);
        let spin_r = Rect::from_center_size(spin_c, vec2(80.0, 80.0));
        let resp = ui.interact(spin_r, ui.id().with("dspin"), Sense::click());
        let auto = self.poker.casino.dragon.auto;
        p.circle_filled(spin_c + vec2(0.0, 4.0), 40.0, Color32::from_black_alpha(140));
        radial(&p, spin_c, 40.0, if resp.hovered() && !busy { Color32::from_rgb(0xff, 0x8a, 0x2a) } else { EMBER }, Color32::from_rgb(0x5a, 0x10, 0x04));
        p.circle_stroke(spin_c, 40.0, Stroke::new(3.0f32, col::GOLD2));
        p.text(spin_c - vec2(0.0, if auto.on() { 6.0 } else { 0.0 }), Align2::CENTER_CENTER, if auto.on() { "STOP" } else { "SPIN" }, theme::bold_font(20.0), Color32::from_rgb(0x1a, 0x06, 0x02));
        if auto.on() {
            p.text(spin_c + vec2(0.0, 15.0), Align2::CENTER_CENTER, format!("auto {}", auto.left_text()), theme::bold_font(10.5), Color32::from_rgb(0x1a, 0x06, 0x02));
        }
        if let Some(f) = self.poker.casino.dragon.stake.armed {
            let cost = pyr_poker::casino::feature::cost(bet, dg::price(f).unwrap_or(100));
            p.text(spin_c + vec2(0.0, 50.0), Align2::CENTER_CENTER, format!("{} · {}", SlotKind::Dragon.describe(f).0, chips_text(cost)), theme::bold_font(11.0), col::GOLD2);
        }
        let space = ui.ctx().input(|i| i.key_pressed(egui::Key::Space)) && !ui.ctx().wants_keyboard_input() && self.poker.casino.info.is_none() && !self.poker.casino.dragon.stake.menu;
        if resp.on_hover_text("Spin (Space)").clicked() || space {
            if auto.on() {
                self.poker.casino.dragon.auto.stop();
            } else if !busy {
                self.dragon_spin(s);
            }
        }
        let mut bx = bar.right() - 16.0;
        let mut small = |ui: &mut egui::Ui, id: &str, text: &str, on: bool, tip: &str| -> egui::Response {
            let w = 20.0 + text.chars().count() as f32 * 7.6;
            let r = Rect::from_min_size(pos2(bx - w, cy - 15.0), vec2(w, 30.0));
            bx -= w + 8.0;
            let resp = ui.interact(r, ui.id().with(("dbtn", id)), Sense::click());
            ui.painter().rect_filled(r, theme::r(15), if on { col::GOLD } else if resp.hovered() { Color32::from_white_alpha(40) } else { Color32::from_white_alpha(18) });
            ui.painter().text(r.center(), Align2::CENTER_CENTER, text, theme::bold_font(12.0), if on { Color32::BLACK } else { col::TEXT });
            resp.on_hover_text(tip)
        };
        if small(ui, "turbo", "Turbo", self.poker.casino.dragon.turbo, "Faster spins").clicked() {
            self.poker.casino.dragon.turbo = !self.poker.casino.dragon.turbo;
        }
        let auto_label = if auto.on() { format!("Auto {}", auto.left_text()) } else { "Auto ⌄".into() };
        let resp = small(ui, "auto", &auto_label, auto.on(), if auto.on() { "Stop spinning by itself" } else { "Spin by itself: 10 to 1,000 times, or until you stop it" });
        if auto.on() {
            if resp.clicked() {
                self.poker.casino.dragon.auto.stop();
            }
        } else if let Some(n) = super::auto::menu(&resp, &mut self.poker.casino.dragon.auto) {
            self.poker.casino.dragon.auto.left = n;
            if !busy {
                self.poker.casino.dragon.next_at = Some(Instant::now());
            }
        }
        if small(ui, "info", "Info", self.poker.casino.info.is_some(), "What it pays, its statistics and its story").clicked() {
            self.info_toggle();
        }
        if super::chips_now(s) < bet {
            p.text(pos2(bar.center().x, bar.bottom() - 8.0), Align2::CENTER_CENTER, "Not enough chips for this bet: lower it, or take the daily chips in the lobby.", theme::font(11.0), col::LOSE);
        }
    }
}
