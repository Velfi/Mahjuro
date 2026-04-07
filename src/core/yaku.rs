//! Yaku (hand pattern) detection and bonus scoring.

use crate::core::hand::{DetectedSet, SetKind, validate_selection};
use crate::core::tile::{Suit, Tile};

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
    /// Mult bonus added (additively, on the chips×mult scoring axis) when
    /// this yaku fires. These are tuned so that stacking 2-3 yaku on a real
    /// hand pushes mult into the ×8-15 range — that's where the chip pile
    /// turns into "explosive" final scores.
    pub fn mult_bonus(self) -> f64 {
        match self {
            YakuKind::AllTriplets => 4.0,
            YakuKind::AllSimples => 2.0,
            YakuKind::Flush => 4.0,
            YakuKind::MixedSets => 2.0,
            YakuKind::FullHand => 5.0,
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

/// Live preview of a yaku for the current selection: how close the player is
/// to qualifying, with a short human-readable hint.
#[derive(Clone, Debug)]
pub struct YakuPreview {
    pub kind: YakuKind,
    /// True if the current selection (when valid) actually awards this yaku.
    pub active: bool,
    /// Progress toward qualifying, in `0.0..=1.0`.
    pub progress: f32,
    /// Short hint text, e.g. "4/5 same suit" or "P·S" (mixed-set checklist).
    pub hint: String,
}

/// Compute a `YakuPreview` for each yaku in the player's available pool, based
/// on the currently-selected tiles. Yaku that need a valid decomposition fall
/// back to a "needs valid hand" hint when the selection doesn't decompose.
pub fn yaku_preview(tiles: &[Tile], available: &[YakuKind]) -> Vec<YakuPreview> {
    let sets_opt = validate_selection(tiles);
    let active_yaku: Vec<YakuKind> = match &sets_opt {
        Some(s) => detect_yaku(tiles, s),
        None => Vec::new(),
    };

    let kinds: Vec<YakuKind> = if available.is_empty() {
        vec![
            YakuKind::AllTriplets,
            YakuKind::AllSimples,
            YakuKind::Flush,
            YakuKind::MixedSets,
            YakuKind::FullHand,
        ]
    } else {
        available.to_vec()
    };

    kinds
        .into_iter()
        .map(|k| {
            let active = active_yaku.contains(&k);
            let (progress, hint) = match k {
                YakuKind::Flush => {
                    if tiles.is_empty() {
                        (0.0, "0/5 same suit".to_string())
                    } else {
                        let mut counts = [0usize; 5];
                        for t in tiles {
                            counts[t.suit as usize] += 1;
                        }
                        let max = counts.iter().copied().max().unwrap_or(0);
                        // Need 5+ tiles all of one suit. Progress is the
                        // single-suit count toward that threshold.
                        let progress = (max as f32 / 5.0).min(1.0);
                        (progress, format!("{max}/5 same suit"))
                    }
                }
                YakuKind::AllSimples => {
                    if tiles.is_empty() {
                        (0.0, "0/0 simples".to_string())
                    } else {
                        let n = tiles
                            .iter()
                            .filter(|t| {
                                matches!(t.suit, Suit::Characters | Suit::Bamboos | Suit::Circles)
                                    && t.rank >= 2
                                    && t.rank <= 8
                            })
                            .count();
                        let total = tiles.len();
                        (n as f32 / total as f32, format!("{n}/{total} simples"))
                    }
                }
                YakuKind::FullHand => {
                    let n = tiles.len().min(14);
                    (n as f32 / 14.0, format!("{n}/14 tiles"))
                }
                YakuKind::AllTriplets => match &sets_opt {
                    Some(s) => {
                        let trips = s.iter().filter(|x| x.kind == SetKind::Triplet).count();
                        let seqs = s.iter().filter(|x| x.kind == SetKind::Sequence).count();
                        // Need ≥2 triplets and zero sequences. Any sequence
                        // disqualifies, so progress collapses to 0 there.
                        if seqs > 0 {
                            (0.0, format!("{seqs} sequence(s)"))
                        } else {
                            let progress = (trips as f32 / 2.0).min(1.0);
                            (progress, format!("{trips}/2+ triplets"))
                        }
                    }
                    None => (0.0, "needs valid hand".to_string()),
                },
                YakuKind::MixedSets => match &sets_opt {
                    Some(s) => {
                        let has_p = s.iter().any(|x| x.kind == SetKind::Pair);
                        let has_t = s.iter().any(|x| x.kind == SetKind::Triplet);
                        let has_q = s.iter().any(|x| x.kind == SetKind::Sequence);
                        let n = (has_p as u32) + (has_t as u32) + (has_q as u32);
                        let mark = |b: bool, c: char| if b { c } else { '·' };
                        let hint = format!(
                            "{}{}{}",
                            mark(has_p, 'P'),
                            mark(has_t, 'T'),
                            mark(has_q, 'S')
                        );
                        (n as f32 / 3.0, hint)
                    }
                    None => (0.0, "···".to_string()),
                },
            };
            YakuPreview {
                kind: k,
                active,
                progress: progress.clamp(0.0, 1.0),
                hint,
            }
        })
        .collect()
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
    let full = is_full_hand(tiles, sets);
    // FullHand strictly contains MixedSets (4 melds + pair always has P+T+S
    // unless all melds are the same kind). Suppress MixedSets when FullHand
    // fires so the 80-point bonus doesn't double-dip on top of FullHand's 200.
    if !full && is_mixed_sets(sets) {
        found.push(YakuKind::MixedSets);
    }
    if full {
        found.push(YakuKind::FullHand);
    }

    found
}

/// All non-pair sets are triplets, and there are at least 2 triplets so a
/// lone meld doesn't trivially earn the bonus.
fn is_all_triplets(sets: &[DetectedSet]) -> bool {
    let triplets = sets.iter().filter(|s| s.kind == SetKind::Triplet).count();
    let sequences = sets.iter().filter(|s| s.kind == SetKind::Sequence).count();
    triplets >= 2 && sequences == 0
}

/// Every tile is a numbered suit (Characters/Bamboos/Circles) with rank 2–8.
/// Requires at least 5 tiles so a single tile or pair doesn't trivially qualify.
fn is_all_simples(tiles: &[Tile]) -> bool {
    tiles.len() >= 5
        && tiles
            .iter()
            .all(|t| t.is_number_tile() && t.rank >= 2 && t.rank <= 8)
}

/// All tiles share the same suit. Requires at least 5 tiles (one meld + a pair)
/// so a bare pair or single meld doesn't trivially qualify.
fn is_flush(tiles: &[Tile]) -> bool {
    if tiles.len() < 5 {
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
        // FullHand suppresses MixedSets to avoid double-dipping the bonus.
        assert!(!yaku.contains(&YakuKind::MixedSets));
    }

    #[test]
    fn no_yaku_on_simple_pair() {
        let tiles = vec![t(Suit::Bamboos, 3, 0), t(Suit::Bamboos, 3, 1)];
        let sets = vec![DetectedSet {
            kind: SetKind::Pair,
            tile_ids: vec![0, 1],
        }];
        let yaku = detect_yaku(&tiles, &sets);
        // A bare pair must not award any yaku — they all gate on a real hand.
        assert!(!yaku.contains(&YakuKind::Flush));
        assert!(!yaku.contains(&YakuKind::AllTriplets));
        assert!(!yaku.contains(&YakuKind::AllSimples));
        assert!(!yaku.contains(&YakuKind::MixedSets));
        assert!(!yaku.contains(&YakuKind::FullHand));
    }

    #[test]
    fn mult_bonus_values() {
        assert_eq!(YakuKind::AllTriplets.mult_bonus(), 4.0);
        assert_eq!(YakuKind::AllSimples.mult_bonus(), 2.0);
        assert_eq!(YakuKind::Flush.mult_bonus(), 4.0);
        assert_eq!(YakuKind::MixedSets.mult_bonus(), 2.0);
        assert_eq!(YakuKind::FullHand.mult_bonus(), 5.0);
    }
}
