//! Pyr Casino's keys, your way (0.13.26): every game's moves and the keys that make them. Each
//! move starts with keys of its own; the Keys window (a game's top bar, the casino's menu bar, a
//! poker table's menu) changes them: click a key and press the one wanted (Ctrl, Alt or Shift
//! with it if you like), two at most a move; a key another move had in the same game moves over,
//! and the window says so. × takes a key off, ↺ puts a move back, and one button puts every
//! key back. Keys never fire while you're typing, nor while the window's open.
//!
//! 0.13.28's moves, words and names, file for file: what's changed is kept in the settings
//! (`casino_keys`) as 0.13.28 keeps it, so a Pyr updated from it keeps its keys.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};

use eframe::egui::{self, Align2, Color32, Key, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use parking_lot::RwLock;
use pyr_poker::casino::Game;

use crate::theme;

/// A chapter of the Keys window: the moves every game has, and each game's own.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Section {
    Every,
    Slots,
    Poker,
    Blackjack,
    Craps,
    Baccarat,
    Roulette,
    Domino,
    Dice,
    Keno,
    Plinko,
}

impl Section {
    pub const ALL: [Section; 11] = [
        Section::Every,
        Section::Slots,
        Section::Poker,
        Section::Blackjack,
        Section::Craps,
        Section::Baccarat,
        Section::Roulette,
        Section::Domino,
        Section::Dice,
        Section::Keno,
        Section::Plinko,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Section::Every => "Every game",
            Section::Slots => "Slots (every machine)",
            Section::Poker => "Poker",
            Section::Blackjack => "Blackjack",
            Section::Craps => "Craps",
            Section::Baccarat => "Baccarat",
            Section::Roulette => "Roulette",
            Section::Domino => "Dominó",
            Section::Dice => "Dice",
            Section::Keno => "Keno",
            Section::Plinko => "Plinko",
        }
    }

    pub fn moves(self) -> &'static [Move] {
        use Move::*;
        match self {
            Section::Every => &[ChipSmaller, ChipBigger],
            Section::Slots => &[SlotsSpin, SlotsBetDown, SlotsBetUp],
            Section::Poker => &[PokerFold, PokerCall, PokerRaise, PokerMin, PokerHalf, PokerThree, PokerPot, PokerAllin, PokerPeek],
            Section::Blackjack => &[BjHit, BjStand, BjDouble, BjSplit, BjDeal, BjRebet],
            Section::Craps => &[CrapsRoll, CrapsClear],
            Section::Baccarat => &[BacDeal, BacClear, BacRebet],
            Section::Roulette => &[RlSpin, RlDouble, RlClear, RlRebet, RlUndo],
            Section::Domino => &[DomGo],
            Section::Dice => &[DiceRoll],
            Section::Keno => &[KenoPlay],
            Section::Plinko => &[PlinkoDrop],
        }
    }

    /// A casino game's chapter.
    pub fn of(game: Game) -> Section {
        match game {
            Game::Slots | Game::Reels | Game::Dragon => Section::Slots,
            Game::Blackjack => Section::Blackjack,
            Game::Roulette => Section::Roulette,
            Game::Baccarat => Section::Baccarat,
            Game::Craps => Section::Craps,
            Game::Dice => Section::Dice,
            Game::Keno => Section::Keno,
            Game::Plinko => Section::Plinko,
        }
    }

    fn of_move(mv: Move) -> Section {
        Section::ALL.into_iter().find(|s| s.moves().contains(&mv)).unwrap_or(Section::Every)
    }
}

/// A move in a game, and the keys it starts with.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Move {
    ChipSmaller,
    ChipBigger,
    SlotsSpin,
    SlotsBetDown,
    SlotsBetUp,
    PokerFold,
    PokerCall,
    PokerRaise,
    PokerMin,
    PokerHalf,
    PokerThree,
    PokerPot,
    PokerAllin,
    PokerPeek,
    BjHit,
    BjStand,
    BjDouble,
    BjSplit,
    BjDeal,
    BjRebet,
    CrapsRoll,
    CrapsClear,
    BacDeal,
    BacClear,
    BacRebet,
    RlSpin,
    RlDouble,
    RlClear,
    RlRebet,
    RlUndo,
    DomGo,
    DiceRoll,
    KenoPlay,
    PlinkoDrop,
}

