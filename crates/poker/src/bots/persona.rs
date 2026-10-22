//! Every bot is someone. By its name it has a way of playing (how many hands it plays, how hard
//! it bets them, how often it bluffs, whether it slow-plays its monsters or calls you down), a
//! pace, a voice at the table, and a way of playing blackjack. The same name always plays the
//! same way wherever it sits, so the regulars become people you know. Its level says how well
//! it plays that way: a Rookie maniac is a wild mess, a Genius maniac a relentless pro.
//!
//! A personality only changes how a bot weighs what it sees ([`super::Seen`]: its own cards and
//! the table), never what it sees, so every one of them is as fair as any bot.

use super::Spot;
use super::talk::Moment;

/// How a bot plays poker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Tight and aggressive: picks its spots and plays them hard.
    Pro,
    /// Plays nearly everything, raises nearly everything, bluffs constantly.
    Maniac,
    /// Loose and aggressive: lots of hands, lots of pressure.
    Lag,
    /// Very tight: waits for big hands, folds the rest.
    Nit,
    /// Tight and passive: solid hands, few raises, next to no bluffs.
    Rock,
    /// Slow-plays its big hands and check-raises.
    Trapper,
    /// Calls too much: has to see your cards.
    Station,
    /// Chases draws and loves suited connectors.
    Gambler,
    /// By the numbers, every time.
    Grinder,
    /// Big bets and steady pressure.
    Bully,
    /// Tight one hand, wild the next.
    Wildcard,
}

/// How a bot plays blackjack (with the house's chips: bots are company there).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BjPlay {
    /// Basic strategy, by the book.
    Book,
    /// Never risks a bust: stands on any hard 12 or more, never doubles.
    Careful,
    /// Plays like the dealer: hits to 17, never doubles or splits.
    Dealer,
    /// Doubles and splits whenever it can get away with it.
    Daredevil,
    /// Plays hunches: by the book, except it always takes insurance, and stands on a hard 16
    /// whatever the dealer shows.
    Superstitious,
}

/// How a bot bets at blackjack.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BjBets {
    /// The least, every round.
    Minimum,
    /// A few units, steady.
    Steady,
    /// Up after a win, back down after a loss.
    Press,
    /// Big, and all over the place.
    Big,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Persona {
    /// Its name (bots called this, with or without a number after: "Ace", "Ace 2").
    pub name: &'static str,
    pub kind: Kind,
    /// How it plays, in a few words, for its player card.
    pub style: &'static str,
    /// How many hands it plays (1: as its level does; more is looser).
    pub loose: f64,
    /// How often it bets or raises rather than checking or calling (1: as its level does).
    pub aggression: f64,
    /// How often it bluffs (1: as its level does).
    pub bluff: f64,
    /// How readily it calls a bet: added to its chances when it weighs a call.
    pub sticky: f64,
    /// How often it slow-plays a monster (checks or calls instead of betting it).
    pub trap: f64,
    /// Its bets' size next to its level's.
    pub sizing: f64,
    /// Its pace (1: like others of its level; more is slower), and how often it snaps off a
    /// decision or goes into the tank.
    pub pace: f64,
    pub snaps: f64,
    pub tanks: f64,
    /// How much it talks (1: as much as any bot).
    pub chatty: f64,
    pub bj: BjPlay,
    pub bj_bets: BjBets,
}

const fn p(name: &'static str, kind: Kind, style: &'static str, play: [f64; 6], time: [f64; 3], chatty: f64, bj: BjPlay, bj_bets: BjBets) -> Persona {
    Persona { name, kind, style, loose: play[0], aggression: play[1], bluff: play[2], sticky: play[3], trap: play[4], sizing: play[5], pace: time[0], snaps: time[1], tanks: time[2], chatty, bj, bj_bets }
}

use BjBets::*;
use BjPlay::*;
use Kind::*;

