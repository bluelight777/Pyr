//! How Reel Row's twelve machines look and talk: their names, symbols, colors and the words of
//! their bonuses. The machines are Chimbita's Reel Row (MIT), brought into Pyr Casino; their
//! math is `pyr_poker::casino::reels`, and their pictures are Meshy's (`casino::pics`).

use eframe::egui::Color32;

/// What makes a machine funny: each of the four funny ones has a gimmick of its own (see
/// `casino::humor`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Humor {
    /// Scoville Slots: a heat meter that climbs with every spin that doesn't pay, and a chili
    /// who sweats it out (and breathes fire when it boils over).
    Heat,
    /// Sugar Rush: the dentist's report, adding up the sugar the reels land, and a tooth that
    /// cracks under it.
    Dentist,
    /// The Daily Grind: a caffeine level that runs down, sleepy reels when it's empty, a mug who
    /// nods off, and office email.
    Office,
    /// Frostbite Fortune: every win a fish story, each taller than the last.
    FishTales,
}

pub struct Theme {
    /// Its pictures' name (`reels-<id>-tile`, `reels-<id>-symbols`).
    pub id: &'static str,
    pub name: &'static str,
    pub tagline: &'static str,
    /// The sheet's nine cells: the six pay symbols (low to high), the wild, the scatter, the
    /// bonus's hazard.
    pub symbols: [&'static str; 9],
    /// The scatters (plural), and the bonus they start.
    pub scatters: &'static str,
    pub bonus: &'static str,
    /// Pick-and-dodge's words: a hazard (and more than one), a life, the gold prize, the end and
    /// the finale.
    pub hazard: &'static str,
    pub hazards: &'static str,
    pub life: &'static str,
    pub gold: &'static str,
    pub busted: &'static str,
    pub finale: &'static str,
    /// Its colors: deep (the cabinet's shadow), mid (the cabinet), and its accent (lights,
    /// words).
    pub deep: Color32,
    pub mid: Color32,
    pub accent: Color32,
    pub humor: Option<Humor>,
}

const fn c(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

pub const THEMES: [Theme; 12] = [
    Theme {
        id: "scoville",
        name: "Scoville Slots",
        tagline: "Climb from jalapeño to Pepper X.",
        symbols: ["Lemon", "Onion", "Garlic", "Tomato", "Avocado", "Fireball", "Chili (wild)", "Hot sauce", "Carolina Reaper"],
        scatters: "hot sauce bottles",
        bonus: "Scoville Showdown",
        hazard: "Carolina Reaper",
        hazards: "Carolina Reapers",
        life: "glass of milk",
        gold: "golden Pepper X",
        busted: "SCOVILLED!",
        finale: "IRON STOMACH",
        deep: c(0x2a, 0x06, 0x04),
        mid: c(0x8a, 0x1a, 0x0a),
        accent: c(0xff, 0xb0, 0x2e),
        humor: Some(Humor::Heat),
    },
    Theme {
        id: "deepsea",
        name: "Deep Sea Treasure",
        tagline: "Dive for pearls in the abyss.",
        symbols: ["Seashell", "Crab", "Angelfish", "Octopus", "Sea turtle", "Sapphire", "Mermaid (wild)", "Anchor", "Shark"],
        scatters: "anchors",
        bonus: "Abyss Dive",
        hazard: "shark",
        hazards: "sharks",
        life: "air tank",
        gold: "king's pearl",
        busted: "SHARK BITE!",
        finale: "TREASURE TROVE",
        deep: c(0x02, 0x12, 0x22),
        mid: c(0x0a, 0x4a, 0x6e),
        accent: c(0x4a, 0xd8, 0xff),
        humor: None,
    },
    Theme {
        id: "cosmic",
        name: "Cosmic Jackpot",
        tagline: "Hyperjump across the galaxy.",
        symbols: ["Moon", "Ringed planet", "Comet", "Flying saucer", "Alien", "Star", "Rocket (wild)", "Nebula", "Black hole"],
        scatters: "nebulas",
        bonus: "Orbit Wheel",
        hazard: "black hole",
        hazards: "black holes",
        life: "shield",
        gold: "star core",
        busted: "SPAGHETTIFIED!",
        finale: "GALAXY CLEARED",
        deep: c(0x07, 0x04, 0x1c),
        mid: c(0x2a, 0x1a, 0x6a),
        accent: c(0xb0, 0x8a, 0xff),
        humor: None,
    },
    Theme {
        id: "sugar",
        name: "Sugar Rush",
        tagline: "Sweets stacked to the sky.",
        symbols: ["Candy", "Lollipop", "Cupcake", "Doughnut", "Chocolate", "Birthday cake", "Cake slice (wild)", "Candy tent", "Toothache"],
        scatters: "candy tents",
        bonus: "Candy Wheel",
        hazard: "toothache",
        hazards: "toothaches",
        life: "toothbrush",
        gold: "golden gumball",
        busted: "SUGAR CRASH!",
        finale: "SWEET TOOTH",
        deep: c(0x2e, 0x06, 0x22),
        mid: c(0xb8, 0x3a, 0x7a),
        accent: c(0xff, 0xd1, 0xe8),
        humor: Some(Humor::Dentist),
    },
    Theme {
        id: "goldrush",
        name: "Gold Rush",
        tagline: "Strike it rich in the hills.",
        symbols: ["Gold ore", "Pickaxe", "Gold coin", "Sack of gold", "Horse", "Medal", "Cowboy hat (wild)", "Dynamite", "Cave-in"],
        scatters: "dynamite sticks",
        bonus: "Mine Shaft",
        hazard: "cave-in",
        hazards: "cave-ins",
        life: "hard hat",
        gold: "mother lode",
        busted: "CAVE-IN!",
        finale: "MOTHER LODE",
        deep: c(0x22, 0x12, 0x04),
        mid: c(0x7a, 0x4a, 0x1a),
        accent: c(0xff, 0xc8, 0x4a),
        humor: None,
    },
    Theme {
        id: "neon",
        name: "Neon Nights",
        tagline: "Spin the big wheel on the Strip.",
        symbols: ["Cherries", "Bell", "Clover", "Seven", "Heart", "Diamond", "Jester (wild)", "Big wheel", "Short circuit"],
        scatters: "wheels",
        bonus: "Big Wheel",
        hazard: "short circuit",
        hazards: "short circuits",
        life: "spare fuse",
        gold: "neon jackpot",
        busted: "BLACKOUT!",
        finale: "LIGHTS ON",
        deep: c(0x0c, 0x02, 0x18),
        mid: c(0x3a, 0x0a, 0x5a),
        accent: c(0xff, 0x4a, 0xd8),
        humor: None,
    },
    Theme {
        id: "tomb",
        name: "Tomb of Ra",
        tagline: "Seize the cursed amulets.",
        symbols: ["Amphora", "Cobra", "Scarab", "Eye of Horus", "Cat of Bastet", "Ankh", "Sun of Ra (wild)", "Scroll", "Mummy"],
        scatters: "scrolls",
        bonus: "Curse of the Tomb",
        hazard: "mummy",
        hazards: "mummies",
        life: "amulet",
        gold: "pharaoh's crown",
        busted: "CURSED!",
        finale: "TOMB RAIDED",
        deep: c(0x16, 0x0e, 0x04),
        mid: c(0x6a, 0x4a, 0x12),
        accent: c(0xff, 0xd2, 0x4a),
        humor: None,
    },
    Theme {
        id: "frost",
        name: "Frostbite Fortune",
        tagline: "Ice fishing from Chicago to Antarctica.",
        symbols: ["Snowflake", "Mittens", "Ice skate", "Penguin", "Seal", "Ice crystal", "Snowman (wild)", "Fishing rod", "Thin ice"],
        scatters: "fishing rods",
        bonus: "Ice Fishing",
        hazard: "thin ice",
        hazards: "patches of thin ice",
        life: "hot cocoa",
        gold: "golden fish",
        busted: "FROZEN!",
        finale: "TROPHY BOARD",
        deep: c(0x03, 0x10, 0x22),
        mid: c(0x1a, 0x4a, 0x7a),
        accent: c(0xbf, 0xe8, 0xff),
        humor: Some(Humor::FishTales),
    },
    Theme {
        id: "cauldron",
        name: "Moonlit Cauldron",
        tagline: "Brew until three potions match.",
        symbols: ["Acorn", "Toadstool", "Candle", "Clover", "Owl", "Crystal ball", "Wizard hat (wild)", "Spell book", "Poison"],
        scatters: "spell books",
        bonus: "Potion Brew",
        hazard: "poison",
        hazards: "bottles of poison",
        life: "antidote",
        gold: "jar of moon dust",
        busted: "HEXED!",
        finale: "MASTER BREWER",
        deep: c(0x0e, 0x06, 0x18),
        mid: c(0x3a, 0x1a, 0x5a),
        accent: c(0x8a, 0xff, 0x6a),
        humor: None,
    },
    Theme {
        id: "hoard",
        name: "Dragon's Hoard",
        tagline: "From hatchling to Elder Dragon.",
        symbols: ["Gold coin", "Shield", "Trophy", "Crown", "Ring", "Ruby", "Dragon egg (wild)", "Dragon flame", "Angry dragon"],
        scatters: "dragon flames",
        bonus: "Dragon's Den",
        hazard: "angry dragon",
        hazards: "angry dragons",
        life: "fire cloak",
        gold: "dragon heart",
        busted: "TORCHED!",
        finale: "FULL HOARD",
        deep: c(0x02, 0x16, 0x0c),
        mid: c(0x0e, 0x4a, 0x2a),
        accent: c(0x6a, 0xff, 0x9a),
        humor: None,
    },
    Theme {
        id: "grind",
        name: "The Daily Grind",
        tagline: "Decaf to Nitro Cold Brew.",
        symbols: ["Coffee beans", "Ice", "Croissant", "Teapot", "Coffee", "Trophy", "Iced coffee (wild)", "Grinder", "Spill"],
        scatters: "grinders",
        bonus: "Cup Shuffle",
        hazard: "spill",
        hazards: "spills",
        life: "napkin",
        gold: "golden bean",
        busted: "SPILLED!",
        finale: "BARISTA OF THE YEAR",
        deep: c(0x16, 0x0c, 0x04),
        mid: c(0x5a, 0x34, 0x18),
        accent: c(0xff, 0xcf, 0x8a),
        humor: Some(Humor::Office),
    },
    Theme {
        id: "cabaret",
        name: "Starlight Cabaret",
        tagline: "From the chorus line to headliner.",
        symbols: ["Feather fan", "Top hat", "Champagne", "Rose", "Masks", "Star", "Plume (wild)", "Spotlight", "Toppled glass"],
        scatters: "spotlights",
        bonus: "Showtime Wheel",
        hazard: "toppled glass",
        hazards: "toppled glasses",
        life: "encore",
        gold: "diamond glass",
        busted: "CURTAIN DOWN!",
        finale: "GRAND FINALE",
        deep: c(0x16, 0x02, 0x06),
        mid: c(0x6a, 0x0a, 0x14),
        accent: c(0xff, 0xd2, 0x7a),
        humor: None,
    },
];

pub fn theme(m: u8) -> &'static Theme {
    &THEMES[(m as usize).min(THEMES.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pyr_poker::casino::reels;

    #[test]
    fn the_app_and_the_house_agree_on_the_machines() {
        assert_eq!(THEMES.len(), reels::MACHINES.len());
        for (i, t) in THEMES.iter().enumerate() {
            assert_eq!(t.name, reels::MACHINES[i].0, "machine {i}");
        }
        // Four funny ones, each with its own gimmick.
        let funny: Vec<_> = THEMES.iter().filter_map(|t| t.humor).collect();
        assert_eq!(funny.len(), 4);
        for (i, h) in funny.iter().enumerate() {
            assert!(!funny[..i].contains(h), "{h:?} twice");
        }
    }
}
