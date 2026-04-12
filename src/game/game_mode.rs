//! Preset game-mode defaults that define starting conditions for a run.

use serde::{Deserialize, Serialize};

use crate::core::relic::RelicId;
use crate::core::rules::RuleModifier;
use crate::core::yaku::YakuKind;
use crate::game::tutorial::LessonDef;
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
    /// When true (default), valid plays commit melds into the structure bank
    /// until cash-in. When false, each play scores immediately (classic).
    #[serde(default = "default_structure_bank")]
    pub structure_bank: bool,
}

fn default_structure_bank() -> bool {
    true
}

impl GameMode {
    /// The default game mode (bamboo & ivory tiles).
    pub fn standard() -> Self {
        Self::with_material(TileMaterial::Bamboo)
    }

    /// Tutorial mode for first-time players. Starts with a tiny hand,
    /// low targets, no yaku, no discards, and no consumables. Lessons
    /// progressively unlock mechanics via `apply_lesson()`.
    pub fn tutorial() -> Self {
        Self {
            starting_gold: 4,
            starting_plays: 5,
            starting_discards: 0,
            hand_size: 8,
            base_target: 50,
            target_scaling: 1.0,
            starting_relics: vec![],
            starting_rules: vec![RuleModifier::PairDoubleScore],
            starting_yaku: vec![],
            consumable_capacity: 0,
            tile_material: TileMaterial::Bamboo,
            structure_bank: true,
        }
    }

    /// Apply a tutorial lesson's overrides to this mode. Called when
    /// advancing to a new lesson mid-run.
    pub fn apply_lesson(&mut self, lesson: &LessonDef) {
        if let Some(hs) = lesson.hand_size {
            self.hand_size = hs;
        } else {
            self.hand_size = 14;
        }
        if let Some(target) = lesson.target_override {
            self.base_target = target;
        }
        self.starting_discards = if lesson.discard_enabled { 4 } else { 0 };
        self.starting_yaku = lesson.allowed_yaku.to_vec();
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
            base_target: 350,
            target_scaling: 1.7,
            starting_relics: vec![],
            starting_rules: vec![RuleModifier::PairDoubleScore],
            starting_yaku: crate::core::yaku::YakuKind::all().to_vec(),
            consumable_capacity: 2,
            tile_material: material,
            structure_bank: true,
        }
    }
}
