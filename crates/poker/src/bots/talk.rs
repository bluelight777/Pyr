//! What bots say at the table: a little trash talk, the way a reformed troll does it (the old
//! jab starts, then they catch themselves). Only ever about what everyone saw happen (a pot
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
                (None, "Thanks for the chips. I was going to say \"easy\", but I'm working on myself."),
                (None, "Good fold. Probably. I'll take what I had to the grave."),
                (None, "You folded. Old me would've called that fear. New me calls it wisdom."),
                (None, "Free chips! Sorry. I mean: what a thoughtful gift."),
                (None, "I could show you my cards. I won't. But I could. Growth is slow."),
                (None, "I'd explain that bet, but my therapist says I don't owe anyone an explanation."),
                (Some(Rookie), "Wait, everyone folded? So I win? Yay! I mean: good game."),
                (Some(Genius), "That fold was right more often than not. Lovely odds. For me."),
                (Some(Shark), "Nobody wanted to play with me? It's fine. I'm fine. Thanks for the chips."),
            ],
            Moment::WonShowdown => &[
                (None, "Read you like a book. A nice book. Five stars. Would read again."),
                (None, "GG. The nice kind of GG, not my old sarcastic kind."),
                (None, "Ship it. Respectfully."),
                (None, "I had a whole victory speech ready. I'm skipping it. You're welcome."),
                (None, "I'd tell you what you did wrong, but nobody asked. I'm learning that."),
                (None, "That pot looked lonely. I gave it a good home."),
                (None, "This is me not gloating. Can you tell? I've been practicing."),
                (None, "Old me would've typed \"too easy\". New me types \"well played, friend\"."),
                (Some(Rookie), "I won? I WON! Sorry. Inside voice. Good hand though!"),
                (Some(Genius), "A reasonable call. Reasonable calls lose sometimes. That's the beauty of it."),
                (Some(Shark), "Thank you for your contribution. Sincerely. No sarcasm. Almost none."),
            ],
            Moment::WonBig => &[
                (None, "That one's going on the fridge."),
                (None, "Big pot! I'm being very humble about it on the outside."),
                (None, "I'm allowed one happy dance. This is it. It's a small dance."),
                (None, "Okay, that felt amazing. One minute of insufferable, then back to nice."),
                (Some(Genius), "That's the pot the math promised me. I'm choosing to be gracious about it."),
            ],
            Moment::LostShowdown => &[
                (None, "Nice hand. I'm not mad. I'm writing it in my journal, but I'm not mad."),
                (None, "Wow. Okay. Deep breaths. Well played."),
                (None, "Old me would've typed something in all caps. New me says congrats. Lowercase."),
                (None, "You got me. Enjoy it. I'll be thinking about it all night, in a healthy way."),
                (None, "Good call. I hate it. I respect it. Mostly the second one."),
                (None, "I had a comeback ready. Then I remembered kindness. Nice hand."),
                (None, "Fine. Fine! Great hand. I'm smiling. Can't you tell?"),
                (Some(Rookie), "Oh no. Oh well! Teach me that one sometime?"),
                (Some(Genius), "Unlucky for me, correct for you. I'll allow it."),
                (Some(Shark), "Respect. Don't get used to it."),
            ],
            Moment::BadBeat => &[
                (None, "The river and I are going to have a calm, grown-up conversation later."),
                (None, "That card was rude. You weren't. Well played."),
                (None, "I'm fine. This is my fine face. Nice hand."),
                (None, "Statistically I was ahead. Emotionally I'm still ahead. Congrats."),
                (None, "Old me would've written a three-paragraph rant. New me: gg. Two letters. So much growth."),
                (Some(Genius), "I was a big favorite there. The deck disagreed. The deck is allowed to disagree."),
            ],
            Moment::AllIn => &[
                (None, "All in. Not to be dramatic. Okay, a little dramatic."),
                (None, "Everything in the middle. Be gentle with me."),
                (None, "All in. I thought long and hard about this. Four seconds, mostly."),
                (None, "Shoving. Respectfully."),
                (None, "All my chips, right there. No pressure. Some pressure."),
                (Some(Rookie), "I'm putting it all in! That's allowed? Great!"),
                (Some(Genius), "All in. The math says so, and for once my heart agrees."),
            ],
            Moment::CalledAllIn => &[
                (None, "I call. Respectfully. Aggressively respectfully."),
                (None, "Let's see them. No hard feelings either way. Some hard feelings."),
                (None, "I call. If I lose, I'll be gracious. I've been practicing that too."),
            ],
            Moment::OtherWonBig => &[
                (None, "Old me would've said \"lucky\". New me says congrats. Earned."),
                (None, "Big pot! Look at you go."),
                (None, "Nice one. I'm clapping. Bots can clap. Don't fact-check that."),
                (None, "Good pot. I'm only a little jealous. A healthy amount."),
            ],
            Moment::Busted => &[
                (None, "Well. I've been humbled. Back to the bot gym."),
                (None, "I'm out. Tell the river I said nothing rude about it."),
                (None, "GG, everyone. I'll go reflect on my choices. All of them."),
            ],
            Moment::OtherBusted => &[
                (None, "GG. The real GG, with feelings. Come back soon."),
                (None, "Tough one. The table's quieter without you. Come back and take my chips."),
                (None, "Good game. I'd say \"get good\", but you already are. Just unlucky."),
            ],
            Moment::Welcome => &[
                (None, "Welcome! I promise to be nice. I've been practicing."),
                (None, "Fresh chips at the table! I mean: hello, friend."),
                (None, "Oh good, company. Pull up a chair. I don't bite. Anymore."),
                (None, "Welcome. Fair warning: I used to be a menace. I'm reformed. Mostly."),
            ],
        }
    }
}

/// A line for `who` (a bot of `level`) at `m`, if it says one now (by the moment's chance, and
/// how chatty it is), not one said lately at the table: in its own voice when it has something
/// for the moment (most of the time), otherwise what any reformed troll would say.
pub fn line(m: Moment, level: Level, who: &Persona, recent: &[&str], rng: &mut Stream) -> Option<&'static str> {
    if (rng.below(10_000) as f64) / 10_000.0 >= (m.chance() * who.chatty).min(0.9) {
        return None;
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
