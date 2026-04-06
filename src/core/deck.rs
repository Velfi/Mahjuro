//! Wall / deck construction and draw.

use rand::rng;
use rand::seq::SliceRandom;

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

pub struct Wall {
    tiles: Vec<Tile>,
    cursor: usize,
}

impl Wall {
    pub fn new(mut tiles: Vec<Tile>) -> Self {
        shuffle_wall(&mut tiles);
        Self { tiles, cursor: 0 }
    }

    /// Create a wall from tiles without shuffling (for deterministic tests).
    #[allow(dead_code)]
    pub fn from_unshuffled(tiles: Vec<Tile>) -> Self {
        Self { tiles, cursor: 0 }
    }

    pub fn from_standard_shuffled() -> Self {
        Self::new(build_wall())
    }

    /// How many tiles remain in the wall.
    pub fn remaining(&self) -> usize {
        self.tiles.len().saturating_sub(self.cursor)
    }

    pub fn draw(&mut self) -> Option<Tile> {
        if self.cursor >= self.tiles.len() {
            return None;
        }
        let t = self.tiles[self.cursor];
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
        assert_eq!(w.len(), 136);
    }
}
