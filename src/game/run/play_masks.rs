//! Candidate play bitmasks for structure commits (game + bot).

use crate::core::hand::validate_selection_with_rules;
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};

#[derive(Clone, Copy)]
struct IndexedTile {
    hand_index: usize,
    tile: Tile,
}

pub fn enumerate_candidate_play_masks(hand: &[Tile], rules: &[RuleModifier]) -> Vec<u32> {
    let mut regular = Vec::with_capacity(hand.len());
    let mut flowers = Vec::new();
    for (hand_index, &tile) in hand.iter().enumerate() {
        let indexed = IndexedTile { hand_index, tile };
        if tile.is_flower() {
            flowers.push(indexed);
        } else {
            regular.push(indexed);
        }
    }
    regular.sort_by_key(|it| it.tile);
    flowers.sort_by_key(|it| it.tile);

    let subset_rules = PlayMaskRules {
        allow_wrap: rules.contains(&RuleModifier::SequenceWrap),
        no_sequences: rules.contains(&RuleModifier::NoSequences),
        require_honor: rules.contains(&RuleModifier::RequireHonor),
        must_play_five: rules.contains(&RuleModifier::MustPlayFive),
        no_flower_wildcards: rules.contains(&RuleModifier::NoFlowerWildcards),
    };

    let mut masks: rustc_hash::FxHashSet<u32> = rustc_hash::FxHashSet::default();
    enumerate_regular_subsets(&regular, &flowers, 0, subset_rules, 0, &mut masks);
    push_kokushi_play_masks(hand, rules, &mut masks);
    let mut out: Vec<u32> = masks.into_iter().collect();
    out.sort_unstable();
    out
}

#[derive(Clone, Copy)]
struct PlayMaskRules {
    allow_wrap: bool,
    no_sequences: bool,
    require_honor: bool,
    must_play_five: bool,
    no_flower_wildcards: bool,
}

fn enumerate_regular_subsets(
    remaining: &[IndexedTile],
    flowers: &[IndexedTile],
    current_mask: u32,
    rules: PlayMaskRules,
    current_tile_count: usize,
    out: &mut rustc_hash::FxHashSet<u32>,
) {
    let PlayMaskRules {
        allow_wrap,
        no_sequences,
        require_honor,
        must_play_five,
        no_flower_wildcards,
    } = rules;
    if current_tile_count > 14 || (must_play_five && current_tile_count > 5) {
        return;
    }

    if remaining.is_empty() {
        emit_leaf_masks(flowers, current_mask, current_tile_count, rules, out);
        return;
    }

    let first = remaining[0];
    enumerate_regular_subsets(
        &remaining[1..],
        flowers,
        current_mask,
        rules,
        current_tile_count,
        out,
    );

    if remaining.len() >= 2
        && same_face(first.tile, remaining[1].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile]))
    {
        enumerate_regular_subsets(
            &remaining[2..],
            flowers,
            current_mask | (1 << first.hand_index) | (1 << remaining[1].hand_index),
            rules,
            current_tile_count + 2,
            out,
        );
    }

    if remaining.len() >= 3
        && same_face(first.tile, remaining[1].tile)
        && same_face(first.tile, remaining[2].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile, remaining[2].tile]))
    {
        enumerate_regular_subsets(
            &remaining[3..],
            flowers,
            current_mask
                | (1 << first.hand_index)
                | (1 << remaining[1].hand_index)
                | (1 << remaining[2].hand_index),
            rules,
            current_tile_count + 3,
            out,
        );
    }

    if remaining.len() >= 4
        && same_face(first.tile, remaining[1].tile)
        && same_face(first.tile, remaining[2].tile)
        && same_face(first.tile, remaining[3].tile)
        && (!require_honor
            || tiles_have_honor(&[
                first.tile,
                remaining[1].tile,
                remaining[2].tile,
                remaining[3].tile,
            ]))
    {
        enumerate_regular_subsets(
            &remaining[4..],
            flowers,
            current_mask
                | (1 << first.hand_index)
                | (1 << remaining[1].hand_index)
                | (1 << remaining[2].hand_index)
                | (1 << remaining[3].hand_index),
            rules,
            current_tile_count + 4,
            out,
        );
    }

    let can_use_flower_wildcard = !flowers.is_empty() && !no_flower_wildcards;
    if can_use_flower_wildcard
        && remaining.len() >= 2
        && same_face(first.tile, remaining[1].tile)
        && (!require_honor || tiles_have_honor(&[first.tile, remaining[1].tile]))
    {
        for (flower_idx, flower) in flowers.iter().copied().enumerate() {
            enumerate_regular_subsets(
                &remaining[2..],
                &remove_flower(flowers, flower_idx),
                current_mask
                    | (1 << first.hand_index)
                    | (1 << remaining[1].hand_index)
                    | (1 << flower.hand_index),
                rules,
                current_tile_count + 3,
                out,
            );
        }
    }

    if !no_sequences && first.tile.is_number_tile() && !require_honor {
        for seq in sequence_candidates(remaining, allow_wrap, can_use_flower_wildcard, first) {
            let mut next_mask = current_mask | (1 << first.hand_index);
            let mut remove = vec![0usize];
            for idx in seq.regular_indices {
                next_mask |= 1 << remaining[idx].hand_index;
                remove.push(idx);
            }
            let rest = remove_indices(remaining, &remove);
            if seq.uses_flower {
                for (flower_idx, flower) in flowers.iter().copied().enumerate() {
                    enumerate_regular_subsets(
                        &rest,
                        &remove_flower(flowers, flower_idx),
                        next_mask | (1 << flower.hand_index),
                        rules,
                        current_tile_count + 3,
                        out,
                    );
                }
            } else {
                enumerate_regular_subsets(
                    &rest,
                    flowers,
                    next_mask,
                    rules,
                    current_tile_count + 3,
                    out,
                );
            }
        }
    }
}

