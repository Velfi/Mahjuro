//! Tile packs — purchasable booster packs that permanently add extra tiles
//! to the wall for the rest of the run.
//!
//! Shop copy, prices, box-art filenames, and tile roll rules live in
//! `assets/data/tile_packs.json`.

use std::collections::HashMap;
use std::sync::OnceLock;

use rand::prelude::IndexedRandom;
use serde::{Deserialize, Serialize};

use super::tile::{Suit, Tile, TileEnhancement, TileFace};
use crate::core::json_asset::load_json_asset;

/// Draw `count` tiles independently from `pool` (with replacement).
#[derive(Clone, Debug, Deserialize)]
struct TilePackRollDef {
    count: u32,
    pool: Vec<TileFace>,
}

impl TilePackRollDef {
    fn tile_count(&self) -> usize {
        self.count as usize
    }

    fn roll_faces(&self) -> Vec<(Suit, u8)> {
        assert!(
            !self.pool.is_empty(),
            "tile pack roll requires a non-empty pool"
        );
        let mut rng = rand::rng();
        (0..self.count)
            .map(|_| {
                let face = self.pool.choose(&mut rng).expect("non-empty pool");
                (face.suit, face.rank)
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct TilePackDefRaw {
    id: TilePackKind,
    name: String,
    description: String,
    shop_price: u32,
    texture_file: String,
    roll: TilePackRollDef,
}

struct TilePackDef {
    name: &'static str,
    description: &'static str,
    shop_price: u32,
    texture_file: &'static str,
    roll: TilePackRollDef,
}

struct TilePackCatalog {
    order: Vec<TilePackKind>,
    by_kind: HashMap<TilePackKind, TilePackDef>,
}

fn tile_pack_catalog() -> &'static TilePackCatalog {
    static CATALOG: OnceLock<TilePackCatalog> = OnceLock::new();
    CATALOG.get_or_init(|| {
        const PATH: &str = "data/tile_packs.json";
        let raw: Vec<TilePackDefRaw> = load_json_asset(PATH, "tile pack data");
        let mut order = Vec::with_capacity(raw.len());
        let mut by_kind = HashMap::with_capacity(raw.len());
        for entry in raw {
            order.push(entry.id);
            by_kind.insert(
                entry.id,
                TilePackDef {
                    name: Box::leak(entry.name.into_boxed_str()),
                    description: Box::leak(entry.description.into_boxed_str()),
                    shop_price: entry.shop_price,
                    texture_file: Box::leak(entry.texture_file.into_boxed_str()),
                    roll: entry.roll,
                },
            );
        }
        TilePackCatalog { order, by_kind }
    })
}

fn tile_pack_def(kind: TilePackKind) -> &'static TilePackDef {
    tile_pack_catalog()
        .by_kind
        .get(&kind)
        .unwrap_or_else(|| panic!("tile pack data missing for {kind:?}"))
}

/// Neutral multiplier for authored pack box-art on [`Object3dKind::Pack`].
pub const PACK_TEXTURE_TINT: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

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
        &tile_pack_catalog().order
    }

    pub fn name(self) -> &'static str {
        tile_pack_def(self).name
    }

    pub fn description(self) -> &'static str {
        tile_pack_def(self).description
    }

    pub fn tile_count(self) -> usize {
        tile_pack_def(self).roll.tile_count()
    }

    /// Neutral tint for authored box-art on [`Object3dKind::Pack`].
    pub fn pack_texture_tint(self) -> [f32; 4] {
        PACK_TEXTURE_TINT
    }

    /// Celebration backdrop / glow colors for this pack kind.
    pub fn celebration_palette(self) -> crate::pack_palette::PackCelebrationPalette {
        crate::pack_palette::for_kind(self)
    }

    /// Asset filename (without directory) for this pack's box art texture
    /// under `assets/textures/tile_packs/`.
    pub fn asset_filename(self) -> &'static str {
        tile_pack_def(self).texture_file
    }

    pub fn shop_price(self) -> u32 {
        tile_pack_def(self).shop_price
    }

    /// Roll a random tile composition for this pack kind.
    pub fn roll_faces(self) -> Vec<(Suit, u8)> {
        tile_pack_def(self).roll.roll_faces()
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
        let catalog = tile_pack_catalog();
        assert_eq!(
            catalog.by_kind.len(),
            ALL.len(),
            "tile_packs.json entry count does not match TilePackKind variant count"
        );
        for &k in ALL {
            let _ = tile_pack_def(k);
        }
        assert_eq!(catalog.order.len(), ALL.len());
    }

    #[test]
    fn roll_faces_produces_configured_counts() {
        for &kind in TilePackKind::all() {
            let faces = kind.roll_faces();
            assert_eq!(faces.len(), kind.tile_count(), "{kind:?}");
        }
    }

    #[test]
    fn roll_faces_only_draw_from_pool() {
        let pool: std::collections::HashSet<(Suit, u8)> = tile_pack_def(TilePackKind::Terminals)
            .roll
            .pool
            .iter()
            .map(|f| (f.suit, f.rank))
            .collect();
        for _ in 0..32 {
            for face in TilePackKind::Terminals.roll_faces() {
                assert!(pool.contains(&face), "rolled {face:?} not in pool");
            }
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
