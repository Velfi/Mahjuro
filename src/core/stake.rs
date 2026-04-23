//! Difficulty tiers applied on top of the baseline `GameMode`.
//!
//! A stake is a small bundle of numeric modifiers — no new rules system,
//! no per-stake relic or yaku pools. The four knobs are:
//!
//! * `base_target_mult` — scales `GameMode::base_target` once at `RunState::new`.
//! * `price_multiplier` — shop price multiplier for relics / zodiacs /
//!   talismans / packs. Applied at call sites in `scenes/shop.rs` (see
//!   `price_multiplier` field on `GameMode`).
//! * `reroll_base_cost` — replaces `REROLL_BASE_COST` for the active run.
//! * `boss_min_ante_floor` — reduces `BossDef::min_ante` in the filter in
//!   `core::boss::pick_for_ante`, letting harder bosses appear earlier.
//!
//! Optionally, a stake may push `RuleModifier`s into `starting_rules` so a
//! run-wide modifier fires every round. Today only `Winter` reserves this
//! slot; the exact flavor is TBD and it defaults to an empty list.

use serde::{Deserialize, Serialize};

use crate::core::rules::RuleModifier;

/// Ordered list of stakes from easiest to hardest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stake {
    Spring,
    Summer,
    Autumn,
    Winter,
}

impl Default for Stake {
    fn default() -> Self {
        Stake::Spring
    }
}

impl Stake {
    /// Every stake, ordered by difficulty. Used by the modal picker and the
    /// unlock-chain logic.
    pub const ALL: [Stake; 4] = [Stake::Spring, Stake::Summer, Stake::Autumn, Stake::Winter];

    /// Short display label for the HUD badge / picker row.
    pub fn label(self) -> &'static str {
        match self {
            Stake::Spring => "Spring",
            Stake::Summer => "Summer",
            Stake::Autumn => "Autumn",
            Stake::Winter => "Winter",
        }
    }

    /// One-line description for the picker row tooltip.
    pub fn description(self) -> &'static str {
        match self {
            Stake::Spring => "Baseline difficulty.",
            Stake::Summer => "+15% targets.",
            Stake::Autumn => "+30% targets, +25% shop, +1 reroll cost, earlier bosses.",
            Stake::Winter => {
                "+50% targets, +50% shop, +2 reroll cost, earliest bosses, sequences lose bonus."
            }
        }
    }

    /// Base-target multiplier applied once at run start.
    pub fn base_target_mult(self) -> f32 {
        match self {
            Stake::Spring => 1.0,
            Stake::Summer => 1.15,
            Stake::Autumn => 1.30,
            Stake::Winter => 1.50,
        }
    }

    /// Shop price multiplier for everything bought from the shop.
    pub fn price_multiplier(self) -> f32 {
        match self {
            Stake::Spring | Stake::Summer => 1.0,
            Stake::Autumn => 1.25,
            Stake::Winter => 1.50,
        }
    }

    /// Base reroll cost in the shop. Increment-per-reroll still applies on top.
    pub fn reroll_base_cost(self) -> u32 {
        match self {
            Stake::Spring | Stake::Summer => 5,
            Stake::Autumn => 6,
            Stake::Winter => 7,
        }
    }

    /// Subtracted from a boss's `min_ante` in `pick_for_ante`, letting higher
    /// stakes meet harder bosses on earlier antes. Zero or positive i32 only
    /// (a positive floor would *delay* bosses — not something any current
    /// stake does).
    pub fn boss_min_ante_floor(self) -> u32 {
        match self {
            Stake::Spring | Stake::Summer => 0,
            Stake::Autumn => 1,
            Stake::Winter => 2,
        }
    }

    /// Run-wide `RuleModifier`s pushed into `GameMode::starting_rules`.
    /// Winter applies `NoSequenceBonus` every round so the sequence lane is
    /// permanently dampened — stacks with Winter's +50% target and +50% shop
    /// without compounding into the near-unwinnable territory that
    /// `PairsScoreZero` would. Spring/Summer/Autumn add nothing here; their
    /// difficulty comes from the numeric knobs alone.
    pub fn starting_rules(self) -> Vec<RuleModifier> {
        match self {
            Stake::Winter => vec![RuleModifier::NoSequenceBonus],
            _ => Vec::new(),
        }
    }

    /// The next stake in the unlock chain, if any. `Winter` is terminal.
    pub fn next(self) -> Option<Stake> {
        match self {
            Stake::Spring => Some(Stake::Summer),
            Stake::Summer => Some(Stake::Autumn),
            Stake::Autumn => Some(Stake::Winter),
            Stake::Winter => None,
        }
    }

    /// The previous stake in the unlock chain, if any. `Spring` has none.
    pub fn previous(self) -> Option<Stake> {
        match self {
            Stake::Spring => None,
            Stake::Summer => Some(Stake::Spring),
            Stake::Autumn => Some(Stake::Summer),
            Stake::Winter => Some(Stake::Autumn),
        }
    }
}

impl std::str::FromStr for Stake {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "spring" => Ok(Stake::Spring),
            "summer" => Ok(Stake::Summer),
            "autumn" | "fall" => Ok(Stake::Autumn),
            "winter" => Ok(Stake::Winter),
            other => Err(format!(
                "unknown stake '{other}' (expected spring|summer|autumn|winter)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_spring() {
        assert_eq!(Stake::default(), Stake::Spring);
    }

    #[test]
    fn all_ordered_by_difficulty() {
        assert!(Stake::Spring < Stake::Summer);
        assert!(Stake::Summer < Stake::Autumn);
        assert!(Stake::Autumn < Stake::Winter);
    }

    #[test]
    fn next_chain() {
        assert_eq!(Stake::Spring.next(), Some(Stake::Summer));
        assert_eq!(Stake::Summer.next(), Some(Stake::Autumn));
        assert_eq!(Stake::Autumn.next(), Some(Stake::Winter));
        assert_eq!(Stake::Winter.next(), None);
    }

    #[test]
    fn multipliers_monotonic() {
        let mut prev = 0.0f32;
        for s in Stake::ALL {
            let m = s.base_target_mult();
            assert!(m >= prev, "base_target_mult not monotonic at {:?}", s);
            prev = m;
        }
    }
}