fn emit_leaf_masks(
    flowers: &[IndexedTile],
    current_mask: u32,
    current_tile_count: usize,
    rules: PlayMaskRules,
    out: &mut rustc_hash::FxHashSet<u32>,
) {
    let must_play_five = rules.must_play_five;
    let round_rules = if rules.no_flower_wildcards {
        &[RuleModifier::NoFlowerWildcards][..]
    } else {
        &[][..]
    };
    for extra_mask in flower_meld_partition_masks(flowers, round_rules) {
        let total_mask = current_mask | extra_mask;
        let total_count = total_mask.count_ones() as usize;
        if total_count == 0 {
            continue;
        }
        if must_play_five {
            if total_count == 5 {
                out.insert(total_mask);
            }
        } else if total_count >= current_tile_count {
            out.insert(total_mask);
        }
    }
}

fn flower_meld_partition_masks(flowers: &[IndexedTile], rules: &[RuleModifier]) -> Vec<u32> {
    let indexed: Vec<(usize, u32)> = flowers.iter().map(|f| (f.hand_index, f.tile.id)).collect();
    crate::core::hand::decomposition::flower_meld_partition_masks(&indexed, rules)
}

fn remove_flower(flowers: &[IndexedTile], remove_idx: usize) -> Vec<IndexedTile> {
    flowers
        .iter()
        .enumerate()
        .filter_map(|(idx, flower)| (idx != remove_idx).then_some(*flower))
        .collect()
}

fn same_face(a: Tile, b: Tile) -> bool {
    a.suit == b.suit && a.rank == b.rank
}

