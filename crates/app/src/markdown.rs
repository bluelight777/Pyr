//! Discord-flavoured markdown subset: **bold**, *italic*, __underline__, ~~strike~~,
//! `code`, ```code blocks```, > quotes, links and @mentions.

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{self, Color32, Stroke};

use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Style {
    bold: bool,
    italic: bool,
    underline: bool,
    strike: bool,
}

#[derive(Debug, PartialEq)]
pub enum Block {
    Text(String),
    Quote(String),
    Code(String),
}

/// Split into text, quote and fenced code blocks.
pub fn blocks(src: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(start) = rest.find("```") {
        let before = &rest[..start];
        push_text(&mut out, before);
        let after = &rest[start + 3..];
        match after.find("```") {
            Some(end) => {
                let mut code = &after[..end];
                // drop an optional language tag on the first line
                if let Some(nl) = code.find('\n') {
                    if !code[..nl].contains(' ') && nl < 20 {
                        code = &code[nl + 1..];
                    }
                }
                out.push(Block::Code(code.trim_end_matches('\n').to_string()));
                rest = &after[end + 3..];
            }
            None => {
                push_text(&mut out, &rest[start..]);
                rest = "";
            }
        }
    }
    push_text(&mut out, rest);
    out
}

fn push_text(out: &mut Vec<Block>, text: &str) {
    let text = text.trim_matches('\n');
    if text.is_empty() {
        return;
    }
    let mut para = String::new();
    let mut quote = String::new();
    for line in text.lines() {
        if let Some(q) = line.strip_prefix("> ").or_else(|| if line == ">" { Some("") } else { None }) {
            if !para.is_empty() {
                out.push(Block::Text(std::mem::take(&mut para)));
            }
            if !quote.is_empty() {
                quote.push('\n');
            }
            quote.push_str(q);
        } else {
            if !quote.is_empty() {
                out.push(Block::Quote(std::mem::take(&mut quote)));
            }
            if !para.is_empty() {
                para.push('\n');
            }
            para.push_str(line);
        }
    }
    if !quote.is_empty() {
        out.push(Block::Quote(quote));
    }
    if !para.is_empty() {
        out.push(Block::Text(para));
    }
}

/// A clickable region inside a laid-out paragraph (char range → URL).
pub struct LinkSpan {
    pub start: usize,
    pub end: usize,
    pub url: String,
}

fn is_url_start(s: &str) -> bool {
    s.starts_with("https://") || s.starts_with("http://") || s.starts_with("ingle://")
}

/// Build a styled layout for one paragraph. Returns the job and link spans (in chars).
pub fn layout(text: &str, size: f32, color: Color32, names: &[String], wrap_width: f32) -> (LayoutJob, Vec<LinkSpan>) {
    let (job, links, _) = layout_emoji(text, size, size * crate::emoji::SCALE, color, names, wrap_width);
    (job, links)
}

