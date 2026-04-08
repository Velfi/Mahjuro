//! Preset game-mode defaults that define starting conditions for a run.

use serde::{Deserialize, Serialize};

use crate::core::relic::RelicId;
use crate::core::rules::RuleModifier;
use crate::core::yaku::YakuKind;

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
}

impl GameMode {
    /// The default game mode.
    pub fn standard() -> Self {
        Self {
            starting_gold: 6,
            starting_plays: 4,
            // Patch A bumped 3→4: most "stuck" runs die early to draw variance,
            // not skill, so a bigger early-game safety net softens the curve
            // without touching boss tension.
            starting_discards: 4,
            hand_size: 14,
            base_target: 200,
            target_scaling: 1.25,
            starting_relics: vec![],
            starting_rules: vec![RuleModifier::PairDoubleScore],
            // Default starting yaku pool: all 12 canonical yaku are available
            // for detection. The run-state Codex loadout decides which receive
            // full-strength scoring vs. half-strength.
            starting_yaku: crate::core::yaku::YakuKind::all().to_vec(),
        }
    }
}
