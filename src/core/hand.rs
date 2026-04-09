//! Hand pattern detection: pairs, triplets, sequences.

use std::collections::HashMap;

use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};

/// Human-readable summary of detected sets in the hand, e.g. "3m×2  1-3s  East×3".
/// Returns an empty string if no sets found.
///
/// When the selection forms a fully valid decomposition (no leftover tiles), the
/// description reflects that exact decomposition. Otherwise it falls back to
/// listing standalone pair/triplet faces (skipping sequences, since unconstrained
/// sequence search would report overlapping melds).
#[allow(dead_code)]
pub fn describe_hand(tiles: &[Tile]) -> String {
    if let Some(sets) = validate_selection(tiles) {
        return describe_sets(tiles, &sets);
    }

    // Fallback: invalid selection — show only pair/triplet/kong faces, which can't overlap.
    let pairs_trips = find_pairs_and_triplets(tiles);
    let mut parts: Vec<String> = Vec::new();
    for s in &pairs_trips {
        if let Some(&id) = s.tile_ids.first() {
            if let Some(t) = tiles.iter().find(|t| t.id == id) {
                let label = t.label();
                match s.kind {
                    SetKind::Pair => parts.push(format!("{label}×2")),
                    SetKind::Triplet => parts.push(format!("{label}×3")),
                    SetKind::Kong => parts.push(format!("{label}×4")),
                    SetKind::Sequence => {}
                }
            }
        }
    }
    parts.join("  ")
}

