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
//! Labels, descriptions, numeric knobs, and optional `starting_rules` slugs
//! live in `assets/data/stakes.json`. `next` / `previous` unlock order stays
//! in this module.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;
use crate::core::rules::RuleModifier;

/// Ordered list of stakes from easiest to hardest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Stake {
    #[serde(alias = "Spring")]
    #[default]
    Spring,
    #[serde(alias = "Summer")]
    Summer,
    #[serde(alias = "Autumn")]
    Autumn,
    #[serde(alias = "Winter")]
    Winter,
}


#[derive(Deserialize)]
struct StakePresentationRaw {
    id: Stake,
    label: String,
    description: String,
    base_target_mult: f32,
    price_multiplier: f32,
    reroll_base_cost: u32,
    boss_min_ante_floor: u32,
    starting_rules: Vec<String>,
}

struct StakePresentation {
    label: &'static str,
    description: &'static str,
    base_target_mult: f32,
    price_multiplier: f32,
    reroll_base_cost: u32,
    boss_min_ante_floor: u32,
    starting_rules: &'static [RuleModifier],
}

fn stake_rule_from_data_slug(s: &str) -> RuleModifier {
    match s {
        "no_sequence_bonus" => RuleModifier::NoSequenceBonus,
        _ => panic!("unknown stake starting_rules slug: {s}"),
    }
}

fn stake_presentations() -> &'static HashMap<Stake, StakePresentation> {
    static MAP: OnceLock<HashMap<Stake, StakePresentation>> = OnceLock::new();
    MAP.get_or_init(|| {
        const PATH: &str = "data/stakes.json";
        let raw: Vec<StakePresentationRaw> = load_json_asset(PATH, "stake data");
        raw.into_iter()
            .map(|r| {
                let rules: Vec<RuleModifier> = r
                    .starting_rules
                    .iter()
                    .map(|s| stake_rule_from_data_slug(s))
                    .collect();
                let starting_rules: &'static [RuleModifier] = Box::leak(rules.into_boxed_slice());
                (
                    r.id,
                    StakePresentation {
                        label: Box::leak(r.label.into_boxed_str()),
                        description: Box::leak(r.description.into_boxed_str()),
                        base_target_mult: r.base_target_mult,
                        price_multiplier: r.price_multiplier,
                        reroll_base_cost: r.reroll_base_cost,
                        boss_min_ante_floor: r.boss_min_ante_floor,
                        starting_rules,
                    },
                )
            })
            .collect()
    })
}

fn stake_presentation(stake: Stake) -> &'static StakePresentation {
    stake_presentations()
        .get(&stake)
        .unwrap_or_else(|| panic!("stake data missing for {stake:?}"))
}

impl Stake {
    /// Every stake, ordered by difficulty. Used by the modal picker and the
    /// unlock-chain logic.
    pub const ALL: [Stake; 4] = [Stake::Spring, Stake::Summer, Stake::Autumn, Stake::Winter];

    /// Short display label for the HUD badge / picker row.
    pub fn label(self) -> &'static str {
        stake_presentation(self).label
    }

    /// One-line description for the picker row tooltip.
    pub fn description(self) -> &'static str {
        stake_presentation(self).description
    }

    /// Base-target multiplier applied once at run start.
    pub fn base_target_mult(self) -> f32 {
        stake_presentation(self).base_target_mult
    }

    /// Shop price multiplier for everything bought from the shop.
    pub fn price_multiplier(self) -> f32 {
        stake_presentation(self).price_multiplier
    }

    /// Base reroll cost in the shop. Increment-per-reroll still applies on top.
    pub fn reroll_base_cost(self) -> u32 {
        stake_presentation(self).reroll_base_cost
    }

    /// Subtracted from a boss's `min_ante` in `pick_for_ante`, letting higher
    /// stakes meet harder bosses on earlier antes. Zero or positive i32 only
    /// (a positive floor would *delay* bosses — not something any current
    /// stake does).
    pub fn boss_min_ante_floor(self) -> u32 {
        stake_presentation(self).boss_min_ante_floor
    }

    /// Run-wide `RuleModifier`s pushed into `GameMode::starting_rules`.
    pub fn starting_rules(self) -> Vec<RuleModifier> {
        stake_presentation(self).starting_rules.to_vec()
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

    #[test]
    fn every_stake_variant_has_one_data_entry() {
        let map = stake_presentations();
        assert_eq!(
            map.len(),
            Stake::ALL.len(),
            "stakes.json entry count does not match Stake variant count"
        );
        for s in Stake::ALL {
            let _ = stake_presentation(s);
        }
    }

    #[test]
    fn json_row_order_matches_stake_all() {
        const PATH: &str = "data/stakes.json";
        let raw: Vec<StakePresentationRaw> = load_json_asset(PATH, "stake data");
        assert_eq!(raw.len(), Stake::ALL.len(), "stakes.json row count");
        for (i, row) in raw.iter().enumerate() {
            assert_eq!(
                row.id,
                Stake::ALL[i],
                "stakes.json row {i}: id {:?} does not match Stake::ALL[{i}] {:?}",
                row.id,
                Stake::ALL[i]
            );
        }
    }

    #[test]
    fn winter_starts_no_sequence_bonus_rule() {
        assert_eq!(
            Stake::Winter.starting_rules(),
            vec![RuleModifier::NoSequenceBonus]
        );
    }
}
