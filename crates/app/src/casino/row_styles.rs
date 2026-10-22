//! How each of Reel Row's machines is built from 0.13.0: none alike. Each stands in its own room
//! (Meshy's art: a hot-sauce cantina, a brass submarine, a starship's bridge, a candy shop, a
//! mine, a neon arcade, a torchlit tomb, an ice-fishing cabin, a witch's kitchen, a jade
//! dragon temple, a coffee house, a theatre) and has its own lettering, its own reel frame, its
//! own spin button and its own way of showing a win.
//!
//! Where the room has a place for the reels (the cantina's board, the submarine's viewport, the
//! candy shop's case, the cabin's window, the arcade's screen, the theatre's stage) they sit in
//! it, the room drawn round them; elsewhere they stand in a cabinet of the room's own making.

use eframe::egui::{Color32, Rect, pos2};

/// The cabinet the reels stand in (or the trim round the room's own place for them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Frame {
    /// Riveted brass (the submarine).
    Brass,
    /// A starship's holographic panel: glass and light, no frame to speak of.
    Holo,
    /// Rough timbers and iron straps (the mine).
    Timber,
    /// Neon tubes in two colours, flickering on.
    Neon,
    /// Carved sandstone and gold (the tomb).
    Stone,
    /// Iced logs (the cabin).
    Ice,
    /// A witch's arch: dark wood, runes that glow.
    Arcane,
    /// Jade and gold, a dragon's scales round it.
    Jade,
    /// A chalkboard in a wooden frame (the coffee house).
    Chalk,
    /// Theatre lights round a gilt frame.
    Marquee,
    /// Candy stripes (the sweet shop).
    Candy,
    /// Scorched wood and chili lights (the cantina).
    Cantina,
}

/// The spin button's shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// A round button with a ring of lights.
    Round,
    /// A hexagon (sci-fi).
    Hex,
    /// A diamond.
    Diamond,
    /// A pill (wide).
    Pill,
    /// A shield (tomb, dragon).
    Shield,
    /// A star.
    Star,
    /// A brass wheel (submarine).
    Wheel,
}

/// What a win looks like: what flies up out of the reels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fx {
    Embers,
    Bubbles,
    Stars,
    Sprinkles,
    Nuggets,
    Sparks,
    Sand,
    Snow,
    Wisps,
    Coins,
    Steam,
    Confetti,
}

