//! Tile packs — purchasable booster packs that permanently add extra tiles
//! to the wall for the rest of the run.
//!
//! Shop copy, prices, and box-art filenames live in `assets/data/tile_packs.json`.
//! Tile generation (`roll_faces` / [`TilePackInstance::tiles_at`]) and foil /
//! seal colors (see [`crate::pack_palette`]) stay in Rust.

use std::collections::HashMap;
use std::sync::OnceLock;

use rand::RngExt;
use rand::prelude::{IndexedRandom, SliceRandom};
use serde::{Deserialize, Serialize};

use super::tile::{Suit, Tile, TileEnhancement};
use crate::core::json_asset::load_json_asset;

#[derive(Deserialize)]
struct TilePackPresentationRaw {
    id: TilePackKind,
    name: String,
    description: String,
    shop_price: u32,
    texture_file: String,
}

struct TilePackPresentation {
    name: &'static str,
    description: &'static str,
    shop_price: u32,
    texture_file: &'static str,
}

fn tile_pack_presentations() -> &'static HashMap<TilePackKind, TilePackPresentation> {
    static MAP: OnceLock<HashMap<TilePackKind, TilePackPresentation>> = OnceLock::new();
    MAP.get_or_init(|| {
        const PATH: &str = "data/tile_packs.json";
        let raw: Vec<TilePackPresentationRaw> = load_json_asset(PATH, "tile pack data");
        raw.into_iter()
            .map(|r| {
                (
                    r.id,
                    TilePackPresentation {
                        name: Box::leak(r.name.into_boxed_str()),
                        description: Box::leak(r.description.into_boxed_str()),
                        shop_price: r.shop_price,
                        texture_file: Box::leak(r.texture_file.into_boxed_str()),
                    },
                )
            })
            .collect()
    })
}

fn tile_pack_presentation(kind: TilePackKind) -> &'static TilePackPresentation {
    tile_pack_presentations()
        .get(&kind)
        .unwrap_or_else(|| panic!("tile pack data missing for {kind:?}"))
}

/// Canonical aspect ratio (width / height) of booster pack cover art.
/// All pack textures are authored at 256×384 (2:3 portrait).
pub const PACK_ASPECT_W_OVER_H: f32 = 2.0 / 3.0;

/// ID offset for pack-generated tiles. Standard wall uses 0–139.
pub const PACK_TILE_ID_BASE: u32 = 1000;
/// Each pack instance gets a block of 16 IDs (more than any single pack needs).
pub const PACK_ID_STRIDE: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TilePackKind {
    #[serde(alias = "Honors")]
    Honors,
    #[serde(alias = "Terminals")]
    Terminals,
    #[serde(alias = "Flowers")]
    Flowers,
    #[serde(alias = "Souzu")]
    Souzu,
    #[serde(alias = "Pinzu")]
    Pinzu,
    #[serde(alias = "Manzu")]
    Manzu,
}

/// One purchased pack: its kind plus the tile faces rolled at buy time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TilePackInstance {
    pub kind: TilePackKind,
    pub faces: Vec<(Suit, u8)>,
}

impl TilePackInstance {
    /// Roll a fresh random composition for this pack kind.
    pub fn new(kind: TilePackKind) -> Self {
        Self {
            kind,
            faces: kind.roll_faces(),
        }
    }

    /// Build wall tiles for this instance at the given ID block.
    pub fn tiles_at(&self, start_id: u32) -> Vec<Tile> {
        faces_to_tiles(&self.faces, start_id)
    }
}

fn faces_to_tiles(faces: &[(Suit, u8)], start_id: u32) -> Vec<Tile> {
    faces
        .iter()
        .enumerate()
        .map(|(i, &(suit, rank))| Tile::new(suit, rank, start_id + i as u32))
        .collect()
}

