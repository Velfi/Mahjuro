//! Rules core: re-exported from [`mahjuro_core`] with game-layer `ordeal` hooks.

pub use mahjuro_core::core::{
    archive_seen, attribution, chamber_target, consumable, credits, debuff, deck, hand,
    hand_intent, memorial_talisman, moon_quips, ordeal_kind, progression, relic,
    relic_desc_template, rules, run_chronicle, scoring, season, staircase_flavor, structure,
    structure_notation, tag, talisman, tile, tile_pack, yaku, zodiac,
};

pub mod json_asset {
    pub use mahjuro_core::core::json_asset::load_json_asset;
}

pub mod ordeal {
    pub use crate::game::ordeal::*;
}
pub use ordeal::{OrdealKind, OrdealKindExt, OrdealTier};
