//! Tutorial state and lesson definitions for the new-player onboarding flow.
//!
//! The tutorial runs during the player's very first run (`runs_completed == 0`).
//! It introduces mechanics one lesson at a time across 9 blinds (3 antes),
//! then hands off to the normal progression system.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::core::hand::SetKind;
use crate::core::yaku::YakuKind;

// ── Tutorial milestones (first-time celebrations) ─────────────────────

/// Milestones that trigger a fireworks celebration modal on first occurrence.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub enum TutorialMilestone {
    FirstPair,
    FirstTriplet,
    FirstSequence,
    FirstDiscard,
    FirstFullHand,
    FirstShopBuy,
}

// ── Tutorial state (serialized inside RunState) ───────────────────────

/// Per-run tutorial tracking. `None` on normal (non-tutorial) runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TutorialState {
    /// Current lesson (1-indexed). Lessons advance when a blind is beaten.
    pub current_lesson: u32,
    /// Lessons whose blind has been beaten.
    pub completed_lessons: HashSet<u32>,
    /// Whether the player dismissed the current hint banner.
    pub hint_dismissed: bool,
    /// Sub-step within the current lesson (drives contextual prompts).
    pub sub_step: u32,
    /// Set once lesson 9 completes — no more tutorial UI after this.
    pub finished: bool,
    /// Consecutive failures on the current blind (drives adaptive difficulty).
    pub retry_count: u32,
    /// Milestones already celebrated (prevents repeat fireworks).
    pub celebrated: HashSet<TutorialMilestone>,
    /// Whether the annotated (slow-mo) cascade has been shown this lesson.
    pub cascade_annotated: bool,
}

impl Default for TutorialState {
    fn default() -> Self {
        Self::new(1)
    }
}

impl TutorialState {
    pub fn new(starting_lesson: u32) -> Self {
        Self {
            current_lesson: starting_lesson,
            completed_lessons: HashSet::new(),
            hint_dismissed: false,
            sub_step: 0,
            finished: false,
            retry_count: 0,
            celebrated: HashSet::new(),
            cascade_annotated: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.current_lesson > 0 && !self.finished
    }

    /// Get the current lesson definition.
    pub fn current_lesson_def(&self) -> &'static LessonDef {
        lesson_def(self.current_lesson)
    }

    /// Advance to the next lesson. Returns the new lesson number,
    /// or `None` if the tutorial is now finished.
    pub fn advance(&mut self) -> Option<u32> {
        self.completed_lessons.insert(self.current_lesson);
        if self.current_lesson >= LESSON_COUNT as u32 {
            self.finished = true;
            return None;
        }
        self.current_lesson += 1;
        self.hint_dismissed = false;
        self.sub_step = 0;
        self.retry_count = 0;
        self.cascade_annotated = false;
        Some(self.current_lesson)
    }

    /// Check if a milestone should be celebrated and mark it.
    /// Returns `true` if this is the first time (should show fireworks).
    pub fn celebrate(&mut self, milestone: TutorialMilestone) -> bool {
        self.celebrated.insert(milestone)
    }

    /// Record a blind failure for adaptive difficulty. After repeated
    /// failures on the same lesson, `retry_target_factor` lowers the
    /// effective target so new players don't get permanently stuck.
    pub fn record_failure(&mut self) {
        self.retry_count = self.retry_count.saturating_add(1);
    }

    /// Multiplicative factor applied to the blind's target score to ease
    /// difficulty after repeated failures. Each retry reduces the target
    /// by 20% (compounding), clamped at 0.6× so the blind never becomes
    /// completely trivial.
    pub fn retry_target_factor(&self) -> f32 {
        (0.8_f32.powi(self.retry_count as i32)).max(0.6)
    }
}

// ── Lesson definitions ────────────────────────────────────────────────

pub const LESSON_COUNT: usize = 9;

pub struct LessonDef {
    pub id: u32,
    /// Italic flavor line shown above the hint.
    pub flavor_text: &'static str,
    /// Primary instructional hint shown at lesson start.
    pub intro_text: &'static str,
    /// Contextual sub-step prompts (indexed by `sub_step`).
    pub step_prompts: &'static [&'static str],
    /// Which meld types are accepted when scoring this lesson.
    pub allowed_sets: &'static [SetKind],
    /// Whether the discard action is available.
    pub discard_enabled: bool,
    /// Whether the shop scene appears after beating this blind.
    pub shop_enabled: bool,
    /// Hand size override (`None` = use mode default of 14).
    pub hand_size: Option<usize>,
    /// Target score override (`None` = use normal scaling).
    pub target_override: Option<u32>,
    /// Yaku that contribute to scoring this lesson.
    pub allowed_yaku: &'static [YakuKind],
    /// Whether the boss blind is replaced with a simplified version.
    pub simplified_boss: bool,
    /// Whether unselected tiles glow when they could form a valid meld
    /// with the current selection.
    pub affinity_glow: bool,
    /// Whether the first scoring cascade this lesson runs in annotated
    /// slow-motion (lesson 5).
    pub annotated_cascade: bool,
}

