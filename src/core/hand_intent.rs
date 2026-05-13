//! Decomposition bias inference: peek at the player's full hand to guess
//! whether they're building a pair-heavy (Chiitoitsu) or triplet-heavy
//! (Toitoi / standard) shape, then rank candidate decompositions accordingly.
//!
//! This is a soft signal layered above [`crate::core::hand`]'s validator —
//! the validator stays deterministic, and this module only re-orders
//! ambiguous results so the chosen decomposition matches what the player is
//! visibly building toward.

use std::collections::HashMap;

use crate::core::hand::{DetectedMeld, MeldKind};
use crate::core::tile::{Suit, Tile};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecompositionBias {
    PreferPairs,
    PreferTriplets,
    Neutral,
}

/// Inspect the player's *entire* hand (selected + unselected) and decide
/// whether their tile shape leans toward seven-pairs or toward
/// triplets/standard. Must NOT be called with just the current selection —
/// that would let the selection bias itself.
pub fn infer_decomposition_bias(full_hand: &[Tile]) -> DecompositionBias {
    let counts = face_counts(full_hand);

    let mut distinct_pairs = 0;
    let mut triplets_or_kongs = 0;
    for &n in counts.values() {
        if n >= 3 {
            triplets_or_kongs += 1;
        } else if n == 2 {
            distinct_pairs += 1;
        }
    }

    if distinct_pairs >= 5 && triplets_or_kongs <= 1 {
        DecompositionBias::PreferPairs
    } else if triplets_or_kongs >= 2 {
        DecompositionBias::PreferTriplets
    } else {
        DecompositionBias::Neutral
    }
}

/// Score a candidate decomposition under the given bias. Higher is better.
/// Pure shape arithmetic — no yaku detection, no scoring engine.
pub fn decomposition_affinity(sets: &[DetectedMeld], bias: DecompositionBias) -> i32 {
    if matches!(bias, DecompositionBias::Neutral) {
        return 0;
    }
    let mut score = 0;
    for s in sets {
        match (bias, s.kind) {
            (DecompositionBias::PreferPairs, MeldKind::Pair) => score += 1,
            (DecompositionBias::PreferPairs, MeldKind::Triplet | MeldKind::Kong) => score -= 1,
            (DecompositionBias::PreferTriplets, MeldKind::Triplet | MeldKind::Kong) => score += 2,
            (DecompositionBias::PreferTriplets, MeldKind::Pair) => score -= 1,
            _ => {}
        }
    }
    score
}

