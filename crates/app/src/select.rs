//! Selecting text in a conversation the way a web page does (Discord's is one): press anywhere
//! over the messages and drag, across as many messages as you like and past the ends of lines;
//! double-click a word, triple-click a paragraph. Ctrl+C copies it, and so does the right-click
//! menu, which also has Copy link over a link. A plain click on a link still opens it.
//!
//! Message text is drawn here (not as egui labels, whose selection stops at the edge of each
//! label and is lost on a right-click): each block of a message (a paragraph, quote or code
//! block) is a `Piece`, kept from one frame to the next so a press can be placed in the text
//! that was on screen.

use std::sync::Arc;

use eframe::egui::text::CCursor;
use eframe::egui::text_selection::CCursorRange;
use eframe::egui::{self, Galley, Pos2, Rect, Sense, vec2};
use ingle_proto::control::{ChannelId, MessageId};

/// A conversation: (server key, channel).
pub type Chat = (String, ChannelId);

/// One block of a message's text, in the order they show: messages by id, then their blocks
/// top to bottom.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Part {
    pub msg: MessageId,
    pub block: u32,
}

/// A place in the text, between two characters.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Spot {
    part: Part,
    at: CCursor,
}

impl Spot {
    fn order(&self) -> (Part, usize) {
        (self.part, self.at.index)
    }
}

/// A block of text as it was drawn.
struct Piece {
    part: Part,
    galley: Arc<Galley>,
    /// Its top left on screen.
    pos: Pos2,
    /// Its links: (first character, one past the last, where to).
    links: Vec<(usize, usize, String)>,
}

impl Piece {
    fn rect(&self) -> Rect {
        Rect::from_min_size(self.pos, self.galley.size())
    }

    fn chars(&self) -> usize {
        self.galley.text().chars().count()
    }

    /// The link under `p`, if any (only over its letters: not beside them).
    fn link_at(&self, p: Pos2) -> Option<&str> {
        let i = char_under(&self.galley, p - self.pos)?;
        self.links.iter().find(|l| l.0 <= i && i < l.1).map(|l| l.2.as_str())
    }
}

/// What one conversation drew in a frame.
struct Drawn {
    chat: Chat,
    pieces: Vec<Piece>,
    /// Each message's whole row (for the right-click menu).
    rows: Vec<(MessageId, Rect)>,
}

/// What a right-click was on (kept while its menu is open).
#[derive(Clone, Debug, Default)]
pub struct Menu {
    /// The selected text, when there was some.
    pub copy: Option<String>,
    pub link: Option<String>,
    pub msg: Option<MessageId>,
}

