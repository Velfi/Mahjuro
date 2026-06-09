//! Decomposition: set enumeration, backtracking, and flower wildcards.
//!
//! See the parent [`crate::core::hand`] module for the public `validate_selection` / `find_*` API.

use rustc_hash::{FxHashMap, FxHashSet};

use crate::core::rules::RuleModifier;
use crate::core::tile::Suit;
use crate::core::tile::Tile;

use super::DetectedMeld;
use super::MeldKind;

/// Canonical key for a decomposition — one `(kind, sorted faces)` per set.
type DecompositionKey = Vec<(MeldKind, Vec<(Suit, u8)>)>;

/// Group tiles by face (suit+rank), keeping one id list per face key.
/// Flower tiles are excluded — they're wildcards, not a groupable face.
fn face_groups(tiles: &[Tile]) -> FxHashMap<(Suit, u8), Vec<u32>> {
    let mut m: FxHashMap<(Suit, u8), Vec<u32>> = FxHashMap::default();
    for t in tiles {
        if t.is_flower() {
            continue;
        }
        m.entry((t.suit, t.rank)).or_default().push(t.id);
    }
    m
}

/// Find pairs, triplets, and kongs from multiset counts. A face with 4+ copies
/// emits a kong (preferred) before falling back to triplet/pair leftovers.
pub fn find_pairs_and_triplets(tiles: &[Tile]) -> Vec<DetectedMeld> {
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
            out.push(DetectedMeld {
                kind: MeldKind::Kong,
                tile_ids: vec![ids[i], ids[i + 1], ids[i + 2], ids[i + 3]],
            });
            i += 4;
        }
        while i + 2 < ids.len() {
            out.push(DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![ids[i], ids[i + 1], ids[i + 2]],
            });
            i += 3;
        }
        if i + 1 < ids.len() {
            out.push(DetectedMeld {
                kind: MeldKind::Pair,
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
pub fn find_sequences(tiles: &[Tile]) -> Vec<DetectedMeld> {
    let mut out = Vec::new();
    let suits = [Suit::Manzu, Suit::Souzu, Suit::Pinzu];

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
                out.push(DetectedMeld {
                    kind: MeldKind::Sequence,
                    tile_ids: take,
                });
            }
        }
    }
    out
}

/// Non-overlapping greedy merge is complex; MVP returns all detected patterns (may overlap).
pub fn detect_all_sets(tiles: &[Tile]) -> Vec<DetectedMeld> {
    let mut v = find_pairs_and_triplets(tiles);
    v.extend(find_sequences(tiles));
    v
}

