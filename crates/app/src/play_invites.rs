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
    /// A game's invite in a message: the game's card with Join (like a server invite's).
    pub(crate) fn play_invite_embed(&mut self, ui: &mut egui::Ui, _st: &AppState, game: &str, room: &str, acts: &mut Vec<Act>) {
        let entry = self.plugins.library.as_ref().and_then(|l| l.find(game)).cloned();
        let installed = self.plugins.installed.get(game).cloned();
        let name = entry.as_ref().map(|e| e.name.clone()).or_else(|| installed.as_ref().map(|p| p.name.clone())).unwrap_or_else(|| game.to_string());
        egui::Frame::new().fill(theme::ELEV()).stroke(Stroke::new(1.0_f32, theme::LINE())).corner_radius(10).inner_margin(egui::Margin::symmetric(14, 12)).show(ui, |ui| {
            ui.set_width(360.0_f32.min(ui.available_width()));
            ui.label(RichText::new("YOU'RE INVITED TO PLAY").size(11.0).strong().color(theme::DIM()));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                self.plugin_icon(ui, game, pyr_proto::library::Kind::Program, 44.0);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(&name).font(theme::bold_font(16.0)).color(theme::TEXT()));
                    let what = match (&installed, &entry) {
                        (Some(_), _) => format!("Room {room}"),
                        (None, Some(_)) => format!("Room {room} · Install it from the Pyr Library to play"),
                        (None, None) => format!("Room {room} · not in the Pyr Library here"),
                    };
                    ui.label(RichText::new(what).size(12.5).color(theme::MUTED()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let label = if installed.is_some() { "Join" } else { "Get it" };
                    if ui.add(theme::primary_button(label).min_size(vec2(72.0, 30.0))).clicked() {
                        acts.push(Act::PlayInvite { game: game.to_string(), room: room.to_string() });
                    }
                });
            });
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