#[derive(Default)]
pub struct Selection {
    /// The conversation it's in.
    chat: Option<Chat>,
    /// Where the press was, and where the pointer is (or was let go).
    anchor: Option<Spot>,
    focus: Option<Spot>,
    dragging: bool,
    /// Which pass `this` is from.
    pass: u64,
    /// Drawn this frame, and last frame (what's on screen: presses are placed in that).
    this: Vec<Drawn>,
    last: Vec<Drawn>,
    pub menu: Option<Menu>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.anchor = None;
        self.focus = None;
        self.dragging = false;
    }

    /// A new frame: what was drawn is what's on screen now.
    fn turn(&mut self, ctx: &egui::Context) {
        let pass = ctx.cumulative_pass_nr();
        if pass == self.pass {
            return;
        }
        self.pass = pass;
        self.last = std::mem::take(&mut self.this);
        // Its conversation isn't showing anymore: it's gone, like on a web page.
        if self.chat.as_ref().is_some_and(|c| !self.last.iter().any(|d| &d.chat == c)) {
            self.clear();
            self.chat = None;
        }
    }

    fn drawing(&mut self, chat: &Chat) -> &mut Drawn {
        match self.this.iter().position(|d| &d.chat == chat) {
            Some(i) => &mut self.this[i],
            None => {
                self.this.push(Drawn { chat: chat.clone(), pieces: Vec::new(), rows: Vec::new() });
                self.this.last_mut().expect("just pushed")
            }
        }
    }

    fn shown(&self, chat: &Chat) -> Option<&Drawn> {
        self.last.iter().find(|d| &d.chat == chat)
    }

    /// Start to end, in reading order.
    fn sorted(&self) -> Option<(Spot, Spot)> {
        let (a, f) = (self.anchor?, self.focus?);
        Some(if f.order() < a.order() { (f, a) } else { (a, f) })
    }

    fn has_text(&self) -> bool {
        self.sorted().is_some_and(|(a, b)| a.order() != b.order())
    }

    /// The part of block `part` that's selected (in `chat`).
    fn range_in(&self, chat: &Chat, part: Part, galley: &Galley) -> Option<(CCursor, CCursor)> {
        if self.chat.as_ref() != Some(chat) {
            return None;
        }
        let (a, b) = self.sorted()?;
        if part < a.part || part > b.part {
            return None;
        }
        let start = if part == a.part { a.at } else { galley.begin() };
        let end = if part == b.part { b.at } else { galley.end() };
        (start.index < end.index).then_some((start, end))
    }

    /// The selected text, blocks on lines of their own.
    pub fn text(&self) -> Option<String> {
        let chat = self.chat.as_ref()?;
        let (a, b) = self.sorted()?;
        let drawn = self.shown(chat)?;
        let mut out = String::new();
        let mut first = true;
        for p in drawn.pieces.iter().filter(|p| a.part <= p.part && p.part <= b.part) {
            let from = if p.part == a.part { a.at.index } else { 0 };
            let to = if p.part == b.part { b.at.index } else { p.chars() };
            if from >= to {
                continue;
            }
            if !first {
                out.push('\n');
            }
            first = false;
            out.extend(p.galley.text().chars().skip(from).take(to - from));
        }
        (!out.is_empty()).then_some(out)
    }

    /// Where in the text `p` is: in a block, or between blocks (the start of the one below),
    /// or past the last one (its end). Left or right of the text counts as its line.
    fn spot_at(&self, chat: &Chat, p: Pos2) -> Option<Spot> {
        let drawn = self.shown(chat)?;
        for piece in &drawn.pieces {
            let r = piece.rect();
            if p.y <= r.bottom() {
                let at = if p.y < r.top() { piece.galley.begin() } else { piece.galley.cursor_from_pos(p - piece.pos) };
                return Some(Spot { part: piece.part, at });
            }
        }
        let last = drawn.pieces.last()?;
        Some(Spot { part: last.part, at: last.galley.end() })
    }

    fn piece_at(&self, chat: &Chat, p: Pos2) -> Option<&Piece> {
        self.shown(chat)?.pieces.iter().find(|x| x.rect().contains(p))
    }

    fn link_at(&self, chat: &Chat, p: Pos2) -> Option<String> {
        self.piece_at(chat, p).and_then(|x| x.link_at(p)).map(str::to_string)
    }

    fn msg_at(&self, chat: &Chat, p: Pos2) -> Option<MessageId> {
        self.shown(chat)?.rows.iter().find(|r| r.1.contains(p)).map(|r| r.0)
    }

    /// The pointer over the messages of `chat` (`bg`: the whole view, under everything in it).
    /// Returns a link that was clicked, to open.
    pub fn input(&mut self, ui: &egui::Ui, bg: &egui::Response, chat: &Chat) -> Option<String> {
        let ctx = ui.ctx().clone();
        self.turn(&ctx);
        let (pointer, pressed, down, shift, escape, copy) = ctx.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.modifiers.shift,
                i.key_pressed(egui::Key::Escape),
                i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut)),
            )
        });
        let here = self.chat.as_ref() == Some(chat);
        if pressed {
            if bg.is_pointer_button_down_on() {
                match pointer.and_then(|p| self.spot_at(chat, p)) {
                    Some(spot) => {
                        // Shift-click: the selection now ends here.
                        if !(shift && here && self.anchor.is_some()) {
                            self.anchor = Some(spot);
                        }
                        self.chat = Some(chat.clone());
                        self.focus = Some(spot);
                        self.dragging = true;
                    }
                    None => self.clear(),
                }
            } else if here && !egui::Popup::is_any_open(&ctx) {
                // Pressed somewhere else (the message box, the channel list…): it's over.
                self.clear();
            }
        }
        let here = self.chat.as_ref() == Some(chat);
        if self.dragging && here {
            if down {
                if let Some(p) = pointer {
                    if let Some(spot) = self.spot_at(chat, p) {
                        self.focus = Some(spot);
                    }
                    // Held above or below the messages: they scroll, so it can go on.
                    let view = ui.clip_rect();
                    let edge = 14.0;
                    let speed = if p.y < view.top() + edge {
                        (view.top() + edge - p.y) * 0.5
                    } else if p.y > view.bottom() - edge {
                        -(p.y - (view.bottom() - edge)) * 0.5
                    } else {
                        0.0
                    };
                    if speed != 0.0 {
                        let speed = speed.signum() * speed.abs().clamp(3.0, 40.0);
                        ui.scroll_with_delta_animation(vec2(0.0, speed), egui::style::ScrollAnimation::none());
                        ctx.request_repaint();
                    }
                }
                ctx.set_cursor_icon(egui::CursorIcon::Text);
            } else {
                self.dragging = false;
            }
        }
        let mut open = None;
        if let Some(p) = pointer.filter(|_| here) {
            if bg.triple_clicked() {
                if let Some(piece) = self.piece_at(chat, p) {
                    let part = piece.part;
                    let end = piece.galley.end();
                    self.anchor = Some(Spot { part, at: CCursor::default() });
                    self.focus = Some(Spot { part, at: end });
                }
            } else if bg.double_clicked() {
                if let Some(piece) = self.piece_at(chat, p) {
                    let i = char_under(&piece.galley, p - piece.pos).unwrap_or_else(|| piece.galley.cursor_from_pos(p - piece.pos).index);
                    let (from, to) = word_at(piece.galley.text(), i);
                    let part = piece.part;
                    self.anchor = Some(Spot { part, at: CCursor::new(from) });
                    self.focus = Some(Spot { part, at: CCursor::new(to) });
                }
            } else if bg.clicked() && !shift {
                open = self.link_at(chat, p);
            }
        }
        if bg.secondary_clicked() {
            self.menu = pointer.map(|p| Menu {
                copy: if here { self.text() } else { None },
                link: self.link_at(chat, p),
                msg: self.msg_at(chat, p),
            });
        }
        // (Escape with the right-click menu open just closes the menu.)
        if here && escape && !egui::Popup::is_any_open(&ctx) {
            self.clear();
        }
        if here && copy && self.has_text() {
            if let Some(text) = self.text() {
                ctx.copy_text(text);
            }
        }
        open
    }

    /// Draws a block of text (`links`: its links, as for `Piece`), with what's selected of it
    /// lit up. Returns where it is.
    pub fn draw(&mut self, ui: &mut egui::Ui, chat: &Chat, part: Part, galley: Arc<Galley>, links: Vec<(usize, usize, String)>) -> Rect {
        let (rect, _) = ui.allocate_exact_size(galley.size(), Sense::hover());
        if ui.is_rect_visible(rect) {
            let mut shown = galley.clone();
            if let Some((a, b)) = self.range_in(chat, part, &galley) {
                egui::text_selection::visuals::paint_text_selection(&mut shown, ui.visuals(), &CCursorRange::two(a, b), None);
            }
            ui.painter().galley(rect.min, shown, ui.visuals().text_color());
            if !self.dragging && ui.rect_contains_pointer(rect) {
                if let Some(p) = ui.ctx().pointer_hover_pos() {
                    let over_link = char_under(&galley, p - rect.min).is_some_and(|i| links.iter().any(|l| l.0 <= i && i < l.1));
                    ui.ctx().set_cursor_icon(if over_link { egui::CursorIcon::PointingHand } else { egui::CursorIcon::Text });
                }
            }
        }
        self.drawing(chat).pieces.push(Piece { part, galley, pos: rect.min, links });
        rect
    }

    /// A message's whole row, as drawn (for what a right-click is on).
    pub fn row(&mut self, chat: &Chat, msg: MessageId, rect: Rect) {
        self.drawing(chat).rows.push((msg, rect));
    }
}