/// Enumerate **every** valid meld decomposition of `tiles`. Unlike
/// [`validate_selection_with_rules`](crate::core::hand::validate_selection_with_rules), which returns the first split found,
/// this returns all of them so the caller can pick the highest-scoring one.
///
/// Includes chiitoitsu as an alternative when 14 tiles split into 7 distinct
/// pairs. De-duplicates decompositions that differ only by tile-id ordering
/// (same multiset of melds by face + kind).
pub fn enumerate_decompositions(tiles: &[Tile], rules: &[RuleModifier]) -> Vec<Vec<DetectedMeld>> {
    if tiles.is_empty() {
        return Vec::new();
    }
    if rules.contains(&RuleModifier::MustPlayFive) && tiles.len() != 5 {
        return Vec::new();
    }
    let mut regular: Vec<Tile> = tiles.iter().filter(|t| !t.is_flower()).copied().collect();
    let flower_ids: Vec<u32> = tiles
        .iter()
        .filter(|t| t.is_flower())
        .map(|t| t.id)
        .collect();
    regular.sort();

    let allow_wrap = rules.contains(&RuleModifier::SequenceWrap);
    let no_sequences = rules.contains(&RuleModifier::NoSequences);
    let require_honor = rules.contains(&RuleModifier::RequireHonor);

    let mut all: Vec<Vec<DetectedMeld>> = Vec::new();
    let mut seen: FxHashSet<DecompositionKey> = FxHashSet::default();

    let tile_lookup = |id: u32| tiles.iter().find(|t| t.id == id).copied();
    let canonicalize = |sets: &[DetectedMeld]| -> DecompositionKey {
        let mut keyed: DecompositionKey = sets
            .iter()
            .map(|s| {
                let mut faces: Vec<(Suit, u8)> = s
                    .tile_ids
                    .iter()
                    .filter_map(|&id| tile_lookup(id).map(|t| (t.suit, t.rank)))
                    .collect();
                faces.sort();
                (s.kind, faces)
            })
            .collect();
        keyed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        keyed
    };

    for (flower_melds, mut wildcards) in flower_meld_partitions_for_rules(&flower_ids, rules) {
        if regular.is_empty() {
            if wildcards.is_empty() && !flower_melds.is_empty() {
                let key = canonicalize(&flower_melds);
                if seen.insert(key) {
                    all.push(flower_melds);
                }
            }
            continue;
        }
        let mut prefix = flower_melds;
        let prefix_len = prefix.len();
        collect_decompositions(
            &regular,
            &mut wildcards,
            &mut prefix,
            allow_wrap,
            &mut |sets: &[DetectedMeld]| {
                if no_sequences && sets.iter().any(|s| s.kind == MeldKind::Sequence) {
                    return;
                }
                if require_honor
                    && !sets.iter().any(|set| {
                        set.tile_ids.iter().any(|id| {
                            tiles
                                .iter()
                                .find(|t| t.id == *id)
                                .is_some_and(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
                        })
                    })
                {
                    return;
                }
                let key = canonicalize(sets);
                if seen.insert(key) {
                    all.push(sets.to_vec());
                }
            },
        );
        prefix.truncate(prefix_len);
    }

    // Chiitoitsu as an alternative decomposition (not just a fallback).
    if flower_ids.is_empty()
        && let Some(pairs) = try_chiitoitsu(&regular)
    {
        let key = canonicalize(&pairs);
        if seen.insert(key) {
            all.push(pairs);
        }
    }

    if flower_ids.is_empty()
        && let Some(kokushi) = try_kokushi_musou(&regular)
    {
        let key = canonicalize(&kokushi);
        if seen.insert(key) {
            all.push(kokushi);
        }
    }

    all
}

/// Collector variant of [`backtrack_decompose_flowers`]: instead of returning
/// on the first success, invokes `on_found` for every complete decomposition.
fn collect_decompositions(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
    allow_wrap: bool,
    on_found: &mut dyn FnMut(&[DetectedMeld]),
) {
    if remaining.is_empty() {
        if flower_pool.is_empty() {
            on_found(found);
        }
        return;
    }
    let first = remaining[0];

    // Kong (4 of a kind).
    if remaining.len() >= 4
        && remaining[1].suit == first.suit
        && remaining[1].rank == first.rank
        && remaining[2].suit == first.suit
        && remaining[2].rank == first.rank
        && remaining[3].suit == first.suit
        && remaining[3].rank == first.rank
    {
        found.push(DetectedMeld {
            kind: MeldKind::Kong,
            tile_ids: vec![
                remaining[0].id,
                remaining[1].id,
                remaining[2].id,
                remaining[3].id,
            ],
        });
        let rest: Vec<Tile> = remaining[4..].to_vec();
        collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
        found.pop();
    }

    // Triplet.
    if remaining.len() >= 3
        && remaining[1].suit == first.suit
        && remaining[1].rank == first.rank
        && remaining[2].suit == first.suit
        && remaining[2].rank == first.rank
    {
        found.push(DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![remaining[0].id, remaining[1].id, remaining[2].id],
        });
        let rest: Vec<Tile> = remaining[3..].to_vec();
        collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
        found.pop();
    }

    // Sequence.
    if first.is_number_tile() && remaining.len() >= 3 {
        collect_sequence(remaining, flower_pool, found, allow_wrap, &first, on_found);
    }

    // Pair.
    if remaining.len() >= 2 && remaining[1].suit == first.suit && remaining[1].rank == first.rank {
        found.push(DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![remaining[0].id, remaining[1].id],
        });
        let rest: Vec<Tile> = remaining[2..].to_vec();
        collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
        found.pop();
    }

    // Flower-assisted melds.
    if !flower_pool.is_empty() {
        if remaining.len() >= 2
            && remaining[1].suit == first.suit
            && remaining[1].rank == first.rank
        {
            let fid = flower_pool.pop().expect("flower pool exhausted");
            found.push(DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![remaining[0].id, remaining[1].id, fid],
            });
            let rest: Vec<Tile> = remaining[2..].to_vec();
            collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
            found.pop();
            flower_pool.push(fid);
        }
        if first.is_number_tile() && remaining.len() >= 2 {
            collect_sequence_with_flower(
                remaining,
                flower_pool,
                found,
                allow_wrap,
                &first,
                on_found,
            );
        }
    }
}

