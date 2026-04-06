//! Temporary rule modifiers for a round.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuleModifier {
    /// 7-8-9 and 8-9-1 style wraps for sequences (applied in hand/scoring when implemented).
    SequenceWrap,
    /// Pair base score ×2.
    PairDoubleScore,
}

/// Blind difficulty chosen before each round.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlindKind {
    Small,
    Big,
    Boss,
}

impl BlindKind {
    /// Multiplier applied to the base target score.
    pub fn target_multiplier(self) -> f32 {
        match self {
            BlindKind::Small => 1.0,
            BlindKind::Big => 1.5,
            BlindKind::Boss => 2.0,
        }
    }

    /// Number of relic choices offered after clearing this blind.
    pub fn relic_choices(self) -> usize {
        match self {
            BlindKind::Small => 2,
            BlindKind::Big => 3,
            BlindKind::Boss => 3,
        }
    }

    /// Gold bonus multiplier for clearing this blind.
    pub fn gold_multiplier(self) -> f32 {
        match self {
            BlindKind::Small => 1.0,
            BlindKind::Big => 1.5,
            BlindKind::Boss => 2.5,
        }
    }

    /// Forced rule modifier for boss blinds (None for small/big).
    pub fn forced_modifier(self) -> Option<RuleModifier> {
        match self {
            BlindKind::Boss => Some(RuleModifier::PairDoubleScore), // placeholder: will rotate
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            BlindKind::Small => "Small Blind",
            BlindKind::Big => "Big Blind",
            BlindKind::Boss => "Boss Blind",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            BlindKind::Small => "×1 target · 2 relic picks · ×1 gold",
            BlindKind::Big => "×1.5 target · 3 relic picks · ×1.5 gold",
            BlindKind::Boss => "×2 target + modifier · 3 relic picks · ×2.5 gold",
        }
    }
}
