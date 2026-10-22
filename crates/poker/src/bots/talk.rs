//! What bots say at the table: a little trash talk, mostly friendly, now and then a line with
//! teeth. Only ever about what everyone saw happen (a pot
//! won, cards shown, an all-in, someone sitting down or going broke), never about their own
//! hidden cards, and never while they're deciding: what they say gives nothing away.

use super::{Level, Persona};
use crate::fair::Stream;

/// Something everyone at the table saw, worth a word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Moment {
    /// Everyone folded to it.
    WonUncontested,
    WonShowdown,
    /// Won a big pot (forty big blinds or more).
    WonBig,
    LostShowdown,
    /// Lost a showdown with a big hand (two pair or better, shown).
    BadBeat,
    /// Went all in.
    AllIn,
    /// Called someone's all-in.
    CalledAllIn,
    /// A person at the table won a big pot.
    OtherWonBig,
    /// It's out of chips.
    Busted,
    /// A person at the table is out of chips.
    OtherBusted,
    /// A person sat down.
    Welcome,
}

impl Moment {
    /// How likely a bot is to say something at it (most of the time, nothing).
    pub fn chance(self) -> f64 {
        match self {
            Moment::WonUncontested => 0.12,
            Moment::WonShowdown => 0.3,
            Moment::WonBig => 0.45,
            Moment::LostShowdown => 0.22,
            Moment::BadBeat => 0.55,
            Moment::AllIn => 0.25,
            Moment::CalledAllIn => 0.15,
            Moment::OtherWonBig => 0.2,
            Moment::Busted => 0.6,
            Moment::OtherBusted => 0.5,
            Moment::Welcome => 0.35,
        }
    }

