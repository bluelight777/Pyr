//! The funny machines' gimmicks, each its own (see `reel_themes::Humor`): Scoville Slots' heat
//! meter, Sugar Rush's dental report, The Daily Grind's caffeine and office email, and
//! Frostbite Fortune's fish stories. All of it is for show: rounds are played by the house the
//! same as on any machine, and the jokes are picked from each round's number, so a round says
//! the same thing whenever it's shown.

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, Pos2, Rect, Shape, Stroke, StrokeKind, pos2, vec2};
use pyr_poker::casino::reels::Outcome;

use super::reel_themes::{Humor, Theme};
use super::{col, pics, radial, vgrad_round};
use crate::poker::cards::chips_text;
use crate::theme;

/// Picks one of `xs` by `n` (a round's number, mixed so neighbors differ).
fn pick<'a>(xs: &[&'a str], n: u64) -> &'a str {
    let h = n.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(17);
    xs[(h % xs.len() as u64) as usize]
}

const HEAT_NAMES: [&str; 6] = ["MILD", "MEDIUM", "HOT", "EXTRA HOT", "INFERNO", "SCOVILLED!"];
const HEAT_UP: [&str; 8] = [
    "Mild. Disappointingly mild.",
    "That spin had the heat of a bell pepper.",
    "The reels are getting spicy. Your eyebrows are worried.",
    "Is it warm in here, or is it the reels?",
    "Sweating? No. That's flavor.",
    "Somebody open a window.",
    "Your tongue has filed a complaint.",
    "Steam coming out of the ears is normal. Probably.",
];
const HEAT_DOWN: [&str; 5] = ["Ahh. A sip of milk.", "Cooling off nicely.", "The burn subsides. For now.", "That win came with a yogurt dip.", "Phew. Back to medium."];
const HEAT_BIG: [&str; 3] = ["The fire department called it in. All clear!", "So hot it went cold. Science!", "A win so big the pepper fainted."];
const BOILED: [&str; 3] = ["SCOVILLED! No chips were harmed.", "SCOVILLED! The reels are fine. The pepper isn't.", "SCOVILLED! Somebody bring the whole cow."];

/// Grams of sugar each of Sugar Rush's symbols lands (with a straight face).
const GRAMS: [u64; 7] = [4, 11, 32, 47, 52, 118, 61];
const MEMOS: [(u64, &str); 7] = [
    (250, "Your dentist just smiled for no reason."),
    (750, "Cavity #1. Your dentist is picking out a new boat."),
    (1_500, "The tooth fairy has asked for a meeting."),
    (3_000, "Your toothbrush has filed for divorce."),
    (6_000, "Achievement unlocked: Root Canal."),
    (10_000, "Your dentist has named their yacht after you."),
    (20_000, "Scientists would like to study your teeth."),
];

const EMAILS: [(&str, &str); 16] = [
    ("Management", "Re: Re: Re: Re: Synergy"),
    ("Calendar", "Meeting to plan the next meeting · 4:59 PM"),
    ("IT", "Have you tried turning the slot machine off and on again?"),
    ("HR", "Reminder: the coffee machine is not a team member."),
    ("Accounting", "Your expense report for \"beans (many)\" was declined."),
    ("Kitchen", "Whoever keeps taking the oat milk: we know."),
    ("Your boss", "Quick question (it is not quick)"),
    ("Facilities", "Casual Friday has been moved to never."),
    ("Printer", "I'm jammed. On purpose."),
    ("Project lead", "Let's circle back on the circle back."),
    ("Management", "Mandatory fun is now mandatory."),
    ("Calendar", "Your 1:1 with yourself was rescheduled."),
    ("IT", "Your password has expired. So has your patience."),
    ("Team", "Who microwaved fish? (It was the penguin.)"),
    ("HR", "Please stop calling the espresso machine \"Mom\"."),
    ("CEO", "Exciting news! (It isn't.)"),
];

