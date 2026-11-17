//! The pages: the side's list of them, the front page, a category's page with each game's
//! props together, and the pieces they share (a prop as a small card, an outcome as a button).
//! A prop's own page, my bets, the best at props and the windows are in `pages`.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Rect, RichText, Sense, Shape, Stroke, pos2, vec2};
use pyr_poker::props::{self as p, PropState, PropView};

use crate::look::*;
use crate::model::{self, Model};
use crate::words::{self, chips_text, count, countdown, local_when, mid_sentence, pct, short_chips};
use crate::{App, Betting, Setting, View, note};

/// A card's frame.
pub fn card() -> egui::Frame {
    egui::Frame::new().fill(CARD).stroke(Stroke::new(1.0_f32, CARD_EDGE)).corner_radius(12).inner_margin(egui::Margin::symmetric(14, 12))
}

/// A heading over a part of a page.
pub fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(16.0);
    ui.label(RichText::new(title).font(bold_font(16.0)).color(TEXT));
    ui.add_space(4.0);
}

/// Where a prop stands, in a few words, and the color to say it in: "Closes in 2 h 14 min",
/// "Under way", "Settled: Ravens".
pub fn standing(prop: &PropView, now: i64) -> (String, Color32) {
    match prop.state {
        PropState::Open if prop.open_at(now) => match prop.closes_at {
            0 => ("Open until it's settled".into(), UP),
            c if c - now < 3_600_000 => (format!("Closes in {}", countdown(c - now)), EMBER2),
            c => (format!("Closes in {}", countdown(c - now)), UP),
        },
        PropState::Open | PropState::Closed => {
            if prop.house && prop.starts_at > 0 && now >= prop.starts_at { ("Under way".into(), LIVE) } else { ("Betting closed".into(), MUTED) }
        }
        PropState::Settled => (format!("Settled: {}", prop.winner.and_then(|w| prop.outcomes.get(w)).cloned().unwrap_or_default()), GOLD),
        PropState::Void => ("Called off".into(), MUTED),
    }
}

/// The little drawings beside the side's first four pages.
#[derive(Clone, Copy)]
enum Mark {
    Home,
    Mine,
    Leaders,
    Results,
}

fn mark(p: &egui::Painter, m: Mark, r: Rect, col: Color32) {
    let c = r.center();
    let u = r.width() / 20.0;
    let s = Stroke::new(1.6 * u, col);
    match m {
        Mark::Home => {
            p.add(Shape::line(vec![c + vec2(-7.5 * u, -0.5 * u), c + vec2(0.0, -7.0 * u), c + vec2(7.5 * u, -0.5 * u)], s));
            p.add(Shape::line(vec![c + vec2(-5.2 * u, -1.5 * u), c + vec2(-5.2 * u, 6.5 * u), c + vec2(5.2 * u, 6.5 * u), c + vec2(5.2 * u, -1.5 * u)], s));
        }
        Mark::Mine => {
            // A betting slip.
            p.rect_stroke(Rect::from_center_size(c, vec2(11.0 * u, 14.0 * u)), CornerRadius::same(2), s, egui::StrokeKind::Middle);
            for dy in [-3.0, 0.0, 3.0] {
                p.line_segment([c + vec2(-3.0 * u, dy * u), c + vec2(3.0 * u, dy * u)], Stroke::new(1.2 * u, col));
            }
        }
        Mark::Leaders => {
            for (dx, h) in [(-5.0, 6.0), (0.0, 12.0), (5.0, 8.5)] {
                p.rect_filled(Rect::from_min_max(c + vec2((dx - 1.7) * u, (6.5 - h) * u), c + vec2((dx + 1.7) * u, 6.5 * u)), CornerRadius::same(1), col);
            }
        }
        Mark::Results => {
            p.circle_stroke(c, 7.0 * u, s);
            p.add(Shape::line(vec![c + vec2(-3.4 * u, 0.2 * u), c + vec2(-0.8 * u, 2.8 * u), c + vec2(3.6 * u, -2.6 * u)], s));
        }
    }
}

