//! All copy for the tutorial campaign, try-it demo, onboarding lessons, and summary.

// ── Part 1 — The Tiles ──────────────────────────────────────────────────────

pub mod tiles {
    pub const INTRO: &str = "Three suits — characters, bamboos, and dots — each ranked 1 through 9.\n\nTiles ranked 1 and 9 are \"terminals\".\n\nRanks 2 through 8 are \"simples\".\n\nIn Mahjong and Mahjuro, suits don't mix.";

    pub const HONOR_SUITS_HEADING: &str = "HONOR SUITS";
    pub const FLOWERS_HEADING: &str = "SPECIAL TILES";

    pub const HONOR_LINES: &[&str] = &[
        "Winds — East, South, West, North.",
        "Dragons — Red, Green, White.",
    ];

    pub const FLOWER_LINES: &[&str] = &["Flowers — wildcards when playing combinations of tiles."];

    pub const FLOWERS_GRID_CALLOUT: &str =
        "Any flower can stand in for another tile when you Play.";
}

// ── Melds (Part 2 intro and demo labels) ────────────────────────────────────

pub mod melds {
    pub const PAGE_TITLE: &str = "Melds";

    pub const PAGE_SUBTITLE: &str =
        "Tile groups you Play into your structure — pairs, sequences, triplets, and kongs.";

    pub const INTRO: &str =
        "A meld is a combination of tiles. Excluding flowers, melds may contain only one suit.";

    pub const SECTION_MELD_SHAPES: &str = "Meld shapes";

    pub const VALID_SEQUENCE: &str = "Valid sequence";
    pub const INVALID_SEQUENCE: &str = "Invalid sequence";

    pub const SECTION_FLOWER_WILDCARDS: &str = "Flower wildcards";

    pub const VALID_FLOWER_MELD: &str = "Valid flower meld";
    pub const INVALID_FLOWER_MELD: &str = "Invalid flower meld";

    pub const VALID_FLOWER_CAPTION: &str = "7 · 7 · Flower";
    pub const INVALID_FLOWER_CAPTION: &str = "4 Manzu / Flower / Flower";

    pub const STRUCTURE_BRIDGE: &str =
        "Played melds sit in your structure until you Cash In.";
}

// ── Scoring (Part 2 subtitle and cross-refs) ────────────────────────────────

pub mod scoring {
    pub const SUBTITLE: &str = "Select melds, Play them, then Cash In to score.";

    pub const FLOW_REMINDER: &str = "Played melds don't score until cashed in. You can cash in **whenever** you have a structure.";
}

// ── Campaign pages ────────────────────────────────────────────────────────────

pub mod campaign {
    use crate::ui::input::InputMode;

    pub const PAGE_TILES_INTRO_TITLE: &str = "The Tiles";
    pub const PAGE_TILES_HONORS_TITLE: &str = "Honors & Flowers";
    pub const PAGE_MELDS_TITLE: &str = super::melds::PAGE_TITLE;
    pub const PAGE_SCORING_TITLE: &str = "Scoring";
    pub const PAGE_TRY_IT_TITLE: &str = "Try It";

    const PAGE_NAV_CALLOUT_CURSOR: &str =
        "When you're ready to continue, click **Next** in the upper right.";
    const PAGE_NAV_CALLOUT_PRESS: &str =
        "When you're ready to continue, press **Next** in the upper right.";
    const PAGE_START_CALLOUT_CURSOR: &str =
        "When you're ready, click **Start** in the upper right to begin your first lesson.";
    const PAGE_START_CALLOUT_PRESS: &str =
        "When you're ready, press **Start** in the upper right to begin your first lesson.";