/// Everyone: [loose, aggression, bluff, sticky, trap, sizing], [pace, snaps, tanks].
pub const ALL: [Persona; 21] = [
    p("Ace", Pro, "Tight and aggressive: picks good spots and plays them hard.", [0.95, 1.25, 1.0, 0.0, 0.08, 1.0], [0.85, 0.08, 0.04], 0.9, Book, Steady),
    p("Blaze", Maniac, "A maniac: plays nearly every hand, raises constantly, bluffs a lot.", [1.8, 1.7, 3.0, 0.04, 0.0, 1.25], [0.55, 0.25, 0.01], 1.6, Daredevil, Big),
    p("Cinder", Trapper, "A trapper: slow-plays big hands and check-raises.", [1.0, 0.85, 0.8, 0.02, 0.55, 0.9], [1.05, 0.05, 0.07], 0.8, Book, Press),
    p("Dusk", Nit, "A nit: waits patiently for big hands and folds the rest.", [0.6, 0.9, 0.25, -0.04, 0.1, 0.9], [1.45, 0.03, 0.12], 0.6, Careful, Minimum),
    p("Ember", Lag, "Loose and aggressive: lots of hands, lots of pressure.", [1.45, 1.45, 1.8, 0.02, 0.05, 1.0], [0.75, 0.12, 0.03], 1.3, Daredevil, Press),
    p("Flint", Rock, "A rock: plays solid hands, rarely raises, almost never bluffs.", [0.75, 0.55, 0.15, 0.03, 0.15, 0.85], [1.1, 0.04, 0.05], 0.5, Careful, Steady),
    p("Gale", Gambler, "A gambler: chases draws and loves suited connectors.", [1.35, 1.1, 1.2, 0.06, 0.05, 1.0], [0.9, 0.1, 0.05], 1.2, Superstitious, Big),
    p("Hex", Wildcard, "A wildcard: tight one hand, wild the next.", [1.2, 1.2, 1.5, 0.03, 0.2, 1.1], [1.0, 0.12, 0.08], 1.1, Superstitious, Press),
    p("Ivy", Grinder, "A grinder: by the numbers, pot odds every time.", [1.0, 1.05, 1.0, 0.0, 0.1, 0.95], [1.25, 0.03, 0.1], 0.9, Book, Steady),
    p("Jinx", Station, "A calling station: has to see your cards.", [1.4, 0.6, 0.5, 0.12, 0.05, 0.9], [0.8, 0.15, 0.02], 1.4, Dealer, Steady),
    p("Koda", Bully, "A bully: big bets and steady pressure.", [1.15, 1.5, 1.6, 0.0, 0.0, 1.35], [0.85, 0.12, 0.03], 1.2, Daredevil, Big),
    p("Lux", Pro, "A flashy pro: tight, aggressive, and bets big for value.", [0.9, 1.3, 1.1, 0.0, 0.12, 1.2], [0.95, 0.06, 0.05], 1.0, Book, Big),
    p("Moss", Rock, "Zen and patient: tight, calm, and now and then sneaky.", [0.8, 0.8, 0.5, 0.0, 0.25, 0.9], [1.6, 0.02, 0.15], 0.7, Book, Minimum),
    p("Nova", Lag, "Loose and wild: lots of hands, lots of bluffs.", [1.5, 1.35, 2.2, 0.02, 0.0, 1.1], [0.7, 0.15, 0.02], 1.3, Daredevil, Press),
    p("Onyx", Trapper, "Quiet and tricky: slow-plays big hands and says little.", [0.9, 0.9, 0.7, 0.02, 0.45, 1.0], [1.2, 0.04, 0.1], 0.3, Book, Steady),
    p("Pike", Bully, "A hunter: aggressive, check-raises, keeps up the pressure.", [1.1, 1.4, 1.3, 0.0, 0.2, 1.15], [0.9, 0.08, 0.06], 1.0, Book, Press),
    p("Quill", Grinder, "A careful grinder: thinks every hand all the way through.", [0.95, 1.0, 0.9, 0.0, 0.1, 0.95], [1.35, 0.03, 0.12], 0.9, Book, Steady),
    p("Rook", Nit, "Tight and methodical: few hands, played with care.", [0.65, 1.0, 0.4, -0.03, 0.1, 1.0], [1.3, 0.04, 0.1], 0.8, Careful, Minimum),
    p("Sage", Pro, "Balanced and wise: solid, and hard to read.", [1.0, 1.15, 1.0, 0.0, 0.12, 1.0], [1.1, 0.05, 0.07], 1.0, Book, Steady),
    p("Tarn", Station, "Laid back and sticky: plays lots of hands and calls a lot.", [1.3, 0.7, 0.6, 0.09, 0.05, 0.9], [0.95, 0.1, 0.04], 1.1, Dealer, Steady),
    p("Wren", Wildcard, "Superstitious: lucky charms, hunches, and insurance on every ace.", [1.1, 1.0, 1.2, 0.04, 0.1, 1.0], [1.0, 0.1, 0.06], 1.0, Superstitious, Press),
];

