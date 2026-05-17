use serde::{Deserialize, Serialize};

use crate::core::boss::BossKind;
use crate::core::hand::MeldKind;
use crate::core::scoring::ScoreBreakdown;
use crate::core::tile::Tile;
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
            0 if !has_selection => "Select two matching tiles.",
            0 | 1 if has_selection && !has_structure => "Press Play to bank your pair into the structure.",
            2 if has_structure => "Press Cash In to score your structure.",
            3 if !self.discard_river_tooltip_shown => {
                "Swap a tile you don't need — select it, then Discard."
            }
            3 => "Try a discard to improve your hand.",
            4 => "Build another meld, then Cash In again to reach the target.",
            _ if !has_selection => "Select matching tiles to form a meld.",
            _ if has_selection && !has_structure => "Press Play to bank your meld.",
            _ if has_structure => "Press Cash In when you're ready to score.",
            _ => "Reach the target score before you run out of plays.",
        }
    }
}

pub const TUTORIAL_BOSS: BossKind = BossKind::Relic;

pub const LESSONS_TARGET: u32 = 100;
pub const LESSONS_HAND_SIZE: usize = 10;
pub const LESSONS_PLAYS: u32 = 3;
pub const LESSONS_DISCARDS: u32 = 2;

pub fn tutorial_yaku() -> Vec<YakuKind> {
    vec![YakuKind::FullHand, YakuKind::Chiitoitsu]
}

/// Tile indices that could extend the current selection into a pair (Lessons blind).
pub fn lessons_affinity_indices(hand: &[Tile], selected: &[bool]) -> Vec<usize> {
    let sel_tiles: Vec<&Tile> = hand
        .iter()
        .zip(selected.iter())
        .filter(|(_, s)| **s)
        .map(|(t, _)| t)
        .collect();

    if sel_tiles.is_empty() {
        return Vec::new();
    }

    let mut affinity = Vec::new();
    for (i, tile) in hand.iter().enumerate() {
        if selected[i] {
            continue;
        }
        if sel_tiles
            .iter()
            .any(|s| s.suit == tile.suit && s.rank == tile.rank)
        {
            affinity.push(i);
        }
    }
    affinity
}

/// Hint text after failing the Lessons blind.
pub fn lessons_failure_feedback(
    round_score: u64,
    target: u32,
    plays_remaining: u32,
) -> String {
    if round_score == 0 {
        return "You scored 0 — select matching tiles, press Play to bank them, then Cash In."
            .to_string();
    }
    if plays_remaining > 0 {
        return format!(
            "You scored {} / {}. You still had {} play{} left — bank another pair and Cash In again.",
            round_score,
            target,
            plays_remaining,
            if plays_remaining == 1 { "" } else { "s" },
        );
    }
    format!(
        "You scored {} / {} — {} short. Try discarding a useless tile, then bank bigger melds before you Cash In.",
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
            "Your last cash-in was a Chicken Hand (no pattern bonus). This run only awards Full Hand and Chiitoitsu — build one of those shapes so mult is not stuck near zero. You were {} / {} ({}%).",
            round_score, target, score_pct,
        );
    }

    if discards_left >= 2 {
        return format!(
            "You scored {} / {} ({}%) but had {} discards left. The Iconoclast weakens honors — favor triplets and runs in bamboo, dots, or characters, then cash in with a yaku.",
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
        "You scored {} / {} — {} short of the target. The Iconoclast debuffs winds and dragons; build melds in bamboo, dots, and characters, bank with Play, then Cash In. Chips × mult: Full Hand or Chiitoitsu is your main mult lever here.",
        round_score, target, gap,
    )
}

pub fn finale_intro_message() -> &'static str {
    "Boss shrine — The Iconoclast\n\n\
     Winds and dragons are debuffed: they still form melds, but score much less. \
     Build in Bamboos, Dots, and Characters. Full Hand and Chiitoitsu (seven pairs) \
     are your best yaku here. You can retry if you miss the target."
}

/// Meld kinds allowed during the Lessons blind (pairs only keeps the loop simple).
pub fn lessons_allowed_melds() -> &'static [MeldKind] {
    &[MeldKind::Pair]
}
