//! Skip-reward tags — Balatro-style one-time bonuses awarded when the player
//! skips a Small or Big blind. Each ante rolls one tag per skippable blind;
//! the tag is shown on the skip altar so the player can weigh skip vs play.

use serde::{Deserialize, Serialize};

/// A one-time bonus awarded for skipping a blind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagKind {
    // ── Instant gold ──────────────────────────────────────────────────
    /// +8 gold immediately.
    GoldIngot,
    /// +20 gold immediately. Only offered from ante 3 onwards.
    TreasureChest,

    // ── Shop modifiers (affect the next shop visit) ───────────────────
    /// Next shop's first reroll is free.
    FreeReroll,
    /// One random relic in the next shop costs 0 (lets you sell first if full).
    #[serde(alias = "relic_offering")]
    PatronGift,
    /// Next shop stocks 2 extra relics.
    RichStock,

    // ── Instant item grants ───────────────────────────────────────────
    /// Gain a random Zodiac card. If inventory is full, gain +4 gold instead.
    ZodiacBlessing,

    // ── Next-round gameplay bonuses ───────────────────────────────────
    /// +1 play next round.
    BonusPlay,
    /// +1 discard next round.
    BonusDiscard,
    /// +2 hand size next round.
    WideHand,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

impl TagKind {
    pub fn name(self) -> &'static str {
        match self {
            TagKind::GoldIngot => "Gold Ingot",
            TagKind::TreasureChest => "Treasure Chest",
            TagKind::FreeReroll => "Free Reroll",
            TagKind::PatronGift => "Patron's Gift",
            TagKind::RichStock => "Rich Stock",
            TagKind::ZodiacBlessing => "Zodiac Blessing",
            TagKind::BonusPlay => "Bonus Play",
            TagKind::BonusDiscard => "Bonus Discard",
            TagKind::WideHand => "Wide Hand",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            TagKind::GoldIngot => "+8 gold",
            TagKind::TreasureChest => "+20 gold",
            TagKind::FreeReroll => "Next shop reroll is free",
            TagKind::PatronGift => "One shop relic is free",
            TagKind::RichStock => "+2 relics in next shop",
            TagKind::ZodiacBlessing => "Gain a random Zodiac",
            TagKind::BonusPlay => "+1 play next round",
            TagKind::BonusDiscard => "+1 discard next round",
            TagKind::WideHand => "+2 hand size next round",
        }
    }

    pub fn rarity(self) -> TagRarity {
        match self {
            TagKind::GoldIngot => TagRarity::Common,
            TagKind::FreeReroll => TagRarity::Common,
            TagKind::BonusPlay => TagRarity::Common,
            TagKind::BonusDiscard => TagRarity::Common,
            TagKind::PatronGift => TagRarity::Uncommon,
            TagKind::RichStock => TagRarity::Uncommon,
            TagKind::ZodiacBlessing => TagRarity::Uncommon,
            TagKind::WideHand => TagRarity::Uncommon,
            TagKind::TreasureChest => TagRarity::Rare,
        }
    }

    /// Minimum ante required for this tag to appear in the pool.
    pub fn min_ante(self) -> u32 {
        match self {
            TagKind::TreasureChest => 3,
            _ => 1,
        }
    }

    /// All tag variants.
    pub fn all() -> &'static [TagKind] {
        &[
            TagKind::GoldIngot,
            TagKind::TreasureChest,
            TagKind::FreeReroll,
            TagKind::PatronGift,
            TagKind::RichStock,
            TagKind::ZodiacBlessing,
            TagKind::BonusPlay,
            TagKind::BonusDiscard,
            TagKind::WideHand,
        ]
    }

    /// Approximate gold-equivalent value for bot skip evaluation.
    pub fn gold_value(self) -> u32 {
        match self {
            TagKind::GoldIngot => 8,
            TagKind::TreasureChest => 20,
            TagKind::FreeReroll => 5,
            TagKind::PatronGift => 10,
            TagKind::RichStock => 6,
            TagKind::ZodiacBlessing => 6,
            TagKind::BonusPlay => 8,
            TagKind::BonusDiscard => 5,
            TagKind::WideHand => 7,
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
        .filter(|t| t.min_ante() <= ante && Some(*t) != exclude)
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
