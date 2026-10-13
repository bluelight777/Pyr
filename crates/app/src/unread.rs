//! What's unread across all of Ingle, for the places outside the window that show it: the icon
//! by the clock, the taskbar button and the window's title.

use ingle_engine::state::AppState;

/// Unread conversations, everywhere.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Unread {
    /// Where: direct messages by who sent them, then channels.
    pub places: Vec<String>,
    /// Mentions and direct messages (what the red badges add up to).
    pub pings: u32,
}

impl Unread {
    /// Muted conversations don't count (unless you're mentioned there), nor do people you
    /// blocked: the same as the dots and badges in the window.
    pub fn of(st: &AppState) -> Unread {
        let several = st.servers.len() > 1;
        let (mut dms, mut channels, mut pings) = (Vec::new(), Vec::new(), 0u32);
        for s in &st.servers {
            let mut ids: Vec<_> = s.unread.iter().chain(s.mentions.keys()).copied().collect();
            ids.sort_unstable();
            ids.dedup();
            for id in ids {
                let mentions = s.mentions.get(&id).copied().unwrap_or(0);
                if let Some(d) = s.dm(id) {
                    if st.blocked.contains(&d.with) || st.muted_channels.contains(&(s.key.clone(), id)) {
                        continue;
                    }
                    let name = s.member_name(&d.with);
                    if !dms.contains(&name) {
                        dms.push(name);
                    }
                    pings += mentions;
                } else if let Some(c) = s.channel(id) {
                    if !s.unread.contains(&id) && mentions == 0 {
                        continue;
                    }
                    channels.push(if several { format!("#{} ({})", c.name, s.name) } else { format!("#{}", c.name) });
                    pings += mentions;
                }
            }
        }
        dms.extend(channels);
        Unread { places: dms, pings }
    }

    pub fn any(&self) -> bool {
        !self.places.is_empty()
    }

    /// For the icon by the clock: `app`, and below it where the unread messages are (Windows
    /// shows at most 127 characters).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn tooltip(&self, app: &str) -> String {
        if !self.any() {
            return app.to_string();
        }
        let mut s = format!("{app}\nUnread: ");
        for (i, p) in self.places.iter().enumerate() {
            let sep = if i == 0 { "" } else { ", " };
            // (Room left for " and 99 more".)
            if s.chars().count() + sep.len() + p.chars().count() > 112 {
                s.push_str(&format!(" and {} more", self.places.len() - i));
                break;
            }
            s.push_str(sep);
            s.push_str(p);
        }
        s
    }

    /// The window's title (on the taskbar button and in Alt+Tab): "(2) Ingle" for two
    /// mentions or direct messages, "• Ingle" for other unread messages.
    pub fn title(&self, app: &str) -> String {
        if self.pings > 0 {
            format!("({}) {app}", self.pings.min(999))
        } else if self.any() {
            format!("• {app}")
        } else {
            app.to_string()
        }
    }
}

/// The badge colours: red for mentions and direct messages, light for other unread messages,
/// each with a dark ring so it shows on a light taskbar too.
#[cfg_attr(not(windows), allow(dead_code))]
pub const RED: [u8; 3] = [0xe5, 0x48, 0x4d];
#[cfg_attr(not(windows), allow(dead_code))]
pub const LIGHT: [u8; 3] = [0xee, 0xe6, 0xdf];
#[cfg_attr(not(windows), allow(dead_code))]
pub const RING: [u8; 3] = [0x15, 0x11, 0x0f];