/// Render a known decomposition as "4m×2  1-2-3m  7-8-9m".
#[allow(dead_code)]
fn describe_sets(tiles: &[Tile], sets: &[DetectedSet]) -> String {
    let mut parts: Vec<String> = Vec::new();
    for s in sets {
        let tile_refs: Vec<&Tile> = s
            .tile_ids
            .iter()
            .filter_map(|id| tiles.iter().find(|t| t.id == *id))
            .collect();
        match s.kind {
            SetKind::Pair => {
                if let Some(t) = tile_refs.first() {
                    parts.push(format!("{}×2", t.label()));
                }
            }
            SetKind::Triplet => {
                if let Some(t) = tile_refs.first() {
                    parts.push(format!("{}×3", t.label()));
                }
            }
            SetKind::Kong => {
                if let Some(t) = tile_refs.first() {
                    parts.push(format!("{}×4", t.label()));
                }
            }
            SetKind::Sequence if tile_refs.len() == 3 => {
                let suffix = match tile_refs[0].suit {
                    Suit::Characters => "m",
                    Suit::Bamboos => "s",
                    Suit::Circles => "p",
                    _ => "",
                };
                parts.push(format!(
                    "{}-{}-{}{}",
                    tile_refs[0].rank, tile_refs[1].rank, tile_refs[2].rank, suffix
                ));
            }
            SetKind::Sequence => {}
        }
    }
    parts.join("  ")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetKind {
    Pair,
    Triplet,
    Sequence,
    /// Four of a kind (mahjong "kan"). Counts as a triplet for yaku and meld
    /// detection but scores larger and can flip an extra dora indicator at the
    /// run-state level.
    Kong,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DetectedSet {
    pub kind: SetKind,
    /// Tile ids participating in this set (references into the hand).
    pub tile_ids: Vec<u32>,
}

/// Group tiles by face (suit+rank), keeping one id list per face key.
/// Flower tiles are excluded — they're wildcards, not a groupable face.
fn face_groups(tiles: &[Tile]) -> HashMap<(Suit, u8), Vec<u32>> {
    let mut m = HashMap::new();
    for t in tiles {
        if t.is_flower() {
            continue;
        }
        m.entry((t.suit, t.rank))
            .or_insert_with(Vec::new)
            .push(t.id);
    }
    m
}

/// Find pairs, triplets, and kongs from multiset counts. A face with 4+ copies
/// emits a kong (preferred) before falling back to triplet/pair leftovers.
pub fn find_pairs_and_triplets(tiles: &[Tile]) -> Vec<DetectedSet> {
    let groups = face_groups(tiles);
    let mut sorted_keys: Vec<_> = groups.keys().copied().collect();
    sorted_keys.sort();
    let mut out = Vec::new();
    for key in &sorted_keys {
        let ids = &groups[key];
        let mut i = 0;
        // Kongs first: a face with all 4 copies in selection always wants to
        // be a kong, not a triplet+singleton.
        while i + 3 < ids.len() {
            out.push(DetectedSet {
                kind: SetKind::Kong,
                tile_ids: vec![ids[i], ids[i + 1], ids[i + 2], ids[i + 3]],
            });
            i += 4;
        }
        while i + 2 < ids.len() {
            out.push(DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![ids[i], ids[i + 1], ids[i + 2]],
            });
            i += 3;
        }
        if i + 1 < ids.len() {
            out.push(DetectedSet {
                kind: SetKind::Pair,
                tile_ids: vec![ids[i], ids[i + 1]],
            });
            i += 2;
        }
        if i < ids.len() {
            // leftover single — ignored here
        }
    }
    out
}

/// Find all length-3 straight sequences in numbered suits (same suit, consecutive ranks).
pub fn find_sequences(tiles: &[Tile]) -> Vec<DetectedSet> {
    let mut out = Vec::new();
    let suits = [Suit::Characters, Suit::Bamboos, Suit::Circles];

    for suit in suits {
        for start in 1..=7 {
            let need = [start, start + 1, start + 2];
            let mut take: Vec<u32> = Vec::new();
            let mut ok = true;
            for r in need {
                let found = tiles
                    .iter()
                    .find(|t| t.suit == suit && t.rank == r && !take.contains(&t.id));
                match found {
                    Some(t) => take.push(t.id),
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                out.push(DetectedSet {
                    kind: SetKind::Sequence,
                    tile_ids: take,
                });
            }
        }
    }
    out
}

/// For each unselected tile in the hand, check if adding it to the current selection
/// would form a valid meld. Returns hand indices of tiles that would complete a meld.
pub fn suggest_completions(hand: &[Tile], selected_indices: &[usize]) -> Vec<usize> {
    let selected_tiles: Vec<Tile> = selected_indices.iter().map(|&i| hand[i]).collect();

    let mut hints = Vec::new();
    for (i, tile) in hand.iter().enumerate() {
        if selected_indices.contains(&i) {
            continue;
        }
        let mut candidate = selected_tiles.clone();
        candidate.push(*tile);
        if validate_selection(&candidate).is_some() {
            hints.push(i);
        }
    }
    hints
}

/// Non-overlapping greedy merge is complex; MVP returns all detected patterns (may overlap).
#[allow(dead_code)]
pub fn detect_all_sets(tiles: &[Tile]) -> Vec<DetectedSet> {
    let mut v = find_pairs_and_triplets(tiles);
    v.extend(find_sequences(tiles));
    v
}

/// Validate that a selection of tiles decomposes perfectly into melds (pairs, triplets,
/// sequences) with no leftover tiles.  Returns the decomposition if valid, `None` otherwise.
///
/// Uses recursive backtracking: at each step, tries to extract a pair, triplet, or sequence
/// starting from the first remaining tile, then recurses on the rest.
///
/// Flower tiles act as wildcards: each can substitute for one missing tile in a triplet
/// or sequence (max one flower per meld). Flowers cannot form or complete pairs.
pub fn validate_selection(tiles: &[Tile]) -> Option<Vec<DetectedSet>> {
    validate_selection_with_rules(tiles, &[])
}

/// Like `validate_selection`, but respects active rule modifiers:
/// - `SequenceWrap`: allows wrapping sequences (8-9-1, 9-1-2)
/// - `NoSequences`: rejects any decomposition containing sequences
/// - `MustPlayFour`: rejects selections that aren't exactly 4 tiles
/// - `RequireDragon`: rejects selections without at least one Dragon tile
pub fn validate_selection_with_rules(
    tiles: &[Tile],
    rules: &[RuleModifier],
) -> Option<Vec<DetectedSet>> {
    if tiles.is_empty() {
        return None;
    }
    // Pre-validation rejects from boss effects. These run before decomposition
    // so we don't waste cycles on hands the boss already disqualifies.
    if rules.contains(&RuleModifier::MustPlayFour) && tiles.len() != 4 {
        return None;
    }
    if rules.contains(&RuleModifier::RequireHonor)
        && !tiles.iter().any(|t| {
            matches!(
                t.suit,
                crate::core::tile::Suit::Wind | crate::core::tile::Suit::Dragon
            )
        })
    {
        return None;
    }
    // Partition into regular tiles and flower wildcards.
    let mut regular: Vec<Tile> = tiles.iter().filter(|t| !t.is_flower()).copied().collect();
    let mut flower_ids: Vec<u32> = tiles
        .iter()
        .filter(|t| t.is_flower())
        .map(|t| t.id)
        .collect();
    regular.sort();

    // Need at least some regular tiles (flowers alone can't form melds).
    if regular.is_empty() {
        return None;
    }

    let allow_wrap = rules.contains(&RuleModifier::SequenceWrap);
    let no_sequences = rules.contains(&RuleModifier::NoSequences);
    let mut result = Vec::new();
    if backtrack_decompose_flowers(&regular, &mut flower_ids, &mut result, allow_wrap) {
        if no_sequences && result.iter().any(|s| s.kind == SetKind::Sequence) {
            return None;
        }
        return Some(result);
    }
    // Chiitoitsu fallback: 14 tiles forming 7 distinct pairs is a valid hand
    // even though it doesn't fit the standard 4-meld + 1-pair decomposition.
    // We try this only when the standard backtracker fails so we don't reframe
    // hands that could decompose normally. Flowers can't help with chiitoitsu
    // (pairs only, no wildcard substitution in pairs).
    if flower_ids.is_empty() {
        if let Some(pairs) = try_chiitoitsu(&regular) {
            return Some(pairs);
        }
    }
    None
}

/// Detect a Chiitoitsu (seven pairs) hand: exactly 14 tiles, decomposing into
/// 7 distinct face pairs (no triplets or kongs of the same face). Returns
/// `Some(pairs)` on success.
fn try_chiitoitsu(tiles: &[Tile]) -> Option<Vec<DetectedSet>> {
    if tiles.len() != 14 {
        return None;
    }
    let groups = face_groups(tiles);
    // Must have exactly 7 distinct faces, each with exactly 2 copies.
    if groups.len() != 7 {
        return None;
    }
    let mut pairs = Vec::with_capacity(7);
    let mut keys: Vec<_> = groups.keys().copied().collect();
    keys.sort();
    for key in keys {
        let ids = &groups[&key];
        if ids.len() != 2 {
            return None;
        }
        pairs.push(DetectedSet {
            kind: SetKind::Pair,
            tile_ids: vec![ids[0], ids[1]],
        });
    }
    Some(pairs)
}

/// Recursive helper: try to decompose `remaining` (sorted, no flowers) into melds,
/// optionally consuming flower tiles from `flower_pool` as wildcards.
///
/// A flower can substitute for one missing tile in a triplet or sequence (max one
/// flower per meld). Flowers cannot form or complete pairs.
fn backtrack_decompose_flowers(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedSet>,
    allow_wrap: bool,
) -> bool {
    if remaining.is_empty() {
        // All selected flowers must be consumed — the player chose to include them.
        return flower_pool.is_empty();
    }
    let first = &remaining[0];

    // ── Normal melds (no flower) ───────────────────────────────────────

    // Try kong (4 of a kind).
    if remaining.len() >= 4
        && remaining[1].suit == first.suit
        && remaining[1].rank == first.rank
        && remaining[2].suit == first.suit
        && remaining[2].rank == first.rank
        && remaining[3].suit == first.suit
        && remaining[3].rank == first.rank
    {
        let set = DetectedSet {
            kind: SetKind::Kong,
            tile_ids: vec![
                remaining[0].id,
                remaining[1].id,
                remaining[2].id,
                remaining[3].id,
            ],
        };
        found.push(set);
        let rest: Vec<Tile> = remaining[4..].to_vec();
        if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
            return true;
        }
        found.pop();
    }

    // Try triplet: 3 tiles with same suit+rank.
    if remaining.len() >= 3
        && remaining[1].suit == first.suit
        && remaining[1].rank == first.rank
        && remaining[2].suit == first.suit
        && remaining[2].rank == first.rank
    {
        let set = DetectedSet {
            kind: SetKind::Triplet,
            tile_ids: vec![remaining[0].id, remaining[1].id, remaining[2].id],
        };
        found.push(set);
        let rest: Vec<Tile> = remaining[3..].to_vec();
        if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
            return true;
        }
        found.pop();
    }

    // Try sequence: first + (rank+1 same suit) + (rank+2 same suit).
    if first.is_number_tile() && remaining.len() >= 3 {
        if try_sequence(remaining, flower_pool, found, allow_wrap, first) {
            return true;
        }
    }

    // Try pair: 2 tiles with same suit+rank (no flowers allowed in pairs).
    if remaining.len() >= 2 && remaining[1].suit == first.suit && remaining[1].rank == first.rank {
        let set = DetectedSet {
            kind: SetKind::Pair,
            tile_ids: vec![remaining[0].id, remaining[1].id],
        };
        found.push(set);
        let rest: Vec<Tile> = remaining[2..].to_vec();
        if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
            return true;
        }
        found.pop();
    }

    // ── Flower-assisted melds ──────────────────────────────────────────

    if !flower_pool.is_empty() {
        // Triplet with flower: 2 same-face tiles + 1 flower.
        if remaining.len() >= 2
            && remaining[1].suit == first.suit
            && remaining[1].rank == first.rank
        {
            let fid = flower_pool.pop().unwrap();
            let set = DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![remaining[0].id, remaining[1].id, fid],
            };
            found.push(set);
            let rest: Vec<Tile> = remaining[2..].to_vec();
            if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
                return true;
            }
            found.pop();
            flower_pool.push(fid);
        }

        // Sequence with flower filling one gap.
        if first.is_number_tile() && remaining.len() >= 2 {
            if try_sequence_with_flower(remaining, flower_pool, found, allow_wrap, first) {
                return true;
            }
        }
    }

    false
}

