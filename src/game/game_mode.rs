//! Preset game-mode defaults that define starting conditions for a run.

use serde::{Deserialize, Serialize};

use crate::core::blind_target::DEFAULT_BASE_TARGET;
use crate::core::relic::RelicId;
use crate::core::rules::RuleModifier;
use crate::core::stake::Stake;
use crate::core::yaku::YakuKind;
use crate::persistence::TileMaterial;

pub const STARTING_GOLD: u32 = 8;
pub const STARTING_PLAYS: u32 = 4;
pub const STARTING_DISCARDS: u32 = 4;
pub const HAND_SIZE: usize = 14;
pub const CONSUMABLE_CAPACITY: usize = 2;

/// All tuneable starting conditions for a run, bundled into one preset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameMode {
    pub starting_gold: u32,
    pub starting_plays: u32,
    pub starting_discards: u32,
    pub hand_size: usize,
    pub base_target: u32,
    pub starting_relics: Vec<RelicId>,
    pub starting_rules: Vec<RuleModifier>,
    pub starting_yaku: Vec<YakuKind>,
    /// Base consumable inventory capacity (before relic bonuses).
    pub consumable_capacity: usize,
    /// Tile set chosen at the start of the run. Affects both visuals and
    /// gameplay bonuses (e.g. Plastic grants +1 discard).
    #[serde(default)]
    pub tile_material: TileMaterial,
    /// Difficulty tier selected at run start. Modulates target score, shop
    /// prices, reroll base cost, and boss min_ante floor — see `core::stake`.
    #[serde(default)]
    pub stake: Stake,
    /// Shop price multiplier derived from `stake` at run creation. Cached on
    /// the mode so every shop-side pricing call (after catalog price + run
    /// modifiers such as [`crate::core::relic::apply_merchants_eye_discount`]
    /// and [`crate::core::relic::relic_shop_price`]) can multiply without
    /// reaching for the stake enum.
    #[serde(default = "default_price_multiplier")]
    pub price_multiplier: f32,
}

fn default_price_multiplier() -> f32 {
    1.0
}

impl GameMode {
    /// The default game mode (bamboo & ivory tiles).
    pub fn standard() -> Self {
        Self::with_material(TileMaterial::Bamboo)
    }

    /// Build a game mode for the given tile material. Material-specific
    /// bonuses are baked in here so the rest of the engine is agnostic.
    pub fn with_material(material: TileMaterial) -> Self {
        Self::with_material_and_stake(material, Stake::default())
    }

    /// Build a game mode for the given tile material AND difficulty stake.
    /// The stake's numeric deltas (target mult, shop price mult, reroll
    /// base cost, boss floor) are folded in here so `RunState::new` and the
    /// shop never reach back to the stake enum directly.
    /// Apply the mode's shop price multiplier to a base price. Returns at
    /// least 1 gold so stake-driven discounts (if any were added later) can't
    /// make items free. Used by every shop-side pricing call in
    /// `scenes/shop.rs`, `game/engine.rs`, and `bot.rs`.
    pub fn scale_shop_price(&self, base: u32) -> u32 {
        ((base as f32 * self.price_multiplier).round() as u32).max(1)
    }

    pub fn with_material_and_stake(material: TileMaterial, stake: Stake) -> Self {
        let (bonus_plays, bonus_discards, bonus_gold): (u32, u32, u32) = match material {
            TileMaterial::Bamboo => (1, 0, 0),
            TileMaterial::Plastic => (0, 1, 0),
            TileMaterial::TortoiseShell => (0, 0, 10),
        };
        // Apply the stake's base-target multiplier once here; per-ante growth is
        // `core::blind_target::TARGET_SCALING`.
        let base_target =
            ((DEFAULT_BASE_TARGET as f32) * stake.base_target_mult()).round() as u32;
        let mut starting_rules = vec![RuleModifier::PairDoubleScore];
        starting_rules.extend(stake.starting_rules());
        Self {
            starting_gold: STARTING_GOLD + bonus_gold,
            starting_plays: STARTING_PLAYS + bonus_plays,
            starting_discards: STARTING_DISCARDS + bonus_discards,
            hand_size: HAND_SIZE,
            base_target,
            starting_relics: vec![],
            starting_rules,
            starting_yaku: crate::core::yaku::YakuKind::all().to_vec(),
            consumable_capacity: CONSUMABLE_CAPACITY,
            tile_material: material,
            stake,
            price_multiplier: stake.price_multiplier(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spring_is_baseline() {
        let m = GameMode::standard();
        assert_eq!(m.stake, Stake::Spring);
        assert_eq!(m.base_target, DEFAULT_BASE_TARGET);
        assert!((m.price_multiplier - 1.0).abs() < 1e-6);
    }

    #[test]
    fn winter_scales_target_and_price() {
        let m = GameMode::with_material_and_stake(TileMaterial::Bamboo, Stake::Winter);
        assert_eq!(m.stake, Stake::Winter);
        assert_eq!(m.base_target, 750); // DEFAULT_BASE_TARGET * 1.5
        assert!((m.price_multiplier - 1.5).abs() < 1e-6);
        // scale_shop_price: 10 * 1.5 = 15
        assert_eq!(m.scale_shop_price(10), 15);
    }

    #[test]
    fn scale_shop_price_floors_at_one() {
        let m = GameMode::with_material_and_stake(TileMaterial::Bamboo, Stake::Spring);
        // With a 1.0 multiplier, a base of 1 stays at 1 (sanity check).
        assert_eq!(m.scale_shop_price(1), 1);
    }
}
