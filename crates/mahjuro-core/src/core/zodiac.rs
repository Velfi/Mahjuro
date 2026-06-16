//! Chinese Zodiac consumable cards — Mahjuro's planet-card analogue.
//!
//! Each Zodiac card levels one or more yaku in [`crate::core::yaku`] for the
//! rest of the run, scaling both chip and Han contributions per
//! `YakuKind::han_bonus_at` / `fu_bonus_at`. Zodiacs are consumed when used
//! on a tile.
//!
//! Display names, asset slugs, primary yaku pairing, and ribbon shop price live
//! in `assets/data/zodiacs.json`. Leveling behaviour stays in [`YakuLevels`].

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;
use crate::core::yaku::YakuKind;

#[derive(Deserialize)]
struct ZodiacCatalogRaw {
    ribbon_shop_price: u32,
    zodiacs: Vec<ZodiacRowRaw>,
}

#[derive(Deserialize)]
struct ZodiacRowRaw {
    id: ZodiacKind,
    name: String,
    slug: String,
    yaku: YakuKind,
}

struct ZodiacRow {
    name: &'static str,
    slug: &'static str,
    yaku: YakuKind,
}

struct ZodiacCatalog {
    ribbon_shop_price: u32,
    by_kind: HashMap<ZodiacKind, ZodiacRow>,
}

fn zodiac_catalog() -> &'static ZodiacCatalog {
    static CATALOG: OnceLock<ZodiacCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        const PATH: &str = "data/zodiacs.json";
        let raw: ZodiacCatalogRaw = load_json_asset(PATH, "zodiac data");
        let ribbon_shop_price = raw.ribbon_shop_price;
        let by_kind: HashMap<ZodiacKind, ZodiacRow> = raw
            .zodiacs
            .into_iter()
            .map(|r| {
                (
                    r.id,
                    ZodiacRow {
                        name: Box::leak(r.name.into_boxed_str()),
                        slug: Box::leak(r.slug.into_boxed_str()),
                        yaku: r.yaku,
                    },
                )
            })
            .collect();
        ZodiacCatalog {
            ribbon_shop_price,
            by_kind,
        }
    })
}

fn zodiac_row(kind: ZodiacKind) -> &'static ZodiacRow {
    zodiac_catalog()
        .by_kind
        .get(&kind)
        .unwrap_or_else(|| panic!("zodiac data missing for {kind:?}"))
}

/// Zodiac ribbon kinds: the thirteen calendar animals (Mouse precedes Rat) plus
/// **Qilin**, **Phoenix**, **Crane**, and **Koi**. Variant order matters for
/// serialization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZodiacKind {
    Mouse,
    Rat,
    Ox,
    Tiger,
    Rabbit,
    Dragon,
    Snake,
    Horse,
    Goat,
    Monkey,
    Rooster,
    Dog,
    Pig,
    Qilin,
    Phoenix,
    Crane,
    Koi,
}