fn tiles_have_honor(tiles: &[Tile]) -> bool {
    tiles
        .iter()
        .any(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
}

fn remove_indices(remaining: &[IndexedTile], remove: &[usize]) -> Vec<IndexedTile> {
    let mut remove_flags = vec![false; remaining.len()];
    for &idx in remove {
        remove_flags[idx] = true;
    }
    remaining
        .iter()
        .enumerate()
        .filter_map(|(idx, tile)| (!remove_flags[idx]).then_some(*tile))
        .collect()
}

#[derive(Clone, Copy)]
struct SequenceCandidate {
    regular_indices: [usize; 2],
    uses_flower: bool,
}

fn sequence_candidates(
    remaining: &[IndexedTile],
    allow_wrap: bool,
    can_use_flower: bool,
    first: IndexedTile,
) -> Vec<SequenceCandidate> {
    let mut out = Vec::new();
    push_sequence_candidate(
        remaining,
        first.tile.suit,
        [first.tile.rank + 1, first.tile.rank + 2],
        false,
        &mut out,
    );
    if can_use_flower {
        push_sequence_candidate(
            remaining,
            first.tile.suit,
            [first.tile.rank + 1],
            true,
            &mut out,
        );
        push_sequence_candidate(
            remaining,
            first.tile.suit,
            [first.tile.rank + 2],
            true,
            &mut out,
        );
    }
    if allow_wrap {
        for needs in wrap_sequence_needs(first.tile.rank) {
            push_sequence_candidate(remaining, first.tile.suit, *needs, false, &mut out);
        }
        if can_use_flower {
            for needs in wrap_sequence_needs(first.tile.rank) {
                push_sequence_candidate(remaining, first.tile.suit, [needs[0]], true, &mut out);
                push_sequence_candidate(remaining, first.tile.suit, [needs[1]], true, &mut out);
            }
        }
    }
    out
}

fn push_sequence_candidate(
    remaining: &[IndexedTile],
    suit: Suit,
    needed_ranks: impl AsRef<[u8]>,
    uses_flower: bool,
    out: &mut Vec<SequenceCandidate>,
) {
    let needed_ranks = needed_ranks.as_ref();
    let mut found = Vec::with_capacity(needed_ranks.len());
    for &rank in needed_ranks {
        let Some((idx, _)) = remaining.iter().enumerate().skip(1).find(|(_, tile)| {
            tile.tile.suit == suit && tile.tile.rank == rank && !found.contains(&tile.hand_index)
        }) else {
            return;
        };
        found.push(remaining[idx].hand_index);
    }

    let mut regular_indices = [0usize; 2];
    for (i, found_hand_index) in found.iter().enumerate() {
        let Some((remaining_idx, _)) = remaining
            .iter()
            .enumerate()
            .find(|(_, tile)| tile.hand_index == *found_hand_index)
        else {
            return;
        };
        regular_indices[i] = remaining_idx;
    }
    out.push(SequenceCandidate {
        regular_indices,
        uses_flower,
    });
}

/// Advance `pos` (length k, strictly increasing) to the next k-combination of indices `0..n`.
fn next_combination_in_range(pos: &mut [usize], n: usize) -> bool {
    let k = pos.len();
    if k == 0 || k > n {
        return false;
    }
    for i in (0..k).rev() {
        let upper = n - k + i;
        if pos[i] < upper {
            pos[i] += 1;
            for j in i + 1..k {
                pos[j] = pos[j - 1] + 1;
            }
            return true;
        }
    }
    false
}

/// Kokushi Musō: twelve singletons + one pair — the meld enumerator never emits these.
fn push_kokushi_play_masks(
    hand: &[Tile],
    rules: &[RuleModifier],
    out: &mut rustc_hash::FxHashSet<u32>,
) {
    if rules.contains(&RuleModifier::MustPlayFive) {
        return;
    }
    let pool: Vec<usize> = hand
        .iter()
        .enumerate()
        .filter(|(_, t)| !t.is_flower() && t.is_kokushi_orphan())
        .map(|(i, _)| i)
        .collect();
    let olen = pool.len();
    if olen < 14 {
        return;
    }
    let mut pos: Vec<usize> = (0..14).collect();
    loop {
        let mask: u32 = pos.iter().fold(0u32, |acc, &pi| acc | (1u32 << pool[pi]));
        let tiles: Vec<Tile> = pos.iter().map(|&pi| hand[pool[pi]]).collect();
        if validate_selection_with_rules(&tiles, rules).is_some() {
            out.insert(mask);
        }
        if !next_combination_in_range(&mut pos, olen) {
            break;
        }
    }
}

fn wrap_sequence_needs(rank: u8) -> &'static [[u8; 2]] {
    match rank {
        1 => &[[2, 3], [9, 2], [8, 9]],
        2 => &[[3, 4], [1, 3], [9, 1]],
        3 => &[[4, 5], [2, 4], [1, 2]],
        4 => &[[5, 6], [3, 5], [2, 3]],
        5 => &[[6, 7], [4, 6], [3, 4]],
        6 => &[[7, 8], [5, 7], [4, 5]],
        7 => &[[8, 9], [6, 8], [5, 6]],
        8 => &[[9, 1], [7, 9], [6, 7]],
        9 => &[[8, 1], [1, 2], [7, 8]],
        _ => &[],
    }
}
