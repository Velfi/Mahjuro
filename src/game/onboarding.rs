use serde::{Deserialize, Serialize};

use crate::core::ordeal::OrdealKind;
use crate::core::yaku::YakuKind;
use crate::game::engine::GameEngine;
use crate::ui::score_format::format_score;

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
    /// Shown in the Lessons banner after a rejected Play attempt.
    #[serde(default)]
    pub invalid_meld_hint: Option<String>,
    /// Selected tile ids tied to [`Self::invalid_meld_hint`]; cleared when selection changes.
    #[serde(default)]
    invalid_meld_hint_tile_ids: Vec<u32>,
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
            invalid_meld_hint: None,
            invalid_meld_hint_tile_ids: Vec::new(),
        }
    }

    pub fn clear_invalid_meld_hint(&mut self) {
        self.invalid_meld_hint = None;
        self.invalid_meld_hint_tile_ids.clear();
    }

    pub fn set_invalid_meld_hint(&mut self, hint: String, tile_ids: Vec<u32>) {
        self.invalid_meld_hint = Some(hint);
        self.invalid_meld_hint_tile_ids = tile_ids;
    }

    pub fn sync_invalid_meld_hint(&mut self, selected_tile_ids: &[u32]) {
        if self.invalid_meld_hint.is_none() {
            return;
        }
        let mut current = selected_tile_ids.to_vec();
        current.sort_unstable();
        let mut stored = self.invalid_meld_hint_tile_ids.clone();
        stored.sort_unstable();
        if current != stored {
            self.clear_invalid_meld_hint();
        }
    }

    pub fn lessons_active(&self) -> bool {
        self.phase == OnboardingPhase::Lessons
    }

    pub fn discard_allowed_in_lessons(&self) -> bool {
        self.step >= 3
    }

    pub fn lessons_prompt<'a>(&'a self, run: &crate::game::run::RunState) -> &'a str {
        if let Some(ref hint) = self.invalid_meld_hint {
            return hint;
        }

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
            4 => "Play another meld, then Cash In again to reach the target.",
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
            format_score(round_score),
            format_score(target as u64),
            plays_remaining,
            if plays_remaining == 1 { "" } else { "s" },
        );
    }
    let gap = target.saturating_sub(round_score.min(u32::MAX as u64) as u32);
    format!(
        "You scored {} / {} — {} short. Try discarding a useless tile, then bank another meld before you Cash In.",
        format_score(round_score),
        format_score(target as u64),
        format_score(gap as u64),
    )
}

/// Hint text after failing the onboarding boss (The Iconoclast — honors debuffed).
pub fn finale_failure_feedback(
    round_score: u64,
    target: u32,
    discards_left: u32,
) -> String {
    let gap = target.saturating_sub(round_score.min(u32::MAX as u64) as u32);
    let score_pct = if target > 0 {
        (round_score as f64 / target as f64 * 100.0).min(100.0) as u32
    } else {
        100
    };

    if round_score == 0 {
        return "You scored 0 — bank valid melds with Play, then press Cash In. If you run out of plays first, the round ends.".to_string();
    }

    if discards_left >= 2 {
        return format!(
            "You scored {} / {} ({}%) but had {} discards left. Honors are debuffed during this ordeal — discard them in favor of Souzu, Pinzu, or Manzu tiles.",
            format_score(round_score),
            format_score(target as u64),
            score_pct,
            discards_left,
        );
    }

    if score_pct >= 75 {
        return format!(
            "Close — {} / {} ({}%). Honors are debuffed during this ordeal; lean on Souzu, Pinzu, and Manzu tiles. A bigger structure pays out more points.",
            format_score(round_score),
            format_score(target as u64),
            score_pct,
        );
    }

    format!(
        "You scored {} / {} — {} short of the target. Honors are debuffed during this ordeal; lean on Souzu, Pinzu, and Manzu tiles. A bigger structure pays out more points.",
        format_score(round_score),
        format_score(target as u64),
        format_score(gap as u64),
    )
}

pub fn finale_intro_message() -> &'static str {
    "You must now prepare to undergo an ordeal — The Iconoclast\n\n\
     The Iconoclast changes the rules of the game, debuffing Wind and Dragon tiles. \
     Debuffed tiles still form melds and yaku, but score for nothing. \
     The blue guide book on the table contains a refresher of mechanics."
}