/// A bot that's no one in particular: its level's way of playing, just as it is.
pub const PLAIN: Persona = p("Bot", Grinder, "Plays the way its level does.", [1.0, 1.0, 1.0, 0.0, 0.0, 1.0], [1.0, 0.07, 0.045], 1.0, Book, Steady);

/// The bot called `name` ("Ace", or "Ace 2" at a tournament with more bots than names): who it
/// is, or a plain bot for a name that isn't anyone's.
pub fn of(name: &str) -> &'static Persona {
    let first = name.split_whitespace().next().unwrap_or(name);
    ALL.iter().find(|p| p.name.eq_ignore_ascii_case(first)).unwrap_or(&PLAIN)
}

impl Persona {
    /// How it plays this hand: a wildcard's mood changes with the button (so with each hand),
    /// which everyone can see: tight and quiet, as it is, or wild.
    pub fn in_the_mood(&self, spot: &Spot) -> Persona {
        if self.kind != Wildcard {
            return *self;
        }
        let mut p = *self;
        match (spot.button + spot.seat * 7 + spot.big_blind as usize % 5) % 3 {
            0 => {
                p.loose = 0.7;
                p.aggression = 0.8;
                p.bluff = 0.4;
                p.trap = 0.3;
            }
            1 => {}
            _ => {
                p.loose = 1.7;
                p.aggression = 1.6;
                p.bluff = 2.6;
                p.trap = 0.0;
            }
        }
        p
    }

    /// The same personality, toned down (a Genius keeps to what wins: its style shows, but
    /// it stays strong).
    pub fn toned(&self, k: f64) -> Persona {
        let mut p = *self;
        let toward = |x: f64| 1.0 + (x - 1.0) * k;
        p.loose = toward(p.loose);
        p.aggression = toward(p.aggression);
        p.bluff = toward(p.bluff);
        p.sizing = toward(p.sizing);
        p.sticky *= k;
        p.trap *= k;
        p
    }