impl App {
    /// The page showing, in a column down the middle that scrolls.
    pub fn page(&mut self, ui: &mut egui::Ui) {
        let now = words::now_ms();
        egui::ScrollArea::vertical().id_salt(format!("{:?}", self.view)).auto_shrink([false, false]).show(ui, |ui| {
            let w = ui.available_width().min(940.0);
            let pad = ((ui.available_width() - w) / 2.0).max(0.0);
            // A page comes in: up a little, out of nothing, in a moment.
            let came = (self.came.elapsed().as_secs_f32() / 0.16).min(1.0);
            if came < 1.0 {
                ui.ctx().request_repaint();
                ui.set_opacity(came);
                ui.add_space((1.0 - came) * 10.0);
            }
            ui.horizontal_top(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_width(w);
                    match self.view.clone() {
                        View::Home => self.home(ui, now),
                        View::Category(c) => self.category(ui, &c, now),
                        View::Prop(id) => self.prop_page(ui, id, now),
                        View::Mine => self.mine_page(ui, now),
                        View::Leaders => self.leaders(ui),
                        View::Results => self.results(ui, now),
                    }
                    ui.add_space(28.0);
                });
            });
        });
    }

    /// Whether a page is the one showing (a prop's page counts as the page it was opened from).
    fn at(&self, view: &View) -> bool {
        let showing = match &self.view {
            View::Prop(_) => self.back.last().unwrap_or(&View::Home),
            v => v,
        };
        showing == view
    }

    /// The pages, down the side: the front page, my bets, the best, results, then each category
    /// with how many props it has to come out.
    pub fn rail(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 2.0;
        ui.add_space(6.0);
        let mine = self.m.mine_live().len();
        for (view, m, name, n) in [(View::Home, Mark::Home, "Front page", None), (View::Mine, Mark::Mine, "My bets", (mine > 0).then_some(mine)), (View::Leaders, Mark::Leaders, "Best at props", None), (View::Results, Mark::Results, "Results", None)] {
            self.rail_item(ui, view, Some(m), name, n);
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            ui.label(RichText::new("CATEGORIES").size(10.5).strong().color(DIM));
        });
        ui.add_space(2.0);
        for (cat, n) in self.m.categories() {
            let name = p::category_name(&cat);
            self.rail_item(ui, View::Category(cat), None, &name, Some(n));
        }
    }

    fn rail_item(&mut self, ui: &mut egui::Ui, view: View, m: Option<Mark>, name: &str, n: Option<usize>) {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
        let on = self.at(&view);
        let pnt = ui.painter();
        if on || resp.hovered() {
            pnt.rect_filled(r, CornerRadius::same(8), if on { ELEV2 } else { ELEV });
        }
        if on {
            pnt.rect_filled(Rect::from_min_size(r.left_top() + vec2(0.0, 8.0), vec2(3.0, r.height() - 16.0)), CornerRadius::same(2), EMBER);
        }
        let icon = Rect::from_center_size(pos2(r.left() + 21.0, r.center().y), vec2(22.0, 22.0));
        let empty = n == Some(0) && !on;
        match (&view, m) {
            (View::Category(c), _) => {
                emblem(pnt, c, icon, &self.art);
                if empty {
                    pnt.rect_filled(icon, CornerRadius::same(5), SIDE.gamma_multiply(0.55));
                }
            }
            (_, Some(m)) => mark(pnt, m, icon, if on { EMBER2 } else { MUTED }),
            _ => {}
        }
        pnt.text(pos2(r.left() + 40.0, r.center().y), Align2::LEFT_CENTER, name, if on { bold_font(13.5) } else { FontId::proportional(13.5) }, if empty { DIM } else { TEXT });
        if let Some(n) = n.filter(|n| *n > 0) {
            pnt.text(pos2(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, n.to_string(), bold_font(11.5), if on { EMBER2 } else { MUTED });
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.back.clear();
            self.go(view);
        }
    }

    /// The pages along the top, when the window's too narrow for them down the side.
    pub fn strip(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::horizontal().id_salt("strip").auto_shrink([false, true]).scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
            ui.horizontal(|ui| {
                let mine = self.m.mine_live().len();
                let mut pages: Vec<(View, String)> = vec![(View::Home, "Front page".into()), (View::Mine, if mine > 0 { format!("My bets  {mine}") } else { "My bets".into() }), (View::Leaders, "Best".into()), (View::Results, "Results".into())];
                pages.extend(self.m.categories().into_iter().filter(|c| c.1 > 0).map(|(c, n)| (View::Category(c.clone()), format!("{}  {n}", p::category_name(&c)))));
                for (view, name) in pages {
                    let on = self.at(&view);
                    let label = RichText::new(name).font(bold_font(13.0)).color(if on { ON_EMBER } else { TEXT });
                    if ui.add(egui::Button::new(label).fill(if on { EMBER2 } else { ELEV }).corner_radius(14)).clicked() {
                        self.back.clear();
                        self.go(view);
                    }
                }
            });
        });
    }

    // ------------------------------------------------------------ the front page

    fn home(&mut self, ui: &mut egui::Ui, now: i64) {
        self.hero(ui, now);
        self.house_note(ui);
        let soon: Vec<PropView> = self.m.closing_soon(now, 3 * 3_600_000, 6).into_iter().cloned().collect();
        if !soon.is_empty() {
            section(ui, "Closing soon");
            self.cards(ui, &soon, now);
        }
        let featured: Vec<PropView> = self.m.featured(now, 16).into_iter().filter(|x| !soon.iter().any(|s| s.id == x.id)).take(9).cloned().collect();
        if !featured.is_empty() {
            section(ui, "Worth a look");
            self.cards(ui, &featured, now);
        }
        section(ui, "Categories");
        self.tiles(ui);
        let latest: Vec<(u64, String, String, i64)> = self
            .m
            .latest_bets(8)
            .into_iter()
            .map(|(x, l)| {
                let o = x.outcomes.get(l.outcome).cloned().unwrap_or_default();
                let what = if l.chips >= 0 { format!("{} bet {} on {o}", l.name, chips_text(l.chips as u64)) } else { format!("{} cashed out {o} for {}", l.name, chips_text(l.chips.unsigned_abs())) };
                (x.id, what, x.question.clone(), l.at)
            })
            .collect();
        if !latest.is_empty() {
            section(ui, "Latest bets");
            card().show(ui, |ui| {
                ui.set_width(ui.available_width());
                for (id, what, question, at) in latest {
                    let row = ui.horizontal(|ui| {
                        ui.label(RichText::new(what).font(bold_font(13.0)).color(TEXT));
                        ui.label(RichText::new(question).size(13.0).color(MUTED));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| ui.label(RichText::new(words::ago(now - at)).size(12.0).color(DIM)));
                    });
                    if ui.interact(row.response.rect, ui.id().with(("latest", id, at)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.go(View::Prop(id));
                    }
                }
            });
        }
    }

    /// The banner across the front page: what this is, and how much is going on.
    fn hero(&mut self, ui: &mut egui::Ui, now: i64) {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 138.0), Sense::hover());
        let pnt = ui.painter().with_clip_rect(r);
        match &self.art.banner {
            Some(t) => {
                pnt.add(cover_rounded(t, r, 14.0, (0.5, 0.36), Color32::from_gray(190)));
            }
            None => {
                pnt.rect_filled(r, CornerRadius::same(14), Color32::from_rgb(0x2a, 0x18, 0x10));
            }
        }
        pnt.add(gradient(r, 14.0, Color32::from_black_alpha(225), Color32::from_black_alpha(40)));
        let narrow = r.width() < 470.0;
        let x = r.left() + if narrow { 16.0 } else { 22.0 };
        pnt.text(pos2(x, r.top() + if narrow { 16.0 } else { 20.0 }), Align2::LEFT_TOP, "Bet on what happens next", bold_font(if narrow { 20.0 } else { 25.0 }), Color32::WHITE);
        let under = if self.m.house_here && self.m.auto.as_ref().is_some_and(|a| a.on) {
            "Games, prices, the weather: props that open, close and settle by themselves."
        } else {
            "Props your server's members open, with Pyr Casino chips. A winning share pays a chip."
        };
        let pale = Color32::from_rgb(0xe8, 0xdc, 0xcc);
        pnt.galley(pos2(x, r.top() + if narrow { 44.0 } else { 54.0 }), lines(&pnt, under, FontId::proportional(13.5), pale, r.right() - x - 16.0, 2), pale);
        let open: Vec<&PropView> = self.m.props.values().filter(|x| x.open_at(now)).collect();
        let in_play: u64 = self.m.props.values().filter(|x| Model::live(x)).map(|x| x.pool).sum();
        let mine = self.m.mine_live().len();
        let mut at = pos2(x, r.bottom() - if narrow { 34.0 } else { 40.0 });
        let size = if narrow { 11.0 } else { 12.0 };
        for (text, col) in [(count(open.len() as u64, "prop open", "props open"), GOLD), (format!("{} chips in play", short_chips(in_play)), pale), (count(mine as u64, "bet of yours", "bets of yours"), pale)] {
            let pr = pill(&pnt, at, &text, col, Color32::from_black_alpha(150), size);
            at.x = pr.right() + if narrow { 5.0 } else { 8.0 };
        }
    }

    /// How the casino's own props stand here, when they aren't simply on and working.
    fn house_note(&mut self, ui: &mut egui::Ui) {
        let (words, action): (String, Option<&str>) = match (&self.m.auto, self.m.house_here) {
            (_, false) => ("This server's Pyr is older than the casino's own props (Pyr 0.15), so only its members' props are here.".into(), None),
            (Some(a), true) if !a.on && a.can_set => ("The casino's own props are off on this server: games, crypto, the weather and more, opening and settling by themselves. You manage it, so they're yours to turn on.".into(), Some("Turn them on…")),
            (Some(a), true) if !a.on => ("The casino's own props are off on this server. Whoever manages it can turn them on: games, crypto, the weather and more, opening and settling by themselves.".into(), None),
            (Some(a), true) if a.can_set && !a.feed_error.is_empty() => (format!("The props feed can't be read just now, so nothing new opens: {}", a.feed_error), None),
            _ => return,
        };
        ui.add_space(10.0);
        egui::Frame::new().fill(ELEV).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(words).size(13.0).color(BODY));
                if let Some(a) = action {
                    if ui.add(primary_button(a)).clicked() && self.setting.is_none() {
                        let now = self.m.auto.clone().unwrap_or_default();
                        self.setting = Some(Setting { on: true, off: now.off, depth: now.depth });
                    }
                }
            });
        });
    }

    /// Props as small cards, as many across as fit.
    fn cards(&mut self, ui: &mut egui::Ui, props: &[PropView], now: i64) {
        let gap = 10.0;
        let cols = ((ui.available_width() + gap) / 290.0).floor().clamp(1.0, 3.0) as usize;
        let w = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
        for row in props.chunks(cols) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for prop in row {
                    self.small_card(ui, prop, now, w.floor());
                }
            });
            ui.add_space(gap - 6.0);
        }
    }

    /// A prop as a small card: its category and when it closes, the question, its two likeliest
    /// outcomes and how much is on it. All the same size, so they sit in rows. A click opens
    /// its page.
    fn small_card(&mut self, ui: &mut egui::Ui, prop: &PropView, now: i64, w: f32) {
        const H: f32 = 174.0;
        let id = ui.id().with(("small", prop.id));
        let cat = prop.category_id();
        let (col, _) = category_look(&cat);
        let (r, resp) = ui.allocate_exact_size(vec2(w, H), Sense::click());
        if ui.is_rect_visible(r) {
            let lift = ui.ctx().animate_bool_with_time(id.with("lift"), resp.hovered(), 0.12);
            let pnt = ui.painter();
            pnt.add(gradient_corner(r, 12.0, mix(CARD, col, 0.20 + 0.12 * lift), CARD));
            pnt.rect_stroke(r, CornerRadius::same(12), Stroke::new(1.0_f32, mix(CARD_EDGE, col, 0.12 + 0.63 * lift)), egui::StrokeKind::Inside);
            let (x0, x1) = (r.left() + 14.0, r.right() - 14.0);
            // Its category, and how long it's open.
            let em = Rect::from_min_size(pos2(x0, r.top() + 12.0), vec2(22.0, 22.0));
            emblem(pnt, &cat, em, &self.art);
            pnt.text(pos2(em.right() + 8.0, em.center().y), Align2::LEFT_CENTER, p::category_name(&cat), bold_font(11.5), mix(col, Color32::WHITE, 0.30));
            let (text, c) = standing(prop, now);
            let g = pnt.layout_no_wrap(text.replace("Closes in ", ""), bold_font(11.5), c);
            let pr = Rect::from_min_size(pos2(x1 - g.size().x - 16.0, em.center().y - 10.0), vec2(g.size().x + 16.0, 20.0));
            pnt.rect_filled(pr, CornerRadius::same(10), c.gamma_multiply(0.14));
            pnt.galley(pos2(pr.left() + 8.0, pr.center().y - g.size().y / 2.0), g, c);
            // What it asks: two lines of it at most.
            pnt.galley(pos2(x0, em.bottom() + 9.0), lines(pnt, &prop.question, bold_font(14.5), TEXT, x1 - x0, 2), TEXT);
            // The two likeliest, each a bar as long as it's likely.
            let prices = prop.prices();
            let mut order: Vec<usize> = (0..prop.outcomes.len()).collect();
            order.sort_by(|a, b| prices[*b].total_cmp(&prices[*a]));
            let mut y = r.bottom() - 34.0 - 2.0 * 22.0 - 4.0;
            for i in order.into_iter().take(2) {
                let bar = Rect::from_min_size(pos2(x0, y), vec2(x1 - x0, 22.0));
                let c = color_of(&prop.outcomes, i);
                let shown = ui.ctx().animate_value_with_time(id.with(("bar", i)), prices[i] as f32, 0.35);
                pnt.rect_filled(bar, CornerRadius::same(6), Color32::from_white_alpha(9));
                pnt.rect_filled(Rect::from_min_size(bar.min, vec2((bar.width() * shown).max(6.0), bar.height())), CornerRadius::same(6), c.gamma_multiply(0.30));
                let pc = pnt.layout_no_wrap(pct(prices[i]), bold_font(12.5), c);
                let name = lines(pnt, &prop.outcomes[i], FontId::proportional(12.5), TEXT, bar.width() - pc.size().x - 26.0, 1);
                pnt.galley(pos2(bar.left() + 9.0, bar.center().y - name.size().y / 2.0), name, TEXT);
                pnt.galley(pos2(bar.right() - 9.0 - pc.size().x, bar.center().y - pc.size().y / 2.0), pc, c);
                y += 26.0;
            }
            // How much is on it; and whether any of it is mine.
            let foot = r.bottom() - 18.0;
            let on_it = if prop.bets == 0 { "No bets yet".to_string() } else { format!("{} in · {}", short_chips(prop.pool), count(prop.bets as u64, "bet", "bets")) };
            pnt.text(pos2(x0, foot), Align2::LEFT_CENTER, on_it, FontId::proportional(11.5), DIM);
            let more = prop.outcomes.len().saturating_sub(2);
            if prop.mine.iter().any(|h| h.shares > 1e-9) {
                pnt.text(pos2(x1, foot), Align2::RIGHT_CENTER, "You're in", bold_font(11.5), GOLD);
            } else if more > 0 {
                pnt.text(pos2(x1, foot), Align2::RIGHT_CENTER, format!("+{more} more"), FontId::proportional(11.5), DIM);
            }
        }
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.go(View::Prop(prop.id));
        }
    }

    /// Every category as a tile: its emblem, its name, and how many props it has to come out.
    fn tiles(&mut self, ui: &mut egui::Ui) {
        let gap = 10.0;
        let cols = ((ui.available_width() + gap) / 215.0).floor().clamp(1.0, 5.0) as usize;
        let w = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
        let cats = self.m.categories();
        for row in cats.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (cat, n) in row {
                    let (r, resp) = ui.allocate_exact_size(vec2(w.floor(), 70.0), Sense::click());
                    let (col, _) = category_look(cat);
                    let pnt = ui.painter().with_clip_rect(r);
                    let lift = ui.ctx().animate_bool_with_time(ui.id().with(("tile", cat)), resp.hovered(), 0.12);
                    let empty = *n == 0;
                    // Its picture, in the dark at the left where its name goes (darker still
                    // when it has nothing open).
                    match self.art.category(cat) {
                        Some((t, focus)) => {
                            pnt.add(cover_rounded(t, r, 12.0, focus, Color32::from_gray(if empty { 70 } else { 150 + (60.0 * lift) as u8 })));
                            pnt.add(gradient(r, 12.0, Color32::from_black_alpha(225), Color32::from_black_alpha(if empty { 120 } else { 40 })));
                        }
                        None => {
                            pnt.add(gradient(r, 12.0, mix(CARD, col, 0.20 + 0.14 * lift), CARD));
                        }
                    }
                    pnt.rect_stroke(r, CornerRadius::same(12), Stroke::new(1.0_f32, mix(CARD_EDGE, col, 0.25 + 0.6 * lift)), egui::StrokeKind::Inside);
                    emblem(&pnt, cat, Rect::from_center_size(pos2(r.left() + 33.0, r.center().y), vec2(40.0, 40.0)), &self.art);
                    pnt.text(pos2(r.left() + 62.0, r.center().y - 9.0), Align2::LEFT_CENTER, p::category_name(cat), bold_font(14.0), if empty { MUTED } else { Color32::WHITE });
                    let under = if empty { "Nothing open".to_string() } else { format!("{n} open") };
                    pnt.text(pos2(r.left() + 62.0, r.center().y + 9.0), Align2::LEFT_CENTER, under, FontId::proportional(12.0), if empty { DIM } else { mix(col, Color32::WHITE, 0.6) });
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.back.clear();
                        self.go(View::Category(cat.clone()));
                    }
                }
            });
            ui.add_space(gap - 6.0);
        }
    }

    // ------------------------------------------------------------ a category

    /// A category's banner: its picture, its emblem and name, a word about it, and how many
    /// props it has to come out.
    fn category_banner(&mut self, ui: &mut egui::Ui, cat: &str, n: usize) {
        let (col, blurb) = category_look(cat);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 126.0), Sense::hover());
        let pnt = ui.painter().with_clip_rect(r);
        match self.art.category(cat) {
            Some((t, focus)) => {
                pnt.add(cover_rounded(t, r, 14.0, focus, Color32::from_gray(215)));
                pnt.add(gradient(r, 14.0, Color32::from_black_alpha(235), Color32::from_black_alpha(20)));
            }
            None => {
                pnt.add(gradient(r, 14.0, mix(CARD, col, 0.35), CARD));
            }
        }
        pnt.rect_stroke(r, CornerRadius::same(14), Stroke::new(1.0_f32, mix(CARD_EDGE, col, 0.3)), egui::StrokeKind::Inside);
        let narrow = r.width() < 470.0;
        let e = if narrow { 42.0 } else { 54.0 };
        let em = Rect::from_min_size(pos2(r.left() + if narrow { 14.0 } else { 22.0 }, r.center().y - e / 2.0), vec2(e, e));
        emblem(&pnt, cat, em, &self.art);
        let x = em.right() + 14.0;
        pnt.text(pos2(x, r.center().y - 14.0), Align2::LEFT_CENTER, p::category_name(cat), bold_font(if narrow { 20.0 } else { 25.0 }), Color32::WHITE);
        let pale = mix(col, Color32::WHITE, 0.55);
        pnt.galley(pos2(x, r.center().y + 3.0), lines(&pnt, blurb, FontId::proportional(13.0), pale, r.right() - x - 16.0, 2), pale);
        if n > 0 && !narrow {
            let text = count(n as u64, "prop", "props");
            let wide = pnt.layout_no_wrap(text.clone(), bold_font(12.0), TEXT).size().x + 14.4;
            pill(&pnt, pos2(r.right() - 16.0 - wide, r.top() + 14.0), &text, TEXT, Color32::from_black_alpha(150), 12.0);
        }
    }

    fn category(&mut self, ui: &mut egui::Ui, cat: &str, now: i64) {
        // (Owned: what's drawn changes the window.)
        let events: Vec<(String, i64, Vec<PropView>)> = self.m.events(cat).into_iter().map(|e| (e.name, e.starts, e.props.into_iter().cloned().collect())).collect();
        let n: usize = events.iter().map(|e| e.2.len()).sum();
        ui.add_space(2.0);
        self.category_banner(ui, cat, n);
        ui.add_space(10.0);
        if events.is_empty() {
            let on = self.m.house_here && self.m.auto.as_ref().is_some_and(|a| a.on && !a.off.iter().any(|c| c == cat));
            let words = match (cat, on) {
                ("community", _) => "Nothing of the members' is open. Open one with New prop, or with /prop in a channel.".to_string(),
                ("casino", _) => "Nothing about the casino is open. Open one with New prop: who wins Friday's tournament?".to_string(),
                (_, true) if SPORTS.contains(&cat) => "No games on the board right now. They come by themselves as they're scheduled, and in the off-season there are none.".to_string(),
                (_, true) => "Nothing open right now. More come by themselves.".to_string(),
                (_, false) => "The casino's own props aren't on for this category here. A member can still open one of their own with New prop.".to_string(),
            };
            return note(ui, "Nothing here yet", &words);
        }
        for (name, starts, props) in &events {
            self.event_card(ui, cat, name, *starts, props, now);
            ui.add_space(8.0);
        }
    }

    /// A game's (or a day's) props together: what it is and when, then a row a prop, each
    /// outcome a button with its odds. A game's rows are named down the left, as on a
    /// sportsbook's board; anything else says what it asks over its outcomes.
    fn event_card(&mut self, ui: &mut egui::Ui, cat: &str, name: &str, starts: i64, props: &[PropView], now: i64) {
        let Some(first) = props.first() else { return };
        let sport = SPORTS.contains(&cat);
        let lone = name.is_empty();
        // (Every prop of a game closes as it starts. Elsewhere each may close in its own time.)
        let together = props.iter().all(|x| x.closes_at == first.closes_at);
        card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                // When, at the right: a game's start (or that it's on); else how long it's open.
                let under_way = first.house && starts > 0 && now >= starts && props.iter().any(|x| !x.open_at(now));
                if under_way {
                    // A dot that beats while it's on.
                    let t = ui.input(|i| i.time) as f32;
                    let (r, _) = ui.allocate_exact_size(vec2(46.0, 18.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(9), LIVE.gamma_multiply(0.22));
                    ui.painter().circle_filled(r.left_center() + vec2(9.0, 0.0), 3.2 + 0.9 * (t * 3.0).sin(), LIVE);
                    ui.painter().text(r.left_center() + vec2(17.0, 0.0), Align2::LEFT_CENTER, "LIVE", bold_font(10.5), LIVE);
                    ui.ctx().request_repaint_after(std::time::Duration::from_millis(60));
                } else if sport && starts > 0 {
                    let soon = starts - now < 3_600_000 && starts > now;
                    ui.label(RichText::new(if soon { format!("in {}", countdown(starts - now)) } else { local_when(starts, now) }).font(bold_font(12.5)).color(if soon { EMBER2 } else { GOLD }));
                } else if together {
                    let (text, c) = standing(first, now);
                    ui.label(RichText::new(text).font(bold_font(12.5)).color(c));
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                    let title = if lone { first.question.as_str() } else { name };
                    let resp = ui.add(egui::Label::new(RichText::new(title).font(bold_font(15.5)).color(TEXT)).wrap().sense(Sense::click()));
                    if lone && resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.go(View::Prop(first.id));
                    }
                });
            });
            // (A game's word is the same for all its props: where it is, or the score.)
            if (sport || lone) && !first.detail.is_empty() {
                ui.label(RichText::new(&first.detail).size(12.0).color(MUTED));
            }
            ui.add_space(4.0);
            for prop in props {
                self.prop_row(ui, prop, if lone { String::new() } else { model::row_label(prop) }, sport, !sport && !lone, !together, now);
            }
        });
    }

    /// A prop as a row: what it asks, then each outcome as a button with its odds. `beside`
    /// puts what it asks down the left (a game's rows, when there's the room); otherwise it
    /// goes over the buttons, with the prop's own word (`detail`) and how long it's open
    /// (`when`: where the props above and below close at other times).
    #[allow(clippy::too_many_arguments)]
    fn prop_row(&mut self, ui: &mut egui::Ui, prop: &PropView, label: String, beside: bool, detail: bool, when: bool, now: i64) {
        let buttons = |ui: &mut egui::Ui, me: &mut App| {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for i in 0..prop.outcomes.len() {
                    me.outcome_button(ui, prop, i, now);
                }
            });
        };
        if beside && ui.available_width() > 560.0 {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(148.0, 30.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.set_width(148.0);
                    if !label.is_empty() {
                        let resp = ui.add(egui::Label::new(RichText::new(&label).font(bold_font(12.5)).color(MUTED)).truncate().sense(Sense::click()));
                        if resp.on_hover_text(&prop.question).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.go(View::Prop(prop.id));
                        }
                    }
                });
                buttons(ui, self);
            });
        } else {
            let detail = detail && !prop.detail.is_empty();
            if !label.is_empty() || detail || when {
                ui.add_space(5.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    if !label.is_empty() {
                        let resp = ui.add(egui::Label::new(RichText::new(&label).font(bold_font(13.0)).color(BODY)).sense(Sense::click()));
                        if resp.on_hover_text("Open its page").on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            self.go(View::Prop(prop.id));
                        }
                    }
                    if detail {
                        ui.label(RichText::new(format!("· {}", prop.detail)).size(12.0).color(MUTED));
                    }
                    if when {
                        let (text, c) = standing(prop, now);
                        ui.label(RichText::new(format!("· {}", mid_sentence(&text).replacen("Closes", "closes", 1))).size(12.0).color(mix(c, MUTED, 0.35)));
                    }
                });
            }
            buttons(ui, self);
        }
        ui.add_space(2.0);
    }

    /// An outcome as a button: its name and its odds, lit a moment when they move, ringed in
    /// gold when I've something on it. A click opens the prop's page, ready to bet on it.
    pub fn outcome_button(&mut self, ui: &mut egui::Ui, prop: &PropView, i: usize, now: i64) {
        let price = prop.prices().get(i).copied().unwrap_or(0.0);
        let col = color_of(&prop.outcomes, i);
        let over = !Model::live(prop);
        let won = prop.winner == Some(i);
        let name = ui.painter().layout_no_wrap(prop.outcomes[i].clone(), FontId::proportional(13.0), if over && !won { DIM } else { TEXT });
        let odds = if over { if won { "won".to_string() } else { String::new() } } else { pct(price) };
        let pc = ui.painter().layout_no_wrap(odds, bold_font(13.0), if won { GOLD } else { col });
        let w = 12.0 + name.size().x + if pc.size().x > 0.0 { 10.0 + pc.size().x } else { 0.0 } + 12.0;
        let (r, resp) = ui.allocate_exact_size(vec2(w.max(64.0), 30.0), Sense::click());
        let pnt = ui.painter();
        let open = prop.open_at(now);
        let fill = if won { Color32::from_rgb(0x3a, 0x2f, 0x14) } else if resp.hovered() { ELEV2 } else { ELEV };
        pnt.rect_filled(r, CornerRadius::same(8), fill);
        // Odds that just moved: lit in green or red, fading.
        if let Some(m) = self.m.moved.get(&(prop.id, i)) {
            let left = fading(m.at, 1.6);
            if left > 0.0 {
                let c = if price as f32 > m.from { UP } else { DOWN };
                pnt.rect_filled(r, CornerRadius::same(8), c.gamma_multiply(0.30 * left));
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(40));
            }
        }
        if !over {
            // (How likely, as a line along the bottom.)
            let bar = Rect::from_min_max(pos2(r.left() + 6.0, r.bottom() - 3.5), pos2(r.right() - 6.0, r.bottom() - 1.5));
            pnt.rect_filled(Rect::from_min_size(bar.min, vec2(bar.width() * price as f32, bar.height())), CornerRadius::same(1), col.gamma_multiply(if open { 0.9 } else { 0.4 }));
        }
        let mine = prop.mine.iter().any(|h| h.outcome == i && h.shares > 1e-9);
        if mine {
            pnt.rect_stroke(r, CornerRadius::same(8), Stroke::new(1.5_f32, GOLD), egui::StrokeKind::Inside);
        }
        pnt.galley(pos2(r.left() + 12.0, r.center().y - name.size().y / 2.0 - 1.0), name, TEXT);
        pnt.galley(pos2(r.right() - 12.0 - pc.size().x, r.center().y - pc.size().y / 2.0 - 1.0), pc, col);
        let hint = if over {
            prop.question.clone()
        } else if mine {
            format!("{}: you've chips on {}", prop.question, prop.outcomes[i])
        } else {
            prop.question.clone()
        };
        if resp.on_hover_text(hint).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            self.go(View::Prop(prop.id));
            if open && (prop.house || prop.author != self.m.me) {
                self.betting = Some(Betting { id: prop.id, outcome: i, amount: "1000".into() });
            }
        }
    }
}

/// Text set in at most `rows` lines `width` wide, the rest of it cut with an ellipsis.
pub fn lines(p: &egui::Painter, text: &str, font: FontId, col: Color32, width: f32, rows: usize) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple(text.to_string(), font, col, width.max(10.0));
    job.wrap.max_rows = rows;
    job.wrap.break_anywhere = rows == 1;
    job.wrap.overflow_character = Some('…');
    p.layout_job(job)
}
