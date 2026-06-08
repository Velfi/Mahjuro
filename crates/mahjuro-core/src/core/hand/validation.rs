//! Selection validation: rule checks and meld-completable hints.
//!
//! Decomposition logic lives in [`super::decomposition`].

use rustc_hash::{FxHashMap, FxHashSet};

use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};

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
    for (flower_melds, mut wildcards) in
        decomposition::flower_meld_partitions_for_rules(&flower_ids, rules)
    {
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

/// Tile ids in `tiles` that are not part of any meld in a valid play.
///
/// When the selection decomposes cleanly, returns ids left over (unused flowers, etc.).
/// When it does not, greedily packs non-overlapping melds from [`decomposition::detect_all_sets`]
/// and marks anything uncovered. If every tile is covered but validation still fails (e.g. boss
/// tile-count rules), all tile ids are returned so the hand still gets feedback.
pub fn non_contributing_tile_ids(tiles: &[Tile], rules: &[RuleModifier]) -> Vec<u32> {
    if tiles.is_empty() {
        return Vec::new();
    }
    if let Some(sets) = validate_selection_with_rules(tiles, rules) {
        let used: FxHashSet<u32> = sets
            .iter()
            .flat_map(|s| s.tile_ids.iter().copied())
            .collect();
        return tiles
            .iter()
            .filter(|t| !used.contains(&t.id))
            .map(|t| t.id)
            .collect();
    }

    let mut melds = decomposition::detect_all_sets(tiles);
    melds.sort_by(|a, b| {
        b.tile_ids
            .len()
            .cmp(&a.tile_ids.len())
            .then_with(|| a.kind.cmp(&b.kind))
    });
    let mut used = FxHashSet::default();
    let mut contributing = FxHashSet::default();
    for meld in melds {
        if meld.tile_ids.iter().all(|id| !used.contains(id)) {
            for &id in &meld.tile_ids {
                used.insert(id);
                contributing.insert(id);
            }
        }
    }
    let mut out: Vec<u32> = tiles
        .iter()
        .filter(|t| !contributing.contains(&t.id))
        .map(|t| t.id)
        .collect();
    if out.is_empty() {
        out = tiles.iter().map(|t| t.id).collect();
    }
    out
}

/// Non-overlapping meld groups for invalid partial selections in the staging-zone preview.
///
/// Valid selections should go through [`validate_selection_with_rules`] (or the run's
/// `try_validate_with_wildcards` + decomposition pick) instead — this is only the fallback
/// when the full selection cannot be played as-is.
pub fn staging_preview_melds(
    tiles: &[Tile],
    rules: &[RuleModifier],
) -> (Vec<DetectedMeld>, Vec<u32>) {
    if tiles.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if let Some(sets) = validate_selection_with_rules(tiles, rules) {
        return (sets, Vec::new());
    }

    let melds = greedy_staging_preview_melds(tiles, rules);
    let used: FxHashSet<u32> = melds
        .iter()
        .flat_map(|s| s.tile_ids.iter().copied())
        .collect();
    let bad: Vec<u32> = tiles
        .iter()
        .filter(|t| !used.contains(&t.id))
        .map(|t| t.id)
        .collect();
    (melds, bad)
}

/// Greedy partial meld packing when neither the full selection nor its contributing
/// subset validates cleanly.
fn greedy_staging_preview_melds(tiles: &[Tile], rules: &[RuleModifier]) -> Vec<DetectedMeld> {
    let mut remaining: FxHashSet<u32> = tiles.iter().map(|t| t.id).collect();
    let tile_by_id =
        |id: u32| -> Option<Tile> { tiles.iter().find(|t| t.id == id).copied() };
    let mut picked = Vec::new();

    let mut flower_ids: Vec<u32> = tiles
        .iter()
        .filter(|t| t.is_flower())
        .map(|t| t.id)
        .collect();
    flower_ids.sort_unstable();
    while flower_ids.len() >= 3 {
        let group: Vec<u32> = flower_ids.drain(..3).collect();
        for id in &group {
            remaining.remove(id);
        }
        picked.push(DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: group,
        });
    }
    if flower_ids.len() >= 2 {
        let group: Vec<u32> = flower_ids.drain(..2).collect();
        for id in &group {
            remaining.remove(id);
        }
        picked.push(DetectedMeld {
            kind: MeldKind::Pair,
            tile_ids: group,
        });
    }
    let mut leftover_flowers = flower_ids;

    if !rules.contains(&RuleModifier::NoFlowerWildcards) && !leftover_flowers.is_empty() {
        let mut face_map: FxHashMap<(Suit, u8), Vec<u32>> = FxHashMap::default();
        for &id in &remaining {
            if let Some(t) = tile_by_id(id)
                && !t.is_flower()
            {
                face_map.entry((t.suit, t.rank)).or_default().push(id);
            }
        }
        let mut faces: Vec<(Suit, u8)> = face_map.keys().copied().collect();
        faces.sort_by_key(|face| std::cmp::Reverse(face_map[face].len()));
        for face in faces {
            let ids = face_map.get_mut(&face).expect("face key");
            while ids.len() >= 2 && !leftover_flowers.is_empty() {
                let flower = leftover_flowers.pop().expect("flower available");
                let id2 = ids.pop().expect("second tile");
                let id1 = ids.pop().expect("first tile");
                remaining.remove(&id1);
                remaining.remove(&id2);
                remaining.remove(&flower);
                picked.push(DetectedMeld {
                    kind: MeldKind::Triplet,
                    tile_ids: vec![id1, id2, flower],
                });
            }
        }
    }

    let remaining_tiles: Vec<Tile> = remaining
        .iter()
        .filter_map(|&id| tile_by_id(id))
        .collect();
    let mut regular_melds = decomposition::detect_all_sets(&remaining_tiles);
    regular_melds.sort_by(|a, b| {
        b.tile_ids
            .len()
            .cmp(&a.tile_ids.len())
            .then_with(|| a.kind.cmp(&b.kind))
    });
    let mut used = FxHashSet::default();
    for meld in regular_melds {
        if meld.tile_ids.iter().all(|id| remaining.contains(id) && !used.contains(id)) {
            for &id in &meld.tile_ids {
                used.insert(id);
                remaining.remove(&id);
            }
            picked.push(meld);
        }
    }

    picked
}

