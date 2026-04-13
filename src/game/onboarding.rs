use serde::{Deserialize, Serialize};

use crate::core::boss::BossKind;
use crate::core::scoring::ScoreBreakdown;
use crate::core::yaku::YakuKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OnboardingPhase {
    Lessons,
    Shop,
    Finale,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OnboardingState {
    pub phase: OnboardingPhase,
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
        }
    }
}

pub const TUTORIAL_BOSS: BossKind = BossKind::Relic;

pub fn tutorial_yaku() -> Vec<YakuKind> {
    vec![YakuKind::FullHand, YakuKind::Chiitoitsu]
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
        (round_score.min(u64::MAX) as f64 / target as f64 * 100.0).min(100.0) as u32
    } else {
        100
    };

    let chicken_only = last_breakdown.is_some_and(|b| {
        b.detected_yaku.contains(&YakuKind::ChickenHand)
            && !b.detected_yaku.contains(&YakuKind::FullHand)
            && !b.detected_yaku.contains(&YakuKind::Chiitoitsu)
    });

    if round_score == 0 {
        return "You scored 0 — bank valid melds with Play, then press Trigger to cash in. If you run out of plays first, the round ends.".to_string();
    }

    if chicken_only {
        return format!(
            "Your last cash-in was a Chicken Hand (no pattern bonus). This run only awards Full Hand and Chiitoitsu — build one of those shapes so mult is not stuck near zero. You were {} / {} ({}%).",
            round_score, target, score_pct,
        );
    }

    if discards_left >= 2 {
        return format!(
            "You scored {} / {} ({}%) but had {} discards left. The Iconoclast weakens honors — favor triplets and runs in bamboo, circles, or characters, then cash in with a yaku.",
            round_score, target, score_pct, discards_left,
        );
    }

    if score_pct >= 75 {
        return format!(
            "Close — {} / {} ({}%). Honors score for less on this shrine; lean on the three suits. Bigger melds and a yaku raise mult more than pairs alone.",
            round_score, target, score_pct,
        );
    }

    format!(
        "You scored {} / {} — {} short of the target. The Iconoclast debuffs winds and dragons; build melds in bamboo, circles, and characters, bank with Play, then Trigger. Chips × mult: Full Hand or Chiitoitsu is your main mult lever here.",
        round_score, target, gap,
    )
}