fn collect_sequence(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
    allow_wrap: bool,
    first: &Tile,
    on_found: &mut dyn FnMut(&[DetectedMeld]),
) {
    let mid = remaining[1..]
        .iter()
        .position(|t| t.suit == first.suit && t.rank == first.rank + 1);
    if let Some(mid_offset) = mid {
        let mid_idx = mid_offset + 1;
        let search_start = mid_idx + 1;
        if let Some(hi_offset) = remaining[search_start..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == first.rank + 2)
        {
            let hi_idx = hi_offset + search_start;
            found.push(DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![remaining[0].id, remaining[mid_idx].id, remaining[hi_idx].id],
            });
            let rest: Vec<Tile> = remaining
                .iter()
                .enumerate()
                .filter_map(|(i, t)| (i != 0 && i != mid_idx && i != hi_idx).then_some(*t))
                .collect();
            collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
            found.pop();
        }
    }
    if allow_wrap {
        collect_wrap_sequence(remaining, flower_pool, found, allow_wrap, first, on_found);
    }
}

fn collect_wrap_sequence(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
    allow_wrap: bool,
    first: &Tile,
    on_found: &mut dyn FnMut(&[DetectedMeld]),
) {
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
        if let Some(mid_offset) = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == other_ranks[0])
        {
            let mid_idx = mid_offset + 1;
            if let Some(hi_idx) = remaining.iter().enumerate().position(|(i, t)| {
                i != 0 && i != mid_idx && t.suit == first.suit && t.rank == other_ranks[1]
            }) {
                found.push(DetectedMeld {
                    kind: MeldKind::Sequence,
                    tile_ids: vec![remaining[0].id, remaining[mid_idx].id, remaining[hi_idx].id],
                });
                let rest: Vec<Tile> = remaining
                    .iter()
                    .enumerate()
                    .filter_map(|(i, t)| (i != 0 && i != mid_idx && i != hi_idx).then_some(*t))
                    .collect();
                collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
                found.pop();
            }
        }
    }
}

fn collect_sequence_with_flower(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
    allow_wrap: bool,
    first: &Tile,
    on_found: &mut dyn FnMut(&[DetectedMeld]),
) {
    if first.rank <= 7
        && let Some(mid_offset) = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == first.rank + 1)
    {
        let mid_idx = mid_offset + 1;
        let fid = flower_pool.pop().expect("flower pool exhausted");
        found.push(DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![remaining[0].id, remaining[mid_idx].id, fid],
        });
        let rest: Vec<Tile> = remaining
            .iter()
            .enumerate()
            .filter_map(|(i, t)| (i != 0 && i != mid_idx).then_some(*t))
            .collect();
        collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
        found.pop();
        flower_pool.push(fid);
    }
    if first.rank <= 7
        && let Some(hi_offset) = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == first.rank + 2)
    {
        let hi_idx = hi_offset + 1;
        let fid = flower_pool.pop().expect("flower pool exhausted");
        found.push(DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![remaining[0].id, fid, remaining[hi_idx].id],
        });
        let rest: Vec<Tile> = remaining
            .iter()
            .enumerate()
            .filter_map(|(i, t)| (i != 0 && i != hi_idx).then_some(*t))
            .collect();
        collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
        found.pop();
        flower_pool.push(fid);
    }
    // Flower as low: F, first, first+1 (needs first.rank >= 2 so flower subs for a real rank).
    if first.rank >= 2
        && let Some(next_offset) = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == first.rank + 1)
    {
        let next_idx = next_offset + 1;
        let fid = flower_pool.pop().expect("flower pool exhausted");
        found.push(DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![fid, remaining[0].id, remaining[next_idx].id],
        });
        let rest: Vec<Tile> = remaining
            .iter()
            .enumerate()
            .filter_map(|(i, t)| (i != 0 && i != next_idx).then_some(*t))
            .collect();
        collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
        found.pop();
        flower_pool.push(fid);
    }
    // Wrapping sequences (8-9-1, 9-1-2) with a flower filling one missing rank.
    if allow_wrap {
        collect_wrap_sequence_with_flower(remaining, flower_pool, found, allow_wrap, first, on_found);
    }
}