impl TilePackKind {
    pub fn all() -> &'static [Self] {
        &[
            Self::Honors,
            Self::Terminals,
            Self::Flowers,
            Self::Souzu,
            Self::Pinzu,
            Self::Manzu,
        ]
    }

    pub fn name(self) -> &'static str {
        tile_pack_presentation(self).name
    }

    pub fn description(self) -> &'static str {
        tile_pack_presentation(self).description
    }

    /// Plastic sleeve tint on pack edges — multiplied under the cover
    /// decal's transparent regions. Sourced from the canonical
    /// per-pack table in [`crate::pack_palette`].
    pub fn foil_tint(self) -> [f32; 4] {
        crate::pack_palette::for_kind(self).foil
    }

    /// Asset filename (without directory) for this pack's box art texture.
    /// Must match the slug baked into [`crate::pack_palette`] and
    /// the files under `assets/textures/tile_packs/`.
    pub fn asset_filename(self) -> &'static str {
        tile_pack_presentation(self).texture_file
    }

    /// Wax-seal color for the merchant-envelope detail centered on the pack
    /// face. Matches cover art and the runtime hover halo on the shop
    /// counter — canonical value in [`crate::pack_palette`].
    ///
    /// Each pack nudges off canonical `RUBY` so the seals read as a
    /// *family* of ceremonial reds rather than six unrelated splotches.
    pub fn seal_color(self) -> [f32; 4] {
        crate::pack_palette::for_kind(self).seal
    }

    pub fn shop_price(self) -> u32 {
        tile_pack_presentation(self).shop_price
    }

    /// Roll a random tile composition for this pack kind.
    pub fn roll_faces(self) -> Vec<(Suit, u8)> {
        let mut rng = rand::rng();

        match self {
            Self::Honors => {
                const HONORS: [(Suit, u8); 7] = [
                    (Suit::Wind, 1),
                    (Suit::Wind, 2),
                    (Suit::Wind, 3),
                    (Suit::Wind, 4),
                    (Suit::Dragon, 1),
                    (Suit::Dragon, 2),
                    (Suit::Dragon, 3),
                ];
                (0..4)
                    .map(|_| *HONORS.choose(&mut rng).expect("honor pool"))
                    .collect()
            }
            Self::Terminals => {
                let suits = [Suit::Manzu, Suit::Souzu, Suit::Pinzu];
                let mut faces = Vec::with_capacity(6);
                for &terminal_rank in &[1u8, 9] {
                    for _ in 0..3 {
                        let suit = suits[rng.random_range(0..3)];
                        faces.push((suit, terminal_rank));
                    }
                }
                faces
            }
            Self::Flowers => (1..=4).map(|rank| (Suit::Flower, rank)).collect(),
            Self::Souzu | Self::Pinzu | Self::Manzu => {
                let suit = match self {
                    Self::Souzu => Suit::Souzu,
                    Self::Pinzu => Suit::Pinzu,
                    Self::Manzu => Suit::Manzu,
                    _ => unreachable!(),
                };
                let mut ranks: Vec<u8> = (1..=9).collect();
                ranks.shuffle(&mut rng);
                ranks.into_iter().take(8).map(|rank| (suit, rank)).collect()
            }
        }
    }

    /// The enhancement that should be pre-stamped on this pack's tiles
    /// at purchase time, if any.
    pub fn pre_enhancement(self) -> Option<TileEnhancement> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack_palette;

    #[test]
    fn every_tile_pack_variant_has_one_data_entry() {
        const ALL: &[TilePackKind] = &[
            TilePackKind::Honors,
            TilePackKind::Terminals,
            TilePackKind::Flowers,
            TilePackKind::Souzu,
            TilePackKind::Pinzu,
            TilePackKind::Manzu,
        ];
        let map = tile_pack_presentations();
        assert_eq!(
            map.len(),
            ALL.len(),
            "tile_packs.json entry count does not match TilePackKind variant count"
        );
        for &k in ALL {
            let _ = tile_pack_presentation(k);
        }
    }

    #[test]
    fn pack_display_names_match_pack_palette() {
        for &k in TilePackKind::all() {
            assert_eq!(
                k.name(),
                pack_palette::for_kind(k).display_name,
                "tile_packs.json name must match pack_palette for {:?}",
                k
            );
        }
    }

    #[test]
    fn roll_faces_produces_expected_counts() {
        for &kind in TilePackKind::all() {
            let faces = kind.roll_faces();
            let expected = match kind {
                TilePackKind::Honors => 4,
                TilePackKind::Terminals => 6,
                TilePackKind::Flowers => 4,
                TilePackKind::Souzu | TilePackKind::Pinzu | TilePackKind::Manzu => 8,
            };
            assert_eq!(faces.len(), expected, "{kind:?}");
        }
    }

    #[test]
    fn instance_tiles_at_matches_rolled_faces() {
        let instance = TilePackInstance::new(TilePackKind::Souzu);
        let tiles = instance.tiles_at(PACK_TILE_ID_BASE);
        assert_eq!(tiles.len(), instance.faces.len());
        for (tile, &(suit, rank)) in tiles.iter().zip(instance.faces.iter()) {
            assert_eq!(tile.suit, suit);
            assert_eq!(tile.rank, rank);
        }
    }
}
