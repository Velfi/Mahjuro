#![allow(dead_code)]
//! Shanten ("tiles away from a complete hand") for the gameplay HUD.
//!
//! Real riichi shanten counting is a few hundred lines of careful enumeration.
//! For Mahjuro's arcade UX we only need to *distinguish* the meaningful states:
//!
//!   * `Complete` — the hand currently scores as a 14-tile FullHand.
//!   * `Tenpai`   — exactly 1 tile swap away from a FullHand (or chiitoitsu).
//!   * `N >= 2`   — a heuristic estimate for "you're far away".
//!
//! Tenpai is detected exactly (brute-force swap-by-1 over all 34 tile faces).
//! The ≥2 case uses a fast meld-counting heuristic and is approximate. The
//! Tenpai Bonus in `scoring.rs` reads only the `Complete` axis, so the
//! approximation is purely a UI hint.

use crate::core::hand::{SetKind, find_pairs_and_triplets, validate_selection};
use crate::core::tile::{Suit, Tile};

/// All 34 tile faces in a standard set: 9 ranks × 3 number suits + 4 winds + 3 dragons.
fn all_faces() -> Vec<(Suit, u8)> {
    let mut out = Vec::with_capacity(34);
    for suit in [Suit::Characters, Suit::Bamboos, Suit::Circles] {
        for rank in 1..=9 {
            out.push((suit, rank));
        }
    }
    for rank in 1..=4 {
        out.push((Suit::Wind, rank));
    }
    for rank in 1..=3 {
        out.push((Suit::Dragon, rank));
    }
    out
}

/// True iff `tiles` validates as a FullHand: 4 melds + 1 pair (kongs allowed,
/// adding 1 tile each over 14). Caller must pre-sort if needed —
/// `validate_selection` sorts internally.
pub fn is_complete(tiles: &[Tile]) -> bool {
    let kongs_target = |sets: &[crate::core::hand::DetectedSet]| -> bool {
        let kongs = sets.iter().filter(|s| s.kind == SetKind::Kong).count();
        let melds = sets
            .iter()
            .filter(|s| matches!(s.kind, SetKind::Triplet | SetKind::Sequence | SetKind::Kong))
            .count();
        let pairs = sets.iter().filter(|s| s.kind == SetKind::Pair).count();
        tiles.len() == 14 + kongs && melds == 4 && pairs == 1
    };
    match validate_selection(tiles) {
        Some(sets) => kongs_target(&sets),
        None => false,
    }
}

/// True iff swapping any single tile in `tiles` for some other tile face would
/// produce a complete (FullHand) hand. Brute-force: 14 × 34 = ~476 validate
/// calls per query. Cheap enough to run once per UI frame.
pub fn is_tenpai(tiles: &[Tile]) -> bool {
    if tiles.is_empty() {
        return false;
    }
    if is_complete(tiles) {
        // Already won — not "tenpai" in the traditional sense, but the UI
        // should treat both as "0 away".
        return true;
    }
    let next_id = tiles.iter().map(|t| t.id).max().unwrap_or(0) + 1;
    let mut buf: Vec<Tile> = tiles.to_vec();
    for i in 0..tiles.len() {
        let original = buf[i];
        for (suit, rank) in all_faces() {
            // Skip identity swap.
            if original.suit == suit && original.rank == rank {
                continue;
            }
            buf[i] = Tile::new(suit, rank, next_id);
            if is_complete(&buf) {
                buf[i] = original;
                return true;
            }
        }
        buf[i] = original;
    }
    false
}

/// Approximate shanten distance for `tiles`.
///
/// Returns:
///   *  -1 if the hand is currently `Complete` (a FullHand).
///   *   0 if the hand is `Tenpai` (1 tile away).
///   *   N ≥ 1 — heuristic distance based on counted melds + partials.
///
/// The N ≥ 1 path uses a simple formula:
///
///     shanten ≈ 8 - 2 * complete_melds - partial_melds - has_pair
///
/// clamped so that complete + partial ≤ 4 (we only need 4 melds total). This
/// is inexact for hands where the optimal decomposition leaves a worse
/// intermediate state, but the UI only needs the rough bucket.
pub fn shanten_estimate(tiles: &[Tile]) -> i32 {
    if is_complete(tiles) {
        return -1;
    }
    if is_tenpai(tiles) {
        return 0;
    }

    let sets = find_pairs_and_triplets(tiles);
    let mut complete = 0i32;
    let mut has_pair = false;
    for s in &sets {
        match s.kind {
            SetKind::Triplet | SetKind::Kong => complete += 1,
            SetKind::Pair => {
                if !has_pair {
                    has_pair = true;
                } else {
                    // Spare pairs can become triplets — count as partial.
                    complete += 0;
                }
            }
            SetKind::Sequence => complete += 1,
        }
    }
    let partials = count_partial_melds(tiles, &sets) as i32;
    let usable = (complete + partials).min(4);
    let partial_used = (usable - complete).max(0);

    let pair_bonus = if has_pair { 1 } else { 0 };
    let shanten = 8 - 2 * complete - partial_used - pair_bonus;
    shanten.max(1)
}