/// Look up a lesson by 1-based ID. Clamps to the valid range so corrupted
/// save data doesn't panic — an out-of-range id silently resolves to the
/// last lesson (graduation) rather than crashing.
pub fn lesson_def(id: u32) -> &'static LessonDef {
    let idx = (id.saturating_sub(1) as usize).min(LESSON_COUNT - 1);
    &LESSONS[idx]
}

static LESSONS: [LessonDef; LESSON_COUNT] = [
    // ── Lesson 1: Tiles & Pairs ───────────────────────────────────
    //
    // step_prompts indices (used by TutorialOverlay):
    //   [0] no selection  [1] has selection
    LessonDef {
        id: 1,
        flavor_text: "Every journey begins with a matching pair.",
        intro_text: "Select two matching tiles, then press Play to score them.",
        step_prompts: &[
            "Select two matching tiles.",
            "Press Play to score your pair!",
        ],
        allowed_sets: &[SetKind::Pair],
        discard_enabled: false,
        shop_enabled: false,
        hand_size: Some(8),
        target_override: Some(50),
        allowed_yaku: &[],
        simplified_boss: false,
        affinity_glow: true,
        annotated_cascade: false,
    },
    // ── Lesson 2: Triplets ────────────────────────────────────────
    //   [0] no selection  [1] has selection
    LessonDef {
        id: 2,
        flavor_text: "Two is company. Three is power.",
        intro_text: "Three matching tiles form a Triplet \u{2014} worth more than a pair!",
        step_prompts: &[
            "Find three identical tiles for a triplet.",
            "Press Play to score!",
        ],
        allowed_sets: &[SetKind::Pair, SetKind::Triplet, SetKind::Kong],
        discard_enabled: false,
        shop_enabled: false,
        hand_size: Some(10),
        target_override: Some(120),
        allowed_yaku: &[],
        simplified_boss: false,
        affinity_glow: true,
        annotated_cascade: false,
    },
    // ── Lesson 3: Sequences ───────────────────────────────────────
    //   [0] no selection  [1] has selection
    //   [2] after scoring — Meld Guide hint
    LessonDef {
        id: 3,
        flavor_text: "The river flows in order.",
        intro_text: "Three consecutive tiles in the same suit form a Sequence.",
        step_prompts: &[
            "Look for three tiles in a row \u{2014} like 2, 3, 4 of the same suit.",
            "Press Play to score your sequence!",
            "Tip: open the Pause menu to find the Meld Guide \u{2014} it shows every pattern!",
        ],
        allowed_sets: &[
            SetKind::Pair,
            SetKind::Triplet,
            SetKind::Kong,
            SetKind::Sequence,
        ],
        discard_enabled: false,
        shop_enabled: false,
        hand_size: Some(12),
        target_override: Some(200),
        allowed_yaku: &[],
        simplified_boss: true,
        affinity_glow: true,
        annotated_cascade: false,
    },
    // ── Lesson 4: Discarding ──────────────────────────────────────
    //   [0] before first score, no selection — prompt discard
    //   [1] has selection + discards available — press Discard
    //   [2] fallback — build melds and play
    LessonDef {
        id: 4,
        flavor_text: "Let go of what doesn't serve you.",
        intro_text: "Discard tiles you don't need to draw better ones from the wall.",
        step_prompts: &[
            "Select tiles that don\u{2019}t fit any meld, then press Discard.",
            "Press Discard to swap them for new tiles!",
            "Build melds and press Play to score!",
        ],
        allowed_sets: &[
            SetKind::Pair,
            SetKind::Triplet,
            SetKind::Kong,
            SetKind::Sequence,
        ],
        discard_enabled: true,
        shop_enabled: false,
        hand_size: None, // full 14
        target_override: Some(250),
        allowed_yaku: &[],
        simplified_boss: false,
        affinity_glow: false,
        annotated_cascade: false,
    },
    // ── Lesson 5: Chips × Mult ────────────────────────────────────
    //   [0] before cascade — prompt play
    //   [1] during cascade — watch breakdown
    LessonDef {
        id: 5,
        flavor_text: "The secret of every master: multiply your fortune.",
        intro_text: "Watch the scoring breakdown \u{2014} your chips are multiplied for the final score!",
        step_prompts: &[
            "Play a hand and watch the cascade closely.",
            "Watch the scoring breakdown \u{2014} Chips \u{00d7} Mult!",
        ],
        allowed_sets: &[
            SetKind::Pair,
            SetKind::Triplet,
            SetKind::Kong,
            SetKind::Sequence,
        ],
        discard_enabled: true,
        shop_enabled: false,
        hand_size: None,
        target_override: Some(350),
        allowed_yaku: &[],
        simplified_boss: false,
        affinity_glow: false,
        annotated_cascade: true,
    },
    // ── Lesson 6: FullHand & Yaku ─────────────────────────────────
    //   [0] no selection  [1] has selection
    //   [2] after scoring — yaku count hint
    LessonDef {
        id: 6,
        flavor_text: "A complete hand opens every door.",
        intro_text: "Build 4 melds + 1 pair (14 tiles) for the FullHand yaku \u{2014} a big multiplier bonus!",
        step_prompts: &[
            "Try to use all 14 tiles: 4 melds and 1 pair for FullHand!",
            "FullHand gives +5 mult \u{2014} a massive boost! Press Play.",
            "FullHand is just one yaku \u{2014} check the Meld Guide (Pause menu) for all 13!",
        ],
        allowed_sets: &[
            SetKind::Pair,
            SetKind::Triplet,
            SetKind::Kong,
            SetKind::Sequence,
        ],
        discard_enabled: true,
        shop_enabled: false,
        hand_size: None,
        target_override: Some(500),
        allowed_yaku: &[YakuKind::FullHand],
        simplified_boss: false,
        affinity_glow: false,
        annotated_cascade: false,
    },
    // ── Lesson 7: Honors & Yakuhai ────────────────────────────────
    //   [0] no selection  [1] has selection
    LessonDef {
        id: 7,
        flavor_text: "Wind and dragon bow to no suit.",
        intro_text: "Honor tiles (Winds & Dragons) can form triplets. A dragon or wind triplet triggers the Yakuhai yaku!",
        step_prompts: &[
            "Look for Wind or Dragon tiles \u{2014} a triplet triggers Yakuhai!",
            "Press Play to score!",
        ],
        allowed_sets: &[
            SetKind::Pair,
            SetKind::Triplet,
            SetKind::Kong,
            SetKind::Sequence,
        ],
        discard_enabled: true,
        shop_enabled: false,
        hand_size: None,
        target_override: Some(600),
        allowed_yaku: &[YakuKind::FullHand, YakuKind::Yakuhai],
        simplified_boss: false,
        affinity_glow: false,
        annotated_cascade: false,
    },
    // ── Lesson 8: The Shop ────────────────────────────────────────
    //   [0] no selection (gameplay)  [1] has selection (gameplay)
    LessonDef {
        id: 8,
        flavor_text: "Gold well spent returns tenfold.",
        intro_text: "Beat this blind to reach the Shop! You\u{2019}ll spend gold on Relics that boost your scoring.",
        step_prompts: &[
            "Beat this blind to reach the Shop \u{2014} build your best melds!",
            "Press Play to score and earn gold for the Shop!",
        ],
        allowed_sets: &[
            SetKind::Pair,
            SetKind::Triplet,
            SetKind::Kong,
            SetKind::Sequence,
        ],
        discard_enabled: true,
        shop_enabled: true,
        hand_size: None,
        target_override: Some(700),
        allowed_yaku: &[YakuKind::FullHand, YakuKind::Yakuhai],
        simplified_boss: false,
        affinity_glow: false,
        annotated_cascade: false,
    },
    // ── Lesson 9: Graduation ──────────────────────────────────────
    LessonDef {
        id: 9,
        flavor_text: "The apprentice becomes the player.",
        intro_text: "You know the basics! From here on, explore yaku patterns, collect relics, and defeat bosses.",
        step_prompts: &[],
        allowed_sets: &[
            SetKind::Pair,
            SetKind::Triplet,
            SetKind::Kong,
            SetKind::Sequence,
        ],
        discard_enabled: true,
        shop_enabled: true,
        hand_size: None,
        target_override: None, // normal scaling from here
        allowed_yaku: &[YakuKind::FullHand, YakuKind::Yakuhai],
        simplified_boss: false,
        affinity_glow: false,
        annotated_cascade: false,
    },
];