fn collect_wrap_sequence_with_flower(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
    allow_wrap: bool,
    first: &Tile,
    on_found: &mut dyn FnMut(&[DetectedMeld]),
) {
    for [present_rank, _flower_rank] in wrap_flower_partner_ranks(first.rank) {
        let Some(present_offset) = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == present_rank)
        else {
            continue;
        };
        let present_idx = present_offset + 1;
        let fid = flower_pool.pop().expect("flower pool exhausted");
        found.push(DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![remaining[0].id, remaining[present_idx].id, fid],
        });
        let rest: Vec<Tile> = remaining
            .iter()
            .enumerate()
            .filter_map(|(i, t)| (i != 0 && i != present_idx).then_some(*t))
            .collect();
        collect_decompositions(&rest, flower_pool, found, allow_wrap, on_found);
        found.pop();
        flower_pool.push(fid);
    }
}

/// Detect a Chiitoitsu (seven pairs) hand: exactly 14 tiles, decomposing into
/// 7 distinct face pairs (no triplets or kongs of the same face). Returns
/// `Some(pairs)` on success.
pub(super) fn try_chiitoitsu(tiles: &[Tile]) -> Option<Vec<DetectedMeld>> {
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
        pairs.push(DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![ids[0], ids[1]],
        });
    }
    Some(pairs)
}

/// Kokushi Musō (thirteen orphans): one of each terminal and honor tile type,
/// plus one duplicate of any of those thirteen faces. Decomposes as twelve
/// [`MeldKind::Single`] sets and one [`MeldKind::Pair`]. Flowers cannot participate.
pub(super) fn try_kokushi_musou(tiles: &[Tile]) -> Option<Vec<DetectedMeld>> {
    if tiles.len() != 14 {
        return None;
    }
    let groups = face_groups(tiles);
    if groups.len() != 13 {
        return None;
    }
    let mut pair_faces = 0u32;
    for (&(suit, rank), ids) in &groups {
        if !Tile::is_kokushi_orphan_face(suit, rank) {
            return None;
        }
        match ids.len() {
            1 => {}
            2 => pair_faces += 1,
            _ => return None,
        }
    }
    if pair_faces != 1 {
        return None;
    }
    let mut keys: Vec<_> = groups.keys().copied().collect();
    keys.sort();
    let mut out = Vec::with_capacity(13);
    for key in keys {
        let ids = &groups[&key];
        if ids.len() == 2 {
            out.push(DetectedMeld {
                kind: MeldKind::Pair,
                tile_ids: vec![ids[0], ids[1]],
            });
        } else {
            out.push(DetectedMeld {
                kind: MeldKind::Single,
                tile_ids: vec![ids[0]],
            });
        }
    }
    Some(out)
}

/// Generate all ways to partition flower tile ids into "flower melds" (pairs,
/// triplets) vs leftover wildcards. Flowers can form melds with each other
/// regardless of rank — any 2 flowers make a pair, any 3 make a triplet.
/// Handles arbitrarily many flowers (e.g. from Wildflower talisman).
/// When `NoFlowerWildcards` is active, only returns partitions that consume
/// every flower in flower-only melds (no leftover wildcards).
pub(crate) fn flower_meld_partitions_for_rules(
    flower_ids: &[u32],
    rules: &[RuleModifier],
) -> Vec<(Vec<DetectedMeld>, Vec<u32>)> {
    let mut results = Vec::new();
    // Push/pop into a single shared scratch buffer instead of cloning
    // `melds_so_far` at every recursion level — was O(2^n) cloning with
    // Wildflower talisman in play.
    let mut scratch: Vec<DetectedMeld> = Vec::new();
    flower_meld_partitions_recurse(flower_ids, &mut scratch, &mut results);
    if rules.contains(&RuleModifier::NoFlowerWildcards) {
        results
            .into_iter()
            .filter(|(_, wildcards)| wildcards.is_empty())
            .collect()
    } else {
        results
    }
}