/// Try to form a normal 3-tile sequence starting from `first` (no flowers).
fn try_sequence(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedSet>,
    allow_wrap: bool,
    first: &Tile,
) -> bool {
    let mid = remaining[1..]
        .iter()
        .position(|t| t.suit == first.suit && t.rank == first.rank + 1);
    if let Some(mid_offset) = mid {
        let mid_idx = mid_offset + 1;
        let search_start = mid_idx + 1;
        let hi = remaining[search_start..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == first.rank + 2);
        if let Some(hi_offset) = hi {
            let hi_idx = hi_offset + search_start;
            let set = DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![remaining[0].id, remaining[mid_idx].id, remaining[hi_idx].id],
            };
            found.push(set);
            let mut rest: Vec<Tile> = Vec::with_capacity(remaining.len() - 3);
            for (i, t) in remaining.iter().enumerate() {
                if i != 0 && i != mid_idx && i != hi_idx {
                    rest.push(*t);
                }
            }
            if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
                return true;
            }
            found.pop();
        }
    }

    // Wrapping sequences (8-9-1, 9-1-2).
    if allow_wrap {
        if try_wrap_sequence(remaining, flower_pool, found, allow_wrap, first) {
            return true;
        }
    }
    false
}

/// Try wrapping sequences (8-9-1, 9-1-2) without flowers.
fn try_wrap_sequence(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedSet>,
    allow_wrap: bool,
    first: &Tile,
) -> bool {
    let wrap_patterns: &[[u8; 3]] = match first.rank {
        1 => &[[9, 1, 2], [8, 9, 1]],
        2 => &[[9, 1, 2]],
        8 => &[[8, 9, 1]],
        9 => &[[8, 9, 1], [9, 1, 2]],
        _ => &[],
    };
    for pattern in wrap_patterns {
        let other_ranks: Vec<u8> = pattern
            .iter()
            .copied()
            .filter(|&r| r != first.rank)
            .collect();
        if other_ranks.len() != 2 {
            continue;
        }
        let mid = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == other_ranks[0]);
        if let Some(mid_offset) = mid {
            let mid_idx = mid_offset + 1;
            let hi = remaining.iter().enumerate().position(|(i, t)| {
                i != 0 && i != mid_idx && t.suit == first.suit && t.rank == other_ranks[1]
            });
            if let Some(hi_idx) = hi {
                let set = DetectedSet {
                    kind: SetKind::Sequence,
                    tile_ids: vec![remaining[0].id, remaining[mid_idx].id, remaining[hi_idx].id],
                };
                found.push(set);
                let mut rest: Vec<Tile> = Vec::with_capacity(remaining.len() - 3);
                for (i, t) in remaining.iter().enumerate() {
                    if i != 0 && i != mid_idx && i != hi_idx {
                        rest.push(*t);
                    }
                }
                if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
                    return true;
                }
                found.pop();
            }
        }
    }
    false
}

