//! Pyr's words: one short word with its picture (an emoji) for each value of a byte, so a few
//! bytes of a hash can be said aloud or compared by eye instead of a row of digits ("Compare
//! words", see `handshake`). Chosen to be easy to say and hear apart: common things, no two
//! that differ by one sound (cat and car, ship and sheep), and no two that said in a row sound
//! like a third (pine, apple: pineapple).

/// The words, by byte value: (picture, word).
pub const WORDS: [(&str, &str); 256] = [
    ("🦊", "fox"), ("🐻", "bear"), ("🐼", "panda"), ("🐨", "koala"),
    ("🐯", "tiger"), ("🦁", "lion"), ("🐮", "cow"), ("🐷", "pig"),
    ("🐸", "frog"), ("🐵", "monkey"), ("🐔", "chicken"), ("🐧", "penguin"),
    ("🦆", "duck"), ("🦅", "eagle"), ("🦉", "owl"), ("🍑", "peach"),
    ("🐺", "wolf"), ("🐴", "horse"), ("🦄", "unicorn"), ("🐝", "bumblebee"),
    ("🦋", "butterfly"), ("🐌", "snail"), ("🐞", "ladybug"), ("🐜", "ant"),
    ("🕷️", "spider"), ("🐢", "turtle"), ("🐍", "snake"), ("🦎", "lizard"),
    ("🦖", "dino"), ("🐙", "octopus"), ("🦑", "squid"), ("🦀", "crab"),
    ("🐠", "fish"), ("🐬", "dolphin"), ("🐳", "whale"), ("🦈", "shark"),
    ("🐊", "crocodile"), ("🦓", "zebra"), ("🦍", "gorilla"), ("🐘", "elephant"),
    ("🦒", "giraffe"), ("🦘", "kangaroo"), ("🐪", "camel"), ("🦙", "llama"),
    ("🐑", "sheep"), ("🐐", "goat"), ("🦌", "reindeer"), ("🐕", "puppy"),
    ("🐈", "cat"), ("🐇", "rabbit"), ("🐿️", "squirrel"), ("🦔", "hedgehog"),
    ("🦦", "otter"), ("🦥", "sloth"), ("🦩", "flamingo"), ("🦚", "peacock"),
    ("🦜", "parrot"), ("🦢", "swan"), ("🐓", "rooster"), ("🦃", "turkey"),
    ("🐁", "mouse"), ("🦭", "seal"), ("🦏", "rhino"), ("🦛", "hippo"),
    ("🐆", "leopard"), ("🦨", "skunk"), ("🐉", "dragon"), ("🍎", "apple"),
    ("🍊", "orange"), ("🍋", "lemon"), ("🍌", "banana"), ("🍉", "watermelon"),
    ("🍇", "grapes"), ("🍓", "strawberry"), ("🍒", "cherry"), ("🥭", "mango"),
    ("🍍", "pineapple"), ("🥥", "coconut"), ("🥝", "kiwi"), ("🍅", "tomato"),
    ("🥑", "avocado"), ("🥦", "broccoli"), ("🥕", "carrot"), ("🌽", "corn"),
    ("🌶️", "chili"), ("🥒", "pickle"), ("🍄", "mushroom"), ("🫑", "pepper"),
    ("🍞", "bread"), ("🥨", "pretzel"), ("🧀", "cheese"), ("🥚", "egg"),
    ("🥞", "pancake"), ("🧇", "waffle"), ("🥓", "bacon"), ("🍔", "burger"),
    ("🍟", "fries"), ("🍕", "pizza"), ("🌭", "hotdog"), ("🥪", "sandwich"),
    ("🌮", "taco"), ("🌯", "burrito"), ("🥗", "salad"), ("🍿", "popcorn"),
    ("🍣", "sushi"), ("🥐", "croissant"), ("🍪", "cookie"), ("🎂", "birthday"),
    ("🍫", "chocolate"), ("🍬", "candy"), ("🍭", "lollipop"), ("🍯", "honey"),
    ("🥛", "milk"), ("☕", "coffee"), ("🧋", "boba"), ("🧃", "juice"),
    ("🧊", "icecube"), ("🥖", "baguette"), ("🥧", "pie"), ("🍜", "noodles"),
    ("🥟", "dumpling"), ("🧂", "salt"), ("🍤", "shrimp"), ("☀️", "sun"),
    ("🌙", "moon"), ("⭐", "star"), ("☁️", "cloud"), ("🌈", "rainbow"),
    ("⚡", "lightning"), ("❄️", "snowflake"), ("🔥", "fire"), ("🌊", "wave"),
    ("🌋", "volcano"), ("🏔️", "summit"), ("🌲", "evergreen"), ("🌴", "palm"),
    ("🌵", "cactus"), ("🌻", "sunflower"), ("🌹", "rose"), ("🌷", "tulip"),
    ("🍀", "clover"), ("🍁", "leaf"), ("🪨", "stone"), ("🌍", "globe"),
    ("☄️", "comet"), ("🌪️", "tornado"), ("⚓", "anchor"), ("🔑", "key"),
    ("🔒", "padlock"), ("🔔", "bell"), ("🎈", "balloon"), ("🎁", "gift"),
    ("🎀", "ribbon"), ("📚", "books"), ("✏️", "pencil"), ("📎", "paperclip"),
    ("✂️", "scissors"), ("🔨", "hammer"), ("🪓", "axe"), ("🔧", "wrench"),
    ("⚙️", "gear"), ("🧲", "magnet"), ("💡", "lightbulb"), ("🕯️", "candle"),
    ("🔦", "flashlight"), ("🧭", "compass"), ("⏰", "alarm"), ("⌛", "hourglass"),
    ("📷", "camera"), ("📺", "television"), ("📻", "radio"), ("☎️", "telephone"),
    ("💻", "laptop"), ("⌨️", "keyboard"), ("🎮", "gamepad"), ("🕹️", "joystick"),
    ("🎲", "dice"), ("🧩", "puzzle"), ("🎯", "target"), ("🎳", "bowling"),
    ("⚽", "soccer"), ("🏀", "basketball"), ("🏈", "football"), ("⚾", "baseball"),
    ("🎾", "tennis"), ("🏐", "volleyball"), ("🏓", "pingpong"), ("🥊", "boxing"),
    ("🛹", "skateboard"), ("🚲", "bicycle"), ("🛴", "scooter"), ("🎸", "guitar"),
    ("🎹", "piano"), ("🥁", "drum"), ("🎺", "trumpet"), ("🎻", "violin"),
    ("🎷", "saxophone"), ("🎤", "microphone"), ("🎧", "headphones"), ("🎨", "paint"),
    ("🖌️", "paintbrush"), ("🧵", "spool"), ("🧶", "yarn"), ("👑", "crown"),
    ("💎", "diamond"), ("👓", "spectacles"), ("👟", "sneaker"), ("🧤", "gloves"),
    ("🧣", "scarf"), ("☂️", "umbrella"), ("🎒", "backpack"), ("🧳", "suitcase"),
    ("🪑", "chair"), ("🪴", "plant"), ("🚪", "door"), ("🧸", "teddy"),
    ("🪁", "kite"), ("🏆", "trophy"), ("🏅", "medal"), ("🎫", "ticket"),
    ("🔭", "telescope"), ("🔬", "microscope"), ("🧺", "basket"), ("🧼", "soap"),
    ("🧱", "brick"), ("🪵", "wood"), ("🛡️", "shield"), ("🎪", "circus"),
    ("🎢", "coaster"), ("🏰", "castle"), ("⛺", "tent"), ("🏠", "home"),
    ("🏝️", "island"), ("🌉", "bridge"), ("🗼", "tower"), ("⛲", "fountain"),
    ("🎆", "fireworks"), ("🚕", "taxi"), ("🚌", "bus"), ("🚜", "tractor"),
    ("🏍️", "motorcycle"), ("🚂", "train"), ("🚁", "helicopter"), ("✈️", "airplane"),
    ("🚀", "rocket"), ("🛸", "ufo"), ("⛵", "sailboat"), ("🚢", "cruise"),
    ("🪂", "parachute"), ("❤️", "heart"), ("👻", "ghost"), ("👽", "alien"),
    ("🤖", "robot"), ("🎃", "pumpkin"), ("⛄", "snowman"), ("🪐", "saturn"),
];

