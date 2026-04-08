//! Temporary rule modifiers for a round.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RuleModifier {
    /// 7-8-9 and 8-9-1 style wraps for sequences.
    SequenceWrap,
    /// Pair base score ×2.
    PairDoubleScore,
    /// Only triplets and pairs allowed — sequences are rejected.
    NoSequences,
    /// Plays per round reduced from 4 to 3.
    ReducedPlays,
    /// Honor tile triplets score ×3.
    HonorTripleScore,
    /// If no sequences in the scored hand, +80 bonus.
    NoSequenceBonus,
    // ── Boss-only scoring/validation effects ────────────────────────────
    /// Pairs contribute zero base chips. (The Hermit boss.)
    PairsScoreZero,
    /// Sequences contribute half their normal base chips. (The Forest boss.)
    SequencesHalved,
    /// Tiles of rank 5 contribute zero point value. (The Drunkard boss.)
    MiddleTilesZero,
    /// Selection must contain exactly 4 tiles. (The Bureaucrat boss.)
    MustPlayFour,
    /// Selection must contain at least one Honor (Wind or Dragon) tile.
    /// (The Dragon final boss.)
    RequireHonor,
    /// Yaku already played this round score at half strength. (The Censor boss.)
    CensorRepeats,
}

impl RuleModifier {
    pub fn name(self) -> &'static str {
        match self {
            RuleModifier::SequenceWrap => "Sequence Wrap",
            RuleModifier::PairDoubleScore => "Pair Double",
            RuleModifier::NoSequences => "No Sequences",
            RuleModifier::ReducedPlays => "Reduced Plays",
            RuleModifier::HonorTripleScore => "Honor Triple",
            RuleModifier::NoSequenceBonus => "No-Seq Bonus",
            RuleModifier::PairsScoreZero => "Silent Pairs",
            RuleModifier::SequencesHalved => "Withered Sequences",
            RuleModifier::MiddleTilesZero => "Drunken Middles",
            RuleModifier::MustPlayFour => "Bureaucratic Form",
            RuleModifier::RequireHonor => "Honor Required",
            RuleModifier::CensorRepeats => "Repeats Censored",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            RuleModifier::SequenceWrap => "Sequences can wrap: 8-9-1, 9-1-2",
            RuleModifier::PairDoubleScore => "Pair base score ×2",
            RuleModifier::NoSequences => "Sequences are not allowed",
            RuleModifier::ReducedPlays => "Only 3 plays this round",
            RuleModifier::HonorTripleScore => "Honor triplets score ×3",
            RuleModifier::NoSequenceBonus => "No sequences in hand → +80 bonus",
            RuleModifier::PairsScoreZero => "Pairs score 0 base chips",
            RuleModifier::SequencesHalved => "Sequences score half base chips",
            RuleModifier::MiddleTilesZero => "Rank-5 tiles score 0",
            RuleModifier::MustPlayFour => "Must play exactly 4 tiles",
            RuleModifier::RequireHonor => "Hand must contain a Wind or Dragon tile",
            RuleModifier::CensorRepeats => "Repeated yaku score at half",
        }
    }
}

/// Blind difficulty chosen before each round.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum BlindKind {
    Small,
    Big,
    Boss,
}

impl BlindKind {
    /// Multiplier applied to the base target score. Small/Big trimmed from
    /// 1.0/1.5 to 0.85/1.35 — most "stuck" runs die early to draw variance, not
    /// skill, so the early curve is softer while Boss tension stays intact.
    pub fn target_multiplier(self) -> f32 {
        match self {
            BlindKind::Small => 0.85,
            BlindKind::Big => 1.35,
            BlindKind::Boss => 1.5,
        }
    }

    /// Round wind for the given ante (1-indexed): East → South → West → North,
    /// cycling. The round wind tile becomes Yakuhai (a triplet/kong of it grants
    /// the Yakuhai yaku) and is shown on the blind card.
    pub fn round_wind_for_ante(ante: u32) -> u8 {
        ((ante.saturating_sub(1)) % 4) as u8 + 1
    }

    /// Display name for a wind rank (1=East, 2=South, 3=West, 4=North).
    #[allow(dead_code)]
    pub fn wind_name(rank: u8) -> &'static str {
        match rank {
            1 => "East",
            2 => "South",
            3 => "West",
            4 => "North",
            _ => "?",
        }
    }

    /// Gold reward granted for skipping this blind. Boss can't be skipped.
    pub fn skip_reward(self) -> u32 {
        match self {
            BlindKind::Small => 3,
            BlindKind::Big => 5,
            BlindKind::Boss => 0,
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

    pub fn name(self) -> &'static str {
        match self {
            BlindKind::Small => "Small Blind",
            BlindKind::Big => "Big Blind",
            BlindKind::Boss => "Boss Blind",
        }
    }

    #[allow(dead_code)]
    pub fn description(self) -> &'static str {
        match self {
            BlindKind::Small => "×0.85 target · ×1 gold",
            BlindKind::Big => "×1.35 target · ×1.5 gold",
            BlindKind::Boss => "×1.5 target · boss effect · ×2.5 gold",
        }
    }

    /// The next blind in the Small → Big → Boss → Small cycle.
    pub fn next(self) -> BlindKind {
        match self {
            BlindKind::Small => BlindKind::Big,
            BlindKind::Big => BlindKind::Boss,
            BlindKind::Boss => BlindKind::Small,
        }
    }
}
