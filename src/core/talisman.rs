//! Talismans — tarot-style consumables that buff every tile in the player's
//! current hand at once.
//!
//! Every talisman applies to the **whole hand** in one click — buff stamps or
//! suit transforms across all tiles at once. The enhancement lives on each
//! [`crate::core::tile::Tile`]
//! while it sits in the hand and is dropped when the tile leaves (played or
//! discarded).
//!
//! Talismans live in the same inventory as Zodiacs (see
//! [`crate::core::consumable::ConsumableInventory`]) — they share slot space
//! so the player chooses how to spend their consumable budget each round.
//!
//! Shop copy, prices, and tablet tint live in `assets/data/talismans.json`.
//! Behaviour (`enhancement`, hand transforms) stays in this module.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::core::json_asset::load_json_asset;
use crate::core::tile::{Tile, TileEnhancement};

#[derive(Deserialize)]
struct TalismanPresentationRaw {
    id: TalismanKind,
    name: String,
    description: String,
    shop_price: u32,
    accent: [f32; 4],
}

struct TalismanPresentation {
    name: &'static str,
    description: &'static str,
    shop_price: u32,
    accent: [f32; 4],
}

fn talisman_presentations() -> &'static HashMap<TalismanKind, TalismanPresentation> {
    static MAP: OnceLock<HashMap<TalismanKind, TalismanPresentation>> = OnceLock::new();
    MAP.get_or_init(|| {
        const PATH: &str = "data/talismans.json";
        let raw: Vec<TalismanPresentationRaw> = load_json_asset(PATH, "talisman data");
        raw.into_iter()
            .map(|r| {
                (
                    r.id,
                    TalismanPresentation {
                        name: Box::leak(r.name.into_boxed_str()),
                        description: Box::leak(r.description.into_boxed_str()),
                        shop_price: r.shop_price,
                        accent: r.accent,
                    },
                )
            })
            .collect()
    })
}

fn talisman_presentation(kind: TalismanKind) -> &'static TalismanPresentation {
    talisman_presentations()
        .get(&kind)
        .unwrap_or_else(|| panic!("talisman data missing for {kind:?}"))
}

/// One talisman variant. Each maps to a single [`TileEnhancement`] kind that
/// gets stamped onto every tile in the hand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TalismanKind {
    #[serde(alias = "jade")]
    Pearl,
    Gilded,
    Polychrome,
    /// Every numbered tile in hand becomes souzu; honors unchanged.
    #[serde(alias = "bamboo")]
    Souzu,
    /// Every numbered tile in hand becomes pinzu; honors unchanged.
    #[serde(alias = "dots")]
    Pinzu,
    /// Every numbered tile in hand becomes manzu; honors unchanged.
    #[serde(alias = "characters")]
    Manzu,
    /// Every numbered tile in hand becomes a random honor; honors unchanged.
    Honors,
    /// Every tile in hand becomes a flower tile.
    Wildflower,
    /// Every tile in hand becomes a copy of a random tile already in hand.
    Conformity,
}