    pub fn page_nav_callout(input_mode: InputMode) -> &'static str {
        match input_mode {
            InputMode::Cursor => PAGE_NAV_CALLOUT_CURSOR,
            InputMode::Keyboard | InputMode::Controller => PAGE_NAV_CALLOUT_PRESS,
        }
    }

    pub fn page_start_callout(input_mode: InputMode) -> &'static str {
        match input_mode {
            InputMode::Cursor => PAGE_START_CALLOUT_CURSOR,
            InputMode::Keyboard | InputMode::Controller => PAGE_START_CALLOUT_PRESS,
        }
    }

    pub const GLOSSARY_DISCARD: &str =
        "{{term:Discard}} — remove unwanted tiles from your hand. Discards are a limited resource.";
    pub const GLOSSARY_PLAY: &str =
        "{{term:Play}} — send melds to your structure. Plays are a limited resource.";
    pub const GLOSSARY_STRUCTURE: &str =
        "{{term:Structure}} — played melds waiting to score. Slots are limited but rearrangeable.";
    pub const GLOSSARY_CASH_IN: &str =
        "{{term:Cash In}} — scores everything in your structure, then resets it.";

    pub const SCORING_GLOSSARY: &[&str] = &[
        GLOSSARY_DISCARD,
        GLOSSARY_PLAY,
        GLOSSARY_STRUCTURE,
        GLOSSARY_CASH_IN,
    ];
}

// ── Try-it demo (Part 2 interactive flash lines) ────────────────────────────

pub mod try_it {
    pub const SUBTITLE: &str =
        "Tap each prop below to preview **Discard**, **Play**, and **Cash In**.";

    pub const HEADING: &str = "The three actions";

    pub const LABEL_DISCARD: &str = "Discard";
    pub const LABEL_PLAY: &str = "Play";
    pub const LABEL_CASH_IN: &str = "Cash In";

    pub const PLAY: &str = "You **Play**ed a meld to your structure.";
    pub const DISCARD: &str = "Discarded tiles are removed to the river.";
    pub const CASH_IN: &str = "Demo: 4 **Fu** × 3 **Han** = 12";

    /// Every flash line; used to reserve callout height for the tallest variant.
    pub const FLASH_LINES: &[&str] = &[CASH_IN, PLAY, DISCARD];
}

// ── Onboarding lessons (gameplay blind banners) ───────────────────────────────

pub mod lessons {
    pub const SELECT: &str = "Select melds from your hand.";
    pub const PLAY: &str = "Press **Play** to move your meld to Structure.";
    pub const CASH_IN: &str = "Press **Cash In** to score your structure.";
    pub const DISCARD: &str = "Select a tile you don't need, then **Discard**.";
    pub const DISCARD_RETRY: &str = "Try a **Discard** to improve your hand.";
    pub const SECOND_SCORE: &str = "Play another meld, then **Cash In** again to reach the target.";
    pub const FALLBACK_PLAY: &str = "Press **Play** to move your meld to Structure.";
    pub const FALLBACK_CASH_IN: &str = "Press **Cash In** when your structure is ready to score.";
    pub const FALLBACK_TARGET: &str = "Reach the target score before you run out of plays.";
    pub const RIVER_TIP: &str = "Discarded tiles sit in the river — they don't score.";

    pub const FAILURE_ZERO: &str =
        "You scored 0 — select melds, **Play** to Structure, then **Cash In**.";
    pub const FINALE_FAILURE_ZERO: &str = "You scored 0 — select melds, **Play** to Structure, then **Cash In**. If you run out of plays first, the round ends.";

    pub const FINALE_INTRO: &str = "You must now prepare to undergo an ordeal — The Iconoclast\n\n\
         The Iconoclast changes the rules of the game, debuffing Wind and Dragon tiles. \
         Debuffed tiles still form melds and yaku, but contribute +0 Fu when you Cash In. \
         The blue **Guide** book on the table contains a refresher of mechanics.";
}

// ── Tutorial summary screen ─────────────────────────────────────────────────

pub mod summary {
    use super::scoring;

    pub const TITLE_WON: &str = "Tutorial Complete";
    pub const TITLE_LOST: &str = "Tutorial Recap";

    pub const SUBTITLE_WON: &str = "You surpassed The Iconoclast and won, an auspicious beginning.";
    pub const SUBTITLE_LOST: &str = "You reached the finale but faltered against The Iconoclast. Perhaps you'll fare better next time.";

    pub const BULLET_SCORING: &str =
        "Select melds, **Play** to Structure, then **Cash In** to score (**Fu** × **Han**).";
    pub const BULLET_DISCARD: &str =
        "**Discard** tiles you don't need to draw replacements and build a stronger structure.";
    pub const BULLET_GUIDE: &str =
        "Open the **Guide** book on the table for tiles, melds, yaku, and scoring.";
    pub const BULLET_YAKU: &str = "Shousangen and Chiitoitsu are good yaku to learn first.";
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