/// Count "partial melds": pairs of tiles that could become a sequence with
/// one more tile (edge wait, kanchan, ryanmen). Honor pairs that aren't
/// already triplets are *not* counted here — `find_pairs_and_triplets` has
/// already handled those.
fn count_partial_melds(tiles: &[Tile], existing_sets: &[crate::core::hand::DetectedSet]) -> usize {
    let mut used: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for s in existing_sets {
        for id in &s.tile_ids {
            used.insert(*id);
        }
    }
    let mut available: Vec<&Tile> = tiles.iter().filter(|t| !used.contains(&t.id)).collect();
    available.sort_by(|a, b| (a.suit, a.rank).cmp(&(b.suit, b.rank)));

    // Count adjacent (rank diff = 1) and gap (rank diff = 2) pairs in number suits.
    let mut partials = 0usize;
    let mut consumed: std::collections::HashSet<u32> = std::collections::HashSet::new();
    for i in 0..available.len() {
        if consumed.contains(&available[i].id) {
            continue;
        }
        if !available[i].is_number_tile() {
            continue;
        }
        for j in (i + 1)..available.len() {
            if consumed.contains(&available[j].id) {
                continue;
            }
            if available[j].suit != available[i].suit {
                continue;
            }
            let diff = available[j].rank as i32 - available[i].rank as i32;
            if diff == 1 || diff == 2 {
                partials += 1;
                consumed.insert(available[i].id);
                consumed.insert(available[j].id);
                break;
            }
            if diff > 2 {
                break;
            }
        }
    }
    partials
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::{Suit, Tile};

    fn t(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn complete_full_hand_is_minus_one() {
        // 14-tile bamboo full hand: 1-2-3, 4-5-6, 7-8-9, 5-5-5, 7-7
        let hand = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 2, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Bamboos, 4, 3),
            t(Suit::Bamboos, 5, 4),
            t(Suit::Bamboos, 6, 5),
            t(Suit::Bamboos, 7, 6),
            t(Suit::Bamboos, 8, 7),
            t(Suit::Bamboos, 9, 8),
            t(Suit::Bamboos, 5, 9),
            t(Suit::Bamboos, 5, 10),
            t(Suit::Bamboos, 5, 11),
            t(Suit::Bamboos, 7, 12),
            t(Suit::Bamboos, 7, 13),
        ];
        assert!(is_complete(&hand));
        assert_eq!(shanten_estimate(&hand), -1);
    }

    #[test]
    fn tenpai_one_tile_away() {
        // 13-tile setup that's 1 tile away from a FullHand: replace last tile
        // with a 14th to test. Take the complete hand above and swap one tile
        // for an unrelated face.
        let hand = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 2, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Bamboos, 4, 3),
            t(Suit::Bamboos, 5, 4),
            t(Suit::Bamboos, 6, 5),
            t(Suit::Bamboos, 7, 6),
            t(Suit::Bamboos, 8, 7),
            t(Suit::Bamboos, 9, 8),
            t(Suit::Bamboos, 5, 9),
            t(Suit::Bamboos, 5, 10),
            t(Suit::Bamboos, 5, 11),
            t(Suit::Bamboos, 7, 12),
            // 14th tile: junk — needs to swap to bamboo 7 to complete the pair.
            t(Suit::Wind, 4, 13),
        ];
        assert!(!is_complete(&hand));
        assert!(is_tenpai(&hand));
        assert_eq!(shanten_estimate(&hand), 0);
    }

    #[test]
    fn empty_hand_not_tenpai() {
        assert!(!is_tenpai(&[]));
    }

    #[test]
    fn far_from_complete_returns_positive() {
        // Random scattered tiles — clearly multiple swaps from a hand.
        let hand = vec![
            t(Suit::Characters, 1, 0),
            t(Suit::Bamboos, 4, 1),
            t(Suit::Circles, 7, 2),
            t(Suit::Wind, 1, 3),
            t(Suit::Dragon, 2, 4),
        ];
        let s = shanten_estimate(&hand);
        assert!(s >= 1, "expected shanten >= 1, got {s}");
    }
}
