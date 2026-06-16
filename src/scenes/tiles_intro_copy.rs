//! Guide tiles page copy.

pub const PAGE_TITLE: &str = "The Tiles";

pub const PAGE_SUBTITLE: &str = "Mahjuro is a game played with tiles.";

pub const INTRO: &str = "Your hand of tiles is drawn from the \"wall\". All tiles have a suit. Some tiles have ranks.\n\nTiles ranked 1 and 9 are called \"terminals\".\nRanks 2 through 8 are called \"simples\".\n\nIn Mahjong and Mahjuro, suits don't mix.";

pub const SECTION_NUMBER_SUITS: &str = "RANKED SUITS (scored by face value)";
pub const SECTION_HONOR_SUITS: &str = "HONOR SUITS (high-scoring, but rarer)";
pub const SECTION_FLOWERS: &str = "SPECIAL TILES";

pub const NUMBER_SUIT_LINES: &[&str] = &[
    "Manzu — ranks 1–9.",
    "Souzu — ranks 1–9.",
    "Pinzu — ranks 1–9.",
];

pub const HONOR_LINES: &[&str] = &[
    "Winds — East, South, West, North.",
    "Dragons — Red, Green, White.",
];

pub const FLOWER_LINES: &[&str] = &["Flowers — wildcards when playing tiles."];