/// Hand-index bitmasks for every way to commit zero or more flower-only pairs /
/// triplets from `flowers` (each entry is `(hand_index, tile_id)`).
pub fn flower_meld_partition_masks(flowers: &[(usize, u32)], rules: &[RuleModifier]) -> Vec<u32> {
    let ids: Vec<u32> = flowers.iter().map(|(_, id)| *id).collect();
    let mut masks = vec![0u32];
    for (melds, _) in flower_meld_partitions_for_rules(&ids, rules) {
        if melds.is_empty() {
            continue;
        }
        let mut mask = 0u32;
        for meld in &melds {
            for &tile_id in &meld.tile_ids {
                if let Some(&(hand_index, _)) = flowers.iter().find(|(_, id)| *id == tile_id) {
                    mask |= 1 << hand_index;
                }
            }
        }
        if mask.count_ones() >= 2 {
            masks.push(mask);
        }
    }
    masks.sort_unstable();
    masks.dedup();
    masks
}

fn flower_meld_partitions_recurse(
    remaining: &[u32],
    scratch: &mut Vec<DetectedMeld>,
    results: &mut Vec<(Vec<DetectedMeld>, Vec<u32>)>,
) {
    // Base: leave all remaining as wildcards.
    results.push((scratch.clone(), remaining.to_vec()));

    // Try consuming a pair.
    if remaining.len() >= 2 {
        scratch.push(DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: vec![remaining[0], remaining[1]],
        });
        flower_meld_partitions_recurse(&remaining[2..], scratch, results);
        scratch.pop();
    }

    // Try consuming a triplet.
    if remaining.len() >= 3 {
        scratch.push(DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![remaining[0], remaining[1], remaining[2]],
        });
        flower_meld_partitions_recurse(&remaining[3..], scratch, results);
        scratch.pop();
    }
}