const SMALL_FISH: [&str; 6] = ["a shy Minnow", "a modest Sardine", "a nervous Smelt", "a bashful Anchovy", "a polite Guppy", "a tiny Tadpole (close enough)"];
const MID_FISH: [&str; 5] = ["a respectable Perch", "a smug Trout", "a well-dressed Bass", "a suspicious Walleye", "a Catfish with opinions"];
const BIG_FISH: [&str; 5] = ["a MIGHTY Salmon", "an unhinged Pike", "a Muskie that bit back", "a Sturgeon older than the lake", "a Halibut of Unusual Size"];
const HUGE_FISH: [&str; 4] = ["THE LEGENDARY GOLDEN COD", "THE HERRING OF DOOM", "A FISH NAMED DAVE", "THE KRAKEN'S LITTLE COUSIN"];
const SMALL_SIZE: [&str; 4] = ["3 inches, generously", "about a thumb", "fits on a cracker", "smaller than the bait"];
const MID_SIZE: [&str; 4] = ["2 feet, give or take", "a solid forearm", "a whole bucket's worth", "it was THIS big"];
const BIG_SIZE: [&str; 4] = ["6 feet. The ice cracked.", "bigger than the penguin", "it needed its own chair", "the photo didn't fit"];
const HUGE_SIZE: [&str; 4] = ["bigger than the boat (there is no boat)", "visible from space", "it caught the penguin first", "the lake is now a puddle"];
const GOT_AWAY: [&str; 5] = ["...the one that got away.", "Nothing. The fish are on a lunch break.", "A boot. Not even a nice boot.", "The penguin slipped again.", "Brr. Nothing biting."];

/// Everything the four gimmicks keep between rounds (this session, while Pyr is open).
#[derive(Default)]
pub struct Fun {
    /// Scoville Slots: the heat (0 to 5) and as it's drawn (eased), when it boiled over, and
    /// what it last said.
    heat: u8,
    heat_shown: f32,
    boiled: Option<Instant>,
    /// Sugar Rush: grams of sugar landed, memos already shown.
    grams: u64,
    grams_shown: f64,
    memos_seen: usize,
    /// The Daily Grind: caffeine (0 to 1) and as drawn, spins since the last email, the email.
    caffeine: Option<f32>,
    caffeine_shown: f32,
    espresso: Option<Instant>,
    since_email: u32,
    email: Option<(usize, Instant)>,
    /// Frostbite Fortune: the three tallest tales so far (times the bet ×100, the tale).
    tales: Vec<(u64, String)>,
    slipped: Option<Instant>,
    /// What's being said, since when (any of them).
    caption: Option<(String, Color32, Instant)>,
}

impl Fun {
    fn say(&mut self, text: impl Into<String>, color: Color32) {
        self.caption = Some((text.into(), color, Instant::now()));
    }

    /// The Daily Grind's reels are slower when there's no caffeine left (it's just for show:
    /// where they stop is the house's).
    pub fn reel_speed(&self, humor: Option<Humor>) -> f32 {
        if humor == Some(Humor::Office) && self.caffeine.unwrap_or(1.0) < 0.15 { 0.62 } else { 1.0 }
    }

    /// A round's over on screen: each gimmick takes it in.
    pub fn round(&mut self, humor: Option<Humor>, o: &Outcome, nonce: u64) {
        let Some(h) = humor else { return };
        let mult = pyr_poker::casino::multiple_x100(o.paid, o.wagered());
        match h {
            Humor::Heat => {
                if o.paid == 0 {
                    self.heat += 1;
                    if self.heat >= 5 {
                        self.boiled = Some(Instant::now());
                        self.heat = 2;
                        self.say(pick(&BOILED, nonce), Color32::from_rgb(0xff, 0x6a, 0x2a));
                    } else {
                        self.say(pick(&HEAT_UP, nonce), col::GOLD2);
                    }
                } else if mult >= 1_000 {
                    self.heat = 0;
                    self.say(pick(&HEAT_BIG, nonce), Color32::from_rgb(0x9a, 0xe0, 0xff));
                } else {
                    self.heat = self.heat.saturating_sub(2);
                    self.say(pick(&HEAT_DOWN, nonce), Color32::from_rgb(0xbf, 0xe8, 0xff));
                }
            }
            Humor::Dentist => {
                let landed: u64 = o.grid.map(|g| g.iter().flatten().filter(|&&s| (s as usize) < GRAMS.len()).map(|&s| GRAMS[s as usize]).sum()).unwrap_or(0);
                // (Wins taste sweeter: double.)
                let add = if o.paid > 0 { landed * 2 } else { landed };
                self.grams += add;
                if let Some((g, memo)) = MEMOS.get(self.memos_seen).copied()
                    && self.grams >= g
                {
                    self.memos_seen += 1;
                    self.say(format!("Dentist's note: {memo}"), Color32::from_rgb(0xff, 0xd1, 0xe8));
                } else if add > 0 {
                    self.say(format!("+{} g of sugar", chips_text(add)), Color32::from_rgb(0xff, 0xd1, 0xe8));
                }
            }
            Humor::Office => {
                let c = self.caffeine.unwrap_or(1.0);
                let c = if mult >= 1_000 {
                    self.espresso = Some(Instant::now());
                    self.say("ESPRESSO SHOT! Productivity +400%", col::GOLD2);
                    1.0
                } else if o.paid > 0 {
                    (c + 0.15).min(1.0)
                } else {
                    (c - 0.07).max(0.0)
                };
                if c < 0.15 && self.caffeine.unwrap_or(1.0) >= 0.15 {
                    self.say("MONDAY MODE: the reels are running on decaf.", Color32::from_rgb(0xc8, 0xb0, 0x90));
                }
                self.caffeine = Some(c);
                self.since_email += 1;
                // An email every six to nine spins.
                if self.since_email >= 6 + (nonce % 4) as u32 {
                    self.since_email = 0;
                    let h = nonce.wrapping_mul(0x9e37_79b9_7f4a_7c15).rotate_left(23);
                    self.email = Some(((h % EMAILS.len() as u64) as usize, Instant::now()));
                }
            }
            Humor::FishTales => {
                if o.paid == 0 {
                    self.slipped = Some(Instant::now());
                    if nonce % 3 == 0 {
                        self.say(pick(&GOT_AWAY, nonce), Color32::from_rgb(0xbf, 0xe8, 0xff));
                    }
                    return;
                }
                let (fish, size) = match mult {
                    0..=99 => (pick(&SMALL_FISH, nonce), pick(&SMALL_SIZE, nonce >> 3)),
                    100..=499 => (pick(&MID_FISH, nonce), pick(&MID_SIZE, nonce >> 3)),
                    500..=1_999 => (pick(&BIG_FISH, nonce), pick(&BIG_SIZE, nonce >> 3)),
                    _ => (pick(&HUGE_FISH, nonce), pick(&HUGE_SIZE, nonce >> 3)),
                };
                let tale = format!("You caught {fish}: {size}.");
                self.tales.push((mult, format!("{fish} · {}", chips_text(o.paid))));
                self.tales.sort_by(|a, b| b.0.cmp(&a.0));
                self.tales.truncate(3);
                self.say(tale, if mult >= 2_000 { col::GOLD2 } else { Color32::from_rgb(0xbf, 0xe8, 0xff) });
            }
        }
    }

