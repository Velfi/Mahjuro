//! Wall / deck construction and draw.

use rand::rng;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};

use crate::core::tile::{Suit, Tile};

/// Standard 136-tile wall (4× each tile, no flowers).
pub fn build_wall() -> Vec<Tile> {
    let mut id = 0u32;
    let mut tiles = Vec::with_capacity(136);

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

    tiles
}

pub fn shuffle_wall(wall: &mut [Tile]) {
    wall.shuffle(&mut rng());
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Wall {
    tiles: Vec<Tile>,
    cursor: usize,
    /// Dora indicator tiles (face determines which tiles are dora).
    dora_indicators: Vec<Tile>,
}

impl Wall {
    pub fn new(mut tiles: Vec<Tile>) -> Self {
        shuffle_wall(&mut tiles);
        // Last tile becomes the dora indicator.
        let indicator = tiles.last().copied();
        let dora_indicators = indicator.into_iter().collect();
        Self {
            tiles,
            cursor: 0,
            dora_indicators,
        }
    }

    /// Create a wall from tiles without shuffling (for deterministic tests).
    #[allow(dead_code)]
    pub fn from_unshuffled(tiles: Vec<Tile>) -> Self {
        let indicator = tiles.last().copied();
        let dora_indicators = indicator.into_iter().collect();
        Self {
            tiles,
            cursor: 0,
            dora_indicators,
        }
    }

    pub fn from_standard_shuffled() -> Self {
        Self::new(build_wall())
    }

    /// The tile faces that count as dora (one rank above each indicator, wrapping).
    pub fn dora_faces(&self) -> Vec<(Suit, u8)> {
        self.dora_indicators
            .iter()
            .map(|t| {
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
                };
                (t.suit, next_rank)
            })
            .collect()
    }

    /// The dora indicator tiles themselves (for UI display).
    pub fn dora_indicator_tiles(&self) -> &[Tile] {
        &self.dora_indicators
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
        assert_eq!(w.len(), 136);
    }
}
