//! A page of Pyr's drawn pieces, to check them by eye (development only: `PYR_DEV_GALLERY=cards`
//! opens it instead of Pyr). Every card face in two colors and in four, at the sizes they're
//! drawn at; every back; my back standing in for an empty one; cards turning over.

use eframe::egui::{self, Color32, Rect, RichText, pos2, vec2};
use pyr_poker::cards::Card;

use crate::poker::cards;

pub fn run(which: String) -> eframe::Result<()> {
    let options = eframe::NativeOptions { viewport: egui::ViewportBuilder::default().with_inner_size([1560.0, 1000.0]).with_title("Pyr gallery"), ..Default::default() };
    eframe::run_native(
        "Pyr gallery",
        options,
        Box::new(move |cc| {
            crate::theme::install(&cc.egui_ctx);
            Ok(Box::new(Gallery { which }))
        }),
    )
}

struct Gallery {
    which: String,
}

impl eframe::App for Gallery {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().frame(egui::Frame::new().fill(Color32::from_rgb(0x14, 0x5c, 0x3a)).inner_margin(16)).show(ctx, |ui| match self.which.as_str() {
            _ => cards_page(ui),
        });
        // (Pictures arrive a moment after they're asked for.)
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn label(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong().size(14.0).color(Color32::WHITE));
}

fn cards_page(ui: &mut egui::Ui) {
    // My back is Jade here: the empty backs below must come out green.
    cards::set_mine("back_jade", false);
    let p = ui.painter().clone();
    let row = |ui: &mut egui::Ui, w: f32, cards: &[(u8, u8)], four: bool| {
        let h = w * 1.4;
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), h + 8.0), egui::Sense::hover());
        for (i, &(rank, suit)) in cards.iter().enumerate() {
            let c = Rect::from_min_size(pos2(r.left() + i as f32 * (w + 8.0), r.top()), vec2(w, h));
            cards::face(&p, c, Card::new(rank, suit), four, false);
        }
    };
    label(ui, "Every card, two colors (72 wide)");
    for suit in [3u8, 2, 1, 0] {
        let all: Vec<(u8, u8)> = (0..13).map(|r| (r, suit)).collect();
        row(ui, 72.0, &all, false);
    }
    label(ui, "Four colors, at my hand's size (96), and the court cards and aces");
    row(ui, 96.0, &[(9, 0), (10, 1), (11, 2), (12, 3), (12, 2), (12, 0), (8, 1), (6, 0), (2, 1), (12, 1)], true);
    label(ui, "Small: shown at a seat (40), and a hand's history (31)");
    let small: Vec<(u8, u8)> = vec![(12, 3), (11, 2), (8, 1), (0, 0), (9, 2), (5, 3)];
    row(ui, 40.0, &small, false);
    row(ui, 31.0, &small, true);
    label(ui, "Backs: Ember, Midnight, Jade, Gilded, Inferno, Aurora; then empty (mine: Jade); then turning over");
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 140.0), egui::Sense::hover());
    let mut x = r.left();
    for d in ["back_ember", "back_midnight", "back_jade", "back_gold", "back_inferno", "back_aurora", ""] {
        cards::back(&p, Rect::from_min_size(pos2(x, r.top()), vec2(96.0, 134.0)), d);
        x += 104.0;
    }
    for k in [0.15f32, 0.35, 0.65, 0.85] {
        let w = (1.0 - 2.0 * k).abs().max(0.04) * 96.0;
        let c = Rect::from_center_size(pos2(x + 48.0, r.top() + 67.0), vec2(w, 134.0));
        if k < 0.5 {
            cards::back(&p, c, "back_ember");
        } else {
            cards::face(&p, c, Card::new(11, 2), false, false);
        }
        x += 104.0;
    }
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), egui::Sense::hover());
    let mut x = r.left();
    for d in ["back_ember", "back_midnight", "back_jade", "back_gold", "back_inferno", "back_aurora", ""] {
        cards::back(&p, Rect::from_min_size(pos2(x, r.top()), vec2(32.0, 45.0)), d);
        x += 40.0;
    }
}
