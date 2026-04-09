//! Preset game-mode defaults that define starting conditions for a run.

use serde::{Deserialize, Serialize};

use crate::core::relic::RelicId;
use crate::core::rules::RuleModifier;
use crate::core::yaku::YakuKind;
use crate::persistence::TileMaterial;

/// All tuneable starting conditions for a run, bundled into one preset.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GameMode {
    pub starting_gold: u32,
    pub starting_plays: u32,
    pub starting_discards: u32,
    pub hand_size: usize,
    pub base_target: u32,
    pub target_scaling: f32,
    pub starting_relics: Vec<RelicId>,
    pub starting_rules: Vec<RuleModifier>,
    pub starting_yaku: Vec<YakuKind>,
    /// Base consumable inventory capacity (before relic bonuses).
    pub consumable_capacity: usize,
    /// Tile set chosen at the start of the run. Affects both visuals and
    /// gameplay bonuses (e.g. Plastic grants +1 discard).
    #[serde(default)]
    pub tile_material: TileMaterial,
}

impl GameMode {
    /// The default game mode (bamboo & ivory tiles).
    pub fn standard() -> Self {
        Self::with_material(TileMaterial::Bamboo)
    }

    /// Build a game mode for the given tile material. Material-specific
    /// bonuses are baked in here so the rest of the engine is agnostic.
    pub fn with_material(material: TileMaterial) -> Self {
        let (bonus_plays, bonus_discards): (u32, u32) = match material {
            TileMaterial::Bamboo => (1, 0),
            TileMaterial::Plastic => (0, 1),
        };
        Self {
            starting_gold: 6,
            starting_plays: 4 + bonus_plays,
            starting_discards: 4 + bonus_discards,
            hand_size: 14,
            base_target: 300,
            target_scaling: 1.3,
            starting_relics: vec![],
            starting_rules: vec![RuleModifier::PairDoubleScore],
            starting_yaku: crate::core::yaku::YakuKind::all().to_vec(),
            consumable_capacity: 2,
            tile_material: material,
        }
    }
}
