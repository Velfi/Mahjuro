//! Tile packs — purchasable booster packs that permanently add extra tiles
//! to the wall for the rest of the run.
//!
//! Shop copy, prices, and box-art filenames live in `assets/data/tile_packs.json`.
//! Tile generation (`generate_tiles`) and foil / seal colors (see
//! [`crate::render::pack_palette`]) stay in Rust.

use std::collections::HashMap;
use std::sync::OnceLock;

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

/// Cheap deterministic PRNG seeded from a u32 (xorshift32).
fn pack_rng_next(state: &mut u32) -> u32 {
    let mut s = *state;
    s ^= s << 13;
    s ^= s >> 17;
    s ^= s << 5;
    *state = s;
    s
}

/// Shuffle a small slice in-place using a seeded xorshift.
fn seeded_shuffle<T>(slice: &mut [T], seed: u32) {
    let mut state = seed.wrapping_add(1).max(1); // ensure non-zero
    for i in (1..slice.len()).rev() {
        let j = (pack_rng_next(&mut state) as usize) % (i + 1);
        slice.swap(i, j);
    }
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
    #[serde(alias = "BambooGrove")]
    BambooGrove,
    #[serde(alias = "CoinCache")]
    CoinCache,
    #[serde(alias = "ScrollLibrary")]
    ScrollLibrary,
}

impl TilePackKind {
    pub fn all() -> &'static [Self] {
        &[
            Self::Honors,
            Self::Terminals,
            Self::Flowers,
            Self::BambooGrove,
            Self::CoinCache,
            Self::ScrollLibrary,
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
    /// per-pack table in [`crate::render::pack_palette`].
    pub fn foil_tint(self) -> [f32; 4] {
        crate::render::pack_palette::for_kind(self).foil
    }

    /// Asset filename (without directory) for this pack's box art texture.
    /// Must match the slug baked into [`crate::render::pack_palette`] and
    /// the files under `assets/textures/tile_packs/`.
    pub fn asset_filename(self) -> &'static str {
        tile_pack_presentation(self).texture_file
    }

    /// Wax-seal color for the merchant-envelope detail centered on the
    /// pack face. Used both at bake time (`scripts/bake_pack_seals.py`,
    /// reading `tools/pack_palette.json`) and at runtime as the hover
    /// halo on the shop counter — the two MUST agree, which is why the
    /// canonical value lives in [`crate::render::pack_palette`].
    ///
    /// Each pack nudges off canonical `RUBY` so the seals read as a
    /// *family* of ceremonial reds rather than six unrelated splotches.
    pub fn seal_color(self) -> [f32; 4] {
        crate::render::pack_palette::for_kind(self).seal
    }

    pub fn shop_price(self) -> u32 {
        tile_pack_presentation(self).shop_price
    }

    /// Generate the extra tiles for this pack. IDs start at `start_id` and
    /// increment sequentially. The caller is responsible for choosing a
    /// non-colliding `start_id` (see [`PACK_TILE_ID_BASE`] / [`PACK_ID_STRIDE`]).
    pub fn generate_tiles(self, start_id: u32) -> Vec<Tile> {
        let mut id = start_id;
        let mut out = Vec::new();
        let mut push = |suit: Suit, rank: u8, tiles: &mut Vec<Tile>| {
            tiles.push(Tile::new(suit, rank, id));
            id += 1;
        };

        // Use start_id as a deterministic seed so composition varies per
        // pack slot but stays stable across rounds and save/load.
        let mut rng_state = start_id.wrapping_mul(2654435761).max(1);

        match self {
            Self::Honors => {
                // 7 tiles drawn from the honor pool.  Build a pool of all 7
                // unique honors, shuffle, then pick how many winds vs dragons
                // to include — sometimes you get 2 of a dragon, sometimes an
                // extra wind instead.
                let mut pool: Vec<(Suit, u8)> = Vec::new();
                for rank in 1..=4 {
                    pool.push((Suit::Wind, rank));
                }
                for rank in 1..=3 {
                    pool.push((Suit::Dragon, rank));
                }
                // Add duplicates of a few random entries to create variety
                let dup1 = (pack_rng_next(&mut rng_state) as usize) % pool.len();
                let dup2 = (pack_rng_next(&mut rng_state) as usize) % pool.len();
                pool.push(pool[dup1]);
                pool.push(pool[dup2]);
                seeded_shuffle(&mut pool, rng_state);
                for &(suit, rank) in pool.iter().take(7) {
                    push(suit, rank, &mut out);
                }
            }
            Self::Terminals => {
                // 6 terminal tiles — still all 1s and 9s, but the suit
                // distribution is randomized (could be 3 souzu-1s and no
                // character-1, etc.)
                let suits = [Suit::Manzu, Suit::Souzu, Suit::Pinzu];
                for &terminal_rank in &[1u8, 9] {
                    for _ in 0..3 {
                        let suit = suits[(pack_rng_next(&mut rng_state) as usize) % 3];
                        push(suit, terminal_rank, &mut out);
                    }
                }
            }
            Self::Flowers => {
                for rank in 1..=4 {
                    push(Suit::Flower, rank, &mut out);
                }
            }
            Self::BambooGrove => {
                let mut ranks: Vec<u8> = (1..=9).collect();
                seeded_shuffle(&mut ranks, rng_state);
                for &rank in ranks.iter().take(8) {
                    push(Suit::Souzu, rank, &mut out);
                }
            }
            Self::CoinCache => {
                let mut ranks: Vec<u8> = (1..=9).collect();
                seeded_shuffle(&mut ranks, rng_state);
                for &rank in ranks.iter().take(8) {
                    push(Suit::Pinzu, rank, &mut out);
                }
            }
            Self::ScrollLibrary => {
                let mut ranks: Vec<u8> = (1..=9).collect();
                seeded_shuffle(&mut ranks, rng_state);
                for &rank in ranks.iter().take(8) {
                    push(Suit::Manzu, rank, &mut out);
                }
            }
        }

        out
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
    use crate::render::pack_palette;

    #[test]
    fn every_tile_pack_variant_has_one_data_entry() {
        const ALL: &[TilePackKind] = &[
            TilePackKind::Honors,
            TilePackKind::Terminals,
            TilePackKind::Flowers,
            TilePackKind::BambooGrove,
            TilePackKind::CoinCache,
            TilePackKind::ScrollLibrary,
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
}
