//! Shared copy for the Guide melds page (tile-shape examples + run terms).

pub const PAGE_TITLE: &str = "Melds & Yaku";

pub const INTRO_LINE_1: &str =
    "Melds are small tile groups — pairs, sequences, triplets, and kongs.";
pub const INTRO_LINE_2: &str =
    "Bank them into your structure, then cash in to score.";

pub const SECTION_STRUCTURE: &str = "STRUCTURE";
pub const STRUCTURE_LINES: &[&str] = &[
    "Your structure is the melds you have banked this round.",
    "Select tiles from your hand and press Play to bank valid melds.",
    "Banked tiles leave your hand and move to the structure.",
    "You can keep banking melds until the structure is full."
    ];
    
    pub const SECTION_CASH_IN: &str = "CASH IN";
    pub const CASH_IN_LINES: &[&str] = &[
    "You can cash in any time a structure exists.",
    "Cashing in scores every meld in the structure.",
    "Score equals chips × mult — modified by tiles, yaku, relics, and bosses.",
    "Cashing in a big structure has a bigger payout than cashing in a small one.",
    "Meet or exceed a round's target score to win that round.",
];