impl Move {
    /// Its name in the settings (0.13.28's).
    pub fn id(self) -> &'static str {
        use Move::*;
        match self {
            ChipSmaller => "chip.smaller",
            ChipBigger => "chip.bigger",
            SlotsSpin => "slots.spin",
            SlotsBetDown => "slots.bet_down",
            SlotsBetUp => "slots.bet_up",
            PokerFold => "poker.fold",
            PokerCall => "poker.call",
            PokerRaise => "poker.raise",
            PokerMin => "poker.min",
            PokerHalf => "poker.half",
            PokerThree => "poker.three",
            PokerPot => "poker.pot",
            PokerAllin => "poker.allin",
            PokerPeek => "poker.peek",
            BjHit => "bj.hit",
            BjStand => "bj.stand",
            BjDouble => "bj.double",
            BjSplit => "bj.split",
            BjDeal => "bj.deal",
            BjRebet => "bj.rebet",
            CrapsRoll => "craps.roll",
            CrapsClear => "craps.clear",
            BacDeal => "bac.deal",
            BacClear => "bac.clear",
            BacRebet => "bac.rebet",
            RlSpin => "rl.spin",
            RlDouble => "rl.double",
            RlClear => "rl.clear",
            RlRebet => "rl.rebet",
            RlUndo => "rl.undo",
            DomGo => "dom.go",
            DiceRoll => "dice.roll",
            KenoPlay => "keno.play",
            PlinkoDrop => "plinko.drop",
        }
    }

    pub fn label(self) -> &'static str {
        use Move::*;
        match self {
            ChipSmaller => "A smaller chip from the rack",
            ChipBigger => "A bigger chip from the rack",
            SlotsSpin | RlSpin => "Spin",
            SlotsBetDown => "Bet less",
            SlotsBetUp => "Bet more",
            PokerFold => "Fold",
            PokerCall => "Check or call",
            PokerRaise => "Bet or raise (the amount set)",
            PokerMin => "Set the amount: the least",
            PokerHalf => "Set the amount: half the pot",
            PokerThree => "Set the amount: three quarters of the pot",
            PokerPot => "Set the amount: the pot",
            PokerAllin => "Set the amount: all in",
            PokerPeek => "Peek at my cards in streamer mode (hold it)",
            BjHit => "Hit",
            BjStand => "Stand",
            BjDouble => "Double",
            BjSplit => "Split",
            BjDeal => "Deal (put the bet down)",
            BjRebet => "Rebet (or clear the bet)",
            CrapsRoll => "Roll (throw the dice, or I'm ready)",
            CrapsClear => "Take my bets down",
            BacDeal => "Deal",
            BacClear | RlClear => "Clear the bets",
            BacRebet | RlRebet => "Rebet",
            RlDouble => "Double the bets",
            RlUndo => "Take back the last chip (against the house)",
            DomGo => "Pass, or deal me in",
            DiceRoll => "Roll",
            KenoPlay => "Play",
            PlinkoDrop => "Drop a ball",
        }
    }

    /// The keys it starts with.
    pub fn defaults(self) -> &'static [&'static str] {
        use Move::*;
        match self {
            ChipSmaller => &["Left"],
            ChipBigger => &["Right"],
            SlotsSpin | CrapsRoll | DiceRoll | KenoPlay | PlinkoDrop | PokerPeek => &["Space"],
            SlotsBetDown => &["Down"],
            SlotsBetUp => &["Up"],
            PokerFold => &["F"],
            PokerCall => &["C"],
            PokerRaise | BjRebet | BacRebet | RlRebet => &["R"],
            PokerMin => &["1"],
            PokerHalf => &["2"],
            PokerThree => &["3"],
            PokerPot => &["4"],
            PokerAllin => &["5"],
            BjHit => &["H"],
            BjStand => &["S"],
            BjDouble | RlDouble => &["D"],
            BjSplit => &["P"],
            BjDeal | BacDeal => &["Enter"],
            CrapsClear | BacClear | RlClear => &["Backspace"],
            RlSpin => &["Space", "Enter"],
            RlUndo => &["Ctrl+Z"],
            DomGo => &["Enter", "Space"],
        }
    }

    fn all() -> impl Iterator<Item = Move> {
        Section::ALL.into_iter().flat_map(|s| s.moves().iter().copied())
    }
}

