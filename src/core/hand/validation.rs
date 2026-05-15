//! Selection validation: rule checks and meld-completable hints.
//!
//! Decomposition logic lives in [`super::decomposition`].

use crate::core::rules::RuleModifier;
use crate::core::tile::Tile;

use super::decomposition;
use super::{DetectedMeld, MeldKind};

/// Like `validate_selection`, but respects active rule modifiers:
/// - `SequenceWrap`: allows wrapping sequences (8-9-1, 9-1-2)
/// - `NoSequences`: rejects any decomposition containing sequences
/// - `MustPlayFour`: rejects selections that aren't exactly 4 tiles
/// - `RequireHonor`: rejects decompositions with no honor tile anywhere
pub fn validate_selection_with_rules(
    tiles: &[Tile],
    rules: &[RuleModifier],
) -> Option<Vec<DetectedMeld>> {
    if tiles.is_empty() {
        return None;
    }
    // Pre-validation rejects from boss effects. These run before decomposition
    // so we don't waste cycles on hands the boss already disqualifies.
    if rules.contains(&RuleModifier::MustPlayFive) && tiles.len() != 5 {
        return None;
    }
    // Partition into regular tiles and flower wildcards.
    let mut regular: Vec<Tile> = tiles.iter().filter(|t| !t.is_flower()).copied().collect();
    let flower_ids: Vec<u32> = tiles
        .iter()
        .filter(|t| t.is_flower())
        .map(|t| t.id)
        .collect();
    regular.sort();

    let allow_wrap = rules.contains(&RuleModifier::SequenceWrap);
    let no_sequences = rules.contains(&RuleModifier::NoSequences);

    // Try each way of splitting flowers into their own melds vs wildcards.
    // Flowers can form pairs (any 2) or triplets (any 3) with each other
    // regardless of rank, in any combination.
    for (flower_melds, mut wildcards) in decomposition::flower_meld_partitions(&flower_ids) {
        if regular.is_empty() {
            // Flower-only hand: valid only when all flowers are consumed as melds.
            if wildcards.is_empty() && !flower_melds.is_empty() {
                return Some(flower_melds);
            }
            continue;
        }

        let mut result = flower_melds;
        if decomposition::backtrack_decompose_flowers(
            &regular,
            &mut wildcards,
            &mut result,
            allow_wrap,
        ) {
            if no_sequences && result.iter().any(|s| s.kind == MeldKind::Sequence) {
                continue;
            }
            if rules.contains(&RuleModifier::RequireHonor)
                && !result.iter().any(|set| {
                    set.tile_ids.iter().any(|id| {
                        tiles.iter().find(|t| t.id == *id).is_some_and(|t| {
                            matches!(
                                t.suit,
                                crate::core::tile::Suit::Wind | crate::core::tile::Suit::Dragon
                            )
                        })
                    })
                })
            {
                continue;
            }
            return Some(result);
        }
    }

    // Chiitoitsu fallback: 14 tiles forming 7 distinct pairs is a valid hand
    // even though it doesn't fit the standard 4-meld + 1-pair decomposition.
    // We try this only when the standard backtracker fails so we don't reframe
    // hands that could decompose normally. Flowers can't help with chiitoitsu
    // (pairs only, no wildcard substitution in pairs).
    if flower_ids.is_empty()
        && let Some(pairs) = decomposition::try_chiitoitsu(&regular)
    {
        return Some(pairs);
    }

    // Kokushi Musō: twelve distinct orphan singletons + one pair on an orphan face.
    if flower_ids.is_empty()
        && let Some(kokushi) = decomposition::try_kokushi_musou(&regular)
    {
        return Some(kokushi);
    }
    None
}

/// Validate that a selection of tiles decomposes perfectly into melds (pairs, triplets,
/// sequences) with no leftover tiles.  Returns the decomposition if valid, `None` otherwise.
///
/// Uses recursive backtracking: at each step, tries to extract a pair, triplet, or sequence
/// starting from the first remaining tile, then recurses on the rest.
///
/// Flower tiles act as wildcards: each can substitute for one missing tile in a triplet
/// or sequence (max one flower per meld). Flowers can also form their own melds with
/// each other regardless of rank: any 2 flowers make a pair, any 3 a triplet, and any
/// 4 form two pairs. Flowers cannot pair with regular tiles.
pub fn validate_selection(tiles: &[Tile]) -> Option<Vec<DetectedMeld>> {
    validate_selection_with_rules(tiles, &[])
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
