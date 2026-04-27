//! Wall / deck construction and draw.

use rand::rng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::core::tile::{Suit, Tile};

/// Tile IDs at or above this value are Overflow extras (5th & 6th copies).
/// Used to strip overflow tiles from the wall and hand if the relic is lost
/// mid-round.
pub const OVERFLOW_TILE_ID_BASE: u32 = 10_000;

/// Standard 140-tile wall (4× each regular tile + 4 unique flower wildcards).
pub fn build_wall() -> Vec<Tile> {
    let mut id = 0u32;
    let mut tiles = Vec::with_capacity(140);

    for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles] {
        for rank in 1..=9 {
            for _ in 0..4 {
                tiles.push(Tile::new(suit, rank, id));
                id += 1;
            }
        }
    }

    for rank in 1..=4 {
        for _ in 0..4 {
            tiles.push(Tile::new(Suit::Wind, rank, id));
            id += 1;
        }
    }

    for rank in 1..=3 {
        for _ in 0..4 {
            tiles.push(Tile::new(Suit::Dragon, rank, id));
            id += 1;
        }
    }

    // 4 unique flower wildcards (one each, not duplicated like regular tiles).
    for rank in 1..=4 {
        tiles.push(Tile::new(Suit::Flower, rank, id));
        id += 1;
    }

    tiles
}

/// Build the 2 extra copies per tile face added by the Overflow relic.
/// IDs start at [`OVERFLOW_TILE_ID_BASE`] so they can be identified and
/// stripped if the relic is lost mid-round.
pub fn build_overflow_extras() -> Vec<Tile> {
    let mut id = OVERFLOW_TILE_ID_BASE;
    // 34 tile faces × 2 extra copies = 68 tiles (no extra flowers).
    let mut tiles = Vec::with_capacity(68);

    for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles] {
        for rank in 1..=9 {
            for _ in 0..2 {
                tiles.push(Tile::new(suit, rank, id));
                id += 1;
            }
        }
    }

    for rank in 1..=4 {
        for _ in 0..2 {
            tiles.push(Tile::new(Suit::Wind, rank, id));
            id += 1;
        }
    }

    for rank in 1..=3 {
        for _ in 0..2 {
            tiles.push(Tile::new(Suit::Dragon, rank, id));
            id += 1;
        }
    }

    tiles
}