/// A key and what's held with it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Combo {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Combo {
    /// "Ctrl+Shift+W", "Space", "Left" (as 0.13.28 writes them: egui's names).
    pub fn parse(text: &str) -> Option<Combo> {
        let mut c = Combo { key: Key::Space, ctrl: false, alt: false, shift: false };
        let parts: Vec<&str> = text.split('+').collect();
        let (last, mods) = parts.split_last()?;
        for m in mods {
            match *m {
                "Ctrl" => c.ctrl = true,
                "Alt" => c.alt = true,
                "Shift" => c.shift = true,
                _ => return None,
            }
        }
        c.key = Key::from_name(last)?;
        Some(c)
    }

    pub fn name(&self) -> String {
        let mut s = String::new();
        for (on, n) in [(self.ctrl, "Ctrl+"), (self.alt, "Alt+"), (self.shift, "Shift+")] {
            if on {
                s.push_str(n);
            }
        }
        s.push_str(self.key.name());
        s
    }

    fn matches(&self, key: Key, m: egui::Modifiers) -> bool {
        key == self.key && (m.ctrl || m.command) == self.ctrl && m.alt == self.alt && m.shift == self.shift
    }
}

/// The keys as changed (the settings' `casino_keys`), for every game to read.
static CHANGED: RwLock<BTreeMap<String, Vec<String>>> = RwLock::new(BTreeMap::new());
/// When the Keys window was last drawn (egui's pass, one more; 0 never): while it's open no game
/// moves for a key.
static SHOWN: AtomicU64 = AtomicU64::new(0);

fn window_open(ctx: &egui::Context) -> bool {
    // (Drawn this pass or the one before.)
    let shown = SHOWN.load(Ordering::Relaxed);
    shown > 0 && shown >= ctx.cumulative_pass_nr()
}

/// The keys as the settings have them now (each frame).
pub fn sync(changed: &BTreeMap<String, Vec<String>>) {
    if *CHANGED.read() != *changed {
        *CHANGED.write() = changed.clone();
    }
}

/// A move's keys now: as changed, or its own.
pub fn keys_of(mv: Move) -> Vec<String> {
    keys_in(&CHANGED.read(), mv)
}

fn keys_in(changed: &BTreeMap<String, Vec<String>>, mv: Move) -> Vec<String> {
    match changed.get(mv.id()) {
        Some(k) => k.clone(),
        None => mv.defaults().iter().map(|s| s.to_string()).collect(),
    }
}

/// The move was made this frame: one of its keys pressed (not while typing, not while the Keys
/// window is open).
pub fn pressed(ctx: &egui::Context, mv: Move) -> bool {
    if window_open(ctx) || ctx.wants_keyboard_input() {
        return false;
    }
    let combos: Vec<Combo> = keys_of(mv).iter().filter_map(|k| Combo::parse(k)).collect();
    if combos.is_empty() {
        return false;
    }
    ctx.input(|i| {
        i.events.iter().any(|e| match e {
            egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } => combos.iter().any(|c| c.matches(*key, *modifiers)),
            _ => false,
        })
    })
}

/// A slot machine's bet a step down (−1) or up (+1) by its key this frame, or neither (0).
pub fn bet_step(ctx: &egui::Context) -> i32 {
    if pressed(ctx, Move::SlotsBetDown) {
        -1
    } else if pressed(ctx, Move::SlotsBetUp) {
        1
    } else {
        0
    }
}

/// One of the move's keys is held down (for a move that lasts while it's held).
pub fn held(ctx: &egui::Context, mv: Move) -> bool {
    if window_open(ctx) || ctx.wants_keyboard_input() {
        return false;
    }
    let combos: Vec<Combo> = keys_of(mv).iter().filter_map(|k| Combo::parse(k)).collect();
    ctx.input(|i| combos.iter().any(|c| i.key_down(c.key) && (i.modifiers.ctrl || i.modifiers.command) == c.ctrl && i.modifiers.alt == c.alt && i.modifiers.shift == c.shift))
}

/// A move's first key now, if it has one ("Space").
pub fn first_key(mv: Move) -> Option<String> {
    keys_of(mv).into_iter().next()
}

/// "Roll  (Space)": a button's words with the move's first key, if it has one (two spaces, as
/// 0.13.28 writes it).
pub fn with_key(words: &str, mv: Move) -> String {
    match keys_of(mv).first() {
        Some(k) => format!("{words}  ({k})"),
        None => words.to_string(),
    }
}

