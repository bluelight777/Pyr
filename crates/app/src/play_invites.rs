//! Games' invites (0.13.15 to 0.13.18, 0.13.27): a game from the Pyr Library asks Pyr to
//! invite a friend to its room (see `plugins::programs`), and the friend gets a direct
//! message with a link, `https://pyr.gg/<game>/<ROOM>`. In Pyr the link shows as the game's
//! card with Join, and clicking it opens the game in that room (installing it first, or
//! updating it when the Library has a newer version). Anywhere else, pyr.gg's page for it opens
//! Pyr with `pyr://play/<game>/<ROOM>`, which does the same.

use eframe::egui::{self, RichText, Stroke, vec2};
use pyr_engine::state::AppState;

use crate::app::{Act, PyrApp};
use crate::theme;

/// A game's invite in a link: (game, room), from `https://pyr.gg/<game>/<ROOM>` (with or
/// without `https://` and `www.`) or `pyr://play/<game>/<ROOM>`. A server's own links
/// (`pyr.gg/<code>`, `pyr.gg/invite/<code>`, `pyr.gg/join/...`) aren't games'.
pub fn game_link(url: &str) -> Option<(String, String)> {
    let t = url.trim().trim_start_matches('<').trim_end_matches('>');
    let lower = t.to_ascii_lowercase();
    let rest = if let Some(r) = lower.strip_prefix("pyr://play/") {
        r.to_string()
    } else {
        let r = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")).unwrap_or(&lower);
        let r = r.strip_prefix("www.").unwrap_or(r);
        r.strip_prefix("pyr.gg/")?.to_string()
    };
    let mut parts = rest.trim_end_matches('/').split('/');
    let (game, room) = (parts.next()?, parts.next()?);
    if parts.next().is_some() || ["invite", "join", "library", "download", "files", "assets"].contains(&game) {
        return None;
    }
    let game_ok = (2..=40).contains(&game.len()) && game.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') && !game.starts_with('-') && !game.ends_with('-');
    let room_ok = (4..=8).contains(&room.len()) && room.chars().all(|c| c.is_ascii_alphanumeric());
    (game_ok && room_ok).then(|| (game.to_string(), room.to_ascii_uppercase()))
}

/// A game's invite links in a message's text.
pub fn links_in(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for word in text.split(|c: char| c.is_whitespace()) {
        let word = word.trim_matches(|c: char| matches!(c, '(' | ')' | '.' | ',' | '!' | '?' | '"' | '\''));
        if let Some(g) = game_link(word) {
            if !out.contains(&g) {
                out.push(g);
            }
        }
    }
    out
}

/// What a game's invite says in the direct message it goes in (0.13.15/0.13.18).
pub fn invite_text(me: &str, game_name: &str, game: &str, room: &str) -> String {
    format!("{me} invited you to play {game_name}, room {room}.\nhttps://pyr.gg/{game}/{room}")
}

impl PyrApp {
    /// A game's invite in a message, as 0.13.28's card has it (seen in it, pointed at): "YOU SENT
    /// A GAME INVITE" on mine, "GAME INVITE" on others'; the game's name (its first letter made a
    /// capital where Pyr doesn't know it) and "Room BDFPA"; Copy code, and Join or Get it.
    pub(crate) fn play_invite_embed(&mut self, ui: &mut egui::Ui, _st: &AppState, game: &str, room: &str, mine: bool, acts: &mut Vec<Act>) {
        let entry = self.plugins.library.as_ref().and_then(|l| l.find(game)).cloned();
        let installed = self.plugins.installed.get(game).cloned();
        let name = entry.as_ref().map(|e| e.name.clone()).or_else(|| installed.as_ref().map(|p| p.name.clone())).unwrap_or_else(|| {
            let mut c = game.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        });
        // (386 by 86 at 1280x800: 360 inside, 12 round it, corners 12, a line of the ember at a little over half.)
        egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0_f32, theme::EMBER().gamma_multiply(0.55))).corner_radius(12).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(360.0_f32.min(ui.available_width()));
            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 60.0), egui::Sense::hover());
            let p = ui.painter();
            p.text(r.left_top() + vec2(0.0, 1.0), egui::Align2::LEFT_TOP, if mine { "YOU SENT A GAME INVITE" } else { "GAME INVITE" }, theme::bold_font(11.0), theme::DIM());
            p.text(r.left_top() + vec2(0.0, 19.0), egui::Align2::LEFT_TOP, &name, theme::bold_font(16.0), theme::TEXT());
            p.text(r.left_top() + vec2(0.0, 43.5), egui::Align2::LEFT_TOP, format!("Room {room}"), theme::font(13.0), theme::MUTED());
            // Join (or Get it) at the right, Copy code beside it (its frame only when pointed at).
            let go = egui::Rect::from_min_size(egui::pos2(r.right() - 84.0, r.center().y - 17.0), vec2(84.0, 34.0));
            let (label, tip) = if installed.is_some() {
                ("Join", format!("Open {name} in room {room}"))
            } else {
                ("Get it", format!("{name} isn't installed: see it in the Pyr Library"))
            };
            if ui.put(go, theme::primary_button(label)).on_hover_text(tip).clicked() {
                acts.push(Act::PlayInvite { game: game.to_string(), room: room.to_string() });
            }
            let copy = egui::Rect::from_min_size(egui::pos2(go.left() - 7.0 - 98.0, r.center().y - 16.0), vec2(98.0, 32.0));
            let b = egui::Button::new(RichText::new("Copy code").color(theme::TEXT())).frame_when_inactive(false);
            if ui.put(copy, b).on_hover_text("Copy the room code").clicked() {
                ui.ctx().copy_text(room.to_string());
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_games_invite_is_its_link() {
        assert_eq!(game_link("https://pyr.gg/pyropoly/BDFPA"), Some(("pyropoly".into(), "BDFPA".into())));
        assert_eq!(game_link("https://www.pyr.gg/pyropoly/bdfpa/"), Some(("pyropoly".into(), "BDFPA".into())));
        assert_eq!(game_link("pyr.gg/torch-and-conquer/Q7XK"), Some(("torch-and-conquer".into(), "Q7XK".into())));
        assert_eq!(game_link("pyr://play/pyropoly/JKTAG"), Some(("pyropoly".into(), "JKTAG".into())));
        // A server's links aren't games'.
        assert_eq!(game_link("https://pyr.gg/qrslmnov"), None);
        assert_eq!(game_link("https://pyr.gg/invite/abcd1234"), None);
        assert_eq!(game_link("https://pyr.gg/join/abcd/Pyr-Setup.exe"), None);
        assert_eq!(game_link("https://example.com/pyropoly/BDFPA"), None);
        assert_eq!(game_link("https://pyr.gg/pyropoly/AB"), None, "a room has 4 to 8 letters and digits");
        assert_eq!(game_link("https://pyr.gg/Pyro_poly/BDFPA"), None);
    }

    #[test]
    fn invites_are_found_in_what_was_said() {
        let text = invite_text("Blue", "Pyropoly", "pyropoly", "BDFPA");
        assert_eq!(text, "Blue invited you to play Pyropoly, room BDFPA.\nhttps://pyr.gg/pyropoly/BDFPA");
        assert_eq!(links_in(&text), vec![("pyropoly".to_string(), "BDFPA".to_string())]);
        assert_eq!(links_in("(https://pyr.gg/pyropoly/BDFPA)."), vec![("pyropoly".to_string(), "BDFPA".to_string())]);
        assert!(links_in("see https://pyr.gg/qrslmnov").is_empty());
    }
}