/// Given a set of detected meld kinds, return which milestone (if any)
/// should be celebrated.
pub fn milestone_for_sets(set_kinds: &[SetKind]) -> Option<TutorialMilestone> {
    // Priority: sequence > triplet > pair (celebrate the most complex new thing).
    if set_kinds.contains(&SetKind::Sequence) {
        return Some(TutorialMilestone::FirstSequence);
    }
    if set_kinds.contains(&SetKind::Triplet) || set_kinds.contains(&SetKind::Kong) {
        return Some(TutorialMilestone::FirstTriplet);
    }
    if set_kinds.contains(&SetKind::Pair) {
        return Some(TutorialMilestone::FirstPair);
    }
    None
}

/// Check if a play's detected sets are all within the lesson's allowed set kinds.
pub fn validate_sets_for_lesson(
    detected: &[SetKind],
    lesson: &LessonDef,
) -> Result<(), &'static str> {
    for kind in detected {
        if !lesson.allowed_sets.contains(kind) {
            return Err(match kind {
                SetKind::Triplet | SetKind::Kong => {
                    "Triplets aren't unlocked yet — try scoring a pair!"
                }
                SetKind::Sequence => "Sequences aren't unlocked yet — try pairs or triplets!",
                SetKind::Pair => "Pairs aren't available in this lesson.",
            });
        }
    }
    Ok(())
}

