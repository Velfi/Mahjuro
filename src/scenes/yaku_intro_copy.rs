//! Shared copy for the Guide yaku intro page (structure, cash in).

pub const PAGE_TITLE: &str = "Yaku";

pub const PAGE_SUBTITLE: &str =
    "Play valid melds into your structure, then Cash In to score. Yaku are bonus patterns in your structure — they add chips and mult when you Cash In.";

pub const SECTION_STRUCTURE: &str = "STRUCTURE";
pub const STRUCTURE_LINES: &[&str] = &[
    "Your structure is the melds you have played this round.",
    "Select tiles from your hand and press Play.",
    "Played tiles leave your hand and move to the structure.",
    "You can keep playing melds until the structure is full.",
];

pub const SECTION_CASH_IN: &str = "CASH IN";
pub const CASH_IN_LINES: &[&str] = &[
    "You can cash in any time a structure exists.",
    "Cashing in scores every meld in the structure.",
    "Score equals chips × mult — modified by tiles, yaku, relics, and bosses.",
    "Cashing in a big structure has a bigger payout than cashing in a small one.",
    "Meet or exceed a round's target score to win that round.",
];
