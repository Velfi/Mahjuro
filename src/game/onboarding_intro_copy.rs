//! Guided onboarding copy — wording aligned with Guide intro modules.

use crate::scenes::scoring_intro_copy;

// ── Lessons blind banner ────────────────────────────────────────────────────

pub const LESSONS_SELECT: &str = "Select melds from your hand.";
pub const LESSONS_PLAY: &str = "Press **Play** to move your meld to Structure.";
pub const LESSONS_CASH_IN: &str = "Press **Cash In** to score your structure.";
pub const LESSONS_DISCARD: &str = "Select a tile you don't need, then **Discard**.";
pub const LESSONS_DISCARD_RETRY: &str = "Try a **Discard** to improve your hand.";
pub const LESSONS_SECOND_SCORE: &str =
    "Play another meld, then **Cash In** again to reach the target.";
pub const LESSONS_FALLBACK_PLAY: &str = "Press **Play** to move your meld to Structure.";
pub const LESSONS_FALLBACK_CASH_IN: &str =
    "Press **Cash In** when your structure is ready to score.";
pub const LESSONS_FALLBACK_TARGET: &str = "Reach the target score before you run out of plays.";
pub const LESSONS_RIVER_TIP: &str = "Discarded tiles sit in the river — they don't score.";

// ── Lessons / finale failure feedback ───────────────────────────────────────

pub const LESSONS_FAILURE_ZERO: &str =
    "You scored 0 — select melds, **Play** to Structure, then **Cash In**.";
pub const FINALE_FAILURE_ZERO: &str = "You scored 0 — select melds, **Play** to Structure, then **Cash In**. If you run out of plays first, the round ends.";

// ── Finale intro ──────────────────────────────────────────────────────────────

pub const FINALE_INTRO: &str = "You must now prepare to undergo an ordeal — The Iconoclast\n\n\
     The Iconoclast changes the rules of the game, debuffing Wind and Dragon tiles. \
     Debuffed tiles still form melds and yaku, but contribute +0 chips when you Cash In. \
     The blue **Guide** book on the table contains a refresher of mechanics.";

// ── Tutorial summary bullets ────────────────────────────────────────────────

pub const SUMMARY_BULLET_SCORING: &str =
    "Select melds, **Play** to Structure, then **Cash In** to score (**chips** × **mult**).";
pub const SUMMARY_BULLET_DISCARD: &str =
    "**Discard** tiles you don't need to draw replacements and build a stronger structure.";
pub const SUMMARY_BULLET_GUIDE: &str =
    "Open the **Guide** book on the table for tiles, melds, yaku, and scoring.";
pub const SUMMARY_BULLET_YAKU: &str = "Full Hand and Chiitoitsu are good yaku to learn first.";
pub const SUMMARY_BULLET_PROGRESS: &str =
    "The more you play, the more the house will reveal to you. How far will you go?";

pub const SUMMARY_BULLETS: &[&str] = &[
    SUMMARY_BULLET_SCORING,
    scoring_intro_copy::FLOW_REMINDER,
    SUMMARY_BULLET_DISCARD,
    SUMMARY_BULLET_GUIDE,
    SUMMARY_BULLET_YAKU,
    SUMMARY_BULLET_PROGRESS,
];
