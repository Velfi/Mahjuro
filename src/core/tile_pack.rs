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
            Self::CoinCache => "+8 circle tiles to the wall",
            Self::ScrollLibrary => "+8 character tiles to the wall",
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

    pub fn shop_price(self) -> u32 {
        match self {
            Self::Honors => 7,
            Self::Terminals => 8,
            Self::Flowers => 6,
            Self::BambooGrove | Self::CoinCache | Self::ScrollLibrary => 8,
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