/// Compute which hand tile indices could extend the current selection into
/// a valid meld of any of the `allowed_sets` types. Used for the affinity
/// glow in early tutorial lessons.
pub fn affinity_tile_indices(
    hand: &[crate::core::tile::Tile],
    selected: &[bool],
    allowed_sets: &[SetKind],
) -> Vec<usize> {
    let sel_tiles: Vec<&crate::core::tile::Tile> = hand
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

        // Check if this tile could form a pair or triplet with selected tiles.
        if allowed_sets.contains(&SetKind::Pair) || allowed_sets.contains(&SetKind::Triplet) {
            if sel_tiles
                .iter()
                .any(|s| s.suit == tile.suit && s.rank == tile.rank)
            {
                affinity.push(i);
                continue;
            }
        }

        // Check if this tile could extend a sequence with selected tiles.
        if allowed_sets.contains(&SetKind::Sequence) && tile.is_number_tile() {
            for &s in &sel_tiles {
                if s.suit == tile.suit && s.is_number_tile() {
                    let diff = (tile.rank as i8 - s.rank as i8).unsigned_abs();
                    if diff == 1 || diff == 2 {
                        affinity.push(i);
                        break;
                    }
                }
            }
        }
    }

    affinity
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lesson_count_matches() {
        assert_eq!(LESSONS.len(), LESSON_COUNT);
        for (i, lesson) in LESSONS.iter().enumerate() {
            assert_eq!(lesson.id, (i + 1) as u32);
        }
    }

    #[test]
    fn advance_through_all_lessons() {
        let mut state = TutorialState::new(1);
        for expected in 2..=9 {
            assert!(state.is_active());
            let next = state.advance();
            assert_eq!(next, Some(expected));
        }
        // Advancing past lesson 9 finishes the tutorial.
        assert!(state.is_active());
        let next = state.advance();
        assert_eq!(next, None);
        assert!(state.finished);
        assert!(!state.is_active());
    }

    #[test]
    fn retry_target_factor() {
        let mut state = TutorialState::new(1);
        assert!((state.retry_target_factor() - 1.0).abs() < 0.001);
        state.record_failure();
        assert!((state.retry_target_factor() - 0.8).abs() < 0.001);
        state.record_failure();
        assert!((state.retry_target_factor() - 0.64).abs() < 0.001);
        // Clamps at 0.6
        state.record_failure();
        assert!((state.retry_target_factor() - 0.6).abs() < 0.001);
    }

    #[test]
    fn milestone_celebration_only_once() {
        let mut state = TutorialState::new(1);
        assert!(state.celebrate(TutorialMilestone::FirstPair));
        assert!(!state.celebrate(TutorialMilestone::FirstPair));
    }

    #[test]
    fn validate_sets_rejects_disallowed() {
        let lesson1 = lesson_def(1);
        assert!(validate_sets_for_lesson(&[SetKind::Pair], lesson1).is_ok());
        assert!(validate_sets_for_lesson(&[SetKind::Triplet], lesson1).is_err());
        assert!(validate_sets_for_lesson(&[SetKind::Sequence], lesson1).is_err());
    }

    #[test]
    fn lesson_progression_unlocks() {
        // Lesson 1: only pairs
        assert_eq!(lesson_def(1).allowed_sets, &[SetKind::Pair]);
        assert!(!lesson_def(1).discard_enabled);
        assert!(!lesson_def(1).shop_enabled);

        // Lesson 4: discards enabled
        assert!(lesson_def(4).discard_enabled);

        // Lesson 6: FullHand yaku
        assert!(lesson_def(6).allowed_yaku.contains(&YakuKind::FullHand));

        // Lesson 8: shop enabled
        assert!(lesson_def(8).shop_enabled);

        // Lesson 9: graduation
        assert!(lesson_def(9).target_override.is_none());
    }
}