impl TalismanKind {
    pub fn all() -> &'static [TalismanKind] {
        &[
            TalismanKind::Pearl,
            TalismanKind::Gilded,
            TalismanKind::Polychrome,
            TalismanKind::Souzu,
            TalismanKind::Pinzu,
            TalismanKind::Manzu,
            TalismanKind::Honors,
            TalismanKind::Wildflower,
            TalismanKind::Conformity,
        ]
    }

    pub fn name(self) -> &'static str {
        talisman_presentation(self).name
    }

    /// One-line shop tooltip.
    pub fn description(self) -> &'static str {
        talisman_presentation(self).description
    }

    /// The enhancement this talisman stamps onto each hand tile.
    /// Returns `None` for talismans that transform or destroy tiles instead.
    pub fn enhancement(self) -> Option<TileEnhancement> {
        match self {
            TalismanKind::Pearl => Some(TileEnhancement::Pearl),
            TalismanKind::Gilded => Some(TileEnhancement::Gilded),
            TalismanKind::Polychrome => Some(TileEnhancement::Polychrome),
            TalismanKind::Souzu
            | TalismanKind::Pinzu
            | TalismanKind::Manzu
            | TalismanKind::Honors
            | TalismanKind::Wildflower
            | TalismanKind::Conformity => None,
        }
    }

    /// Tint color for this talisman's tablet. Drives the `base_color`
    /// passed to `talisman_material`, biasing the chitin thin-film palette.
    pub fn accent_color(self) -> [f32; 4] {
        talisman_presentation(self).accent
    }

    /// Flat shop price in gold. Suit / transform talismans share one tier.
    pub fn shop_price(self) -> u32 {
        talisman_presentation(self).shop_price
    }

    /// JSON / asset stem (`pearl`, `gilded`, …).
    pub fn asset_slug(self) -> &'static str {
        match self {
            Self::Pearl => "pearl",
            Self::Gilded => "gilded",
            Self::Polychrome => "polychrome",
            Self::Souzu => "souzu",
            Self::Pinzu => "pinzu",
            Self::Manzu => "manzu",
            Self::Honors => "honors",
            Self::Wildflower => "wildflower",
            Self::Conformity => "conformity",
        }
    }

    /// Grayscale relief heightmap for the octagonal tablet mesh (`lit_mesh` chitin).
    pub fn heightmap_asset_path(self) -> &'static str {
        match self {
            Self::Pearl => "textures/talismans/talisman_pearl.png",
            Self::Gilded => "textures/talismans/talisman_gilded.png",
            Self::Polychrome => "textures/talismans/talisman_polychrome.png",
            Self::Souzu => "textures/talismans/talisman_souzu.png",
            Self::Pinzu => "textures/talismans/talisman_pinzu.png",
            Self::Manzu => "textures/talismans/talisman_manzu.png",
            Self::Honors => "textures/talismans/talisman_honors.png",
            Self::Wildflower => "textures/talismans/talisman_wildflower.png",
            Self::Conformity => "textures/talismans/talisman_conformity.png",
        }
    }

    /// Octagon silhouette mask (white = tablet, black = void) for chitin discard.
    pub fn mask_asset_path(self) -> &'static str {
        match self {
            Self::Pearl => "textures/talismans/talisman_pearl_mask.png",
            Self::Gilded => "textures/talismans/talisman_gilded_mask.png",
            Self::Polychrome => "textures/talismans/talisman_polychrome_mask.png",
            Self::Souzu => "textures/talismans/talisman_souzu_mask.png",
            Self::Pinzu => "textures/talismans/talisman_pinzu_mask.png",
            Self::Manzu => "textures/talismans/talisman_manzu_mask.png",
            Self::Honors => "textures/talismans/talisman_honors_mask.png",
            Self::Wildflower => "textures/talismans/talisman_wildflower_mask.png",
            Self::Conformity => "textures/talismans/talisman_conformity_mask.png",
        }
    }

    /// `(path, gpu label)` pairs in [`Self::all()`] order — see `build.rs` loader.
    pub fn heightmap_paths() -> &'static [(&'static str, &'static str)] {
        &[
            ("textures/talismans/talisman_pearl.png", "talisman-pearl-hm"),
            ("textures/talismans/talisman_gilded.png", "talisman-gilded-hm"),
            (
                "textures/talismans/talisman_polychrome.png",
                "talisman-polychrome-hm",
            ),
            ("textures/talismans/talisman_souzu.png", "talisman-souzu-hm"),
            ("textures/talismans/talisman_pinzu.png", "talisman-pinzu-hm"),
            (
                "textures/talismans/talisman_manzu.png",
                "talisman-manzu-hm",
            ),
            ("textures/talismans/talisman_honors.png", "talisman-honors-hm"),
            (
                "textures/talismans/talisman_wildflower.png",
                "talisman-wildflower-hm",
            ),
            (
                "textures/talismans/talisman_conformity.png",
                "talisman-conformity-hm",
            ),
        ]
    }

    pub fn mask_paths() -> &'static [(&'static str, &'static str)] {
        &[
            (
                "textures/talismans/talisman_pearl_mask.png",
                "talisman-pearl-mask",
            ),
            (
                "textures/talismans/talisman_gilded_mask.png",
                "talisman-gilded-mask",
            ),
            (
                "textures/talismans/talisman_polychrome_mask.png",
                "talisman-polychrome-mask",
            ),
            (
                "textures/talismans/talisman_souzu_mask.png",
                "talisman-souzu-mask",
            ),
            ("textures/talismans/talisman_pinzu_mask.png", "talisman-pinzu-mask"),
            (
                "textures/talismans/talisman_manzu_mask.png",
                "talisman-manzu-mask",
            ),
            (
                "textures/talismans/talisman_honors_mask.png",
                "talisman-honors-mask",
            ),
            (
                "textures/talismans/talisman_wildflower_mask.png",
                "talisman-wildflower-mask",
            ),
            (
                "textures/talismans/talisman_conformity_mask.png",
                "talisman-conformity-mask",
            ),
        ]
    }
}