/// Short player-facing copy for why a selection cannot be played as a meld.
pub fn selection_rejection_hint(tiles: &[Tile], rules: &[RuleModifier]) -> String {
    if tiles.is_empty() {
        return "Select tiles to form a valid meld.".to_string();
    }
    if rules.contains(&RuleModifier::MustPlayFive) && tiles.len() != 5 {
        return format!(
            "This round requires exactly 5 tiles per play — you selected {}.",
            tiles.len()
        );
    }
    if tiles.len() == 1 {
        return "A meld needs at least two matching tiles, or three for a run.".to_string();
    }

    let bad_ids = non_contributing_tile_ids(tiles, rules);
    if bad_ids.len() == tiles.len() {
        return match tiles.len() {
            2 => "These two tiles aren't a pair — select two of the same tile.".to_string(),
            _ if same_numbered_suit(tiles) => {
                "These tiles aren't a run — pick three consecutive tiles in the same suit."
                    .to_string()
            }
            _ => {
                "These tiles don't form a valid meld — try a pair, triplet, or three-in-a-row."
                    .to_string()
            }
        };
    }
    if !bad_ids.is_empty() {
        return "Some selected tiles don't fit this meld — deselect the extras.".to_string();
    }
    "These tiles don't form a valid meld.".to_string()
}

fn same_numbered_suit(tiles: &[Tile]) -> bool {
    use crate::core::tile::Suit;
    let mut suit = None;
    for tile in tiles {
        if tile.is_flower() {
            return false;
        }
        if !matches!(tile.suit, Suit::Manzu | Suit::Souzu | Suit::Pinzu) {
            return false;
        }
        match suit {
            None => suit = Some(tile.suit),
            Some(s) if s == tile.suit => {}
            _ => return false,
        }
    }
    suit.is_some()
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