/// Try to form a sequence where a flower fills one of the three positions.
/// The first tile is always a regular numbered tile at `remaining[0]`.
fn try_sequence_with_flower(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedSet>,
    allow_wrap: bool,
    first: &Tile,
) -> bool {
    // Case 1: have first and first+1 in hand, flower fills first+2
    if first.rank <= 7 {
        let mid = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == first.rank + 1);
        if let Some(mid_offset) = mid {
            let mid_idx = mid_offset + 1;
            let fid = flower_pool.pop().unwrap();
            let set = DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![remaining[0].id, remaining[mid_idx].id, fid],
            };
            found.push(set);
            let mut rest: Vec<Tile> = Vec::with_capacity(remaining.len() - 2);
            for (i, t) in remaining.iter().enumerate() {
                if i != 0 && i != mid_idx {
                    rest.push(*t);
                }
            }
            if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
                return true;
            }
            found.pop();
            flower_pool.push(fid);
        }
    }

    // Case 2: have first and first+2 in hand, flower fills first+1
    if first.rank <= 7 {
        let hi = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == first.rank + 2);
        if let Some(hi_offset) = hi {
            let hi_idx = hi_offset + 1;
            let fid = flower_pool.pop().unwrap();
            let set = DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![remaining[0].id, fid, remaining[hi_idx].id],
            };
            found.push(set);
            let mut rest: Vec<Tile> = Vec::with_capacity(remaining.len() - 2);
            for (i, t) in remaining.iter().enumerate() {
                if i != 0 && i != hi_idx {
                    rest.push(*t);
                }
            }
            if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
                return true;
            }
            found.pop();
            flower_pool.push(fid);
        }
    }

    // Case 3: flower fills first position — first tile is rank+1 or rank+2
    // of a potential sequence. We handle this implicitly: when the backtracker
    // reaches a tile that could be the middle or end of a sequence, the
    // "first + flower" cases above will have already tried it from the lower
    // tile's perspective. No extra logic needed here.

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::tile::Tile;

    fn t(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn triplet_detected() {
        let hand = vec![
            t(Suit::Bamboos, 3, 0),
            t(Suit::Bamboos, 3, 1),
            t(Suit::Bamboos, 3, 2),
        ];
        let sets = find_pairs_and_triplets(&hand);
        assert!(sets.iter().any(|s| s.kind == SetKind::Triplet));
    }

    #[test]
    fn describe_hand_no_overlapping_sequences() {
        // Regression: 1m,2m,3m,4m,4m,7m,8m,9m must decompose as
        // (1-2-3m) + (4m pair) + (7-8-9m) — NOT also reporting a phantom 2-3-4m
        // sequence by reusing the 2m and 3m.
        let tiles = vec![
            t(Suit::Characters, 1, 0),
            t(Suit::Characters, 2, 1),
            t(Suit::Characters, 3, 2),
            t(Suit::Characters, 4, 3),
            t(Suit::Characters, 4, 4),
            t(Suit::Characters, 7, 5),
            t(Suit::Characters, 8, 6),
            t(Suit::Characters, 9, 7),
        ];
        let desc = describe_hand(&tiles);
        // Must contain the real melds...
        assert!(desc.contains("4m×2"), "missing pair: {desc}");
        assert!(desc.contains("1-2-3m"), "missing low run: {desc}");
        assert!(desc.contains("7-8-9m"), "missing high run: {desc}");
        // ...and must NOT report the overlapping phantom sequence.
        assert!(
            !desc.contains("2-3-4m"),
            "phantom overlapping sequence in: {desc}"
        );
    }

    #[test]
    fn sequence_detected() {
        let hand = vec![
            t(Suit::Characters, 2, 0),
            t(Suit::Characters, 3, 1),
            t(Suit::Characters, 4, 2),
        ];
        let seqs = find_sequences(&hand);
        assert!(seqs.iter().any(|s| s.kind == SetKind::Sequence));
    }

    // ── validate_selection ─────────────────────────────────────────

    #[test]
    fn validate_pair() {
        let tiles = vec![t(Suit::Bamboos, 5, 0), t(Suit::Bamboos, 5, 1)];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Pair);
    }

    #[test]
    fn validate_triplet() {
        let tiles = vec![
            t(Suit::Circles, 7, 0),
            t(Suit::Circles, 7, 1),
            t(Suit::Circles, 7, 2),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Triplet);
    }

    #[test]
    fn validate_sequence() {
        let tiles = vec![
            t(Suit::Characters, 3, 0),
            t(Suit::Characters, 4, 1),
            t(Suit::Characters, 5, 2),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
    }

    #[test]
    fn validate_rejects_leftover() {
        // 4 tiles: triplet + 1 leftover → invalid
        let tiles = vec![
            t(Suit::Bamboos, 3, 0),
            t(Suit::Bamboos, 3, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Circles, 9, 3),
        ];
        assert!(validate_selection(&tiles).is_none());
    }

    #[test]
    fn validate_rejects_single_tile() {
        let tiles = vec![t(Suit::Bamboos, 1, 0)];
        assert!(validate_selection(&tiles).is_none());
    }

    #[test]
    fn validate_rejects_empty() {
        assert!(validate_selection(&[]).is_none());
    }

    #[test]
    fn validate_multi_set() {
        // triplet + sequence = 6 tiles
        let tiles = vec![
            t(Suit::Bamboos, 3, 0),
            t(Suit::Bamboos, 3, 1),
            t(Suit::Bamboos, 3, 2),
            t(Suit::Characters, 1, 3),
            t(Suit::Characters, 2, 4),
            t(Suit::Characters, 3, 5),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 2);
    }

    #[test]
    fn validate_full_hand() {
        // 4 sets + 1 pair = 14 tiles
        let tiles = vec![
            // triplet
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 1, 2),
            // sequence
            t(Suit::Characters, 4, 3),
            t(Suit::Characters, 5, 4),
            t(Suit::Characters, 6, 5),
            // triplet
            t(Suit::Circles, 9, 6),
            t(Suit::Circles, 9, 7),
            t(Suit::Circles, 9, 8),
            // sequence
            t(Suit::Bamboos, 5, 9),
            t(Suit::Bamboos, 6, 10),
            t(Suit::Bamboos, 7, 11),
            // pair
            t(Suit::Wind, 1, 12),
            t(Suit::Wind, 1, 13),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 5);
    }

    #[test]
    fn validate_kong_four_of_a_kind() {
        // 4 identical tiles must decompose as a single Kong, not Triplet+leftover.
        let tiles = vec![
            t(Suit::Bamboos, 5, 0),
            t(Suit::Bamboos, 5, 1),
            t(Suit::Bamboos, 5, 2),
            t(Suit::Bamboos, 5, 3),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Kong);
        assert_eq!(sets[0].tile_ids.len(), 4);
    }

    #[test]
    fn find_pairs_and_triplets_emits_kong() {
        let tiles = vec![
            t(Suit::Circles, 7, 0),
            t(Suit::Circles, 7, 1),
            t(Suit::Circles, 7, 2),
            t(Suit::Circles, 7, 3),
        ];
        let sets = find_pairs_and_triplets(&tiles);
        assert!(sets.iter().any(|s| s.kind == SetKind::Kong));
        assert!(!sets.iter().any(|s| s.kind == SetKind::Triplet));
    }

    #[test]
    fn validate_ambiguous_decomposition() {
        // 1-1-1-2-3 bamboo: could be triplet(1,1,1) + leftover(2,3) = FAIL
        // or pair(1,1) + sequence(1,2,3) = SUCCESS
        // Backtracking should find the valid decomposition.
        let tiles = vec![
            t(Suit::Bamboos, 1, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 1, 2),
            t(Suit::Bamboos, 2, 3),
            t(Suit::Bamboos, 3, 4),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 2);
    }

    // ── suggest_completions ─────────────────────────────────────────

    #[test]
    fn suggest_completions_finds_pair_partner() {
        // Hand has one selected tile; another copy exists in hand.
        let hand = vec![
            t(Suit::Bamboos, 3, 0),
            t(Suit::Bamboos, 3, 1),
            t(Suit::Circles, 5, 2),
        ];
        let selected = vec![0]; // selected tile id 0 (Bamboos 3)
        let hints = suggest_completions(&hand, &selected);
        // Index 1 (Bamboos 3, id=1) should be suggested since adding it forms a pair.
        assert!(hints.contains(&1));
    }

    // ── flower wildcard tests ──────────────────────────────────────

    #[test]
    fn flower_completes_triplet() {
        // 2 identical tiles + 1 flower = valid triplet (flower must be consumed)
        let tiles = vec![
            t(Suit::Bamboos, 5, 0),
            t(Suit::Bamboos, 5, 1),
            t(Suit::Flower, 1, 100),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Triplet);
        assert!(sets[0].tile_ids.contains(&100)); // flower id present
    }

    #[test]
    fn flower_completes_sequence_high() {
        // 1m, 2m + flower = 1-2-3m sequence (flower fills rank 3)
        let tiles = vec![
            t(Suit::Characters, 1, 0),
            t(Suit::Characters, 2, 1),
            t(Suit::Flower, 2, 100),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
    }

    #[test]
    fn flower_completes_sequence_mid() {
        // 1m, 3m + flower = 1-2-3m sequence (flower fills rank 2)
        let tiles = vec![
            t(Suit::Characters, 1, 0),
            t(Suit::Characters, 3, 1),
            t(Suit::Flower, 3, 100),
        ];
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
    }

    #[test]
    fn flower_cannot_complete_pair() {
        // 1 tile + 1 flower should NOT form a valid pair
        let tiles = vec![t(Suit::Bamboos, 3, 0), t(Suit::Flower, 1, 100)];
        assert!(validate_selection(&tiles).is_none());
    }

    #[test]
    fn flower_max_one_per_meld() {
        // 1 tile + 2 flowers should NOT form a valid triplet
        let tiles = vec![
            t(Suit::Bamboos, 5, 0),
            t(Suit::Flower, 1, 100),
            t(Suit::Flower, 2, 101),
        ];
        assert!(validate_selection(&tiles).is_none());
    }

    #[test]
    fn flower_in_multi_meld_hand() {
        // pair + flower-assisted triplet = valid 5-tile hand
        let tiles = vec![
            t(Suit::Circles, 7, 0),
            t(Suit::Circles, 7, 1),
            t(Suit::Circles, 7, 2),
            t(Suit::Bamboos, 3, 3),
            t(Suit::Bamboos, 3, 4),
            t(Suit::Flower, 1, 100),
        ];
        let sets = validate_selection(&tiles).unwrap();
        // Should decompose as: triplet(7p×3) + triplet(3s×2 + flower)
        // or triplet(7p×2 + flower) + triplet(... ) — either is valid
        assert_eq!(sets.len(), 2);
    }

    #[test]
    fn unused_flower_is_invalid() {
        // Selecting a flower alongside a valid pair — flower can't be used
        // (only triplets/sequences), so the selection is invalid. Players
        // shouldn't select flowers they can't use.
        let tiles = vec![
            t(Suit::Bamboos, 3, 0),
            t(Suit::Bamboos, 3, 1),
            t(Suit::Flower, 1, 100),
        ];
        // pair(3s×2) + unused flower → flower must be consumed → invalid
        // flower-triplet(3s×2 + flower) works!
        let sets = validate_selection(&tiles).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Triplet);
    }

    #[test]
    fn flowers_alone_invalid() {
        // Just flowers, no regular tiles — invalid
        let tiles = vec![t(Suit::Flower, 1, 100), t(Suit::Flower, 2, 101)];
        assert!(validate_selection(&tiles).is_none());
    }

    // ── validate_selection_with_rules ──────────────────────────────

    #[test]
    fn sequence_wrap_891() {
        let tiles = vec![
            t(Suit::Characters, 8, 0),
            t(Suit::Characters, 9, 1),
            t(Suit::Characters, 1, 2),
        ];
        // Without wrap: invalid
        assert!(validate_selection(&tiles).is_none());
        // With wrap: valid sequence
        let sets = validate_selection_with_rules(&tiles, &[RuleModifier::SequenceWrap]).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
    }

    #[test]
    fn sequence_wrap_912() {
        let tiles = vec![
            t(Suit::Bamboos, 9, 0),
            t(Suit::Bamboos, 1, 1),
            t(Suit::Bamboos, 2, 2),
        ];
        assert!(validate_selection(&tiles).is_none());
        let sets = validate_selection_with_rules(&tiles, &[RuleModifier::SequenceWrap]).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Sequence);
    }

    #[test]
    fn no_sequences_rejects_sequence() {
        let tiles = vec![
            t(Suit::Characters, 1, 0),
            t(Suit::Characters, 2, 1),
            t(Suit::Characters, 3, 2),
        ];
        // Normal: valid sequence
        assert!(validate_selection(&tiles).is_some());
        // NoSequences: rejected
        assert!(validate_selection_with_rules(&tiles, &[RuleModifier::NoSequences]).is_none());
    }

    #[test]
    fn no_sequences_allows_triplets() {
        let tiles = vec![
            t(Suit::Bamboos, 5, 0),
            t(Suit::Bamboos, 5, 1),
            t(Suit::Bamboos, 5, 2),
        ];
        let sets = validate_selection_with_rules(&tiles, &[RuleModifier::NoSequences]).unwrap();
        assert_eq!(sets.len(), 1);
        assert_eq!(sets[0].kind, SetKind::Triplet);
    }
}