    /// The host's pose: pleased (false) or dismayed (true), for this machine now.
    fn dismayed(&self, h: Humor, now: Instant) -> bool {
        match h {
            Humor::Heat => self.heat >= 3 || self.boiled.is_some_and(|t| now.duration_since(t) < Duration::from_secs(3)),
            Humor::Dentist => self.memos_seen >= 2,
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
        // The Daily Grind: an email, sliding in at the bottom of the reels.
        if h == Humor::Office
            && let Some((i, since)) = self.email
        {
            let k = now.duration_since(since).as_secs_f32();
            if k < 5.0 {
                let slide = (k / 0.35).min(1.0).min((5.0 - k) / 0.35);
                let (from, subject) = EMAILS[i];
                let w = (reels.width() * 0.62).clamp(260.0, 460.0);
                let r = Rect::from_min_size(pos2(reels.right() - w + (1.0 - slide) * (w + 40.0), reels.bottom() - 72.0), vec2(w, 60.0));
                p.rect_filled(r.translate(vec2(0.0, 4.0)), theme::r(10), Color32::from_black_alpha(120));
                p.rect_filled(r, theme::r(10), Color32::from_rgb(0xf6, 0xf2, 0xea));
                p.rect_filled(Rect::from_min_size(r.min, vec2(6.0, r.height())), theme::r(3), Color32::from_rgb(0x2f, 0x6f, 0xe0));
                p.text(r.left_top() + vec2(18.0, 10.0), Align2::LEFT_TOP, format!("✉  {from}"), theme::bold_font(13.0), Color32::from_rgb(0x20, 0x24, 0x30));
                let g = p.layout(subject.to_string(), theme::font(12.5), Color32::from_rgb(0x3a, 0x40, 0x50), r.width() - 30.0);
                p.galley(r.left_top() + vec2(18.0, 31.0), g, Color32::from_rgb(0x3a, 0x40, 0x50));
            }
        }
        // What's being said, over the top of the reels.
        if let Some((text, color, since)) = &self.caption {
            let k = now.duration_since(*since).as_secs_f32();
            let hold = 3.2 + text.len() as f32 * 0.03;
            if k < hold {
                let a = (k / 0.2).min(1.0) * ((hold - k) / 0.4).clamp(0.0, 1.0);
                let pop = 1.0 + (1.0 - (k / 0.25).min(1.0)) * 0.12;
                let font = theme::bold_font(17.0 * pop);
                let g = p.layout(text.clone(), font, color.gamma_multiply(a), (reels.width() - 40.0).max(160.0));
                let r = Rect::from_center_size(pos2(reels.center().x, reels.top() + 26.0 + g.size().y / 2.0), g.size() + vec2(28.0, 16.0));
                p.rect_filled(r, theme::r(14), Color32::from_black_alpha((200.0 * a) as u8));
                p.rect_stroke(r, theme::r(14), Stroke::new(1.5f32, t.accent.gamma_multiply(0.7 * a)), StrokeKind::Inside);
                p.galley(r.center() - g.size() / 2.0, g, color.gamma_multiply(a));
            } else {
                self.caption = None;
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
        p.text(paper.left_top() + vec2(10.0, 10.0), Align2::LEFT_TOP, "DENTAL REPORT", theme::bold_font(13.0), Color32::from_rgb(0xc8, 0x2a, 0x5a));
        let grams = self.grams_shown.round() as u64;
        p.text(paper.left_top() + vec2(10.0, 32.0), Align2::LEFT_TOP, "Sugar this visit:", theme::font(12.0), ink);
        p.text(paper.left_top() + vec2(10.0, 48.0), Align2::LEFT_TOP, format!("{} g", chips_text(grams)), theme::bold_font(20.0), ink);
        p.text(paper.left_top() + vec2(10.0, 74.0), Align2::LEFT_TOP, format!("(about {} sugar cubes)", chips_text(grams / 4)), theme::font(11.5), Color32::from_rgb(0x6a, 0x6a, 0x7a));
        let cavities = self.grams / 750;
        p.text(paper.left_top() + vec2(10.0, 96.0), Align2::LEFT_TOP, format!("Cavities: {cavities}"), theme::bold_font(13.0), if cavities > 0 { Color32::from_rgb(0xc8, 0x2a, 0x3a) } else { ink });
        let advice = match cavities {
            0 => "Floss daily. Please.",
            1..=2 => "Floss. Please.",
            3..=7 => "We need to talk.",
            _ => "There's no saving it.",
        };
        if paper.height() > 132.0 {
            p.text(paper.left_top() + vec2(10.0, 120.0), Align2::LEFT_TOP, advice, theme::font(12.0), Color32::from_rgb(0x6a, 0x6a, 0x7a));
        }
    }

    /// The Daily Grind: the caffeine level, the reels nodding off without it.
    fn caffeine_meter(&mut self, p: &egui::Painter, r: Rect, reels: Rect, now: Instant, time: f32) {
        let c = self.caffeine.unwrap_or(1.0);
        self.caffeine_shown += (c - self.caffeine_shown) * 0.08;
        let mug = Rect::from_center_size(pos2(r.left() + 52.0, r.center().y), vec2(70.0, (r.height() - 40.0).min(120.0)));
        p.rect_filled(mug, theme::r(10), Color32::from_rgb(0xf4, 0xee, 0xe2));
        let coffee = Rect::from_min_max(pos2(mug.left() + 6.0, mug.bottom() - 6.0 - (mug.height() - 12.0) * self.caffeine_shown), mug.max - vec2(6.0, 6.0));
        p.rect_filled(coffee, theme::r(6), Color32::from_rgb(0x5a, 0x32, 0x16));
        p.circle_stroke(mug.right_center() + vec2(10.0, 0.0), 14.0, Stroke::new(6.0f32, Color32::from_rgb(0xf4, 0xee, 0xe2)));
        p.text(pos2(mug.right() + 34.0, mug.top() + 8.0), Align2::LEFT_TOP, "CAFFEINE", theme::bold_font(12.0), col::MUTED);
        p.text(pos2(mug.right() + 34.0, mug.top() + 26.0), Align2::LEFT_TOP, format!("{:.0}%", c * 100.0), theme::bold_font(22.0), if c < 0.15 { Color32::from_rgb(0xc8, 0x6a, 0x4a) } else { col::GOLD2 });
        let state = if self.espresso.is_some_and(|t| now.duration_since(t) < Duration::from_secs(3)) {
            "ESPRESSO!"
        } else if c < 0.15 {
            "Monday mode"
        } else if c < 0.5 {
            "Needs a refill"
        } else {
            "Fully caffeinated"
        };
        p.text(pos2(mug.right() + 34.0, mug.top() + 56.0), Align2::LEFT_TOP, state, theme::font(12.0), col::MUTED);
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
        p.text(pos2(board.center().x, board.top() + 16.0), Align2::CENTER_CENTER, "TROPHY WALL", theme::bold_font(13.0), Color32::from_rgb(0xff, 0xe2, 0x9a));
        if self.tales.is_empty() {
            let g = p.layout("Nothing mounted yet. Every win is a fish story.".into(), theme::font(12.0), Color32::from_rgb(0xe8, 0xd8, 0xc0), board.width() - 24.0);
            p.galley(pos2(board.left() + 12.0, board.top() + 36.0), g, Color32::WHITE);
            return;
        }
        for (i, (_, tale)) in self.tales.iter().enumerate() {
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