pub struct Style {
    /// The marquee's lettering (`theme::display_font`).
    pub font: &'static str,
    /// The room's place for the reels (fractions of its art, across and down), if it has one.
    pub window: Option<Rect>,
    pub frame: Frame,
    pub button: Button,
    pub fx: Fx,
    /// The reels' backing: top and bottom.
    pub reel_top: Color32,
    pub reel_bottom: Color32,
    /// A second colour for lights and trim (the theme's accent is the first).
    pub second: Color32,
    /// How dark the room is drawn behind the reels (0 bright, 1 black).
    pub dim: f32,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

const fn w(x0: f32, y0: f32, x1: f32, y1: f32) -> Option<Rect> {
    Some(Rect { min: pos2(x0, y0), max: pos2(x1, y1) })
}

pub const STYLES: [Style; 12] = [
    // Scoville Slots: the cantina's chalk-and-wood menu board, chili lights, embers.
    Style { font: "bangers", window: w(0.28, 0.245, 0.72, 0.765), frame: Frame::Cantina, button: Button::Star, fx: Fx::Embers, reel_top: c(0x2a, 0x0a, 0x04), reel_bottom: c(0x12, 0x04, 0x02), second: c(0xff, 0x40, 0x20), dim: 0.25 },
    // Deep Sea Treasure: the submarine's viewport, a brass wheel to spin, bubbles.
    Style { font: "pirata", window: w(0.29, 0.28, 0.71, 0.715), frame: Frame::Brass, button: Button::Wheel, fx: Fx::Bubbles, reel_top: c(0x04, 0x22, 0x3a), reel_bottom: c(0x01, 0x0a, 0x16), second: c(0xd8, 0xa0, 0x4a), dim: 0.2 },
    // Cosmic Jackpot: a hologram over the bridge's window, a hex button, stars.
    Style { font: "orbitron", window: None, frame: Frame::Holo, button: Button::Hex, fx: Fx::Stars, reel_top: c(0x10, 0x0c, 0x30), reel_bottom: c(0x04, 0x02, 0x12), second: c(0x4a, 0xe8, 0xff), dim: 0.3 },
    // Sugar Rush: the candy shop's glass case, a pill of a button, sprinkles.
    Style { font: "chewy", window: w(0.275, 0.385, 0.725, 0.80), frame: Frame::Candy, button: Button::Pill, fx: Fx::Sprinkles, reel_top: c(0x4a, 0x10, 0x34), reel_bottom: c(0x22, 0x06, 0x18), second: c(0x8a, 0xff, 0xd8), dim: 0.05 },
    // Gold Rush: a timber-framed cabinet in the mine, a diamond button, nuggets.
    Style { font: "rye", window: None, frame: Frame::Timber, button: Button::Diamond, fx: Fx::Nuggets, reel_top: c(0x2a, 0x1a, 0x08), reel_bottom: c(0x10, 0x08, 0x02), second: c(0xc8, 0x8a, 0x3a), dim: 0.35 },
    // Neon Nights: the arcade's big screen, neon tubes, sparks.
    Style { font: "monoton", window: w(0.255, 0.245, 0.745, 0.705), frame: Frame::Neon, button: Button::Round, fx: Fx::Sparks, reel_top: c(0x16, 0x04, 0x2a), reel_bottom: c(0x06, 0x01, 0x10), second: c(0x30, 0xe8, 0xff), dim: 0.15 },
    // Tomb of Ra: a sandstone tablet between the torches, a shield button, sand.
    Style { font: "cinzel", window: None, frame: Frame::Stone, button: Button::Shield, fx: Fx::Sand, reel_top: c(0x2a, 0x1e, 0x0a), reel_bottom: c(0x12, 0x0c, 0x04), second: c(0x2a, 0xc8, 0xb0), dim: 0.3 },
    // Frostbite Fortune: the cabin's window, iced logs, snow.
    Style { font: "iceberg", window: w(0.285, 0.375, 0.715, 0.805), frame: Frame::Ice, button: Button::Round, fx: Fx::Snow, reel_top: c(0x0a, 0x22, 0x3a), reel_bottom: c(0x02, 0x0c, 0x18), second: c(0x8a, 0xff, 0xc8), dim: 0.15 },
    // Moonlit Cauldron: a witch's arch, glowing runes, wisps.
    Style { font: "creepster", window: None, frame: Frame::Arcane, button: Button::Star, fx: Fx::Wisps, reel_top: c(0x1a, 0x0a, 0x2a), reel_bottom: c(0x08, 0x02, 0x10), second: c(0xc0, 0x6a, 0xff), dim: 0.35 },
    // Dragon's Hoard: jade and gold in the temple, a shield button, coins.
    Style { font: "uncial", window: None, frame: Frame::Jade, button: Button::Shield, fx: Fx::Coins, reel_top: c(0x06, 0x26, 0x18), reel_bottom: c(0x02, 0x10, 0x08), second: c(0xff, 0x4a, 0x3a), dim: 0.35 },
    // The Daily Grind: a chalkboard on the café wall, a pill button, steam.
    Style { font: "pacifico", window: None, frame: Frame::Chalk, button: Button::Pill, fx: Fx::Steam, reel_top: c(0x1e, 0x22, 0x1e), reel_bottom: c(0x0e, 0x10, 0x0e), second: c(0xf0, 0xe6, 0xd2), dim: 0.3 },
    // Starlight Cabaret: on the stage, theatre lights round it, confetti.
    Style { font: "limelight", window: w(0.315, 0.43, 0.725, 0.885), frame: Frame::Marquee, button: Button::Star, fx: Fx::Confetti, reel_top: c(0x2a, 0x04, 0x0c), reel_bottom: c(0x10, 0x01, 0x04), second: c(0xff, 0xf0, 0xc8), dim: 0.2 },
];

pub fn style(m: u8) -> &'static Style {
    &STYLES[(m as usize).min(STYLES.len() - 1)]
}