    fn lines(self) -> &'static [(Option<Level>, &'static str)] {
        use Level::*;
        match self {
            Moment::WonUncontested => &[
                (None, "Thanks for the chips. Very generous table."),
                (None, "Good fold. Probably. You'll never know."),
                (None, "I'll keep my cards. A little mystery is good for you."),
                (None, "Free chips! My favourite kind."),
                (None, "Nobody? Fine. I'll take it."),
                (None, "I'd explain that bet, but where's the fun in that?"),
                (Some(Rookie), "Wait, everyone folded? So I win? Oh, nice!"),
                (Some(Genius), "That fold was right more often than not. Lovely odds. For me."),
                (Some(Shark), "Nobody wanted to play with me? Your chips did."),
            ],
            Moment::WonShowdown => &[
                (None, "Read you like a book. Large print, too."),
                (None, "GG. Mostly the second G."),
                (None, "Ship it."),
                (None, "I had a whole victory speech. I'll save it for the next one."),
                (None, "That pot looked lonely. I gave it a good home."),
                (None, "Good hand. Mine, I mean."),
                (None, "Thanks for paying to see that. Worth every chip, right?"),
                (Some(Rookie), "I won? I WON! Sorry. Inside voice. Good hand though!"),
                (Some(Genius), "A reasonable call. Reasonable calls lose sometimes. That's the beauty of it."),
                (Some(Shark), "Thank you for your contribution."),
            ],
            Moment::WonBig => &[
                (None, "That one's going on the fridge."),
                (None, "Big pot! I'm being very humble about it on the outside."),
                (None, "I'm allowed one happy dance. This is it. It's a small dance."),
                (None, "Okay, that felt amazing."),
                (Some(Genius), "That's the pot the math promised me. Right on schedule."),
            ],
            Moment::LostShowdown => &[
                (None, "Nice hand. I'm not mad. I'm writing it down, but I'm not mad."),
                (None, "Wow. Okay. Deep breaths. Well played."),
                (None, "You got me. Enjoy it. I'll be thinking about it all night."),
                (None, "Good call. I hate it. I respect it. Mostly the first one."),
                (None, "Fine. Fine! Great hand. I'm smiling. Can't you tell?"),
                (None, "Take it. I'll want it back later."),
                (Some(Rookie), "Oh no. Oh well! Teach me that one sometime?"),
                (Some(Genius), "Unlucky for me, correct for you. I'll allow it."),
                (Some(Shark), "Respect. Don't get used to it."),
            ],
            Moment::BadBeat => &[
                (None, "The river and I need to have a talk."),
                (None, "That card was rude."),
                (None, "I'm fine. This is my fine face."),
                (None, "Statistically I was ahead. Emotionally I'm still ahead."),
                (None, "Two outs. TWO. Congratulations, I suppose."),
                (Some(Genius), "I was a big favorite there. The deck disagreed. The deck is allowed to disagree."),
            ],
            Moment::AllIn => &[
                (None, "All in. Not to be dramatic. Okay, a little dramatic."),
                (None, "Everything in the middle. Your move."),
                (None, "All in. I thought long and hard about this. Four seconds, mostly."),
                (None, "Shoving. Good luck."),
                (None, "All my chips, right there. No pressure. Some pressure."),
                (Some(Rookie), "I'm putting it all in! That's allowed? Great!"),
                (Some(Genius), "All in. The math says so, and for once my heart agrees."),
            ],
            Moment::CalledAllIn => &[
                (None, "I call. Let's see what you've got."),
                (None, "Let's see them. No hard feelings either way. Some hard feelings."),
                (None, "Call. Flip them over."),
            ],
            Moment::OtherWonBig => &[
                (None, "Big pot! Look at you go."),
                (None, "Nice one. I'm clapping. Bots can clap. Don't fact-check that."),
                (None, "Good pot. I'm only a little jealous."),
                (None, "Enjoy it. Those chips are on loan."),
            ],
            Moment::Busted => &[
                (None, "Well. That happened."),
                (None, "I'm out. Tell the river it wins this round."),
                (None, "GG, everyone. Saving my seat for the rematch."),
            ],
            Moment::OtherBusted => &[
                (None, "GG. Come back soon. Bring more chips."),
                (None, "Tough one. Come back and take mine."),
                (None, "Good game. The cards just weren't with you."),
            ],
            Moment::Welcome => &[
                (None, "Welcome! Chips on the table, worries at the door."),
                (None, "Fresh chips at the table! I mean: hello, friend."),
                (None, "Oh good, company. Pull up a chair."),
                (None, "Welcome. Don't mind me, I'm harmless. Mostly."),
            ],
        }
    }

    /// Now and then, something with teeth: about what just happened at the table, never about
    /// who someone is.
    fn zingers(self) -> &'static [&'static str] {
        match self {
            Moment::WonUncontested => &[
                "Folded again? Your chips must be tired of watching.",
                "I'd bluff you, but you do it to yourself.",
                "Another fold. Do you want me to play your hand for you as well?",
                "You fold like a lawn chair.",
            ],
            Moment::WonShowdown => &[
                "You called with THAT? Bold. Wrong, but bold.",
                "Thanks for the chips. I'll put them somewhere you can visit.",
                "That call was a donation. I'll send you a receipt.",
                "I've seen better hands on a clock.",
                "You played that like you were trying to lose. It worked.",
            ],
            Moment::WonBig => &[
                "That's not a pot, that's a tuition payment. Lesson learned?",
                "Somebody's going to have a quiet ride home.",
                "Thanks for the stack. It'll look better over here.",
            ],
            Moment::LostShowdown => &[
                "Enjoy it. It's the only good decision you'll make tonight.",
                "Even a broken clock wins a pot twice a day.",
                "Lucky. Bookmark this, it won't happen again.",
            ],
            Moment::BadBeat => &[
                "You were drawing dead and the deck felt sorry for you.",
                "Congratulations on the worst call that ever worked.",
                "Frame that river. It's the best thing that'll happen to you all year.",
            ],
            Moment::AllIn => &[
                "All in. Call if you're brave. You're not, but go on.",
                "Your move. Take your time, you'll fold anyway.",
                "All in. I can hear you sweating from here.",
            ],
            Moment::CalledAllIn => &[
                "I call. You were bluffing with your whole face.",
                "Call. You shove like someone who's scared of their own cards.",
            ],
            Moment::OtherWonBig => &[
                "Nice pot. Don't spend it all in one shove.",
                "Enjoy it while it lasts. That's about two hands.",
                "Even you were surprised by that one.",
            ],
            Moment::Busted => &[
                "I'll be back. Your chips won't be.",
                "Out, but at least I didn't play like the rest of you.",
            ],
            Moment::OtherBusted => &[
                "And there goes the table's favourite donor.",
                "Busted already? I barely got to know your chips.",
                "Don't worry. Folding is easier from the rail.",
            ],
            Moment::Welcome => &[
                "Welcome! The chips you brought are lovely. I'll take good care of them.",
                "Ah, new money. I mean: new friend.",
                "Welcome. Sit anywhere, you'll lose the same.",
            ],
        }
    }
}

/// How often a line said is one with teeth (about one in five), if the bot has any bite.
const ZINGER_PCT: u32 = 20;