/// Paint a round badge (with its ring) into `w`×`h` BGRA pixels with straight alpha (what
/// Windows icons use), centred at (`cx`, `cy`), smoothed at the edges.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn paint_dot(px: &mut [u8], w: usize, h: usize, (cx, cy): (f32, f32), r: f32, ring: f32, fill: [u8; 3]) {
    for y in 0..h {
        for x in 0..w {
            let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            // How much of the pixel the whole badge (ring included) covers, and the fill.
            let outer = (r + ring + 0.5 - d).clamp(0.0, 1.0);
            if outer <= 0.0 {
                continue;
            }
            let inner = (r + 0.5 - d).clamp(0.0, 1.0);
            let col: [f32; 3] = std::array::from_fn(|i| RING[i] as f32 * (1.0 - inner) + fill[i] as f32 * inner);
            let i = (y * w + x) * 4;
            let (da, sa) = (px[i + 3] as f32 / 255.0, outer);
            let a = sa + da * (1.0 - sa);
            for (c, v) in col.iter().enumerate() {
                // BGRA: blue first.
                let j = i + 2 - c;
                let dst = px[j] as f32;
                px[j] = if a > 0.0 { ((v * sa + dst * da * (1.0 - sa)) / a).round() as u8 } else { 0 };
            }
            px[i + 3] = (a * 255.0).round() as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use ingle_engine::state::{AppState, ServerView};
    use ingle_proto::control::{Channel, ChannelKind, Dm};

    use super::*;

    fn channel(id: i64, name: &str) -> Channel {
        Channel { id, kind: ChannelKind::Text, name: name.into(), topic: String::new(), position: 0, last_message: None, last_from: None }
    }

    fn state() -> AppState {
        let mut s = ServerView::new("k".into(), "Brethren".into(), String::new());
        s.channels = vec![channel(1, "general"), channel(2, "off-topic"), channel(3, "quiet")];
        s.dms = vec![Dm { id: 9, with: "kain".into(), last_message: Some(50), last_from: None }];
        let mut st = AppState::default();
        st.servers.push(s);
        st
    }

    #[test]
    fn nothing_unread() {
        let u = Unread::of(&state());
        assert!(!u.any());
        assert_eq!(u.title("Ingle"), "Ingle");
        assert_eq!(u.tooltip("Ingle"), "Ingle");
    }

    #[test]
    fn direct_messages_first_then_channels() {
        let mut st = state();
        let s = &mut st.servers[0];
        s.unread.extend([2, 9, 1]);
        s.mentions.insert(9, 2);
        s.mentions.insert(1, 1);
        let u = Unread::of(&st);
        // (Kain isn't in the member list here, so he goes by his id.)
        let kain = st.servers[0].member_name("kain");
        assert_eq!(u.places, [kain.as_str(), "#general", "#off-topic"]);
        assert_eq!(u.pings, 3);
        assert_eq!(u.title("Ingle"), "(3) Ingle");
        assert_eq!(u.tooltip("Ingle"), format!("Ingle\nUnread: {kain}, #general, #off-topic"));
    }

    #[test]
    fn plain_unread_gets_a_dot_and_muted_or_blocked_dont_count() {
        let mut st = state();
        st.servers[0].unread.extend([2, 9]);
        st.servers[0].mentions.insert(9, 1);
        st.muted_channels.insert(("k".into(), 9));
        let u = Unread::of(&st);
        assert_eq!(u.places, ["#off-topic"]);
        assert_eq!(u.title("Ingle"), "• Ingle");
        st.muted_channels.clear();
        st.blocked.insert("kain".into());
        assert_eq!(Unread::of(&st).places, ["#off-topic"]);
    }

    #[test]
    fn badge_dots_cover_the_corner_and_leave_the_rest() {
        let (w, h) = (16usize, 16usize);
        let mut px = vec![0u8; w * h * 4];
        // An opaque grey icon, then a red badge bottom right.
        for p in px.chunks_mut(4) {
            p.copy_from_slice(&[0x40, 0x40, 0x40, 0xff]);
        }
        paint_dot(&mut px, w, h, (12.0, 12.0), 3.0, 1.0, RED);
        let at = |x: usize, y: usize| -> [u8; 4] { px[(y * w + x) * 4..(y * w + x) * 4 + 4].try_into().unwrap() };
        assert_eq!(at(12, 12), [RED[2], RED[1], RED[0], 0xff], "the middle is the badge's colour (BGRA)");
        assert_eq!(at(2, 2), [0x40, 0x40, 0x40, 0xff], "far from it nothing changes");
        let ring = at(12, 8); // 3.5 px from the centre: on the ring
        assert!(ring[0] < 0x40 && ring[3] == 0xff, "the ring is dark: {ring:?}");

        // On a see-through corner the badge brings its own opacity.
        let mut clear = vec![0u8; w * h * 4];
        paint_dot(&mut clear, w, h, (8.0, 8.0), 5.5, 1.0, LIGHT);
        assert_eq!(clear[(8 * w + 8) * 4 + 3], 0xff);
        assert_eq!(clear[3], 0, "corners stay see-through");
    }

    #[test]
    fn long_lists_are_cut_to_fit() {
        let mut st = state();
        let s = &mut st.servers[0];
        s.channels = (1..=30).map(|i| channel(i, &format!("channel-number-{i}"))).collect();
        s.unread.extend(1..=30);
        let t = Unread::of(&st).tooltip("Ingle");
        assert!(t.chars().count() <= 127, "{} chars: {t}", t.chars().count());
        assert!(t.ends_with("more"), "{t}");
    }
}