fn face_counts(tiles: &[Tile]) -> HashMap<(Suit, u8), usize> {
    let mut m: HashMap<(Suit, u8), usize> = HashMap::new();
    for t in tiles {
        if t.is_flower() {
            continue;
        }
        *m.entry((t.suit, t.rank)).or_insert(0) += 1;
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::Suit;

    fn t(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    fn pair(a: u32, b: u32) -> DetectedMeld {
        DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![a, b],
        }
    }

    fn triplet(a: u32, b: u32, c: u32) -> DetectedMeld {
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![a, b, c],
        }
    }

    fn kong(a: u32, b: u32, c: u32, d: u32) -> DetectedMeld {
        DetectedMeld {
            kind: MeldKind::Kong,
            tile_ids: vec![a, b, c, d],
        }
    }

    fn seq(a: u32, b: u32, c: u32) -> DetectedMeld {
        DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![a, b, c],
        }
    }

    /// Hand with 6 distinct pairs, no triplets — clearly chiitoitsu.
    fn pair_heavy_hand() -> Vec<Tile> {
        let mut id = 0;
        let mut next = || {
            let v = id;
            id += 1;
            v
        };
        vec![
            t(Suit::Bamboos, 1, next()),
            t(Suit::Bamboos, 1, next()),
            t(Suit::Bamboos, 3, next()),
            t(Suit::Bamboos, 3, next()),
            t(Suit::Circles, 2, next()),
            t(Suit::Circles, 2, next()),
            t(Suit::Circles, 5, next()),
            t(Suit::Circles, 5, next()),
            t(Suit::Characters, 4, next()),
            t(Suit::Characters, 4, next()),
            t(Suit::Characters, 7, next()),
            t(Suit::Characters, 7, next()),
            t(Suit::Wind, 1, next()),
            t(Suit::Dragon, 2, next()),
        ]
    }

    /// Hand with two triplets and assorted leftovers — leans toitoi/standard.
    fn triplet_heavy_hand() -> Vec<Tile> {
        let mut id = 0;
        let mut next = || {
            let v = id;
            id += 1;
            v
        };
        vec![
            t(Suit::Bamboos, 5, next()),
            t(Suit::Bamboos, 5, next()),
            t(Suit::Bamboos, 5, next()),
            t(Suit::Circles, 7, next()),
            t(Suit::Circles, 7, next()),
            t(Suit::Circles, 7, next()),
            t(Suit::Characters, 1, next()),
            t(Suit::Characters, 2, next()),
            t(Suit::Characters, 3, next()),
            t(Suit::Bamboos, 1, next()),
            t(Suit::Bamboos, 2, next()),
            t(Suit::Bamboos, 3, next()),
            t(Suit::Wind, 1, next()),
            t(Suit::Wind, 1, next()),
        ]
    }

    #[test]
    fn pair_heavy_hand_prefers_pairs() {
        assert_eq!(
            infer_decomposition_bias(&pair_heavy_hand()),
            DecompositionBias::PreferPairs
        );
    }

    #[test]
    fn triplet_heavy_hand_prefers_triplets() {
        assert_eq!(
            infer_decomposition_bias(&triplet_heavy_hand()),
            DecompositionBias::PreferTriplets
        );
    }

    #[test]
    fn mixed_hand_is_neutral() {
        let hand = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 2, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Circles, 4, 3),
            t(Suit::Circles, 5, 4),
            t(Suit::Circles, 6, 5),
            t(Suit::Characters, 7, 6),
            t(Suit::Characters, 8, 7),
            t(Suit::Characters, 9, 8),
            t(Suit::Bamboos, 5, 9),
            t(Suit::Bamboos, 5, 10),
            t(Suit::Wind, 2, 11),
            t(Suit::Wind, 3, 12),
            t(Suit::Dragon, 1, 13),
        ];
        assert_eq!(infer_decomposition_bias(&hand), DecompositionBias::Neutral);
    }

    #[test]
    fn pair_bias_prefers_two_pairs_over_kong() {
        let two_pairs = vec![pair(0, 1), pair(2, 3)];
        let one_kong = vec![kong(0, 1, 2, 3)];
        let bias = DecompositionBias::PreferPairs;
        assert!(decomposition_affinity(&two_pairs, bias) > decomposition_affinity(&one_kong, bias));
    }

    #[test]
    fn triplet_bias_prefers_kong_over_two_pairs() {
        let two_pairs = vec![pair(0, 1), pair(2, 3)];
        let one_kong = vec![kong(0, 1, 2, 3)];
        let bias = DecompositionBias::PreferTriplets;
        assert!(decomposition_affinity(&one_kong, bias) > decomposition_affinity(&two_pairs, bias));
    }

    #[test]
    fn pair_bias_prefers_pair_plus_seq_over_triplet_plus_orphans() {
        let pair_plus_seq = vec![pair(0, 1), seq(2, 3, 4)];
        let triplet_with_seq_neighbours = vec![triplet(0, 1, 2), seq(3, 4, 5)];
        let bias = DecompositionBias::PreferPairs;
        assert!(
            decomposition_affinity(&pair_plus_seq, bias)
                > decomposition_affinity(&triplet_with_seq_neighbours, bias)
        );
    }

    #[test]
    fn neutral_bias_is_indifferent() {
        let two_pairs = vec![pair(0, 1), pair(2, 3)];
        let one_kong = vec![kong(0, 1, 2, 3)];
        assert_eq!(
            decomposition_affinity(&two_pairs, DecompositionBias::Neutral),
            decomposition_affinity(&one_kong, DecompositionBias::Neutral)
        );
    }
}
