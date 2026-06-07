//! Temptations — Balatro-style one-time bonuses awarded when the player
//! skips a Small or Big blind. Each ante rolls one temptation per skippable blind;
//! it is shown on the skip altar so the player can weigh skip vs play.
//!
//! Player-facing copy, rarity tier, pool gates, and bot yen equivalents live
//! in `assets/data/tags.json`. Rolling (`roll_tag`) stays here.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;

/// A one-time bonus awarded for skipping a blind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagKind {
    // ── Instant yen ──────────────────────────────────────────────────
    /// +8 yen immediately.
    GoldIngot,
    /// +20 yen immediately. Only offered from ante 3 onwards.
    TreasureChest,

    // ── Shop modifiers (affect the next shop visit) ───────────────────
    /// Next shop's first restock is free.
    #[serde(alias = "free_reroll")]
    FreeRestock,
    /// One random relic in the next shop costs 0 (lets you sell first if full).
    PatronGift,
    /// Next shop stocks 2 extra relics.
    RichStock,

    // ── Instant item grants ───────────────────────────────────────────
    /// Gain two random Zodiacs. If inventory is full, gain +4 yen instead.
    ZodiacBlessing,

    // ── Next-round gameplay bonuses ───────────────────────────────────
    /// +1 play next round.
    BonusPlay,
    /// +1 discard next round.
    BonusDiscard,
    /// +2 hand size next round.
    WideHand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagRarity {
    Common,
    Uncommon,
    Rare,
}

impl TagRarity {
    pub fn weight(self) -> u32 {
        match self {
            TagRarity::Common => 10,
            TagRarity::Uncommon => 5,
            TagRarity::Rare => 2,
        }
    }
}

#[derive(Deserialize)]
struct TagPresentationRaw {
    id: TagKind,
    name: String,
    description: String,
    rarity: TagRarity,
    #[serde(alias = "min_ante")]
    min_wing: u32,
    yen_value: u32,
}

struct TagPresentation {
    name: &'static str,
    description: &'static str,
    rarity: TagRarity,
    min_wing: u32,
    yen_value: u32,
}

fn tag_presentations() -> &'static HashMap<TagKind, TagPresentation> {
    static MAP: OnceLock<HashMap<TagKind, TagPresentation>> = OnceLock::new();
    MAP.get_or_init(|| {
        const PATH: &str = "data/tags.json";
        let raw: Vec<TagPresentationRaw> = load_json_asset(PATH, "tag data");
        raw.into_iter()
            .map(|r| {
                (
                    r.id,
                    TagPresentation {
                        name: Box::leak(r.name.into_boxed_str()),
                        description: Box::leak(r.description.into_boxed_str()),
                        rarity: r.rarity,
                        min_wing: r.min_wing,
                        yen_value: r.yen_value,
                    },
                )
            })
            .collect()
    })
}

fn tag_presentation(kind: TagKind) -> &'static TagPresentation {
    tag_presentations()
        .get(&kind)
        .unwrap_or_else(|| panic!("tag data missing for {kind:?}"))
}

impl TagKind {
    pub fn name(self) -> &'static str {
        tag_presentation(self).name
    }

    pub fn description(self) -> &'static str {
        tag_presentation(self).description
    }

    pub fn rarity(self) -> TagRarity {
        tag_presentation(self).rarity
    }

    /// Minimum wing required for this tag to appear in the pool.
    pub fn min_wing(self) -> u32 {
        tag_presentation(self).min_wing
    }

    /// All tag variants.
    pub fn all() -> &'static [TagKind] {
        &[
            TagKind::GoldIngot,
            TagKind::TreasureChest,
            TagKind::FreeRestock,
            TagKind::PatronGift,
            TagKind::RichStock,
            TagKind::ZodiacBlessing,
            TagKind::BonusPlay,
            TagKind::BonusDiscard,
            TagKind::WideHand,
        ]
    }

    /// Approximate yen-equivalent value for bot skip evaluation.
    pub fn yen_value(self) -> u32 {
        tag_presentation(self).yen_value
    }

    /// Stable atlas / JSON id (`assets/data/tags.json`, `textures/temptations/atlas.toml`).
    pub fn atlas_slug(self) -> &'static str {
        match self {
            TagKind::GoldIngot => "gold_ingot",
            TagKind::TreasureChest => "treasure_chest",
            TagKind::FreeRestock => "free_restock",
            TagKind::PatronGift => "patron_gift",
            TagKind::RichStock => "rich_stock",
            TagKind::ZodiacBlessing => "zodiac_blessing",
            TagKind::BonusPlay => "bonus_play",
            TagKind::BonusDiscard => "bonus_discard",
            TagKind::WideHand => "wide_hand",
        }
    }
}

/// Pick a random tag from the eligible pool, excluding `exclude` (used to
/// prevent the same tag on both Small and Big within one ante).
pub fn roll_tag(ante: u32, exclude: Option<TagKind>) -> TagKind {
    use rand::RngExt;
    let mut rng = rand::rng();

    let pool: Vec<TagKind> = TagKind::all()
        .iter()
        .copied()
        .filter(|t| t.min_wing() <= ante && Some(*t) != exclude)
        .collect();

    let total_weight: u32 = pool.iter().map(|t| t.rarity().weight()).sum();
    let mut roll = rng.random_range(0..total_weight);
    for tag in &pool {
        let w = tag.rarity().weight();
        if roll < w {
            return *tag;
        }
        roll -= w;
    }
    // Fallback (shouldn't happen if pool is non-empty).
    *pool.last().unwrap_or(&TagKind::GoldIngot)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tag_variant_has_one_data_entry() {
        let map = tag_presentations();
        assert_eq!(
            map.len(),
            TagKind::all().len(),
            "tags.json entry count does not match TagKind variant count"
        );
        for &k in TagKind::all() {
            let _ = tag_presentation(k);
        }
    }

    #[test]
    fn json_row_order_matches_tag_kind_all() {
        const PATH: &str = "data/tags.json";
        let raw: Vec<TagPresentationRaw> = load_json_asset(PATH, "tag data");
        let all = TagKind::all();
        assert_eq!(raw.len(), all.len(), "tags.json row count");
        for (i, row) in raw.iter().enumerate() {
            assert_eq!(
                row.id, all[i],
                "tags.json row {i}: id {:?} does not match TagKind::all()[{i}] {:?}",
                row.id, all[i]
            );
        }
    }
}
