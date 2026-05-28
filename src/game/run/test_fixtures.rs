//! Shared deterministic run state and structure fixtures for unit tests.

use crate::core::deck::{Wall, build_wall};
use crate::core::hand::{DetectedMeld, MeldKind};
use crate::core::tile::{Suit, Tile};
use crate::game::game_mode::{GameMode, HAND_SIZE};
use crate::game::run::RunState;

/// Standard 14-tile winning shape (pair + three sequences + triplet) with stable tile ids.
pub fn winning_structure() -> (Vec<Tile>, Vec<DetectedMeld>) {
    let tiles = vec![
        Tile::new(Suit::Manzu, 1, 1),
        Tile::new(Suit::Manzu, 1, 2),
        Tile::new(Suit::Manzu, 2, 3),
        Tile::new(Suit::Manzu, 3, 4),
        Tile::new(Suit::Manzu, 4, 5),
        Tile::new(Suit::Pinzu, 2, 6),
        Tile::new(Suit::Pinzu, 3, 7),
        Tile::new(Suit::Pinzu, 4, 8),
        Tile::new(Suit::Souzu, 5, 9),
        Tile::new(Suit::Souzu, 6, 10),
        Tile::new(Suit::Souzu, 7, 11),
        Tile::new(Suit::Wind, 1, 12),
        Tile::new(Suit::Wind, 1, 13),
        Tile::new(Suit::Wind, 1, 14),
    ];
    let sets = vec![
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![1, 2],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![3, 4, 5],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![6, 7, 8],
        },
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![9, 10, 11],
        },
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![12, 13, 14],
        },
    ];
    (tiles, sets)
}

/// Run with zero starting yen, no starting rules/yaku, and a deterministic unshuffled wall.
pub fn test_run() -> RunState {
    let mode = GameMode {
        starting_yen: 0,
        starting_rules: vec![],
        starting_yaku: vec![],
        ..GameMode::standard()
    };
    deterministic_run_with_mode(mode)
}

/// Unshuffled wall + first `HAND_SIZE` tiles drawn; structure bank cleared.
pub fn deterministic_run_with_mode(mode: GameMode) -> RunState {
    let mut run = RunState::new(mode);
    let tiles = build_wall();
    let mut wall = Wall::from_unshuffled(tiles);
    let mut hand = Vec::with_capacity(HAND_SIZE);
    for _ in 0..HAND_SIZE {
        if let Some(t) = wall.draw() {
            hand.push(t);
        }
    }
    run.wall = wall;
    run.hand = hand;
    run.selected = vec![false; run.hand.len()];
    run.structure_sets.clear();
    run.structure_tiles.clear();
    run.last_breakdown = None;
    run
}