/// The character whose glyph is under `p` (galley coordinates), if any.
fn char_under(galley: &Galley, p: egui::Vec2) -> Option<usize> {
    let mut first = 0;
    for row in &galley.rows {
        if row.min_y() <= p.y && p.y <= row.max_y() {
            let x = p.x - row.pos.x;
            return row.glyphs.iter().position(|g| {
                let r = g.logical_rect();
                r.min.x <= x && x < r.max.x
            })
            .map(|i| first + i);
        }
        first += row.char_count_including_newline();
    }
    None
}

/// The word (or run of spaces, or of other marks) at character `i`: (start, end).
fn word_at(text: &str, i: usize) -> (usize, usize) {
    let chars: Vec<char> = text.chars().collect();
    if chars.is_empty() {
        return (0, 0);
    }
    let i = i.min(chars.len() - 1);
    let kind = |c: char| if c.is_alphanumeric() || c == '_' || c == '\'' { 0 } else if c.is_whitespace() { 1 } else { 2 };
    let k = kind(chars[i]);
    let mut from = i;
    while from > 0 && kind(chars[from - 1]) == k {
        from -= 1;
    }
    let mut to = i + 1;
    while to < chars.len() && kind(chars[to]) == k {
        to += 1;
    }
    (from, to)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words() {
        let t = "see https://iptv-org.github.io/iptv/index.m3u now";
        assert_eq!(word_at(t, 0), (0, 3));
        assert_eq!(word_at(t, 3), (3, 4));
        // In a link: just that word of it.
        let i = t.find("iptv").unwrap();
        assert_eq!(word_at(t, i + 1), (i, i + 4));
        assert_eq!(word_at("don't stop", 2), (0, 5));
        assert_eq!(word_at("", 3), (0, 0));
        assert_eq!(word_at("end", 10), (0, 3));
    }

    #[test]
    fn order() {
        let a = Spot { part: Part { msg: 5, block: 0 }, at: CCursor::new(9) };
        let b = Spot { part: Part { msg: 7, block: 0 }, at: CCursor::new(1) };
        let c = Spot { part: Part { msg: 5, block: 1 }, at: CCursor::new(0) };
        assert!(a.order() < b.order());
        assert!(a.order() < c.order() && c.order() < b.order());
        let mut s = Selection { chat: Some(("k".into(), 1)), anchor: Some(b), focus: Some(a), ..Default::default() };
        assert_eq!(s.sorted(), Some((a, b)));
        assert!(s.has_text());
        s.focus = Some(b);
        assert!(!s.has_text());
    }
}
