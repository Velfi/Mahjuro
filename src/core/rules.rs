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
    /// Selection must contain exactly 5 tiles. (The Bureaucrat boss.)
    MustPlayFive,
    /// Structure in the selection must contain at least one Honor
    /// (Wind or Dragon) tile somewhere.
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
            RuleModifier::MustPlayFive => "Bureaucratic Form",
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
            RuleModifier::MustPlayFive => "Must play exactly 5 tiles",
            RuleModifier::RequireHonor => "Structure must contain an honor tile",
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

    /// Gold reward formerly granted for skipping this blind. Replaced by
    /// skip-reward tags (`core::tag`), but kept for reference / tests.
    #[allow(dead_code)]
    pub fn skip_reward(self) -> u32 {
        match self {
            BlindKind::Small => 3,
            BlindKind::Big => 5,
            BlindKind::Boss => 0,
        }
    }

    /// Flat gold reward for clearing this blind. Balatro-style: a fixed
    /// payout per blind, not scaled by overscoring. Late-run income comes
    /// from interest on banked gold and unused-plays payout, not from
    /// blowing past the target.
    pub fn clear_reward(self) -> u32 {
        match self {
            BlindKind::Small => 3,
            BlindKind::Big => 4,
            BlindKind::Boss => 5,
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
            BlindKind::Small => "×1 gold",
            BlindKind::Big => "×1.5 gold",
            BlindKind::Boss => "boss effect · ×2.5 gold",
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