pub fn shuffle_wall(wall: &mut [Tile]) {
    wall.shuffle(&mut rng());
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wall {
    tiles: Vec<Tile>,
    cursor: usize,
    /// Dora tiles, displayed on the plinth and used to score the bonus.
    /// Stored as the actual dora face (not the traditional +1 indicator),
    /// so the tile the player sees is the tile that pays out.
    dora_indicators: Vec<Tile>,
}

/// Traditional mahjong picks an "indicator" tile and the dora is the next rank.
/// We skip that indirection: given a raw wall tile, this returns the dora face
/// directly (preserving the source tile's id so dedup and wall-tracking still
/// work).
fn dora_from_wall_pick(t: Tile) -> Tile {
    let next_rank = match t.suit {
        Suit::Characters | Suit::Bamboos | Suit::Circles => {
            if t.rank >= 9 {
                1
            } else {
                t.rank + 1
            }
        }
        Suit::Wind => {
            if t.rank >= 4 {
                1
            } else {
                t.rank + 1
            }
        }
        Suit::Dragon => {
            if t.rank >= 3 {
                1
            } else {
                t.rank + 1
            }
        }
        Suit::Flower | Suit::Season => return t,
    };
    Tile {
        rank: next_rank,
        ..t
    }
}

impl Wall {
    pub fn new(mut tiles: Vec<Tile>) -> Self {
        shuffle_wall(&mut tiles);
        let dora_indicators = tiles
            .last()
            .copied()
            .map(dora_from_wall_pick)
            .into_iter()
            .collect();
        Self {
            tiles,
            cursor: 0,
            dora_indicators,
        }
    }

    /// Create a wall from tiles without shuffling (for deterministic tests).
    #[cfg(test)]
    pub fn from_unshuffled(tiles: Vec<Tile>) -> Self {
        let dora_indicators = tiles
            .last()
            .copied()
            .map(dora_from_wall_pick)
            .into_iter()
            .collect();
        Self {
            tiles,
            cursor: 0,
            dora_indicators,
        }
    }

    pub fn from_standard_shuffled() -> Self {
        Self::new(build_wall())
    }

    /// Build a wall with tile-pack extras injected, then filter removed IDs.
    /// Pack tiles get pre-stamped enhancements from `enhancements` (e.g.
    /// Polychrome pack tiles carry their ×1.2 mult enhancement).
    /// When `overflow` is true, 2 extra copies per tile face are added with
    /// IDs starting at [`OVERFLOW_TILE_ID_BASE`] so they can be stripped
    /// mid-round if the relic is lost.
    pub fn from_filtered_with_packs(
        removed: &std::collections::HashSet<u32>,
        packs: &[crate::core::tile_pack::TilePackKind],
        enhancements: &std::collections::BTreeMap<u32, super::tile::TileEnhancement>,
        overflow: bool,
    ) -> Self {
        use crate::core::tile_pack::{PACK_ID_STRIDE, PACK_TILE_ID_BASE};

        let mut tiles = build_wall();
        if overflow {
            tiles.extend(build_overflow_extras());
        }
        for (i, pack) in packs.iter().enumerate() {
            let start_id = PACK_TILE_ID_BASE + (i as u32) * PACK_ID_STRIDE;
            let mut pack_tiles = pack.generate_tiles(start_id);
            for t in &mut pack_tiles {
                if let Some(&enh) = enhancements.get(&t.id) {
                    t.enhancement = Some(enh);
                }
            }
            tiles.extend(pack_tiles);
        }
        if !removed.is_empty() {
            tiles.retain(|t| !removed.contains(&t.id));
        }
        Self::new(tiles)
    }

    /// The tile faces that count as dora. Flower/Season tiles are never dora.
    pub fn dora_faces(&self) -> Vec<(Suit, u8)> {
        self.dora_indicators
            .iter()
            .filter(|t| !matches!(t.suit, Suit::Flower | Suit::Season))
            .map(|t| (t.suit, t.rank))
            .collect()
    }

    /// The dora indicator tiles themselves (for UI display).
    pub fn dora_indicator_tiles(&self) -> &[Tile] {
        &self.dora_indicators
    }

    /// Replace the dora indicator list with a single tile of the given face.
    /// Reuses the id of an existing wall tile matching the face when possible
    /// so any id-keyed bookkeeping (dedup, enhancement lookups) stays valid;
    /// otherwise synthesizes a high id that won't collide with live tiles.
    pub fn set_sole_dora(&mut self, suit: Suit, rank: u8) {
        let id = self
            .tiles
            .iter()
            .find(|t| t.suit == suit && t.rank == rank)
            .map(|t| t.id)
            .unwrap_or(u32::MAX);
        self.dora_indicators = vec![Tile::new(suit, rank, id)];
    }

    /// Reveal an additional dora (used by the Dora Crown relic). Picks the
    /// next unused wall tile from the back — by id, so it never collides with
    /// an existing pick and never gets drawn into a hand — then stores the
    /// corresponding dora face.
    pub fn reveal_extra_dora_indicator(&mut self) {
        let mut idx = self.tiles.len();
        while idx > 0 {
            idx -= 1;
            let candidate = self.tiles[idx];
            if matches!(candidate.suit, Suit::Flower | Suit::Season) {
                continue;
            }
            if self.dora_indicators.iter().any(|t| t.id == candidate.id) {
                continue;
            }
            self.dora_indicators.push(dora_from_wall_pick(candidate));
            return;
        }
    }

    /// How many tiles remain in the wall.
    pub fn remaining(&self) -> usize {
        self.tiles.len().saturating_sub(self.cursor)
    }

    /// Peek at the next `n` tiles that would be drawn, without consuming them.
    /// Returns fewer than `n` tiles if the wall is nearly empty.
    pub fn peek_next(&self, n: usize) -> &[Tile] {
        let end = (self.cursor + n).min(self.tiles.len());
        &self.tiles[self.cursor..end]
    }

    pub fn draw(&mut self) -> Option<Tile> {
        if self.cursor >= self.tiles.len() {
            return None;
        }
        let t = self.tiles[self.cursor];
        self.cursor += 1;
        Some(t)
    }

    /// Draw the first remaining tile matching the given suit and rank.
    /// If found, swaps it to the cursor position and advances the cursor.
    pub fn draw_matching(&mut self, suit: Suit, rank: u8) -> Option<Tile> {
        let pos = self.tiles[self.cursor..]
            .iter()
            .position(|t| t.suit == suit && t.rank == rank)?;
        let idx = self.cursor + pos;
        self.tiles.swap(self.cursor, idx);
        self.draw()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_count() {
        let w = build_wall();
        assert_eq!(w.len(), 140); // 136 standard + 4 flowers
    }
}