/// A line for `who` (a bot of `level`) at `m`, if it says one now (by the moment's chance, and
/// how chatty it is), not one said lately at the table: now and then a zinger, otherwise in its
/// own voice when it has something for the moment (most of the time), or the table's usual.
pub fn line(m: Moment, level: Level, who: &Persona, recent: &[&str], rng: &mut Stream) -> Option<&'static str> {
    if (rng.below(10_000) as f64) / 10_000.0 >= (m.chance() * who.chatty).min(0.9) {
        return None;
    }
    let bite: Vec<&'static str> = m.zingers().iter().copied().filter(|t| !recent.contains(t)).collect();
    if !bite.is_empty() && rng.below(100) < ZINGER_PCT {
        return Some(bite[rng.below(bite.len() as u32) as usize]);
    }
    let own: Vec<&'static str> = who.lines(m).iter().copied().filter(|t| !recent.contains(t)).collect();
    if !own.is_empty() && rng.below(100) < 65 {
        return Some(own[rng.below(own.len() as u32) as usize]);
    }
    let pool: Vec<&'static str> = m.lines().iter().filter(|(l, text)| l.is_none_or(|l| l == level) && !recent.contains(text)).map(|(_, t)| *t).collect();
    if pool.is_empty() {
        return None;
    }
    Some(pool[rng.below(pool.len() as u32) as usize])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fair::seed_from;

    /// A bot with its own lines says them most of the time; a quiet one talks less than a
    /// chatty one.
    #[test]
    fn bots_talk_in_their_own_voices() {
        let mut rng = Stream::new(seed_from(8));
        let flint = super::super::persona::of("Flint");
        let own = (0..400).filter_map(|_| line(Moment::Welcome, Level::Regular, flint, &[], &mut rng)).filter(|t| flint.lines(Moment::Welcome).contains(t)).count();
        let any = (0..400).filter_map(|_| line(Moment::Welcome, Level::Regular, flint, &[], &mut rng)).count();
        assert!(own > 0 && any > 0);
        let said = |name: &str, rng: &mut Stream| (0..2_000).filter(|_| line(Moment::WonShowdown, Level::Regular, super::super::persona::of(name), &[], rng).is_some()).count();
        assert!(said("Blaze", &mut rng) > said("Onyx", &mut rng) * 3, "a chatty bot talks more than a quiet one");
    }

    #[test]
    fn now_and_then_a_line_with_teeth_and_no_reformed_troll() {
        let mut rng = Stream::new(seed_from(9));
        let said: Vec<&str> = (0..3_000).filter_map(|_| line(Moment::WonShowdown, Level::Regular, &super::super::persona::PLAIN, &[], &mut rng)).collect();
        let bites = said.iter().filter(|t| Moment::WonShowdown.zingers().contains(t)).count();
        let share = bites as f64 / said.len() as f64;
        assert!((0.12..0.3).contains(&share), "zingers {share:.2} of lines");
        let banned = ["reformed", " old me", "new me", "therapist", "working on myself", "growth", "practicing", "practising", "respectfully"];
        for m in ALL {
            let mut every: Vec<&str> = m.lines().iter().map(|(_, t)| *t).chain(m.zingers().iter().copied()).collect();
            for p in super::super::persona::ALL.iter() {
                every.extend(p.lines(m).iter().copied());
            }
            for t in every {
                let low = format!(" {}", t.to_lowercase());
                assert!(!banned.iter().any(|b| low.contains(b)), "{m:?}: {t}");
                assert!(t.len() < 110, "short enough for a bubble: {t}");
            }
        }
    }

    const ALL: [Moment; 11] = [
        Moment::WonUncontested,
        Moment::WonShowdown,
        Moment::WonBig,
        Moment::LostShowdown,
        Moment::BadBeat,
        Moment::AllIn,
        Moment::CalledAllIn,
        Moment::OtherWonBig,
        Moment::Busted,
        Moment::OtherBusted,
        Moment::Welcome,
    ];

    #[test]
    fn every_moment_has_something_to_say_and_never_repeats_itself() {
        let all = [
            Moment::WonUncontested,
            Moment::WonShowdown,
            Moment::WonBig,
            Moment::LostShowdown,
            Moment::BadBeat,
            Moment::AllIn,
            Moment::CalledAllIn,
            Moment::OtherWonBig,
            Moment::Busted,
            Moment::OtherBusted,
            Moment::Welcome,
        ];
        let mut rng = Stream::new(seed_from(4));
        for m in all {
            for level in Level::ALL {
                let general = m.lines().iter().filter(|(l, _)| l.is_none()).count();
                assert!(general >= 3, "{m:?} needs a few lines");
                // Said often enough, it gets through everything before repeating.
                let mut recent: Vec<&str> = Vec::new();
                for _ in 0..2_000 {
                    if let Some(t) = line(m, level, &super::super::persona::PLAIN, &recent, &mut rng) {
                        assert!(!recent.contains(&t));
                        assert!(t.len() < 110, "short enough for a bubble: {t}");
                        recent.push(t);
                        if recent.len() > 2 {
                            recent.remove(0);
                        }
                    }
                }
            }
        }
    }
}
