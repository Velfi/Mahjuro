use serde::{Deserialize, Serialize};

use crate::core::ordeal::OrdealKind;
use crate::core::scoring::ScoreBreakdown;
use crate::core::yaku::YakuKind;
use crate::game::engine::GameEngine;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnboardingPhase {
    Lessons,
    Shop,
    Finale,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnboardingState {
    pub phase: OnboardingPhase,
    /// Step index for the guided Lessons blind (0–4).
    pub step: u32,
    /// Shown the discard-river tooltip after the player's first discard.
    pub discard_river_tooltip_shown: bool,
    /// True after the player has cashed in at least once this Lessons blind.
    pub scored_once: bool,
    /// One-shot boss debrief banner at the start of the Finale blind.
    pub finale_intro_shown: bool,
}

impl Default for OnboardingState {
    fn default() -> Self {
        Self::new()
    }
}

impl OnboardingState {
    pub fn new() -> Self {
        Self {
            phase: OnboardingPhase::Lessons,
            step: 0,
            discard_river_tooltip_shown: false,
            scored_once: false,
            finale_intro_shown: false,
        }
    }

    pub fn lessons_active(&self) -> bool {
        self.phase == OnboardingPhase::Lessons
    }

    pub fn discard_allowed_in_lessons(&self) -> bool {
        self.step >= 3
    }

    pub fn lessons_prompt(&self, run: &crate::game::run::RunState) -> &'static str {
        let gameplay = GameEngine::read(run);
        let has_selection = gameplay.selected_count > 0;
        let has_structure = gameplay.has_structure;

        match self.step {
            0 if !has_selection => "Select tiles that form a valid meld.",
            0 | 1 if has_selection && !has_structure => {
                "Press Play to bank your meld into the structure."
            }
            2 if has_structure => "Press Cash In to score your structure.",
            3 if !self.discard_river_tooltip_shown => {
                "Swap a tile you don't need — select it, then Discard."
            }
            3 => "Try a discard to improve your hand.",
            4 => "Bank another meld, then Cash In again to reach the target.",
            _ if !has_selection => "Select tiles to form a valid meld.",
            _ if has_selection && !has_structure => "Press Play to bank your meld.",
            _ if has_structure => "Press Cash In when you're ready to score.",
            _ => "Reach the target score before you run out of plays.",
        }
    }
}

pub const TUTORIAL_ORDEAL: OrdealKind = OrdealKind::Relic;

pub const LESSONS_TARGET: u32 = 100;
pub const LESSONS_HAND_SIZE: usize = 10;
pub const LESSONS_PLAYS: u32 = 3;
pub const LESSONS_DISCARDS: u32 = 2;

pub fn tutorial_yaku() -> Vec<YakuKind> {
    vec![
        YakuKind::ChickenHand,
        YakuKind::FullHand,
        YakuKind::Chiitoitsu,
    ]
}

/// Hint text after failing the Lessons blind.
pub fn lessons_failure_feedback(round_score: u64, target: u32, plays_remaining: u32) -> String {
    if round_score == 0 {
        return "You scored 0 — select valid tiles, press Play to bank them, then Cash In."
            .to_string();
    }
    if plays_remaining > 0 {
        return format!(
            "You scored {} / {}. You still had {} play{} left — bank another meld and Cash In again.",
            round_score,
            target,
            plays_remaining,
            if plays_remaining == 1 { "" } else { "s" },
        );
    }
    format!(
        "You scored {} / {} — {} short. Try discarding a useless tile, then bank another meld before you Cash In.",
        round_score,
        target,
        target.saturating_sub(round_score.min(u32::MAX as u64) as u32),
    )
}

/// Hint text after failing the onboarding boss (The Iconoclast — honors debuffed).
pub fn finale_failure_feedback(
    round_score: u64,
    target: u32,
    discards_left: u32,
    last_breakdown: Option<&ScoreBreakdown>,
) -> String {
    let gap = target.saturating_sub(round_score.min(u32::MAX as u64) as u32);
    let score_pct = if target > 0 {
        (round_score as f64 / target as f64 * 100.0).min(100.0) as u32
    } else {
        100
    };

    let chicken_only = last_breakdown.is_some_and(|b| {
        b.detected_yaku.contains(&YakuKind::ChickenHand)
            && !b.detected_yaku.contains(&YakuKind::FullHand)
            && !b.detected_yaku.contains(&YakuKind::Chiitoitsu)
    });

    if round_score == 0 {
        return "You scored 0 — bank valid melds with Play, then press Cash In. If you run out of plays first, the round ends.".to_string();
    }

    if chicken_only {
        return format!(
            "Your last cash-in was a Chicken Hand — legal, but the lowest-scoring yaku (no mult or chip bonus). Full Hand and Chiitoitsu raise mult much more on this shrine. You were {} / {} ({}%).",
            round_score, target, score_pct,
        );
    }

    if discards_left >= 2 {
        return format!(
            "You scored {} / {} ({}%) but had {} discards left. The Iconoclast weakens honors — favor triplets and runs in souzu, pinzu, or manzu, then cash in with a yaku.",
            round_score, target, score_pct, discards_left,
        );
    }

    if score_pct >= 75 {
        return format!(
            "Close — {} / {} ({}%). Honors are debuffed on this shrine; lean on the three suits. Bigger melds and a yaku raise mult more than pairs alone.",
            round_score, target, score_pct,
        );
    }

    format!(
        "You scored {} / {} — {} short of the target. The Iconoclast debuffs winds and dragons; build melds in souzu, pinzu, and manzu, bank with Play, then Cash In. Chips × mult: Full Hand or Chiitoitsu is your main mult lever here.",
        round_score, target, gap,
    )
}

pub fn finale_intro_message() -> &'static str {
    "You must now prepare to undergo an ordeal — The Iconoclast\n\n\
     The Iconoclast changes the rules of the game, debuffing Wind and Dragon tiles. \
     They still form melds, but they score for nothing. \
     The bigger the structure, the bigger the score. Open the Guide book on the table to learn more."
}
