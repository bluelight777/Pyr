//! The right-click menu on boxes you type in (an invite link, a server's address, your name):
//! Cut, Copy, Paste and Select all, like other apps' (the message box has its own, with
//! spellcheck).

use std::ops::Range;

use eframe::egui;
use egui::text::{CCursor, CCursorRange};

/// Gives the box `resp` (a `TextEdit` showing `text`) its right-click menu. `one_line`: what's
/// pasted becomes one line (a link copied with its line break still pastes as just the link).
pub(crate) fn menu(ui: &egui::Ui, resp: &egui::Response, text: &mut String, one_line: bool) {
    let id = resp.id;
    let ctx = ui.ctx().clone();
    // What was selected when the menu opened: opening it takes the focus from the box.
    let kept = id.with("menu-selection");
    if resp.secondary_clicked() {
        let sel = selection(&ctx, id);
        ctx.data_mut(|d| d.insert_temp(kept, sel));
    }
    resp.context_menu(|ui| {
        let n = text.chars().count();
        let sel: Option<Range<usize>> = ctx.data(|d| d.get_temp(kept)).flatten().filter(|r: &Range<usize>| r.end <= n);
        if let Some(r) = sel.clone().filter(|r| !r.is_empty()) {
            let picked: String = text.chars().skip(r.start).take(r.len()).collect();
            if ui.add(egui::Button::new("Cut").shortcut_text("Ctrl+X")).clicked() {
                ctx.copy_text(picked.clone());
                replace_chars(text, r.clone(), "");
                put_cursor(&ctx, id, r.start..r.start);
                ui.close();
            }
            if ui.add(egui::Button::new("Copy").shortcut_text("Ctrl+C")).clicked() {
                ctx.copy_text(picked);
                ui.close();
            }
        }
        let clip = clipboard_text();
        if ui.add_enabled(clip.is_some(), egui::Button::new("Paste").shortcut_text("Ctrl+V")).clicked() {
            if let Some(t) = clip {
                let t = if one_line { t.split(['\r', '\n']).map(str::trim).filter(|l| !l.is_empty()).collect::<Vec<_>>().join(" ") } else { t };
                // Over what's selected, or where the cursor was (the end, if it was nowhere).
                let at = sel.unwrap_or(n..n);
                replace_chars(text, at.clone(), &t);
                let end = at.start + t.chars().count();
                put_cursor(&ctx, id, end..end);
            }
            ui.close();
        }
        if ui.add_enabled(n > 0, egui::Button::new("Select all").shortcut_text("Ctrl+A")).clicked() {
            put_cursor(&ctx, id, 0..n);
            ui.close();
        }
    });
}

/// The text on the clipboard, if there's any.
fn clipboard_text() -> Option<String> {
    arboard::Clipboard::new().ok().and_then(|mut c| c.get_text().ok()).filter(|t| !t.is_empty())
}

/// The box `id`'s selection (or its cursor, as an empty range), in characters.
fn selection(ctx: &egui::Context, id: egui::Id) -> Option<Range<usize>> {
    egui::TextEdit::load_state(ctx, id).and_then(|s| s.cursor.char_range()).map(|r| {
        let (a, b) = (r.primary.index, r.secondary.index);
        a.min(b)..a.max(b)
    })
}

/// Selects `r` in the box `id` (a cursor, when it's empty) and gives the box the focus back.
fn put_cursor(ctx: &egui::Context, id: egui::Id, r: Range<usize>) {
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(r.start), CCursor::new(r.end))));
    state.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Puts `with` in place of the characters `range` of `text`.
fn replace_chars(text: &mut String, range: Range<usize>, with: &str) {
    let byte = |i: usize| text.char_indices().nth(i).map(|(b, _)| b).unwrap_or(text.len());
    let (a, b) = (byte(range.start), byte(range.end));
    text.replace_range(a..b, with);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacing_counts_characters_not_bytes() {
        let mut t = "héllo wörld".to_string();
        replace_chars(&mut t, 6..11, "there");
        assert_eq!(t, "héllo there");
        replace_chars(&mut t, 0..0, "→ ");
        assert_eq!(t, "→ héllo there");
        let n = t.chars().count();
        replace_chars(&mut t, n..n, "!");
        assert_eq!(t, "→ héllo there!");
    }
}