/// Like `layout`, with room left for colour emoji (`emoji_size` across): also returns where
/// each one starts (char index in the job) and what it is.
pub fn layout_emoji(
    text: &str,
    size: f32,
    emoji_size: f32,
    color: Color32,
    names: &[String],
    wrap_width: f32,
) -> (LayoutJob, Vec<LinkSpan>, Vec<(usize, crate::emoji::EmojiRef)>) {
    let mut emojis = Vec::new();
    let (lead_fmt, rest_fmt) = crate::emoji::formats(emoji_size);
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap_width;
    let mut links = Vec::new();
    let mut style = Style::default();
    let mut chars_out = 0usize;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    let mut buf = String::new();

    let fmt = |st: Style, code: bool, link: bool, mention: bool| -> TextFormat {
        let font = if code {
            theme::mono(size - 1.0)
        } else if st.bold {
            theme::bold_font(size)
        } else {
            theme::font(size)
        };
        let mut f = TextFormat::simple(font, color);
        if st.italic {
            f.italics = true;
        }
        if st.underline || link {
            f.underline = Stroke::new(1.0f32, if link { theme::LINK } else { color });
        }
        if st.strike {
            f.strikethrough = Stroke::new(1.0f32, color);
        }
        if code {
            f.background = theme::CODE_BG;
            f.color = Color32::from_rgb(0xf2, 0xd9, 0xc4);
        }
        if link {
            f.color = theme::LINK;
        }
        if mention {
            f.background = Color32::from_rgb(0x4a, 0x2c, 0x1a);
            f.color = theme::EMBER2;
        }
        f
    };

    let flush = |job: &mut LayoutJob, buf: &mut String, chars_out: &mut usize, f: TextFormat| {
        if !buf.is_empty() {
            *chars_out += buf.chars().count();
            job.append(buf, 0.0, f);
            buf.clear();
        }
    };

    while i < chars.len() {
        // backslash escapes: \* \_ \~ \` \@ \\ show the character itself
        if chars[i] == '\\' && i + 1 < chars.len() && matches!(chars[i + 1], '*' | '_' | '~' | '`' | '@' | '\\' | '>' | '|') {
            buf.push(chars[i + 1]);
            i += 2;
            continue;
        }
        let rest: String = chars[i..chars.len().min(i + 9)].iter().collect();
        // inline code
        if chars[i] == '`' {
            if let Some(end) = chars[i + 1..].iter().position(|c| *c == '`') {
                flush(&mut job, &mut buf, &mut chars_out, fmt(style, false, false, false));
                let code: String = chars[i + 1..i + 1 + end].iter().collect();
                let mut b = code;
                flush(&mut job, &mut b, &mut chars_out, fmt(style, true, false, false));
                i += end + 2;
                continue;
            }
        }
        // links
        if is_url_start(&rest) && (i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(') {
            flush(&mut job, &mut buf, &mut chars_out, fmt(style, false, false, false));
            let mut j = i;
            while j < chars.len() && !chars[j].is_whitespace() {
                j += 1;
            }
            // trailing punctuation isn't part of the link
            while j > i && matches!(chars[j - 1], '.' | ',' | ')' | '!' | '?' | ':' | ';') {
                j -= 1;
            }
            let url: String = chars[i..j].iter().collect();
            let start = chars_out;
            let mut b = url.clone();
            flush(&mut job, &mut b, &mut chars_out, fmt(style, false, true, false));
            links.push(LinkSpan { start, end: chars_out, url });
            i = j;
            continue;
        }
        // mentions
        if chars[i] == '@' && (i == 0 || chars[i - 1].is_whitespace()) {
            let lower: String = chars[i + 1..].iter().collect::<String>().to_lowercase();
            // The longest known name (people, roles, everyone, here) that follows the @.
            let name = names
                .iter()
                .filter(|n| !n.is_empty() && lower.starts_with(n.as_str()))
                .filter(|n| lower[n.len()..].chars().next().map(|c| !c.is_alphanumeric() && c != '_').unwrap_or(true))
                .map(|n| n.chars().count())
                .max();
            if let Some(n) = name {
                flush(&mut job, &mut buf, &mut chars_out, fmt(style, false, false, false));
                let mut b: String = chars[i..i + 1 + n].iter().collect();
                flush(&mut job, &mut b, &mut chars_out, fmt(style, false, false, true));
                i += 1 + n;
                continue;
            }
        }
        // emphasis markers
        let two: String = chars[i..chars.len().min(i + 2)].iter().collect();
        let toggled = match two.as_str() {
            "**" => Some((2, Style { bold: !style.bold, ..style })),
            "__" => Some((2, Style { underline: !style.underline, ..style })),
            "~~" => Some((2, Style { strike: !style.strike, ..style })),
            _ if chars[i] == '*' || (chars[i] == '_' && (i == 0 || !chars[i - 1].is_alphanumeric())) => {
                Some((1, Style { italic: !style.italic, ..style }))
            }
            _ => None,
        };
        if let Some((n, new_style)) = toggled {
            // only toggle on if a closing marker exists later
            let closing_exists = {
                let marker: String = chars[i..i + n].iter().collect();
                let after: String = chars[i + n..].iter().collect();
                after.contains(&marker)
            };
            let turning_off = (n == 2 && ((two == "**" && style.bold) || (two == "__" && style.underline) || (two == "~~" && style.strike)))
                || (n == 1 && style.italic);
            if closing_exists || turning_off {
                flush(&mut job, &mut buf, &mut chars_out, fmt(style, false, false, false));
                style = new_style;
                i += n;
                continue;
            }
        }
        if let Some((n, e)) = crate::emoji::match_at(&chars, i) {
            flush(&mut job, &mut buf, &mut chars_out, fmt(style, false, false, false));
            emojis.push((chars_out, e));
            let mut lead = chars[i].to_string();
            flush(&mut job, &mut lead, &mut chars_out, lead_fmt.clone());
            if n > 1 {
                let mut rest: String = chars[i + 1..i + n].iter().collect();
                flush(&mut job, &mut rest, &mut chars_out, rest_fmt.clone());
            }
            i += n;
            continue;
        }
        buf.push(chars[i]);
        i += 1;
    }
    flush(&mut job, &mut buf, &mut chars_out, fmt(style, false, false, false));
    (job, links, emojis)
}

/// Turns a server emoji's file id into a texture (or none yet).
pub type CustomEmoji<'a> = &'a mut dyn FnMut(&str) -> Option<egui::TextureId>;

/// Draw a message body; `names` (lowercase) are what @mentions highlight.
pub fn show(ui: &mut egui::Ui, text: &str, color: Color32, names: &[String], custom: CustomEmoji) {
    let width = ui.available_width();
    let emoji_size = if crate::emoji::is_jumbo(text) { crate::emoji::JUMBO_SIZE } else { 14.5 * crate::emoji::SCALE };
    for block in blocks(text) {
        match block {
            Block::Code(code) => {
                egui::Frame::new()
                    .fill(theme::CODE_BG)
                    .stroke(Stroke::new(1.0f32, theme::LINE))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(ui, |ui| {
                        ui.set_width(width - 24.0);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(code).font(theme::mono(13.0)).color(Color32::from_rgb(0xf2, 0xd9, 0xc4)),
                            )
                            .selectable(true),
                        );
                    });
            }
            Block::Quote(q) => {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(4.0, 18.0), egui::Sense::hover());
                    ui.painter().rect_filled(r, 2, theme::DIM);
                    paragraph(ui, &q, theme::MUTED, names, width - 16.0, emoji_size, custom);
                });
            }
            Block::Text(t) => paragraph(ui, &t, color, names, width, emoji_size, custom),
        }
    }
}

