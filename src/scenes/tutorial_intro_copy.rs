//! All copy for the tutorial campaign, try-it demo, onboarding lessons, and summary.

// ── Part 1 — The Tiles ──────────────────────────────────────────────────────

pub mod tiles {
    pub const INTRO: &str = "Your hand of tiles is drawn from the \"wall\". All tiles have a suit. Some tiles have ranks.\nTiles ranked 1 and 9 are called \"terminals\". Tiles ranked 2–8 are called \"simples\".";

    pub const NUMBER_SUITS_HEADING: &str = "RANKED SUITS (scored by face value)";
    pub const HONOR_SUITS_HEADING: &str = "HONOR SUITS (high-scoring, but rarer)";
    pub const FLOWERS_HEADING: &str = "SPECIAL TILES";

    pub const NUMBER_SUIT_LINES: &[&str] = &[
        "Manzu — ranks 1–9.",
        "Souzu — ranks 1–9.",
        "Pinzu — ranks 1–9.",
    ];

    pub const HONOR_LINES: &[&str] = &[
        "Winds — East, South, West, North.",
        "Dragons — Red, Green, White.",
    ];

    pub const FLOWER_LINES: &[&str] = &["Flowers — wildcards when playing combinations of tiles."];
}

// ── Melds (Part 2 intro and demo labels) ────────────────────────────────────

pub mod melds {
    pub const PAGE_TITLE: &str = "Melds";

    pub const PAGE_SUBTITLE: &str = "Melds are small tile groups — pairs, sequences, triplets, and kongs. Valid melds can be played into your structure.";

    pub const VALID_SEQUENCE: &str = "Valid sequence";
    pub const INVALID_SEQUENCE: &str = "Invalid sequence";
}

// ── Scoring (Part 2 subtitle and cross-refs) ────────────────────────────────

pub mod scoring {
    pub const SUBTITLE: &str = "Select melds, Play them, then Cash In to score.";

    pub const FLOW_REMINDER: &str = "Played melds don't score until cashed in. You can cash in **whenever** you have a structure.";
}

// ── Campaign pages ────────────────────────────────────────────────────────────

pub mod campaign {
    use super::{melds, scoring};

    pub const PART1_TITLE: &str = "Part 1 — The Tiles";
    pub const PART1_CALLOUT: &str = "Next: melds and how to score.";

    pub const PART2_TITLE: &str = "Part 2 — Melds & Scoring";
    pub const PART2_CALLOUT: &str = "Your actions are limited — make them count.";

    pub const GLOSSARY_DISCARD: &str =
        "Discard — remove unwanted tiles from your hand. Discards are a limited resource.";
    pub const GLOSSARY_PLAY: &str =
        "Play — send melds to your structure. Plays are a limited resource.";
    pub const GLOSSARY_STRUCTURE: &str =
        "Structure — played melds that will score when you cash in. The structure has a fluid, but limited number of slots for melds.";
    pub const GLOSSARY_CASH_IN: &str =
        "Cash In — scores tiles in your structure and then resets it.";

    pub const PART2_GLOSSARY: &[&str] = &[
        melds::PAGE_SUBTITLE,
        GLOSSARY_DISCARD,
        GLOSSARY_PLAY,
        GLOSSARY_STRUCTURE,
        GLOSSARY_CASH_IN,
        scoring::FLOW_REMINDER,
    ];

    pub const PART2_SUBTITLE: &str = scoring::SUBTITLE;
}

// ── Try-it demo (Part 2 interactive flash lines) ────────────────────────────

pub mod try_it {
    pub const PROMPT: &str =
        "These are the **Discard**, **Play**, and **Cash In** buttons. Try them.";
    pub const PLAY: &str = "You **Play**ed a meld to your structure.";
    pub const DISCARD: &str = "Discarded tiles are removed to the river.";
    pub const CASH_IN: &str = "Demo: 4 **chips** × 3 **mult** = 12";

    /// Every flash line; used to reserve callout height for the tallest variant.
    pub const FLASH_LINES: &[&str] = &[PROMPT, CASH_IN, PLAY, DISCARD];
}

// ── Onboarding lessons (gameplay blind banners) ───────────────────────────────

pub mod lessons {
    pub const SELECT: &str = "Select melds from your hand.";
    pub const PLAY: &str = "Press **Play** to move your meld to Structure.";
    pub const CASH_IN: &str = "Press **Cash In** to score your structure.";
    pub const DISCARD: &str = "Select a tile you don't need, then **Discard**.";
    pub const DISCARD_RETRY: &str = "Try a **Discard** to improve your hand.";
    pub const SECOND_SCORE: &str =
        "Play another meld, then **Cash In** again to reach the target.";
    pub const FALLBACK_PLAY: &str = "Press **Play** to move your meld to Structure.";
    pub const FALLBACK_CASH_IN: &str =
        "Press **Cash In** when your structure is ready to score.";
    pub const FALLBACK_TARGET: &str = "Reach the target score before you run out of plays.";
    pub const RIVER_TIP: &str = "Discarded tiles sit in the river — they don't score.";

    pub const FAILURE_ZERO: &str =
        "You scored 0 — select melds, **Play** to Structure, then **Cash In**.";
    pub const FINALE_FAILURE_ZERO: &str = "You scored 0 — select melds, **Play** to Structure, then **Cash In**. If you run out of plays first, the round ends.";

    pub const FINALE_INTRO: &str = "You must now prepare to undergo an ordeal — The Iconoclast\n\n\
         The Iconoclast changes the rules of the game, debuffing Wind and Dragon tiles. \
         Debuffed tiles still form melds and yaku, but contribute +0 chips when you Cash In. \
         The blue **Guide** book on the table contains a refresher of mechanics.";
}

// ── Tutorial summary screen ─────────────────────────────────────────────────

pub mod summary {
    use super::scoring;

    pub const TITLE_WON: &str = "Tutorial Complete";
    pub const TITLE_LOST: &str = "Tutorial Recap";

    pub const SUBTITLE_WON: &str =
        "You surpassed The Iconoclast and won, an auspicious beginning.";
    pub const SUBTITLE_LOST: &str =
        "You reached the finale but faltered against The Iconoclast. Perhaps you'll fare better next time.";

    pub const BULLET_SCORING: &str =
        "Select melds, **Play** to Structure, then **Cash In** to score (**chips** × **mult**).";
    pub const BULLET_DISCARD: &str =
        "**Discard** tiles you don't need to draw replacements and build a stronger structure.";
    pub const BULLET_GUIDE: &str =
        "Open the **Guide** book on the table for tiles, melds, yaku, and scoring.";
    pub const BULLET_YAKU: &str = "Full Hand and Chiitoitsu are good yaku to learn first.";
    pub const BULLET_PROGRESS: &str =
        "The more you play, the more the house will reveal to you. How far will you go?";

    pub const BULLETS: &[&str] = &[
        BULLET_SCORING,
        scoring::FLOW_REMINDER,
        BULLET_DISCARD,
        BULLET_GUIDE,
        BULLET_YAKU,
        BULLET_PROGRESS,
    ];
}
