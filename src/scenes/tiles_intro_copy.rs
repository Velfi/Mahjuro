//! Shared copy for the tiles intro lesson used by Guide and Tutorial Campaign.

pub const PAGE_TITLE: &str = "The Tiles";

/// Legacy block for tutorial height estimates.
pub const INTRO: &str = "Tiles are the pieces drawn from the wall. Most tiles have 4 copies.\nSuits are tile families. A meld usually uses tiles from one suit.";

pub const SECTION_NUMBER_SUITS: &str = "NUMBER SUITS";
pub const SECTION_HONOR_SUITS: &str = "HONOR SUITS";
pub const SECTION_FLOWERS: &str = "FLOWERS";
pub const SECTION_RANK_TERMS: &str = "RANK TERMS";

// Tutorial campaign still references these heading names for layout.
pub const NUMBER_SUITS_HEADING: &str = SECTION_NUMBER_SUITS;
pub const HONOR_SUITS_HEADING: &str = SECTION_HONOR_SUITS;
pub const RANK_TERMS_HEADING: &str = SECTION_RANK_TERMS;
pub const SEQUENCE_RULES_HEADING: &str = "SEQUENCE RULES";

pub const NUMBER_SUIT_LINES: &[&str] = &[
    "Manzu — ranks 1–9.",
    "Souzu — ranks 1–9.",
    "Pinzu — ranks 1–9.",
];

pub const HONOR_LINES: &[&str] = &[
    "Winds — East, South, West, North.",
    "Dragons — Red, Green, White.",
];

pub const FLOWER_LINES: &[&str] = &["Flowers — wildcards in melds."];

pub const RANK_TERM_LINES: &[&str] = &[
    "Ranks — numbers on number-suit tiles.",
    "Simples — ranks 2–8.",
    "Terminals — ranks 1 and 9.",
    "Honors — Winds and Dragons; no rank.",
];

/// Tutorial campaign page 1 still teaches sequence rules inline.
pub const SEQUENCE_RULE_LINES: &[&str] = &[
    "Only number suits can form sequences.",
    "Sequences must stay in one suit.",
    "Honors cannot form sequences.",
];