/// The Keys window's state: the chapter first (the game it was opened from), a move's key
/// waiting to be pressed (its place, 0 or 1), and what the last change was.
#[derive(Clone, Debug, PartialEq)]
pub struct KeysWindow {
    pub first: Option<Section>,
    pub waiting: Option<(Move, usize)>,
    pub said: Option<String>,
}

impl KeysWindow {
    pub fn new(first: Option<Section>) -> Self {
        KeysWindow { first, waiting: None, said: None }
    }
}

/// Do moves `a` and `b` ever go together (the same game's, or every game's)?
fn same_game(a: Move, b: Move) -> bool {
    let (sa, sb) = (Section::of_move(a), Section::of_move(b));
    sa == sb || sa == Section::Every || sb == Section::Every
}

/// The Keys window's insides (in the app's dialog frame): `changed` the settings' keys, altered
/// in place. Returns whether it's done (Done, or Esc while not waiting for a key).
pub fn window_ui(ui: &mut egui::Ui, w: &mut KeysWindow, changed: &mut BTreeMap<String, Vec<String>>, room: f32) -> bool {
    SHOWN.store(ui.ctx().cumulative_pass_nr() + 1, Ordering::Relaxed);
    let mut done = false;
    // A key pressed while one's waited for: it's that key's now (Esc stops waiting).
    if let Some((mv, slot)) = w.waiting {
        let got = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => Some((*key, *modifiers)),
                _ => None,
            })
        });
        if let Some((key, m)) = got {
            ui.input_mut(|i| i.events.retain(|e| !matches!(e, egui::Event::Key { .. })));
            w.waiting = None;
            if key != Key::Escape {
                let combo = Combo { key, ctrl: m.ctrl || m.command, alt: m.alt, shift: m.shift }.name();
                let mut keys = keys_in(changed, mv);
                keys.retain(|k| *k != combo);
                if slot < keys.len() {
                    keys[slot] = combo.clone();
                } else {
                    keys.push(combo.clone());
                }
                keys.truncate(2);
                set(changed, mv, keys);
                // (Another move of the same game had it: it moves over.)
                let mut from = None;
                for other in Move::all().filter(|o| *o != mv && same_game(*o, mv)) {
                    let mut k = keys_in(changed, other);
                    if k.contains(&combo) {
                        k.retain(|x| *x != combo);
                        set(changed, other, k);
                        from = Some(other);
                    }
                }
                w.said = Some(match from {
                    Some(o) => format!("{}: {combo}. It was on {}, which doesn't have it any more.", mv.label(), o.label()),
                    None => format!("{}: {combo}.", mv.label()),
                });
            }
        }
    } else if ui.input(|i| i.key_pressed(Key::Escape)) {
        done = true;
    }
    ui.set_width(560.0);
    crate::dialogs::title(ui, "Keys");
    ui.label(
        RichText::new("Click a key, then press the one you want it to be (with Ctrl, Alt or Shift too, if you like). Esc stops waiting. × takes a key off; ↺ puts a move back as it was.")
            .size(12.5)
            .color(theme::MUTED()),
    );
    if let Some(s) = &w.said {
        ui.add_space(6.0);
        ui.label(RichText::new(s).size(12.5).color(super::col::GOLD2));
    }
    ui.add_space(7.0);
    let order: Vec<Section> = match w.first {
        Some(f) => std::iter::once(f).chain(Section::ALL.into_iter().filter(|s| *s != f)).collect(),
        None => Section::ALL.to_vec(),
    };
    // The list: half the window and 141 more, as 0.13.28's (the words over it don't shorten it:
    // the window grows with them), never more than the room left.
    let half = ui.ctx().content_rect().height() * 0.5 + 141.0;
    let list_h = half.min(room - ui.min_rect().height() - 50.0).max(120.0);
    egui::ScrollArea::vertical().max_height(list_h).auto_shrink([false, true]).show(ui, |ui| {
        for sec in order {
            let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 25.0), Sense::hover());
            let this = w.first == Some(sec);
            let name = if this { format!("{} (this game)", sec.name()) } else { sec.name().to_string() };
            ui.painter().text(pos2(head.left(), head.top() + 7.0), Align2::LEFT_TOP, name, theme::bold_font(14.0), if this { super::col::GOLD2 } else { theme::TEXT() });
            for &mv in sec.moves() {
                move_row(ui, w, changed, mv);
            }
        }
    });
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.add(egui::Button::new(RichText::new("Put every key back as it was").size(14.0)).min_size(vec2(0.0, 30.0))).clicked() {
            changed.clear();
            w.waiting = None;
            w.said = Some("Every key is back as it was.".into());
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(theme::primary_button("Done").min_size(vec2(84.0, 32.0))).clicked() {
                done = true;
            }
        });
    });
    done
}

