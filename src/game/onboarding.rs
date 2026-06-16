use serde::{Deserialize, Serialize};

use crate::core::ordeal::OrdealKind;
use crate::core::yaku::YakuKind;
use crate::game::engine::GameEngine;
use crate::scenes::tutorial_intro_copy::lessons as copy;
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
    /// Number of successful hold-to-confirm actions demonstrated by the player.
    #[serde(default)]
    hold_action_successes: u8,
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
            hold_action_successes: 0,
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

    pub fn hold_tooltip_enabled(&self) -> bool {
        self.hold_action_successes < HOLD_TOOLTIP_SUCCESS_TARGET
    }

    pub fn notify_hold_success(&mut self) {
        if self.hold_action_successes < HOLD_TOOLTIP_SUCCESS_TARGET {
            self.hold_action_successes += 1;
        }
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
            0 if !has_selection => copy::SELECT,
            0 | 1 if has_selection && !has_structure => copy::PLAY,
            2 if has_structure => copy::CASH_IN,
            3 if !self.discard_river_tooltip_shown => copy::DISCARD,
            3 => copy::DISCARD_RETRY,
            4 => copy::SECOND_SCORE,
            _ if !has_selection => copy::SELECT,
            _ if has_selection && !has_structure => copy::FALLBACK_PLAY,
            _ if has_structure => copy::FALLBACK_CASH_IN,
            _ => copy::FALLBACK_TARGET,
        }
    }
}

pub const TUTORIAL_ORDEAL: OrdealKind = OrdealKind::Relic;

pub const LESSONS_TARGET: u32 = 100;
pub const LESSONS_PLAYS: u32 = 3;
pub const LESSONS_DISCARDS: u32 = 2;
pub const HOLD_TOOLTIP_SUCCESS_TARGET: u8 = 3;
pub const HOLD_TOOLTIP_COPY: &str = "Try holding the button";

pub fn tutorial_yaku() -> Vec<YakuKind> {
    YakuKind::all().to_vec()
}

/// Hint text after failing the Lessons blind.
pub fn lessons_failure_feedback(round_score: u64, target: u32, plays_remaining: u32) -> String {
    if round_score == 0 {
        return copy::FAILURE_ZERO.to_string();
    }
    if plays_remaining > 0 {
        return format!(
            "You scored {} / {}. You still had {} play{} left — play another meld, then **Cash In** again.",
            format_score(round_score),
            format_score(target as u64),
            plays_remaining,
            if plays_remaining == 1 { "" } else { "s" },
        );
    }
    let gap = target.saturating_sub(round_score.min(u32::MAX as u64) as u32);
    format!(
        "You scored {} / {} — {} short. **Discard** a useless tile, play another meld to Structure, then **Cash In**.",
        format_score(round_score),
        format_score(target as u64),
        format_score(gap as u64),
    )
}

/// Hint text after failing the onboarding boss (The Iconoclast — honors debuffed).
pub fn finale_failure_feedback(round_score: u64, target: u32, discards_left: u32) -> String {
    let gap = target.saturating_sub(round_score.min(u32::MAX as u64) as u32);
    let score_pct = if target > 0 {
        (round_score as f64 / target as f64 * 100.0).min(100.0) as u32
    } else {
        100
    };

    if round_score == 0 {
        return copy::FINALE_FAILURE_ZERO.to_string();
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
    copy::FINALE_INTRO
}
