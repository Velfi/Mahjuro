//! Tile packs — purchasable booster packs that permanently add extra tiles
//! to the wall for the rest of the run.

use serde::{Deserialize, Serialize};

use super::tile::{Suit, Tile, TileEnhancement};

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
pub enum TilePackKind {
    /// +7 honor tiles: one of each wind (E/S/W/N) + one of each dragon (R/G/W).
    Honors,
    /// +6 terminal tiles: one extra 1 and one extra 9 per numbered suit.
    Terminals,
    /// +4 flower wildcards (F1–F4), doubling the flower pool.
    Flowers,
    /// +8 bamboo suit tiles (ranks 1–8).
    BambooGrove,
    /// +8 circle suit tiles (ranks 1–8).
    CoinCache,
    /// +8 character suit tiles (ranks 1–8).
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
        match self {
            Self::Honors => "Honors Pack",
            Self::Terminals => "Terminals Pack",
            Self::Flowers => "Flowers Pack",
            Self::BambooGrove => "Bamboo Grove",
            Self::CoinCache => "Coin Cache",
            Self::ScrollLibrary => "Scroll Library",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Honors => "+7 honor tiles to the wall (winds + dragons)",
            Self::Terminals => "+6 terminal tiles to the wall (1s and 9s)",
            Self::Flowers => "+4 flower wildcards to the wall",
            Self::BambooGrove => "+8 bamboo tiles to the wall",
            Self::CoinCache => "+8 dots tiles to the wall",
            Self::ScrollLibrary => "+8 character tiles to the wall",
        }
    }

    /// Foil wrapper tint for this pack kind. The pack body is rendered as
    /// shiny metallic foil in this colour; the pack art is composited on top
    /// as a decal via its alpha channel. Chosen to evoke each pack's theme
    /// while staying muted enough that the decal art reads clearly.
    pub fn foil_tint(self) -> [f32; 4] {
        match self {
            // Honors: imperial gold foil.
            Self::Honors => [0.92, 0.78, 0.38, 1.0],
            // Terminals: weathered copper/bronze.
            Self::Terminals => [0.78, 0.52, 0.32, 1.0],
            // Flowers: soft rose-pink foil.
            Self::Flowers => [0.92, 0.62, 0.70, 1.0],
            // Bamboo Grove: jade green.
            Self::BambooGrove => [0.48, 0.78, 0.52, 1.0],
            // Coin Cache: polished silver with a cool cast.
            Self::CoinCache => [0.78, 0.82, 0.88, 1.0],
            // Scroll Library: lacquer indigo.
            Self::ScrollLibrary => [0.42, 0.48, 0.78, 1.0],
        }
    }

    /// Asset filename (without directory) for this pack's box art texture.
    pub fn asset_filename(self) -> &'static str {
        match self {
            Self::Honors => "pack_honors.png",
            Self::Terminals => "pack_terminals.png",
            Self::Flowers => "pack_flowers.png",
            Self::BambooGrove => "pack_bamboo_grove.png",
            Self::CoinCache => "pack_coin_cache.png",
            Self::ScrollLibrary => "pack_scroll_library.png",
        }
    }

    /// Wax-seal color for the merchant-envelope detail centered on the pack
    /// face. Each pack picks a wax tone that contrasts with its [`foil_tint`]
    /// so the seal reads as a focal point at thumbnail size — typically a
    /// deep, slightly warm red, shifted per kind to harmonize with the
    /// wrapper. Used by the seal-baking pipeline (`scripts/bake_pack_seals.py`)
    /// and as the hover-glow accent for the pack on the shop counter.
    pub fn seal_color(self) -> [f32; 4] {
        match self {
            // Honors (gold foil) — imperial cinnabar.
            Self::Honors => [0.74, 0.18, 0.16, 1.0],
            // Terminals (copper foil) — oxblood, deeper than the wrapper.
            Self::Terminals => [0.56, 0.14, 0.12, 1.0],
            // Flowers (rose foil) — plum wax so it doesn't melt into the wrapper.
            Self::Flowers => [0.52, 0.14, 0.30, 1.0],
            // Bamboo (jade foil) — vermilion, classic temple-stamp red.
            Self::BambooGrove => [0.78, 0.18, 0.14, 1.0],
            // Coin Cache (silver foil) — burgundy.
            Self::CoinCache => [0.58, 0.10, 0.18, 1.0],
            // Scroll Library (indigo foil) — sealing-wax red.
            Self::ScrollLibrary => [0.72, 0.18, 0.18, 1.0],
        }
    }

    pub fn shop_price(self) -> u32 {
        match self {
            Self::Flowers => 5,
            Self::Honors | Self::Terminals => 6,
            Self::BambooGrove | Self::CoinCache | Self::ScrollLibrary => 7,
        }
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
                // distribution is randomized (could be 3 bamboo-1s and no
                // character-1, etc.)
                let suits = [Suit::Characters, Suit::Bamboos, Suit::Circles];
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
                    push(Suit::Bamboos, rank, &mut out);
                }
            }
            Self::CoinCache => {
                let mut ranks: Vec<u8> = (1..=9).collect();
                seeded_shuffle(&mut ranks, rng_state);
                for &rank in ranks.iter().take(8) {
                    push(Suit::Circles, rank, &mut out);
                }
            }
            Self::ScrollLibrary => {
                let mut ranks: Vec<u8> = (1..=9).collect();
                seeded_shuffle(&mut ranks, rng_state);
                for &rank in ranks.iter().take(8) {
                    push(Suit::Characters, rank, &mut out);
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
