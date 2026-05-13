use super::*;
use crate::core::tile::{Suit, Tile};
use proptest::prelude::*;
use std::collections::HashSet;

const NUMBER_SUITS: [Suit; 3] = [Suit::Characters, Suit::Bamboos, Suit::Circles];
const ALL_SUITS: [Suit; 5] = [
    Suit::Characters,
    Suit::Bamboos,
    Suit::Circles,
    Suit::Wind,
    Suit::Dragon,
];

/// Generate a random valid meld (pair, triplet, or sequence) starting from `id_start`.
fn arb_meld(id_start: u32) -> BoxedStrategy<Vec<Tile>> {
    prop_oneof![
        // Pair: same suit+rank, 2 tiles
        (0..5usize, 1..=9u8).prop_map(move |(si, rank)| {
            let suit = ALL_SUITS[si];
            let rank = match suit {
                Suit::Wind => (rank - 1) % 4 + 1,
                Suit::Dragon => (rank - 1) % 3 + 1,
                _ => rank,
            };
            vec![
                Tile::new(suit, rank, id_start),
                Tile::new(suit, rank, id_start + 1),
            ]
        }),
        // Triplet: same suit+rank, 3 tiles
        (0..5usize, 1..=9u8).prop_map(move |(si, rank)| {
            let suit = ALL_SUITS[si];
            let rank = match suit {
                Suit::Wind => (rank - 1) % 4 + 1,
                Suit::Dragon => (rank - 1) % 3 + 1,
                _ => rank,
            };
            vec![
                Tile::new(suit, rank, id_start),
                Tile::new(suit, rank, id_start + 1),
                Tile::new(suit, rank, id_start + 2),
            ]
        }),
        // Sequence: consecutive ranks in a number suit, 3 tiles
        (0..3usize, 1..=7u8).prop_map(move |(si, start)| {
            let suit = NUMBER_SUITS[si];
            vec![
                Tile::new(suit, start, id_start),
                Tile::new(suit, start + 1, id_start + 1),
                Tile::new(suit, start + 2, id_start + 2),
            ]
        }),
    ]
    .boxed()
}

/// Generate a hand composed of 1..=5 valid melds (known-valid by construction).
fn arb_valid_hand() -> BoxedStrategy<Vec<Tile>> {
    (1..=5usize)
        .prop_flat_map(|count| {
            // Generate `count` melds with non-overlapping id ranges.
            // Each meld uses at most 3 ids; space them by 3.
            let strategies: Vec<_> = (0..count).map(|i| arb_meld(i as u32 * 3)).collect();
            strategies
        })
        .prop_map(|melds| {
            let mut tiles = Vec::new();
            let mut next_id = 0u32;
            for meld in melds {
                for t in meld {
                    tiles.push(Tile::new(t.suit, t.rank, next_id));
                    next_id += 1;
                }
            }
            tiles
        })
        .boxed()
}

/// Generate a random tile for fuzz testing.
fn arb_tile(id: u32) -> BoxedStrategy<Tile> {
    (0..5usize, 1..=9u8)
        .prop_map(move |(si, rank)| {
            let suit = ALL_SUITS[si];
            let rank = match suit {
                Suit::Wind => (rank - 1) % 4 + 1,
                Suit::Dragon => (rank - 1) % 3 + 1,
                _ => rank,
            };
            Tile::new(suit, rank, id)
        })
        .boxed()
}

/// Generate a random hand of 1..=14 tiles (may or may not be valid).
fn arb_random_hand() -> BoxedStrategy<Vec<Tile>> {
    (1..=14usize)
        .prop_flat_map(|n| {
            let strats: Vec<_> = (0..n).map(|i| arb_tile(i as u32)).collect();
            strats
        })
        .boxed()
}

// ── Property: known-valid hands always pass ────────────────────

proptest! {
    #[test]
    fn valid_hands_always_accepted(tiles in arb_valid_hand()) {
        let result = validate_selection(&tiles);
        prop_assert!(result.is_some(), "constructed valid hand was rejected: {:?}", tiles);
    }
}

// ── Property: if accepted, all tile IDs are covered exactly once ─

proptest! {
    #[test]
    fn accepted_covers_all_ids(tiles in arb_random_hand()) {
        if let Some(sets) = validate_selection(&tiles) {
            let input_ids: HashSet<u32> = tiles.iter().map(|t| t.id).collect();
            let mut output_ids = Vec::new();
            for s in &sets {
                output_ids.extend(&s.tile_ids);
            }
            let output_set: HashSet<u32> = output_ids.iter().copied().collect();

            // Every input tile accounted for.
            prop_assert_eq!(&input_ids, &output_set, "tile IDs mismatch");
            // No duplicates in output.
            prop_assert_eq!(output_ids.len(), output_set.len(), "duplicate tile IDs in sets");
        }
    }
}

// ── Property: if accepted, each set is genuinely valid ─────────

proptest! {
    #[test]
    fn accepted_sets_are_genuine(tiles in arb_random_hand()) {
        if let Some(sets) = validate_selection(&tiles) {
            for s in &sets {
                let set_tiles: Vec<&Tile> = s.tile_ids.iter()
                    .filter_map(|id| tiles.iter().find(|t| t.id == *id))
                    .collect();

                match s.kind {
                    MeldKind::Pair => {
                        prop_assert_eq!(set_tiles.len(), 2);
                        prop_assert_eq!(set_tiles[0].suit, set_tiles[1].suit);
                        prop_assert_eq!(set_tiles[0].rank, set_tiles[1].rank);
                    }
                    MeldKind::Triplet => {
                        prop_assert_eq!(set_tiles.len(), 3);
                        prop_assert_eq!(set_tiles[0].suit, set_tiles[1].suit);
                        prop_assert_eq!(set_tiles[0].suit, set_tiles[2].suit);
                        prop_assert_eq!(set_tiles[0].rank, set_tiles[1].rank);
                        prop_assert_eq!(set_tiles[0].rank, set_tiles[2].rank);
                    }
                    MeldKind::Sequence => {
                        prop_assert_eq!(set_tiles.len(), 3);
                        let mut ranks: Vec<u8> = set_tiles.iter().map(|t| t.rank).collect();
                        ranks.sort();
                        prop_assert_eq!(set_tiles[0].suit, set_tiles[1].suit);
                        prop_assert_eq!(set_tiles[0].suit, set_tiles[2].suit);
                        prop_assert_eq!(ranks[1], ranks[0] + 1, "sequence not consecutive");
                        prop_assert_eq!(ranks[2], ranks[1] + 1, "sequence not consecutive");
                    }
                    MeldKind::Kong => {
                        prop_assert_eq!(set_tiles.len(), 4);
                        for i in 1..4 {
                            prop_assert_eq!(set_tiles[0].suit, set_tiles[i].suit);
                            prop_assert_eq!(set_tiles[0].rank, set_tiles[i].rank);
                        }
                    }
                    MeldKind::Single => {
                        prop_assert_eq!(set_tiles.len(), 1);
                    }
                }
            }
        }
    }
}

// ── Property: tile count invariant ─────────────────────────────

proptest! {
    #[test]
    fn accepted_tile_count_matches(tiles in arb_random_hand()) {
        if let Some(sets) = validate_selection(&tiles) {
            let total: usize = sets.iter().map(|s| s.tile_ids.len()).sum();
            prop_assert_eq!(total, tiles.len(), "set tile count != input tile count");
        }
    }
}

// ── Property: single tile always rejected ──────────────────────

proptest! {
    #[test]
    fn single_tile_rejected(tile in arb_tile(0)) {
        prop_assert!(validate_selection(&[tile]).is_none());
    }
}