impl ZodiacKind {
    /// All ribbon kinds: calendar order (Mouse precedes Rat), then Qilin,
    /// Phoenix, Crane.
    pub fn all() -> &'static [ZodiacKind] {
        &[
            ZodiacKind::Mouse,
            ZodiacKind::Rat,
            ZodiacKind::Ox,
            ZodiacKind::Tiger,
            ZodiacKind::Rabbit,
            ZodiacKind::Dragon,
            ZodiacKind::Snake,
            ZodiacKind::Horse,
            ZodiacKind::Goat,
            ZodiacKind::Monkey,
            ZodiacKind::Rooster,
            ZodiacKind::Dog,
            ZodiacKind::Pig,
            ZodiacKind::Qilin,
            ZodiacKind::Phoenix,
            ZodiacKind::Crane,
            ZodiacKind::Koi,
        ]
    }

    /// English display name.
    pub fn name(self) -> &'static str {
        zodiac_row(self).name
    }

    /// Primary yaku shown on shop ribbons and celebrations (first in [`Self::yaku_levels`]).
    pub fn yaku(self) -> YakuKind {
        zodiac_row(self).yaku
    }

    /// Every yaku this ribbon levels when consumed.
    pub fn yaku_levels(self) -> &'static [YakuKind] {
        match self {
            ZodiacKind::Mouse => &[YakuKind::Honitsu],
            ZodiacKind::Rat => &[YakuKind::Chinitsu],
            ZodiacKind::Ox => &[YakuKind::Toitoi],
            ZodiacKind::Tiger => &[YakuKind::Honroutou],
            ZodiacKind::Rabbit => &[YakuKind::Iipeikou, YakuKind::Ryanpeikou],
            ZodiacKind::Dragon => &[YakuKind::Daisangen],
            ZodiacKind::Snake => &[YakuKind::Ittsu],
            ZodiacKind::Horse => &[YakuKind::SanshokuDoujun, YakuKind::SanshokuDoukou],
            ZodiacKind::Goat => &[YakuKind::Junchan],
            ZodiacKind::Monkey => &[YakuKind::Tanyao],
            ZodiacKind::Rooster => &[YakuKind::ChickenHand],
            ZodiacKind::Dog => &[YakuKind::Yakuhai],
            ZodiacKind::Pig => &[YakuKind::Chiitoitsu],
            ZodiacKind::Qilin => &[YakuKind::KokushiMusou],
            ZodiacKind::Phoenix => &[YakuKind::Chanta],
            ZodiacKind::Crane => &[YakuKind::Pinfu],
            ZodiacKind::Koi => &[YakuKind::Shousangen],
        }
    }

    /// Lowercase slug used for asset filenames (e.g. `dragon` →
    /// `assets/textures/zodiacs/zodiac_dragon.png`). See
    /// `scripts/generate_zodiac_ribbons.py`.
    pub fn slug(self) -> &'static str {
        zodiac_row(self).slug
    }

    /// Look up the zodiac ribbon that levels a given yaku, if any.
    pub fn for_yaku(yaku: YakuKind) -> Option<ZodiacKind> {
        Self::all()
            .iter()
            .copied()
            .find(|z| z.yaku_levels().contains(&yaku))
    }

    /// Shop price in yen for buying one ribbon (same catalog price for every kind).
    pub fn shop_price() -> u32 {
        zodiac_catalog().ribbon_shop_price
    }

    /// Whether this ribbon may appear in shop stock or random zodiac grants.
    /// Requires at least one bound yaku to have been cashed in before.
    pub fn eligible_for_spawn(self, yaku_scored: impl Fn(YakuKind) -> bool) -> bool {
        self.yaku_levels().iter().any(|&yk| yaku_scored(yk))
    }

    /// Additive Han granted to each bound yaku level when this zodiac is
    /// consumed. Default is +0.5 Han per level; Rooster (Chicken Hand) is a
    /// chips-only stabilizer and grants +0.0 Han.
    pub fn level_up_han_per_level(self) -> f64 {
        match self {
            ZodiacKind::Rooster => 0.0,
            ZodiacKind::Mouse => 0.5,
            ZodiacKind::Rat => 0.5,
            ZodiacKind::Ox => 0.5,
            ZodiacKind::Tiger => 0.5,
            ZodiacKind::Rabbit => 0.5,
            ZodiacKind::Dragon => 0.5,
            ZodiacKind::Snake => 0.5,
            ZodiacKind::Horse => 0.5,
            ZodiacKind::Goat => 0.5,
            ZodiacKind::Monkey => 0.5,
            ZodiacKind::Dog => 0.5,
            ZodiacKind::Pig => 0.5,
            ZodiacKind::Qilin => 0.5,
            ZodiacKind::Phoenix => 0.5,
            ZodiacKind::Crane => 0.5,
            ZodiacKind::Koi => 0.5,
        }
    }

    /// Additive Fu granted to each bound yaku level when this zodiac is
    /// consumed. Default is +30 Fu; Rooster (Chicken Hand) gets +50 Fu.
    pub fn level_up_fu_per_level(self) -> i32 {
        match self {
            ZodiacKind::Rooster => 50,
            _ => 30,
        }
    }

    /// Ribbons eligible for shop stock and random zodiac grants.
    pub fn spawn_pool(yaku_scored: impl Fn(YakuKind) -> bool) -> Vec<ZodiacKind> {
        Self::all()
            .iter()
            .copied()
            .filter(|z| z.eligible_for_spawn(&yaku_scored))
            .collect()
    }
}

/// Small inventory of Zodiac cards held mid-run. Capacity is set by the run
/// (default 2, +1 from Brocade Pouch). Pushing past capacity is rejected —
/// the caller chooses to use, sell, or skip.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ZodiacInventory {
    pub items: Vec<ZodiacKind>,
    pub capacity: usize,
}

impl Default for ZodiacInventory {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            capacity: 2,
        }
    }
}

impl ZodiacInventory {
    pub fn is_full(&self) -> bool {
        self.items.len() >= self.capacity
    }

    /// Try to add a card. Returns `true` on success, `false` if full.
    pub fn try_push(&mut self, z: ZodiacKind) -> bool {
        if self.is_full() {
            false
        } else {
            self.items.push(z);
            true
        }
    }

    /// Remove and return the card at `index`, if any.
    pub fn take(&mut self, index: usize) -> Option<ZodiacKind> {
        if index < self.items.len() {
            Some(self.items.remove(index))
        } else {
            None
        }
    }
}

/// Map of YakuKind → current level for the run. Defaults to level 1 for any
/// yaku not yet leveled. Centralized so scoring + UI both read the same source
/// of truth.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct YakuLevels {
    /// Sparse map: only yaku that have been leveled appear here.
    pub levels: rustc_hash::FxHashMap<YakuKind, u32>,
}