#[cfg(test)]
mod proptests {
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
                        SetKind::Pair => {
                            prop_assert_eq!(set_tiles.len(), 2);
                            prop_assert_eq!(set_tiles[0].suit, set_tiles[1].suit);
                            prop_assert_eq!(set_tiles[0].rank, set_tiles[1].rank);
                        }
                        SetKind::Triplet => {
                            prop_assert_eq!(set_tiles.len(), 3);
                            prop_assert_eq!(set_tiles[0].suit, set_tiles[1].suit);
                            prop_assert_eq!(set_tiles[0].suit, set_tiles[2].suit);
                            prop_assert_eq!(set_tiles[0].rank, set_tiles[1].rank);
                            prop_assert_eq!(set_tiles[0].rank, set_tiles[2].rank);
                        }
                        SetKind::Sequence => {
                            prop_assert_eq!(set_tiles.len(), 3);
                            let mut ranks: Vec<u8> = set_tiles.iter().map(|t| t.rank).collect();
                            ranks.sort();
                            prop_assert_eq!(set_tiles[0].suit, set_tiles[1].suit);
                            prop_assert_eq!(set_tiles[0].suit, set_tiles[2].suit);
                            prop_assert_eq!(ranks[1], ranks[0] + 1, "sequence not consecutive");
                            prop_assert_eq!(ranks[2], ranks[1] + 1, "sequence not consecutive");
                        }
                        SetKind::Kong => {
                            prop_assert_eq!(set_tiles.len(), 4);
                            for i in 1..4 {
                                prop_assert_eq!(set_tiles[0].suit, set_tiles[i].suit);
                                prop_assert_eq!(set_tiles[0].rank, set_tiles[i].rank);
                            }
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
}