/// Apply a talisman to every tile in the given hand. A tile can only carry
/// one talisman mark at a time, so any existing enhancement is **replaced**
/// by the new one (you can re-stamp the hand at will — most-recent wins).
/// Returns the number of tiles that were stamped.
pub fn apply_to_hand(hand: &mut [Tile], kind: TalismanKind) -> usize {
    let Some(enh) = kind.enhancement() else {
        return 0;
    };
    for tile in hand.iter_mut() {
        tile.enhancement = Some(enh);
    }
    hand.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::Suit;

    #[test]
    fn apply_stamps_every_tile() {
        let mut hand = vec![
            Tile::new(Suit::Souzu, 3, 0),
            Tile::new(Suit::Pinzu, 7, 1),
            Tile::new(Suit::Dragon, 1, 2),
        ];
        apply_to_hand(&mut hand, TalismanKind::Pearl);
        for t in &hand {
            assert_eq!(t.enhancement, Some(TileEnhancement::Pearl));
        }
    }

    #[test]
    fn apply_replaces_existing_enhancement() {
        // Each tile can carry only one talisman mark at a time — applying a
        // new talisman overwrites whatever was on the tile before.
        let mut hand = vec![Tile::new(Suit::Souzu, 5, 0)];
        apply_to_hand(&mut hand, TalismanKind::Gilded);
        apply_to_hand(&mut hand, TalismanKind::Pearl);
        assert_eq!(hand[0].enhancement, Some(TileEnhancement::Pearl));
    }

    #[test]
    fn each_enhancement_kind_has_unique_enhancement() {
        let mut seen = rustc_hash::FxHashSet::default();
        for &k in TalismanKind::all() {
            if let Some(e) = k.enhancement() {
                assert!(seen.insert(e), "duplicate for {:?}", k);
            }
        }
    }

    #[test]
    fn souzu_has_no_enhancement() {
        assert_eq!(TalismanKind::Souzu.enhancement(), None);
    }

    /// Every `TalismanKind` variant must appear exactly once in `talismans.json`.
    #[test]
    fn heightmap_and_mask_paths_match_all_kinds() {
        let all = TalismanKind::all();
        assert_eq!(TalismanKind::heightmap_paths().len(), all.len());
        assert_eq!(TalismanKind::mask_paths().len(), all.len());
        for (i, &k) in all.iter().enumerate() {
            let (hm, _) = TalismanKind::heightmap_paths()[i];
            let (mask, _) = TalismanKind::mask_paths()[i];
            assert!(
                hm.ends_with(&format!("talisman_{}.png", k.asset_slug())),
                "height path order mismatch for {k:?}"
            );
            assert!(
                mask.ends_with(&format!("talisman_{}_mask.png", k.asset_slug())),
                "mask path order mismatch for {k:?}"
            );
        }
    }

    #[test]
    fn every_talisman_variant_has_one_data_entry() {
        const ALL: &[TalismanKind] = &[
            TalismanKind::Pearl,
            TalismanKind::Gilded,
            TalismanKind::Polychrome,
            TalismanKind::Souzu,
            TalismanKind::Pinzu,
            TalismanKind::Manzu,
            TalismanKind::Honors,
            TalismanKind::Wildflower,
            TalismanKind::Conformity,
        ];
        let map = talisman_presentations();
        assert_eq!(
            map.len(),
            ALL.len(),
            "talismans.json entry count does not match TalismanKind variant count"
        );
        for &k in ALL {
            let _ = talisman_presentation(k);
        }
    }
}
