//! Wall / deck construction and draw.

use std::sync::Arc;

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

    for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu] {
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

    for suit in [Suit::Manzu, Suit::Souzu, Suit::Pinzu] {
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

/// Draw pile. The tile list is shared via [`Arc`] across planner checkpoints;
/// only `cursor` (and rarely tile order after `draw_matching`) vary per branch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wall {
    #[serde(
        serialize_with = "serialize_tiles_arc",
        deserialize_with = "deserialize_tiles_arc"
    )]
    tiles: Arc<Vec<Tile>>,
    cursor: usize,
    /// Dora tiles, displayed on the plinth and used to score the bonus.
    /// Stored as the actual dora face (not the traditional +1 indicator),
    /// so the tile the player sees is the tile that pays out.
    dora_indicators: Vec<Tile>,
}

fn serialize_tiles_arc<S: serde::Serializer>(
    tiles: &Arc<Vec<Tile>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    tiles.as_ref().serialize(serializer)
}

fn deserialize_tiles_arc<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Arc<Vec<Tile>>, D::Error> {
    Vec::<Tile>::deserialize(deserializer).map(Arc::new)
}

/// Traditional mahjong picks an "indicator" tile and the dora is the next rank.
/// We skip that indirection: given a raw wall tile, this returns the dora face
/// directly (preserving the source tile's id so dedup and wall-tracking still
/// work).
fn dora_from_wall_pick(t: Tile) -> Tile {
    let next_rank = match t.suit {
        Suit::Manzu | Suit::Souzu | Suit::Pinzu => {
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
            tiles: Arc::new(tiles),
            cursor: 0,
            dora_indicators,
        }
    }

    /// Create a wall from tiles without shuffling (deterministic draws for tests and fixtures).
    pub fn from_unshuffled(tiles: Vec<Tile>) -> Self {
        let dora_indicators = tiles
            .last()
            .copied()
            .map(dora_from_wall_pick)
            .into_iter()
            .collect();
        Self {
            tiles: Arc::new(tiles),
            cursor: 0,
            dora_indicators,
        }
    }

    pub fn from_standard_shuffled() -> Self {
        Self::new(build_wall())
    }

    /// Build a wall with tile-pack extras injected, then filter removed IDs.
    /// Pack tiles get pre-stamped enhancements from `enhancements` if any
    /// pack opts in via `TilePackKind::pre_enhancement`.
    /// When `overflow` is true, 2 extra copies per tile face are added with
    /// IDs starting at [`OVERFLOW_TILE_ID_BASE`] so they can be stripped
    /// mid-round if the relic is lost.
    pub fn from_filtered_with_packs(
        removed: &rustc_hash::FxHashSet<u32>,
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

    /// The tile faces that count as dora (same faces shown on the plinth).
    pub fn dora_faces(&self) -> Vec<(Suit, u8)> {
        self.dora_indicators
            .iter()
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
        let tiles = Arc::make_mut(&mut self.tiles);
        let pos = tiles[self.cursor..]
            .iter()
            .position(|t| t.suit == suit && t.rank == rank)?;
        let idx = self.cursor + pos;
        tiles.swap(self.cursor, idx);
        let t = tiles[self.cursor];
        self.cursor += 1;
        Some(t)
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

    #[test]
    fn dora_faces_include_flower_indicators() {
        let mut wall = Wall::from_unshuffled(build_wall());
        wall.set_sole_dora(Suit::Flower, 2);
        assert_eq!(wall.dora_faces(), vec![(Suit::Flower, 2)]);
        assert_eq!(
            wall.dora_indicator_tiles(),
            &[Tile::new(Suit::Flower, 2, wall.dora_indicator_tiles()[0].id)]
        );
    }

    #[test]
    fn reveal_extra_dora_can_pick_flower_from_wall() {
        let mut tiles = build_wall();
        let flower = Tile::new(Suit::Flower, 3, 9999);
        tiles.push(flower);
        let mut wall = Wall::from_unshuffled(tiles);
        wall.set_sole_dora(Suit::Manzu, 1);
        wall.reveal_extra_dora_indicator();
        assert!(
            wall.dora_faces()
                .iter()
                .any(|&(s, r)| s == Suit::Flower && r == 3),
            "expected flower dora from back-of-wall pick, got {:?}",
            wall.dora_faces()
        );
    }

    #[test]
    fn wall_clone_shares_tile_storage() {
        let mut wall = Wall::from_unshuffled(build_wall());
        let checkpoint = wall.clone();
        assert!(Arc::ptr_eq(&wall.tiles, &checkpoint.tiles));
        assert_eq!(wall.cursor, checkpoint.cursor);

        wall.draw();
        assert_eq!(wall.cursor, checkpoint.cursor + 1);
        assert_eq!(wall.remaining(), checkpoint.remaining() - 1);
        assert!(Arc::ptr_eq(&wall.tiles, &checkpoint.tiles));

        wall = checkpoint.clone();
        assert_eq!(wall.remaining(), checkpoint.remaining());
    }
}