impl YakuLevels {
    /// Current level of `yaku` (defaults to 1 if never leveled).
    pub fn level_of(&self, yaku: YakuKind) -> u32 {
        self.levels.get(&yaku).copied().unwrap_or(1)
    }

    /// Increment a yaku's level by 1 (used when a Zodiac card is consumed).
    pub fn level_up(&mut self, yaku: YakuKind) -> u32 {
        let current = self.level_of(yaku);
        let next = current + 1;
        self.levels.insert(yaku, next);
        next
    }

    /// Level every yaku tied to a zodiac ribbon. Returns the last yaku's new level.
    pub fn level_up_for_zodiac(&mut self, zodiac: ZodiacKind) -> u32 {
        let mut last = 1;
        for &yaku in zodiac.yaku_levels() {
            last = self.level_up(yaku);
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_scoring_yaku_has_exactly_one_ribbon() {
        for &yk in YakuKind::all() {
            if yk == YakuKind::ChickenHand {
                continue;
            }
            let ribbons: Vec<_> = ZodiacKind::all()
                .iter()
                .copied()
                .filter(|z| z.yaku_levels().contains(&yk))
                .collect();
            assert_eq!(
                ribbons.len(),
                1,
                "{yk:?} should map to one ribbon, got {ribbons:?}"
            );
        }
    }

    #[test]
    fn for_yaku_round_trips() {
        for &z in ZodiacKind::all() {
            for &yk in z.yaku_levels() {
                assert_eq!(ZodiacKind::for_yaku(yk), Some(z), "{yk:?}");
            }
        }
    }

    #[test]
    fn rabbit_and_horse_level_two_yaku() {
        assert_eq!(
            ZodiacKind::Rabbit.yaku_levels(),
            &[YakuKind::Iipeikou, YakuKind::Ryanpeikou]
        );
        assert_eq!(
            ZodiacKind::Horse.yaku_levels(),
            &[YakuKind::SanshokuDoujun, YakuKind::SanshokuDoukou]
        );
    }

    #[test]
    fn level_up_for_zodiac_levels_all_bound_yaku() {
        let mut yl = YakuLevels::default();
        let new_level = yl.level_up_for_zodiac(ZodiacKind::Rabbit);
        assert_eq!(new_level, 2);
        assert_eq!(yl.level_of(YakuKind::Iipeikou), 2);
        assert_eq!(yl.level_of(YakuKind::Ryanpeikou), 2);
    }

    #[test]
    fn every_zodiac_variant_has_one_data_entry() {
        let cat = zodiac_catalog();
        assert_eq!(
            cat.by_kind.len(),
            ZodiacKind::all().len(),
            "zodiacs.json row count does not match ZodiacKind variant count"
        );
        for &z in ZodiacKind::all() {
            let _ = zodiac_row(z);
        }
    }

    #[test]
    fn json_row_order_matches_zodiac_kind_all() {
        const PATH: &str = "data/zodiacs.json";
        let raw: ZodiacCatalogRaw = load_json_asset(PATH, "zodiac data");
        let all = ZodiacKind::all();
        assert_eq!(raw.zodiacs.len(), all.len(), "zodiacs array length");
        for (i, row) in raw.zodiacs.iter().enumerate() {
            assert_eq!(
                row.id, all[i],
                "zodiacs.json row {i}: id {:?} does not match ZodiacKind::all()[{i}] {:?}",
                row.id, all[i]
            );
        }
    }

    #[test]
    fn inventory_respects_capacity() {
        let mut inv = ZodiacInventory::default();
        assert!(inv.try_push(ZodiacKind::Rat));
        assert!(inv.try_push(ZodiacKind::Ox));
        assert!(inv.is_full());
        assert!(!inv.try_push(ZodiacKind::Tiger));
    }

    #[test]
    fn yaku_levels_default_to_one() {
        let yl = YakuLevels::default();
        assert_eq!(yl.level_of(YakuKind::Toitoi), 1);
    }

    #[test]
    fn level_up_increments() {
        let mut yl = YakuLevels::default();
        assert_eq!(yl.level_up(YakuKind::Toitoi), 2);
        assert_eq!(yl.level_up(YakuKind::Toitoi), 3);
        assert_eq!(yl.level_of(YakuKind::Toitoi), 3);
    }

    #[test]
    fn zodiac_level_up_han_bonuses_are_half_increments() {
        for &z in ZodiacKind::all() {
            let han = z.level_up_han_per_level();
            assert_eq!(
                han,
                (han * 2.0).round() / 2.0,
                "{z:?} Han per level {han} is not a half increment"
            );
        }
    }

    #[test]
    fn zodiac_level_up_fu_bonuses_match_default_and_rooster_exception() {
        for &z in ZodiacKind::all() {
            if z == ZodiacKind::Rooster {
                assert_eq!(z.level_up_fu_per_level(), 50);
            } else {
                assert_eq!(z.level_up_fu_per_level(), 30);
            }
        }
    }
}
