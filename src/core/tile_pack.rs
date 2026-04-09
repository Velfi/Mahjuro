//! Tile packs — purchasable booster packs that permanently add extra tiles
//! to the wall for the rest of the run.

use serde::{Deserialize, Serialize};

use super::tile::{Suit, Tile, TileEnhancement};

/// ID offset for pack-generated tiles. Standard wall uses 0–139.
pub const PACK_TILE_ID_BASE: u32 = 1000;
/// Each pack instance gets a block of 16 IDs (more than any single pack needs).
pub const PACK_ID_STRIDE: u32 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TilePackKind {
    /// +7 honor tiles: one of each wind (E/S/W/N) + one of each dragon (R/G/W).
    Honors,
    /// +4 random numbered tiles, pre-enhanced with Polychrome (×1.2 mult).
    Polychrome,
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
            Self::Polychrome,
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
            Self::Polychrome => "Polychrome Pack",
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
            Self::Polychrome => "+4 Polychrome numbered tiles to the wall",
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
            Self::Polychrome => "pack_polychrome.png",
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
            Self::Polychrome => 12,
            Self::Terminals => 8,
            Self::Flowers => 6,
            Self::BambooGrove | Self::CoinCache | Self::ScrollLibrary => 8,
        }
    }

    /// Generate the extra tiles for this pack. IDs start at `start_id` and
    /// increment sequentially. The caller is responsible for choosing a
    /// non-colliding `start_id` (see [`PACK_TILE_ID_BASE`] / [`PACK_ID_STRIDE`]).
    ///
    /// Polychrome pack tiles are returned *without* their enhancement —
    /// the caller should stamp `TileEnhancement::Polychrome` via the
    /// `tile_enhancements` map so the enhancement persists across rounds.
    pub fn generate_tiles(self, start_id: u32) -> Vec<Tile> {
        let mut id = start_id;
        let mut out = Vec::new();
        let mut push = |suit: Suit, rank: u8, tiles: &mut Vec<Tile>| {
            tiles.push(Tile::new(suit, rank, id));
            id += 1;
        };

        match self {
            Self::Honors => {
                for rank in 1..=4 {
                    push(Suit::Wind, rank, &mut out);
                }
                for rank in 1..=3 {
                    push(Suit::Dragon, rank, &mut out);
                }
            }
            Self::Polychrome => {
                // One tile from each numbered suit + one random extra.
                // Use a deterministic pattern rather than RNG so the tiles
                // are stable across save/load (IDs are deterministic).
                push(Suit::Characters, 5, &mut out);
                push(Suit::Bamboos, 5, &mut out);
                push(Suit::Circles, 5, &mut out);
                push(Suit::Characters, 9, &mut out);
            }
            Self::Terminals => {
                for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles] {
                    push(suit, 1, &mut out);
                    push(suit, 9, &mut out);
                }
            }
            Self::Flowers => {
                for rank in 1..=4 {
                    push(Suit::Flower, rank, &mut out);
                }
            }
            Self::BambooGrove => {
                for rank in 1..=8 {
                    push(Suit::Bamboos, rank, &mut out);
                }
            }
            Self::CoinCache => {
                for rank in 1..=8 {
                    push(Suit::Circles, rank, &mut out);
                }
            }
            Self::ScrollLibrary => {
                for rank in 1..=8 {
                    push(Suit::Characters, rank, &mut out);
                }
            }
        }

        out
    }

    /// The enhancement that should be pre-stamped on this pack's tiles
    /// at purchase time, if any.
    pub fn pre_enhancement(self) -> Option<TileEnhancement> {
        match self {
            Self::Polychrome => Some(TileEnhancement::Polychrome),
            _ => None,
        }
    }
}