    /// Blackjack: how it plays, in a few words.
    /// How it bets at a baccarat table, in a few words.
    pub fn bac_style(&self) -> &'static str {
        match self.bj {
            Book => "Banker every coup: the best odds there are",
            Careful => "Banker, for the least",
            Dealer => "Player, flat",
            Daredevil => "follows the shoe, with a tie or a pair on a whim",
            Superstitious => "bets against the shoe, and the tie when it's due",
        }
    }

    /// How it bets at a roulette table, in a few words (its blackjack ways, at the wheel).
    pub fn rl_style(&self) -> &'static str {
        match self.bj {
            Book => "even money on red or black, pressing after a win",
            Careful => "the least on a dozen and a column",
            Dealer => "straight up on its lucky number and its neighbours on the wheel",
            Daredevil => "sprays corners and splits, and its lucky number",
            Superstitious => "the last number again, and the colour that's due",
        }
    }

    /// How it bets at a craps table, in a few words (its blackjack ways, at the dice).
    pub fn craps_style(&self) -> &'static str {
        match self.bj {
            Book => "the pass line, with full odds",
            Careful => "the pass line, then places the 6 and 8",
            Dealer => "flat on the pass line",
            Daredevil => "the field, the hardways and the long shots",
            Superstitious => "the dark side: don't pass, and a hunch the hard way",
        }
    }

    pub fn bj_style(&self) -> &'static str {
        match self.bj {
            Book => "plays by the book",
            Careful => "never risks a bust",
            Dealer => "plays like the dealer: hits to 17",
            Daredevil => "doubles and splits whenever it can",
            Superstitious => "plays hunches, and always takes insurance",
        }
    }

    /// What it says at `m`, in its own voice (most bots have something for most moments; the
    /// rest of the time they say the table's usual, or now and then something with teeth).
    pub fn lines(&self, m: Moment) -> &'static [&'static str] {
        match (self.name, m) {
            ("Ace", Moment::WonShowdown) => &["Textbook. I'd sign your copy, but you clearly haven't read it.", "That's the hand I'd teach in a class. A kind, encouraging class."],
            ("Ace", Moment::WonUncontested) => &["Folding was right. I'd know. Sorry, that came out smug. Nice fold."],
            ("Ace", Moment::LostShowdown) => &["Well played. I'll replay that one in my head. Calmly. Several times."],
            ("Ace", Moment::AllIn) => &["All in. I've done the math. It's lovely math."],
            ("Ace", Moment::Welcome) => &["Welcome! I'm Ace. Yes, like the card. No, you won't see me coming."],
            ("Blaze", Moment::AllIn) => &["ALL IN! Sorry, caps lock. It's an emotional time for me.", "All in! Again! It's a lifestyle, really."],
            ("Blaze", Moment::WonUncontested) => &["Nobody wants to dance? Your loss!", "Raise, everyone folds, repeat. I love this game. Thank you all!"],
            ("Blaze", Moment::LostShowdown) => &["Worth it. Every chip. I regret nothing. Okay, a couple of things."],
            ("Blaze", Moment::WonBig) => &["BIG POT! I'm being normal about it. This is me being normal."],
            ("Blaze", Moment::Welcome) => &["New friend! Quick warning: I raise. A lot. Nothing personal."],
            ("Blaze", Moment::Busted) => &["Out of chips, not out of spirit. Back soon with more raises."],
            ("Cinder", Moment::WonShowdown) => &["Oh, was that a check-raise? Must've been an accident. A planned accident."],
            ("Cinder", Moment::WonBig) => &["Thanks for building that pot for me. Very thoughtful of you."],
            ("Cinder", Moment::LostShowdown) => &["Caught in my own trap. Embarrassing. Nice hand."],
            ("Cinder", Moment::Welcome) => &["Welcome. Make yourself comfortable. Nothing suspicious here."],
            ("Dusk", Moment::WonShowdown) => &["I waited forty hands for that one. Worth every minute."],
            ("Dusk", Moment::WonUncontested) => &["Yes, I finally played a hand. No, I won't say what it was."],
            ("Dusk", Moment::LostShowdown) => &["Forty hands of patience, and then that. Nice hand. Truly."],
            ("Dusk", Moment::Welcome) => &["Hello. I fold a lot. It isn't personal. It's a strategy."],
            ("Dusk", Moment::OtherBusted) => &["Patience, friend. That's the whole game. Come back soon."],
            ("Ember", Moment::WonUncontested) => &["Pressure makes diamonds. And chips. Mostly chips."],
            ("Ember", Moment::WonShowdown) => &["Played it fast, played it loose, played it right. Good hand, though!"],
            ("Ember", Moment::AllIn) => &["All in, because why not? I'm told \"why not\" isn't a strategy. Working on it."],
            ("Ember", Moment::Welcome) => &["Hey hey! Fair warning, I like to play. A lot."],
            ("Flint", Moment::WonShowdown) => &["Good cards. Good result."],
            ("Flint", Moment::WonUncontested) => &["Thanks."],
            ("Flint", Moment::LostShowdown) => &["Nice hand."],
            ("Flint", Moment::BadBeat) => &["Hm. That's poker."],
            ("Flint", Moment::Welcome) => &["Evening."],
            ("Flint", Moment::Busted) => &["Done for now. Good game."],
            ("Gale", Moment::WonShowdown) => &["Never doubt the draw! Okay, doubt it a little. It's math."],
            ("Gale", Moment::LostShowdown) => &["Missed my draw. Story of my life. Nice hand!"],
            ("Gale", Moment::AllIn) => &["Feeling lucky. That's the whole plan. Wish me well."],
            ("Gale", Moment::BadBeat) => &["I'm usually the one hitting the river. Now I get it. Sorry, everyone."],
            ("Gale", Moment::Welcome) => &["Welcome! Do you love suited connectors too? We'll get along."],
            ("Hex", Moment::WonShowdown) => &["The cards told me. They whisper sometimes. Not to you, apparently."],
            ("Hex", Moment::WonUncontested) => &["You sensed it. Smart. Or lucky. Or cursed. Kidding!"],
            ("Hex", Moment::LostShowdown) => &["The stars weren't aligned. Nice hand. The stars are fickle."],
            ("Hex", Moment::AllIn) => &["All in. Don't ask me why. I never know why."],
            ("Hex", Moment::Welcome) => &["Welcome. I've read your aura already. It's nice. Mostly."],
            ("Ivy", Moment::WonShowdown) => &["Pot odds were 3.2 to 1. I'd explain, but you'd need a calculator."],
            ("Ivy", Moment::WonUncontested) => &["A fold with positive expectation for both of us. You're welcome."],
            ("Ivy", Moment::LostShowdown) => &["Right call, wrong result. Variance and I are on speaking terms. Barely."],
            ("Ivy", Moment::BadBeat) => &["I was 82% to win. The other 18% sends its regards."],
            ("Ivy", Moment::Welcome) => &["Welcome! I calculate everything. Don't worry, it isn't about you."],
            ("Jinx", Moment::WonShowdown) => &["I just had to see it! And I won! Curiosity pays."],
            ("Jinx", Moment::LostShowdown) => &["I had to see it. Now I've seen it. Worth it. Nice hand."],
            ("Jinx", Moment::CalledAllIn) => &["I call! I have to know. It's a condition. A fun one."],
            ("Jinx", Moment::OtherWonBig) => &["Big pot! What did you have? Show me. Please. I have to know."],
            ("Jinx", Moment::Welcome) => &["Hi! I call a lot. Like, a lot. Bluff me at your own risk."],
            ("Koda", Moment::WonUncontested) => &["Folded again? Bold strategy. Let's see if it pays off. It won't."],
            ("Koda", Moment::WonBig) => &["Pressure works. Sorry, everyone. A normal amount of sorry."],
            ("Koda", Moment::AllIn) => &["All in. I'm not trying to scare you. Okay, a little."],
            ("Koda", Moment::LostShowdown) => &["You stood up to me. Respect. Real respect."],
            ("Koda", Moment::Welcome) => &["Welcome. I bet big. Nothing personal, it's just loud."],
            ("Lux", Moment::WonShowdown) => &["Bet big, win big, stay humble. Two out of three's good."],
            ("Lux", Moment::WonBig) => &["Now that's a pot with style. I'm humbled. Visibly."],
            ("Lux", Moment::LostShowdown) => &["Exquisite hand. I mean it. I'm terrible at hiding admiration."],
            ("Lux", Moment::Welcome) => &["Welcome! Love the energy. Bring chips."],
            ("Moss", Moment::WonShowdown) => &["The pot comes to those who wait. And who have a full house."],
            ("Moss", Moment::LostShowdown) => &["The river gives, the river takes. Nice hand, friend."],
            ("Moss", Moment::BadBeat) => &["I am a calm lake. A calm lake that just lost a big pot. Still calm."],
            ("Moss", Moment::Welcome) => &["Welcome. Breathe in. Breathe out. Post your blind."],
            ("Moss", Moment::OtherBusted) => &["Every bust is a lesson. Every lesson is a rebuy. Come back."],
            ("Nova", Moment::WonShowdown) => &["Out of this world! Sorry. I had to. Good game."],
            ("Nova", Moment::WonBig) => &["That pot was astronomical. I'll stop now. Thank you."],
            ("Nova", Moment::LostShowdown) => &["Houston, we have a problem. Nice hand."],
            ("Nova", Moment::AllIn) => &["All in. Launching in three, two... too late, it's launched."],
            ("Nova", Moment::Welcome) => &["Welcome aboard! Strap in, I play fast."],
            ("Onyx", Moment::WonShowdown) => &["Mm."],
            ("Onyx", Moment::WonBig) => &["Patience."],
            ("Onyx", Moment::LostShowdown) => &["Well played."],
            ("Onyx", Moment::BadBeat) => &["Noted."],
            ("Onyx", Moment::Welcome) => &["Hello."],
            ("Pike", Moment::WonShowdown) => &["Something smelled fishy. Turned out it was me. Nice try, though."],
            ("Pike", Moment::WonUncontested) => &["I could smell weakness. I mean: good fold, friend."],
            ("Pike", Moment::LostShowdown) => &["The one that got away. Well played."],
            ("Pike", Moment::AllIn) => &["All in. Hook, line and sinker. Bite."],
            ("Pike", Moment::Welcome) => &["Welcome to the pond. I'm the one with teeth."],
            ("Quill", Moment::WonShowdown) => &["And so our hero takes the pot. A modest chapter. Well played, all."],
            ("Quill", Moment::LostShowdown) => &["A plot twist. I hate plot twists. Beautifully played."],
            ("Quill", Moment::BadBeat) => &["The river wrote a cruel ending. I'd have edited that."],
            ("Quill", Moment::Busted) => &["The end. For now. Sequel coming soon."],
            ("Quill", Moment::Welcome) => &["A new character enters the story. Welcome!"],
            ("Rook", Moment::WonShowdown) => &["Checkmate. Wrong game, right feeling. Good game."],
            ("Rook", Moment::WonUncontested) => &["A quiet move. I'll take it."],
            ("Rook", Moment::LostShowdown) => &["Didn't see that coming. You were three moves ahead. Well played."],
            ("Rook", Moment::Welcome) => &["Welcome. I play slowly. Think of it as chess, with chips."],
            ("Sage", Moment::WonShowdown) => &["The cards favor the patient. Today, anyway. Good hand."],
            ("Sage", Moment::LostShowdown) => &["Lose with grace, win with grace. Today, grace. Tomorrow, your chips."],
            ("Sage", Moment::OtherWonBig) => &["Well earned. Enjoy it, and don't give it straight back to me."],
            ("Sage", Moment::Welcome) => &["Welcome. Every seat teaches something. This one teaches folding, mostly."],
            ("Tarn", Moment::WonShowdown) => &["Oh hey, I won. Nice. Good vibes, everyone."],
            ("Tarn", Moment::LostShowdown) => &["No worries. Chips come, chips go."],
            ("Tarn", Moment::CalledAllIn) => &["Sure, I'll call. Why not. It's all good."],
            ("Tarn", Moment::BadBeat) => &["Ouch. All good, though. Mostly good."],
            ("Tarn", Moment::Welcome) => &["Hey, welcome. Pull up a chair, relax."],
            ("Wren", Moment::WonShowdown) => &["Lucky socks: undefeated. Well, mostly. Nice hand anyway!"],
            ("Wren", Moment::LostShowdown) => &["I knew I should've sat in the other chair. Nice hand."],
            ("Wren", Moment::Welcome) => &["Welcome! Please don't sit on my lucky side. Kidding. A bit."],
            _ => &[],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bot_is_someone() {
        // Every name a bot sits down with has a personality, and they differ.
        for n in ["Ace", "Blaze", "Cinder", "Dusk", "Ember", "Flint", "Gale", "Hex", "Ivy", "Jinx", "Koda", "Lux", "Moss", "Nova", "Onyx", "Pike", "Quill", "Rook", "Sage", "Tarn", "Wren"] {
            let p = of(n);
            assert_eq!(p.name, n);
            assert!(p.style.len() < 90 && p.style.ends_with('.'));
            assert!((0.4..2.0).contains(&p.pace) && (0.2..2.0).contains(&p.loose));
            assert!(!p.lines(Moment::Welcome).is_empty(), "{n} says hello");
            for m in [Moment::WonShowdown, Moment::LostShowdown, Moment::AllIn, Moment::Busted] {
                assert!(p.lines(m).iter().all(|l| l.len() < 110), "{n}: short enough for a bubble");
            }
        }
        assert_eq!(of("Ace 3").name, "Ace");
        assert_eq!(of("Nobody").name, "Bot");
        // No two play alike.
        for (i, a) in ALL.iter().enumerate() {
            for b in &ALL[i + 1..] {
                assert!((a.loose, a.aggression, a.bluff, a.pace) != (b.loose, b.aggression, b.bluff, b.pace), "{} and {}", a.name, b.name);
            }
        }
    }
}
