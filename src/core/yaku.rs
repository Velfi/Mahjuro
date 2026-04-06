//! Yaku (hand pattern) detection and bonus scoring.

use crate::core::hand::{DetectedSet, SetKind};
use crate::core::tile::Tile;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum YakuKind {
    /// All melds are triplets (pairs allowed as the one pair).
    AllTriplets,
    /// Every tile is a numbered suit tile with rank 2–8 (no terminals or honors).
    AllSimples,
    /// All tiles belong to a single suit.
    Flush,
    /// Decomposition contains at least one pair, one triplet, and one sequence.
    MixedSets,
    /// Full 14-tile hand: 4 melds + 1 pair.
    FullHand,
}

impl YakuKind {
    pub fn bonus_points(self) -> i32 {
        match self {
            YakuKind::AllTriplets => 100,
            YakuKind::AllSimples => 60,
            YakuKind::Flush => 120,
            YakuKind::MixedSets => 50,
            YakuKind::FullHand => 200,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            YakuKind::AllTriplets => "All Triplets",
            YakuKind::AllSimples => "All Simples",
            YakuKind::Flush => "Flush",
            YakuKind::MixedSets => "Mixed Sets",
            YakuKind::FullHand => "Full Hand",
        }
    }
}

/// Detect all yaku patterns present in a scored hand.
pub fn detect_yaku(tiles: &[Tile], sets: &[DetectedSet]) -> Vec<YakuKind> {
    let mut found = Vec::new();

    if is_all_triplets(sets) {
        found.push(YakuKind::AllTriplets);
    }
    if is_all_simples(tiles) {
        found.push(YakuKind::AllSimples);
    }
    if is_flush(tiles) {
        found.push(YakuKind::Flush);
    }
    if is_mixed_sets(sets) {
        found.push(YakuKind::MixedSets);
    }
    if is_full_hand(tiles, sets) {
        found.push(YakuKind::FullHand);
    }

    found
}

/// All non-pair sets are triplets.
fn is_all_triplets(sets: &[DetectedSet]) -> bool {
    let non_pair: Vec<_> = sets.iter().filter(|s| s.kind != SetKind::Pair).collect();
    !non_pair.is_empty() && non_pair.iter().all(|s| s.kind == SetKind::Triplet)
}

/// Every tile is a numbered suit (Characters/Bamboos/Circles) with rank 2–8.
fn is_all_simples(tiles: &[Tile]) -> bool {
    !tiles.is_empty()
        && tiles
            .iter()
            .all(|t| t.is_number_tile() && t.rank >= 2 && t.rank <= 8)
}

/// All tiles share the same suit.
fn is_flush(tiles: &[Tile]) -> bool {
    if tiles.is_empty() {
        return false;
    }
    let suit = tiles[0].suit;
    tiles.iter().all(|t| t.suit == suit)
}

/// Decomposition has at least one of each: Pair, Triplet, Sequence.
fn is_mixed_sets(sets: &[DetectedSet]) -> bool {
    let has_pair = sets.iter().any(|s| s.kind == SetKind::Pair);
    let has_triplet = sets.iter().any(|s| s.kind == SetKind::Triplet);
    let has_sequence = sets.iter().any(|s| s.kind == SetKind::Sequence);
    has_pair && has_triplet && has_sequence
}

/// 14 tiles decomposed into exactly 4 melds + 1 pair.
fn is_full_hand(tiles: &[Tile], sets: &[DetectedSet]) -> bool {
    if tiles.len() != 14 {
        return false;
    }
    let melds = sets
        .iter()
        .filter(|s| matches!(s.kind, SetKind::Triplet | SetKind::Sequence))
        .count();
    let pairs = sets.iter().filter(|s| s.kind == SetKind::Pair).count();
    melds == 4 && pairs == 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::{Suit, Tile};

    fn t(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn detect_all_triplets() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 1, 2),
            t(Suit::Circles, 5, 3),
            t(Suit::Circles, 5, 4),
            t(Suit::Circles, 5, 5),
            t(Suit::Wind, 1, 6),
            t(Suit::Wind, 1, 7),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::AllTriplets));
        assert!(!yaku.contains(&YakuKind::AllSimples));
    }

    #[test]
    fn detect_all_simples() {
        let tiles = vec![
            t(Suit::Bamboos, 2, 0),
            t(Suit::Bamboos, 3, 1),
            t(Suit::Bamboos, 4, 2),
            t(Suit::Circles, 5, 3),
            t(Suit::Circles, 5, 4),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::AllSimples));
    }

    #[test]
    fn all_simples_rejects_terminals() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0), // rank 1 = terminal
            t(Suit::Bamboos, 2, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Circles, 5, 3),
            t(Suit::Circles, 5, 4),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(!yaku.contains(&YakuKind::AllSimples));
    }

    #[test]
    fn detect_flush() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 2, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Bamboos, 5, 3),
            t(Suit::Bamboos, 5, 4),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![3, 4],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::Flush));
    }

    #[test]
    fn detect_mixed_sets() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 1, 2),
            t(Suit::Characters, 4, 3),
            t(Suit::Characters, 5, 4),
            t(Suit::Characters, 6, 5),
            t(Suit::Wind, 1, 6),
            t(Suit::Wind, 1, 7),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![6, 7],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::MixedSets));
    }

    #[test]
    fn detect_full_hand() {
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 1, 2),
            t(Suit::Characters, 4, 3),
            t(Suit::Characters, 5, 4),
            t(Suit::Characters, 6, 5),
            t(Suit::Circles, 9, 6),
            t(Suit::Circles, 9, 7),
            t(Suit::Circles, 9, 8),
            t(Suit::Bamboos, 5, 9),
            t(Suit::Bamboos, 6, 10),
            t(Suit::Bamboos, 7, 11),
            t(Suit::Wind, 1, 12),
            t(Suit::Wind, 1, 13),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![3, 4, 5],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![6, 7, 8],
            },
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![9, 10, 11],
            },
            DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![12, 13],
            },
        ];
        let yaku = detect_yaku(&tiles, &sets);
        assert!(yaku.contains(&YakuKind::FullHand));
        assert!(yaku.contains(&YakuKind::MixedSets));
    }

    #[test]
    fn no_yaku_on_simple_pair() {
        let tiles = vec![t(Suit::Bamboos, 3, 0), t(Suit::Bamboos, 3, 1)];
        let sets = vec![DetectedSet {
            kind: SetKind::Pair,
            tile_ids: vec![0, 1],
        }];
        let yaku = detect_yaku(&tiles, &sets);
        // Flush is detected (single suit), but not AllTriplets, AllSimples needs 2-8, etc.
        assert!(!yaku.contains(&YakuKind::AllTriplets));
        assert!(!yaku.contains(&YakuKind::MixedSets));
        assert!(!yaku.contains(&YakuKind::FullHand));
    }

    #[test]
    fn bonus_points_values() {
        assert_eq!(YakuKind::AllTriplets.bonus_points(), 100);
        assert_eq!(YakuKind::AllSimples.bonus_points(), 60);
        assert_eq!(YakuKind::Flush.bonus_points(), 120);
        assert_eq!(YakuKind::MixedSets.bonus_points(), 50);
        assert_eq!(YakuKind::FullHand.bonus_points(), 200);
    }
}
