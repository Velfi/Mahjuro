//! Shared copy for the Guide scoring basics page (page 4).

pub const PAGE_TITLE: &str = "Scoring Basics";
pub const SUBTITLE: &str = "Play to your Structure · Cash In when ready";

pub const SECTION_LOOP: &str = "THE LOOP";
pub const LOOP_CAPTION: &str =
    "Play melds to Structure → Cash In → ↻ repeat · You don't score until Cash In";

pub const SECTION_TILES: &str = "TILES & CHIPS";
pub const TILES_INTRO: &str = "Tiles in your Structure are tallied when you Cash In.";

pub const SECTION_YAKU: &str = "YAKU";
pub const YAKU_INTRO: &str =
    "Yaku are bonus patterns made from melds. When you Cash In, each yaku your structure matches adds chips and mult.";
pub const YAKU_TABLE_HEADER: (&str, &str, &str) = ("Yaku", "+Mult", "+Chips");
pub const YAKU_TABLE_ROWS: &[(&str, &str, &str)] = &[
    ("Tanyao", "2.0", "30"),
    ("Toitoi", "3.0", "42"),
    ("Yakuhai", "3.0", "40"),
];

pub const SECTION_SCORE: &str = "YOUR SCORE";
pub const SCORE_INTRO: &str = "When you Cash In:";
pub const FINAL_EQUATION: &str = "score = chips × mult";
pub const SCORE_CHIPS_LINE: &str =
    "Chips = tile values + yaku chips + dora + relics";
pub const SCORE_MULT_LINE: &str =
    "Mult = 1.0 + yaku mult + relic mult + boss rules";
pub const SCORE_EXAMPLE: &str = "Example: 200 chips × 3.0 mult = 600 score";

/// Arrow glyph between loop diagram stages.
pub const LOOP_ARROW: &str = "➡️";