fn paragraph(ui: &mut egui::Ui, text: &str, color: Color32, names: &[String], width: f32, emoji_size: f32, custom: CustomEmoji) {
    let (job, links, emojis) = layout_emoji(text, 14.5, emoji_size, color, names, width);
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let resp = ui.add(egui::Label::new(galley.clone()).selectable(true).sense(egui::Sense::click()));
    if !emojis.is_empty() {
        let starts: Vec<usize> = emojis.iter().map(|e| e.0).collect();
        for (r, (_, e)) in crate::emoji::placed(&galley, &starts).into_iter().zip(&emojis) {
            if r.is_positive() && ui.is_rect_visible(r.translate(resp.rect.min.to_vec2())) {
                let rect = r.translate(resp.rect.min.to_vec2());
                crate::emoji::paint(ui.painter(), rect, e, custom);
                if let crate::emoji::EmojiRef::Custom { name, .. } = e {
                    let id = ui.id().with(("emoji", r.min.x as i32, r.min.y as i32));
                    ui.interact(rect, id, egui::Sense::hover()).on_hover_text(format!(":{name}:"));
                }
            }
        }
    }
    if links.is_empty() {
        return;
    }
    if let Some(pos) = resp.hover_pos() {
        let local = pos - resp.rect.min;
        let idx = galley.cursor_from_pos(local).index;
        if links.iter().any(|l| idx >= l.start && idx < l.end) {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
    }
    if resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let idx = galley.cursor_from_pos(pos - resp.rect.min).index;
            if let Some(l) = links.iter().find(|l| idx >= l.start && idx < l.end) {
                ui.ctx().open_url(egui::OpenUrl::new_tab(l.url.clone()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_code_and_quotes() {
        let b = blocks("hi\n> quoted\n> more\nafter\n```rust\nfn main() {}\n```\ntail");
        assert_eq!(
            b,
            vec![
                Block::Text("hi".into()),
                Block::Quote("quoted\nmore".into()),
                Block::Text("after".into()),
                Block::Code("fn main() {}".into()),
                Block::Text("tail".into()),
            ]
        );
    }

    #[test]
    fn finds_links_and_strips_markers() {
        let (job, links) = layout("see https://example.com/x, **bold** and `code`", 14.0, Color32::WHITE, &[], 400.0);
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].url, "https://example.com/x");
        assert_eq!(job.text, "see https://example.com/x, bold and code");
        let span: String = job.text.chars().skip(links[0].start).take(links[0].end - links[0].start).collect();
        assert_eq!(span, "https://example.com/x");
    }

    #[test]
    fn lone_asterisk_is_literal() {
        let (job, _) = layout("2 * 3 = 6", 14.0, Color32::WHITE, &[], 400.0);
        assert_eq!(job.text, "2 * 3 = 6");
    }

    #[test]
    fn backslash_escapes_markers() {
        let (job, _) = layout(r"\*not italic\* and \`not code\` but C:\path stays", 14.0, Color32::WHITE, &[], 400.0);
        assert_eq!(job.text, r"*not italic* and `not code` but C:\path stays");
        assert!(job.sections.iter().all(|s| s.format.background == Color32::TRANSPARENT && !s.format.italics));
    }
}
