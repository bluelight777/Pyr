//! Autoplay for the slots: a number of spins or until it's stopped, and whether a big win
//! stops it. Shared by every slot machine in Pyr Casino.

use eframe::egui::{self, RichText};

use crate::theme;

/// Spinning until it's stopped (STOP, leaving, or running out of chips).
pub const FOREVER: u32 = u32::MAX;
/// The numbers of spins the menu offers.
pub const COUNTS: [u32; 7] = [10, 25, 50, 100, 200, 500, 1000];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Auto {
    /// Spins left ([`FOREVER`]: until stopped); 0 when it's off.
    pub left: u32,
    /// A big win stops it, to enjoy it (on unless switched off).
    pub big_stops: bool,
}

impl Default for Auto {
    fn default() -> Self {
        Auto { left: 0, big_stops: true }
    }
}

impl Auto {
    pub fn on(&self) -> bool {
        self.left > 0
    }

    pub fn stop(&mut self) {
        self.left = 0;
    }

    /// A round's over: one spin fewer to go (never fewer when it spins until stopped), or none
    /// at all after a big win when that stops it. Whether it goes on.
    pub fn spun(&mut self, big: bool) -> bool {
        if self.left == 0 {
            return false;
        }
        if big && self.big_stops {
            self.left = 0;
            return false;
        }
        if self.left != FOREVER {
            self.left -= 1;
        }
        self.left > 0
    }

    /// The spins left as shown: a number, or ∞.
    pub fn left_text(&self) -> String {
        if self.left == FOREVER { "∞".into() } else { self.left.to_string() }
    }
}

/// The menu under the Auto button (while it isn't running): how many spins, or until it's
/// stopped, and whether a big win stops it. The number chosen, when one was.
pub fn menu(resp: &egui::Response, auto: &mut Auto) -> Option<u32> {
    let mut chosen = None;
    egui::Popup::menu(resp).align(egui::RectAlign::TOP_END).show(|ui| {
        crate::menu::style(ui);
        ui.label(RichText::new("SPIN BY ITSELF").font(theme::bold_font(11.0)).color(theme::DIM()));
        for n in COUNTS {
            if ui.button(format!("{} spins", crate::poker::cards::chips_text(n as u64))).clicked() {
                chosen = Some(n);
                ui.close();
            }
        }
        if ui.button("Until I stop it  ∞").clicked() {
            chosen = Some(FOREVER);
            ui.close();
        }
        ui.separator();
        if crate::menu::check(ui, auto.big_stops, "A big win stops it", false).clicked() {
            auto.big_stops = !auto.big_stops;
        }
    });
    chosen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_down_or_goes_on_until_stopped() {
        let mut a = Auto { left: 2, big_stops: true };
        assert!(a.spun(false) && a.left == 1);
        assert!(!a.spun(false) && !a.on());
        let mut a = Auto { left: FOREVER, big_stops: true };
        for _ in 0..10_000 {
            assert!(a.spun(false));
        }
        assert_eq!(a.left_text(), "∞");
        // A big win stops it, unless that's switched off.
        assert!(!a.spun(true) && !a.on());
        let mut a = Auto { left: FOREVER, big_stops: false };
        assert!(a.spun(true) && a.on());
        a.stop();
        assert!(!a.spun(false));
    }
}