/// `n` words from the start of `hash`.
pub fn words(hash: &[u8], n: usize) -> Vec<(&'static str, &'static str)> {
    hash.iter().take(n).map(|b| WORDS[*b as usize]).collect()
}

/// The words as you'd say them: "fox, volcano, guitar".
pub fn spoken(words: &[(&str, &str)]) -> String {
    words.iter().map(|(_, w)| *w).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_byte_has_its_own_word_and_picture() {
        let mut w: Vec<&str> = WORDS.iter().map(|(_, w)| *w).collect();
        let mut e: Vec<&str> = WORDS.iter().map(|(e, _)| *e).collect();
        w.sort();
        w.dedup();
        e.sort();
        e.dedup();
        assert_eq!((w.len(), e.len()), (256, 256));
        assert!(WORDS.iter().all(|(_, w)| w.chars().all(|c| c.is_ascii_lowercase()) && (2..=10).contains(&w.len())));
        // No two words said in a row sound like a third ("pine" then "apple": "pineapple").
        for (_, a) in WORDS {
            for (_, b) in WORDS {
                assert!(a == b || !b.starts_with(a) || !WORDS.iter().any(|(_, c)| b.strip_prefix(a) == Some(c)), "{a} + another word makes {b}");
            }
        }
        assert_eq!(spoken(&words(&[0, 1], 2)), "fox, bear");
    }
}
