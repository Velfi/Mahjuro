//! Decomposition bias inference: peek at the player's full hand to guess
//! whether they're building a pair-heavy (Chiitoitsu) or triplet-heavy
//! (Toitoi / standard) shape, then rank candidate decompositions accordingly.
//!
//! This is a soft signal layered above [`crate::core::hand`]'s validator —
//! the validator stays deterministic, and this module only re-orders
//! ambiguous results so the chosen decomposition matches what the player is
//! visibly building toward.

use rustc_hash::FxHashMap;

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

/// When multiple decompositions validate, pick the one with highest shape affinity.
pub fn pick_best_decomposition_by_affinity(
    default_sets: Vec<DetectedMeld>,
    alternatives: Vec<Vec<DetectedMeld>>,
    full_hand: &[Tile],
) -> Vec<DetectedMeld> {
    if alternatives.len() <= 1 {
        return default_sets;
    }
    let bias = infer_decomposition_bias(full_hand);
    let mut best = default_sets;
    let mut best_affinity = decomposition_affinity(&best, bias);
    for candidate in alternatives {
        let affinity = decomposition_affinity(&candidate, bias);
        if affinity > best_affinity {
            best_affinity = affinity;
            best = candidate;
        }
    }
    best
}

/// Meld whose every tile is a flower (e.g. `FF`, `FFF`).
pub fn is_flower_only_meld(meld: &DetectedMeld, tiles: &[Tile]) -> bool {
    !meld.tile_ids.is_empty()
        && meld.tile_ids.iter().all(|&id| {
            tiles.iter().find(|t| t.id == id).is_some_and(|t| t.is_flower())
        })
}

/// Descending tile-counts per meld, e.g. kong+pair → `[4, 2]`.
pub fn meld_sizes_desc(sets: &[DetectedMeld]) -> Vec<usize> {
    let mut sizes: Vec<usize> = sets.iter().map(|s| s.tile_ids.len()).collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    sizes
}

/// Rank tuple for largest-chunk decomposition picking (≤6 tile plays).
/// Primary: lexicographically largest descending meld sizes; then fewer
/// flower-only melds; then fewer total melds.
pub fn decomposition_chunk_rank(tiles: &[Tile], sets: &[DetectedMeld]) -> (Vec<usize>, usize, usize) {
    let sizes = meld_sizes_desc(sets);
    let flower_only = sets
        .iter()
        .filter(|s| is_flower_only_meld(s, tiles))
        .count();
    (sizes, flower_only, sets.len())
}

/// Compare two decompositions for largest-chunk policy. Greater = better.
pub fn compare_decompositions_by_chunks(
    tiles: &[Tile],
    a: &[DetectedMeld],
    b: &[DetectedMeld],
) -> std::cmp::Ordering {
    let rank_a = decomposition_chunk_rank(tiles, a);
    let rank_b = decomposition_chunk_rank(tiles, b);

    for (sa, sb) in rank_a.0.iter().zip(rank_b.0.iter()) {
        match sa.cmp(sb) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
    }
    match rank_a.0.len().cmp(&rank_b.0.len()) {
        std::cmp::Ordering::Equal => {}
        other => return other,
    }
    match rank_b.1.cmp(&rank_a.1) {
        std::cmp::Ordering::Equal => {}
        other => return other,
    }
    match rank_b.2.cmp(&rank_a.2) {
        std::cmp::Ordering::Equal => {}
        other => return other,
    }
    std::cmp::Ordering::Equal
}

/// Pick the decomposition with the largest meld chunks among `alternatives`.
pub fn pick_best_decomposition_by_chunks(
    tiles: &[Tile],
    alternatives: &[Vec<DetectedMeld>],
) -> Vec<DetectedMeld> {
    alternatives
        .iter()
        .max_by(|a, b| compare_decompositions_by_chunks(tiles, a, b))
        .cloned()
        .unwrap_or_default()
}