/// A move's keys as changed (or put back: its own again, out of the list).
fn set(changed: &mut BTreeMap<String, Vec<String>>, mv: Move, keys: Vec<String>) {
    let own: Vec<String> = mv.defaults().iter().map(|s| s.to_string()).collect();
    if keys == own {
        changed.remove(mv.id());
    } else {
        changed.insert(mv.id().to_string(), keys);
    }
}

/// A move's row: its words, its two keys (or None and +), and ↺ when it's not as it was.
fn move_row(ui: &mut egui::Ui, w: &mut KeysWindow, changed: &mut BTreeMap<String, Vec<String>>, mv: Move) {
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::hover());
    let p = ui.painter().clone();
    // (A row's words and keys sit a pixel below its middle, as 0.13.28's.)
    p.text(pos2(row.left() + 4.0, row.center().y + 1.0), Align2::LEFT_CENTER, mv.label(), theme::font(13.0), theme::BODY());
    let keys = keys_in(changed, mv);
    let x0 = row.left() + 292.0;
    for slot in 0..2 {
        let r = Rect::from_min_size(pos2(x0 + slot as f32 * 120.0, row.top() + 3.0), vec2(112.0, 26.0));
        let id = ui.id().with(("casino-key", mv.id(), slot));
        let resp = ui.interact(r, id, Sense::click());
        let waiting = w.waiting == Some((mv, slot));
        let key = keys.get(slot);
        let (fill, edge) = if waiting {
            (super::col::GOLD.gamma_multiply(0.35), super::col::GOLD2)
        } else if resp.hovered() {
            (Color32::from_rgb(0x38, 0x34, 0x31), Color32::from_rgb(0x54, 0x51, 0x4e))
        } else {
            (Color32::from_rgb(0x2a, 0x26, 0x23), Color32::from_rgb(0x48, 0x45, 0x42))
        };
        p.rect_filled(r, theme::r(7), fill);
        p.rect_stroke(r, theme::r(7), Stroke::new(1.0f32, edge), StrokeKind::Inside);
        if waiting {
            p.text(r.center(), Align2::CENTER_CENTER, "Press a key…", theme::bold_font(12.5), super::col::GOLD2);
        } else if let Some(k) = key {
            // (The key's name in the room the × leaves.)
            p.text(r.center() - vec2(7.0, 0.0), Align2::CENTER_CENTER, k, theme::bold_font(12.5), theme::TEXT());
            // ×: takes it off (two strokes, as 0.13.28 draws it).
            let xc = pos2(r.right() - 11.5, r.center().y + 2.0);
            let xr = ui.interact(Rect::from_center_size(xc, vec2(18.0, 20.0)), id.with("off"), Sense::click());
            let ink = Stroke::new(1.5f32, if xr.hovered() { theme::TEXT() } else { theme::DIM() });
            p.line_segment([xc + vec2(-2.5, -2.5), xc + vec2(2.5, 2.5)], ink);
            p.line_segment([xc + vec2(-2.5, 2.5), xc + vec2(2.5, -2.5)], ink);
            if xr.on_hover_text("Take this key off").clicked() {
                let mut k = keys.clone();
                k.remove(slot);
                let left = k.clone();
                set(changed, mv, k);
                w.said = Some(if left.is_empty() { format!("{}: no key now.", mv.label()) } else { format!("{}: {}.", mv.label(), left.join(", ")) });
                w.waiting = None;
                continue;
            }
        } else if slot == 0 {
            p.text(r.center(), Align2::CENTER_CENTER, "None", theme::bold_font(12.5), theme::DIM());
        } else {
            p.text(r.center(), Align2::CENTER_CENTER, "+", theme::bold_font(12.5), theme::DIM());
        }
        // (A key added goes in the first free place.)
        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            w.waiting = Some((mv, if key.is_some() { slot } else { keys.len().min(slot) }));
        }
    }
    // ↺: as it was.
    if changed.contains_key(mv.id()) {
        let r = Rect::from_center_size(pos2(x0 + 251.0, row.center().y + 1.0), vec2(22.0, 26.0));
        let resp = ui.interact(r, ui.id().with(("casino-key-back", mv.id())), Sense::click());
        p.text(r.center(), Align2::CENTER_CENTER, "↺", theme::bold_font(15.0), if resp.hovered() { theme::TEXT() } else { theme::MUTED() });
        if resp.on_hover_text("Put it back as it was").clicked() {
            changed.remove(mv.id());
            w.said = Some(format!("{}: as it was ({}).", mv.label(), mv.defaults().join(", ")));
            w.waiting = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The names 0.13.28 keeps them under, move for move.
    #[test]
    fn moves_have_0_13_28s_names() {
        let ids: Vec<&str> = Move::all().map(|m| m.id()).collect();
        assert_eq!(
            ids,
            [
                "chip.smaller", "chip.bigger", "slots.spin", "slots.bet_down", "slots.bet_up", "poker.fold", "poker.call", "poker.raise", "poker.min", "poker.half", "poker.three", "poker.pot",
                "poker.allin", "poker.peek", "bj.hit", "bj.stand", "bj.double", "bj.split", "bj.deal", "bj.rebet", "craps.roll", "craps.clear", "bac.deal", "bac.clear", "bac.rebet", "rl.spin",
                "rl.double", "rl.clear", "rl.rebet", "rl.undo", "dom.go", "dice.roll", "keno.play", "plinko.drop"
            ]
        );
    }

    /// Every key a move starts with reads back as written, and keys read as 0.13.28 wrote them.
    #[test]
    fn keys_read_and_write_as_0_13_28s() {
        for mv in Move::all() {
            for k in mv.defaults() {
                let c = Combo::parse(k).unwrap_or_else(|| panic!("{k} doesn't read"));
                assert_eq!(c.name(), *k);
            }
        }
        let c = Combo::parse("Ctrl+Shift+W").unwrap();
        assert_eq!((c.key, c.ctrl, c.alt, c.shift), (Key::W, true, false, true));
        assert_eq!(Combo { key: Key::ArrowLeft, ctrl: false, alt: true, shift: false }.name(), "Alt+Left");
        assert_eq!(Combo::parse("Left").unwrap().key, Key::ArrowLeft);
        assert_eq!(Combo::parse("1").unwrap().key, Key::Num1);
        assert!(Combo::parse("Hyper+X").is_none());
        assert!(Combo::parse("").is_none());
    }

    /// A move put back as it was leaves the settings; one changed is kept as 0.13.28 keeps it.
    #[test]
    fn only_what_changed_is_kept() {
        let mut changed = BTreeMap::new();
        set(&mut changed, Move::ChipBigger, vec!["Ctrl+Shift+W".into()]);
        assert_eq!(serde_json::to_string(&changed).unwrap(), r#"{"chip.bigger":["Ctrl+Shift+W"]}"#);
        assert_eq!(keys_in(&changed, Move::ChipBigger), ["Ctrl+Shift+W"]);
        assert_eq!(keys_in(&changed, Move::ChipSmaller), ["Left"]);
        set(&mut changed, Move::ChipBigger, vec!["Right".into()]);
        assert!(changed.is_empty());
        // No key at all is a change too.
        set(&mut changed, Move::SlotsSpin, vec![]);
        assert_eq!(keys_in(&changed, Move::SlotsSpin), Vec::<String>::new());
    }

    /// Moves of the same game share no key: what moves over, and what doesn't.
    #[test]
    fn games_and_every_game() {
        assert!(same_game(Move::BjHit, Move::BjDeal));
        assert!(same_game(Move::ChipBigger, Move::PokerFold));
        assert!(!same_game(Move::PokerFold, Move::BjHit));
        // (Each game's own keys don't clash as they start.)
        for sec in Section::ALL {
            let mut seen: Vec<&str> = Vec::new();
            for mv in sec.moves().iter().chain(Section::Every.moves().iter().filter(|_| sec != Section::Every)) {
                for k in mv.defaults() {
                    assert!(!seen.contains(k), "{k} twice in {}", sec.name());
                    seen.push(k);
                }
            }
        }
    }

    #[test]
    fn hints_have_two_spaces() {
        assert_eq!(with_key("Roll", Move::DiceRoll), "Roll  (Space)");
    }
}