/// Recursive helper: try to decompose `remaining` (sorted, no flowers) into melds,
/// optionally consuming flower tiles from `flower_pool` as wildcards.
///
/// A flower can substitute for one missing tile in a triplet or sequence (max one
/// flower per meld). Flowers cannot pair with regular tiles (flower-only pairs
/// are handled by `flower_meld_partitions` before this function runs).
pub(super) fn backtrack_decompose_flowers(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
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
        let set = DetectedMeld {
            kind: MeldKind::Kong,
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
        let set = DetectedMeld {
            kind: MeldKind::Triplet,
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
    if first.is_number_tile()
        && remaining.len() >= 3
        && try_sequence(remaining, flower_pool, found, allow_wrap, first)
    {
        return true;
    }

    // Try pair: 2 tiles with same suit+rank (no flowers allowed in pairs).
    if remaining.len() >= 2 && remaining[1].suit == first.suit && remaining[1].rank == first.rank {
        let set = DetectedMeld {
            kind: MeldKind::Pair,
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
            let fid = flower_pool
                .pop()
                .expect("flower pool exhausted mid-backtrack");
            let set = DetectedMeld {
                kind: MeldKind::Triplet,
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
        if first.is_number_tile()
            && remaining.len() >= 2
            && try_sequence_with_flower(remaining, flower_pool, found, allow_wrap, first)
        {
            return true;
        }
    }

    false
}

/// Try to form a normal 3-tile sequence starting from `first` (no flowers).
fn try_sequence(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
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
            let set = DetectedMeld {
                kind: MeldKind::Sequence,
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
    if allow_wrap && try_wrap_sequence(remaining, flower_pool, found, allow_wrap, first) {
        return true;
    }
    false
}

/// Try wrapping sequences (8-9-1, 9-1-2) without flowers.
fn try_wrap_sequence(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
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
                let set = DetectedMeld {
                    kind: MeldKind::Sequence,
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
    found: &mut Vec<DetectedMeld>,
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
            let fid = flower_pool
                .pop()
                .expect("flower pool exhausted mid-backtrack");
            let set = DetectedMeld {
                kind: MeldKind::Sequence,
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
            let fid = flower_pool
                .pop()
                .expect("flower pool exhausted mid-backtrack");
            let set = DetectedMeld {
                kind: MeldKind::Sequence,
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

    // Case 3: flower fills the low slot — F, first, first+1.
    // Needed when the real tiles start at rank >= 2 (e.g. 8+9+F = 7-8-9),
    // since the lower-rank anchor that would drive cases 1/2 doesn't exist.
    if first.rank >= 2 {
        let next = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == first.rank + 1);
        if let Some(next_offset) = next {
            let next_idx = next_offset + 1;
            let fid = flower_pool
                .pop()
                .expect("flower pool exhausted mid-backtrack");
            let set = DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![fid, remaining[0].id, remaining[next_idx].id],
            };
            found.push(set);
            let mut rest: Vec<Tile> = Vec::with_capacity(remaining.len() - 2);
            for (i, t) in remaining.iter().enumerate() {
                if i != 0 && i != next_idx {
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

    // Wrapping sequences (8-9-1, 9-1-2) with a flower filling one missing rank.
    if allow_wrap
        && try_wrap_sequence_with_flower(remaining, flower_pool, found, allow_wrap, first)
    {
        return true;
    }

    false
}

/// Try a wrapping sequence (8-9-1, 9-1-2) where a flower supplies one of the
/// two ranks the player is missing; the partner rank must be present as a real
/// tile in `remaining`.
fn try_wrap_sequence_with_flower(
    remaining: &[Tile],
    flower_pool: &mut Vec<u32>,
    found: &mut Vec<DetectedMeld>,
    allow_wrap: bool,
    first: &Tile,
) -> bool {
    for [present_rank, _flower_rank] in wrap_flower_partner_ranks(first.rank) {
        let Some(present_offset) = remaining[1..]
            .iter()
            .position(|t| t.suit == first.suit && t.rank == present_rank)
        else {
            continue;
        };
        let present_idx = present_offset + 1;
        let fid = flower_pool
            .pop()
            .expect("flower pool exhausted mid-backtrack");
        let set = DetectedMeld {
            kind: MeldKind::Sequence,
            tile_ids: vec![remaining[0].id, remaining[present_idx].id, fid],
        };
        found.push(set);
        let mut rest: Vec<Tile> = Vec::with_capacity(remaining.len() - 2);
        for (i, t) in remaining.iter().enumerate() {
            if i != 0 && i != present_idx {
                rest.push(*t);
            }
        }
        if backtrack_decompose_flowers(&rest, flower_pool, found, allow_wrap) {
            return true;
        }
        found.pop();
        flower_pool.push(fid);
    }
    false
}

/// For a wrapping sequence anchored at `rank`, enumerate `[present, flower]`
/// rank pairs: the partner rank that must exist among the real tiles, and the
/// rank a flower fills. Each wrap pattern (8-9-1, 9-1-2) contributes both ways
/// of choosing which of its two non-anchor ranks the flower supplies.
fn wrap_flower_partner_ranks(rank: u8) -> Vec<[u8; 2]> {
    let wrap_patterns: &[[u8; 3]] = match rank {
        1 => &[[9, 1, 2], [8, 9, 1]],
        2 => &[[9, 1, 2]],
        8 => &[[8, 9, 1]],
        9 => &[[8, 9, 1], [9, 1, 2]],
        _ => &[],
    };
    let mut out = Vec::new();
    for pattern in wrap_patterns {
        let others: Vec<u8> = pattern.iter().copied().filter(|&r| r != rank).collect();
        if others.len() != 2 {
            continue;
        }
        out.push([others[0], others[1]]);
        out.push([others[1], others[0]]);
    }
    out
}
