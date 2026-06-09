//! Difficulty tiers applied on top of the baseline `GameMode`.
//!
//! A season is a small bundle of numeric modifiers — no new rules system,
//! no per-season relic or yaku pools. The three knobs are:
//!
//! * `base_target_mult` — scales `GameMode::base_target` once at `RunState::new`.
//! * `restock_base_cost` — base shop restock price; each paid restock this visit
//!   adds another `restock_base_cost` on top.
//! * `ordeal_min_wing_floor` — reduces `OrdealDef::min_ante` in the filter in
//!   `core::ordeal::pick_for_wing`, letting harder bosses appear earlier.
//!
//! Labels, descriptions, and numeric knobs live in `assets/data/seasons.json`.
//! `next` / `previous` unlock order stays in this module.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;

/// Ordered list of seasons from easiest to hardest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum Season {
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
struct SeasonPresentationRaw {
    id: Season,
    label: String,
    description: String,
    base_target_mult: f32,
    #[serde(alias = "reroll_base_cost")]
    restock_base_cost: u32,
    ordeal_min_wing_floor: u32,
}

struct SeasonPresentation {
    label: &'static str,
    description: &'static str,
    base_target_mult: f32,
    restock_base_cost: u32,
    ordeal_min_wing_floor: u32,
}

fn season_presentations() -> &'static HashMap<Season, SeasonPresentation> {
    static MAP: OnceLock<HashMap<Season, SeasonPresentation>> = OnceLock::new();
    MAP.get_or_init(|| {
        const PATH: &str = "data/seasons.json";
        let raw: Vec<SeasonPresentationRaw> = load_json_asset(PATH, "season data");
        raw.into_iter()
            .map(|r| {
                (
                    r.id,
                    SeasonPresentation {
                        label: Box::leak(r.label.into_boxed_str()),
                        description: Box::leak(r.description.into_boxed_str()),
                        base_target_mult: r.base_target_mult,
                        restock_base_cost: r.restock_base_cost,
                        ordeal_min_wing_floor: r.ordeal_min_wing_floor,
                    },
                )
            })
            .collect()
    })
}

fn season_presentation(season: Season) -> &'static SeasonPresentation {
    season_presentations()
        .get(&season)
        .unwrap_or_else(|| panic!("season data missing for {season:?}"))
}

impl Season {
    /// Every season, ordered by difficulty. Used by the modal picker and the
    /// unlock-chain logic.
    pub const ALL: [Season; 4] = [
        Season::Spring,
        Season::Summer,
        Season::Autumn,
        Season::Winter,
    ];

    /// Short display label for the HUD badge / picker row.
    pub fn label(self) -> &'static str {
        season_presentation(self).label
    }

    /// One-line description for the picker row tooltip.
    pub fn description(self) -> &'static str {
        season_presentation(self).description
    }

    /// Base-target multiplier applied once at run start.
    pub fn base_target_mult(self) -> f32 {
        season_presentation(self).base_target_mult
    }

    /// Base restock cost in the shop — also the per-restock increment within a visit.
    pub fn restock_base_cost(self) -> u32 {
        season_presentation(self).restock_base_cost
    }

    /// Yen cost for the next shop restock this visit.
    ///
    /// `paid_restock_count` is how many restocks were already paid for this visit
    /// (free restocks from tags or relic waivers do not count).
    pub fn shop_restock_cost(self, paid_restock_count: u32) -> u32 {
        let base = self.restock_base_cost();
        base.saturating_mul(paid_restock_count.saturating_add(1))
    }

    /// Subtracted from a boss's `min_ante` in `pick_for_wing`, letting higher
    /// seasons meet harder bosses on earlier antes. Zero or positive i32 only
    /// (a positive floor would *delay* bosses — not something any current
    /// season does).
    pub fn ordeal_min_wing_floor(self) -> u32 {
        season_presentation(self).ordeal_min_wing_floor
    }

    /// The next season in the unlock chain, if any. `Winter` is terminal.
    pub fn next(self) -> Option<Season> {
        match self {
            Season::Spring => Some(Season::Summer),
            Season::Summer => Some(Season::Autumn),
            Season::Autumn => Some(Season::Winter),
            Season::Winter => None,
        }
    }

    /// The previous season in the unlock chain, if any. `Spring` has none.
    pub fn previous(self) -> Option<Season> {
        match self {
            Season::Spring => None,
            Season::Summer => Some(Season::Spring),
            Season::Autumn => Some(Season::Summer),
            Season::Winter => Some(Season::Autumn),
        }
    }
}

impl std::str::FromStr for Season {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "spring" => Ok(Season::Spring),
            "summer" => Ok(Season::Summer),
            "autumn" | "fall" => Ok(Season::Autumn),
            "winter" => Ok(Season::Winter),
            other => Err(format!(
                "unknown season '{other}' (expected spring|summer|autumn|winter)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_spring() {
        assert_eq!(Season::default(), Season::Spring);
    }

    #[test]
    fn all_ordered_by_difficulty() {
        assert!(Season::Spring < Season::Summer);
        assert!(Season::Summer < Season::Autumn);
        assert!(Season::Autumn < Season::Winter);
    }

    #[test]
    fn next_chain() {
        assert_eq!(Season::Spring.next(), Some(Season::Summer));
        assert_eq!(Season::Summer.next(), Some(Season::Autumn));
        assert_eq!(Season::Autumn.next(), Some(Season::Winter));
        assert_eq!(Season::Winter.next(), None);
    }

    #[test]
    fn shop_restock_cost_scales_with_visit_count() {
        assert_eq!(Season::Spring.shop_restock_cost(0), 3);
        assert_eq!(Season::Spring.shop_restock_cost(1), 6);
        assert_eq!(Season::Spring.shop_restock_cost(2), 9);
        assert_eq!(Season::Winter.shop_restock_cost(0), 5);
        assert_eq!(Season::Winter.shop_restock_cost(2), 15);
    }

    #[test]
    fn multipliers_monotonic() {
        let mut prev = 0.0f32;
        for s in Season::ALL {
            let m = s.base_target_mult();
            assert!(m >= prev, "base_target_mult not monotonic at {:?}", s);
            prev = m;
        }
    }

    #[test]
    fn every_season_variant_has_one_data_entry() {
        let map = season_presentations();
        assert_eq!(
            map.len(),
            Season::ALL.len(),
            "seasons.json entry count does not match Season variant count"
        );
        for s in Season::ALL {
            let _ = season_presentation(s);
        }
    }

    #[test]
    fn json_row_order_matches_season_all() {
        const PATH: &str = "data/seasons.json";
        let raw: Vec<SeasonPresentationRaw> = load_json_asset(PATH, "season data");
        assert_eq!(raw.len(), Season::ALL.len(), "seasons.json row count");
        for (i, row) in raw.iter().enumerate() {
            assert_eq!(
                row.id,
                Season::ALL[i],
                "seasons.json row {i}: id {:?} does not match Season::ALL[{i}] {:?}",
                row.id,
                Season::ALL[i]
            );
        }
    }
}