fn face_counts(tiles: &[Tile]) -> FxHashMap<(Suit, u8), usize> {
    let mut m: FxHashMap<(Suit, u8), usize> = FxHashMap::default();
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
            t(Suit::Souzu, 1, next()),
            t(Suit::Souzu, 1, next()),
            t(Suit::Souzu, 3, next()),
            t(Suit::Souzu, 3, next()),
            t(Suit::Pinzu, 2, next()),
            t(Suit::Pinzu, 2, next()),
            t(Suit::Pinzu, 5, next()),
            t(Suit::Pinzu, 5, next()),
            t(Suit::Manzu, 4, next()),
            t(Suit::Manzu, 4, next()),
            t(Suit::Manzu, 7, next()),
            t(Suit::Manzu, 7, next()),
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
            t(Suit::Souzu, 5, next()),
            t(Suit::Souzu, 5, next()),
            t(Suit::Souzu, 5, next()),
            t(Suit::Pinzu, 7, next()),
            t(Suit::Pinzu, 7, next()),
            t(Suit::Pinzu, 7, next()),
            t(Suit::Manzu, 1, next()),
            t(Suit::Manzu, 2, next()),
            t(Suit::Manzu, 3, next()),
            t(Suit::Souzu, 1, next()),
            t(Suit::Souzu, 2, next()),
            t(Suit::Souzu, 3, next()),
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
            t(Suit::Souzu, 1, 0),
            t(Suit::Souzu, 2, 1),
            t(Suit::Souzu, 3, 2),
            t(Suit::Pinzu, 4, 3),
            t(Suit::Pinzu, 5, 4),
            t(Suit::Pinzu, 6, 5),
            t(Suit::Manzu, 7, 6),
            t(Suit::Manzu, 8, 7),
            t(Suit::Manzu, 9, 8),
            t(Suit::Souzu, 5, 9),
            t(Suit::Souzu, 5, 10),
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

    mod chunk_rank {
        use super::*;
        use crate::core::hand::enumerate_decompositions;
        use crate::core::structure_notation::{format_structure_notation, parse_structure_notation};

        fn best_chunk_decomposition(notation: &str) -> (Vec<Tile>, Vec<DetectedMeld>) {
            let (tiles, _) = parse_structure_notation(notation).expect("notation");
            let alts = enumerate_decompositions(&tiles, &[]);
            assert!(
                !alts.is_empty(),
                "expected valid decompositions for {notation}"
            );
            let best = pick_best_decomposition_by_chunks(&tiles, &alts);
            (tiles, best)
        }

        #[test]
        fn two_triplets_merge_to_kong_plus_pair() {
            let (tiles, best) = best_chunk_decomposition("222m 222m");
            assert_eq!(format_structure_notation(&tiles, &best), "2222m 22m");
        }

        fn tiles_manual() -> Vec<Tile> {
            vec![
                Tile::new(Suit::Manzu, 2, 0),
                Tile::new(Suit::Manzu, 2, 1),
                Tile::new(Suit::Manzu, 2, 2),
                Tile::new(Suit::Manzu, 2, 3),
                Tile::new(Suit::Manzu, 2, 4),
                Tile::new(Suit::Flower, 1, 5),
            ]
        }

        #[test]
        fn triplet_plus_wildcard_triplet_stays() {
            let tiles = tiles_manual();
            let alts = enumerate_decompositions(&tiles, &[]);
            let best = pick_best_decomposition_by_chunks(&tiles, &alts);
            assert_eq!(meld_sizes_desc(&best), vec![3, 3]);
            assert!(
                !best.iter().any(|s| is_flower_only_meld(s, &tiles)),
                "flower should wildcard into numbered triplet, not flower-only meld"
            );
        }

        #[test]
        fn four_twos_two_flowers_prefers_largest_chunks() {
            let tiles = vec![
                Tile::new(Suit::Manzu, 2, 0),
                Tile::new(Suit::Manzu, 2, 1),
                Tile::new(Suit::Manzu, 2, 2),
                Tile::new(Suit::Manzu, 2, 3),
                Tile::new(Suit::Flower, 1, 4),
                Tile::new(Suit::Flower, 2, 5),
            ];
            let alts = enumerate_decompositions(&tiles, &[]);
            let best = pick_best_decomposition_by_chunks(&tiles, &alts);
            for alt in &alts {
                assert!(
                    compare_decompositions_by_chunks(&tiles, &best, alt).is_ge(),
                    "best must maximize chunk rank"
                );
            }
            assert_eq!(meld_sizes_desc(&best), vec![4, 2]);
        }

        #[test]
        fn flower_only_triplet_loses_to_numbered_wildcard_triplet() {
            let (tiles, alts) = {
                let (tiles, _) = parse_structure_notation("222m f1f2f3").expect("notation");
                let alts = enumerate_decompositions(&tiles, &[]);
                (tiles, alts)
            };
            let best = pick_best_decomposition_by_chunks(&tiles, &alts);
            assert_eq!(best.len(), 2);
            assert!(best.iter().any(|s| s.kind == MeldKind::Triplet && !is_flower_only_meld(s, &tiles)));
            assert!(
                compare_decompositions_by_chunks(&tiles, &best, &alts[0]).is_ge(),
                "numbered triplet + flower triplet beats flower-only alternatives when sizes tie"
            );
        }

        #[test]
        fn kong_beats_two_triplets_lexicographically() {
            let two_triplets = vec![triplet(0, 1, 2), triplet(3, 4, 5)];
            let kong_pair = vec![kong(0, 1, 2, 3), pair(4, 5)];
            let tiles: Vec<Tile> = (0..6)
                .map(|i| Tile::new(Suit::Manzu, 2, i))
                .collect();
            assert_eq!(
                compare_decompositions_by_chunks(&tiles, &kong_pair, &two_triplets),
                std::cmp::Ordering::Greater
            );
        }
    }
}
