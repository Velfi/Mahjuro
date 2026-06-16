//! Structure scoring: commit melds from hand into a held area, then trigger to score.

use crate::core::hand::{DetectedMeld, kong_structure_bonus};
use crate::core::rules::RuleModifier;
use crate::core::tile::{Suit, Tile};
use crate::core::yaku::{YakuKind, detect_yaku_with_wind, is_complete_winning_hand};

/// How structure scoring was initiated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StructureTriggerKind {
    /// Player pressed Trigger with a valid structure.
    Manual,
    /// Structure completed a full winning hand.
    AutoFull,
    /// No plays left — last chance score.
    AutoNoPlays,
}

/// Metadata passed into [`crate::core::relic::ScoreContext`] when scoring from structure.
#[derive(Clone, Copy, Debug)]
pub struct StructureTriggerMeta {
    pub meld_count: u32,
    /// If no yaku detected, inject Chicken Hand (base ×1 mult).
    pub inject_chicken_if_no_yaku: bool,
}

/// Sum of tile point values played into structure — exposed for tier HUD.
pub fn played_meld_fu(tiles: &[Tile], sets: &[DetectedMeld]) -> i32 {
    sets.iter()
        .flat_map(|s| s.tile_ids.iter())
        .filter_map(|id| tiles.iter().find(|t| t.id == *id))
        .map(|t| t.point_value() as i32)
        .sum()
}

pub fn is_winning_structure_shape(tiles: &[Tile], sets: &[DetectedMeld]) -> bool {
    is_complete_winning_hand(tiles, sets)
}

/// Smallest legal play into structure is a pair (2 tiles).
const MIN_MELD_COMMIT_TILES: usize = 2;

/// Fewest tiles the player can commit on the next play under active round rules.
pub fn structure_min_commit_tiles(rules: &[RuleModifier]) -> usize {
    if rules.contains(&RuleModifier::MustPlayFive) {
        5
    } else {
        MIN_MELD_COMMIT_TILES
    }
}

/// Tile slots still open in structure before hitting the standard budget
/// (`hand_size` tiles, plus kong overflow).
pub fn structure_remaining_tile_slots(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    hand_size: usize,
) -> usize {
    let capacity = hand_size + kong_structure_bonus(sets.iter());
    capacity.saturating_sub(tiles.len())
}

/// True when structure has no room left for another meld under the standard tile
/// budget (`hand_size` tiles, plus kong overflow) and the active commit rules
/// (e.g. Bureaucratic Form requires exactly five tiles per play).
pub fn structure_cannot_grow_further(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    hand_size: usize,
    rules: &[RuleModifier],
) -> bool {
    if sets.is_empty() {
        return false;
    }
    let remaining = structure_remaining_tile_slots(tiles, sets, hand_size);
    remaining < structure_min_commit_tiles(rules)
}

/// Player-facing callout after playing into structure (tile slots left).
pub fn structure_remaining_slots_callout(remaining: usize) -> String {
    match remaining {
        0 => "Cash In Time!".to_string(),
        1 => "One slot remains empty".to_string(),
        2 => "Two slots remain empty".to_string(),
        n => format!("{n} slots remain empty"),
    }
}

fn structure_contains_honor(tiles: &[Tile], sets: &[DetectedMeld]) -> bool {
    sets.iter().any(|set| {
        set.tile_ids.iter().any(|id| {
            tiles
                .iter()
                .find(|t| t.id == *id)
                .is_some_and(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
        })
    })
}

fn yaku_non_empty_filtered(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    available: &[YakuKind],
) -> bool {
    let all = detect_yaku_with_wind(tiles, sets, round_wind, bonus_round_wind, None);
    let filtered: Vec<YakuKind> = if available.is_empty() {
        all
    } else {
        all.into_iter().filter(|y| available.contains(y)).collect()
    };
    !filtered.is_empty()
}

/// Whether the player may press Trigger.
pub fn can_trigger_structure(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    round_wind: Option<u8>,
    bonus_round_wind: Option<u8>,
    available_yaku: &[YakuKind],
    rules: &[RuleModifier],
) -> bool {
    if sets.is_empty() {
        return false;
    }
    if rules.contains(&RuleModifier::RequireHonor) && !structure_contains_honor(tiles, sets) {
        return false;
    }
    if yaku_non_empty_filtered(tiles, sets, round_wind, bonus_round_wind, available_yaku) {
        return true;
    }
    if is_winning_structure_shape(tiles, sets) {
        return true;
    }
    true
}

/// Yaku eligible for Star Tile's post-cash-in level roll.
///
/// Chicken Hand is injected when a structure cash-in has no unlocked yaku; treat
/// that cash-in as having scored yaku even if the breakdown vec were empty.
pub fn star_tile_yaku_pool(
    detected_yaku: &[YakuKind],
    structure_meta: Option<StructureTriggerMeta>,
    tiles: &[Tile],
    sets: &[DetectedMeld],
) -> Vec<YakuKind> {
    if !detected_yaku.is_empty() {
        return detected_yaku.to_vec();
    }
    if structure_meta.is_some_and(|m| m.inject_chicken_if_no_yaku)
        && is_complete_winning_hand(tiles, sets)
    {
        return vec![YakuKind::ChickenHand];
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hand::MeldKind;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn require_honor_blocks_trigger_when_any_set_lacks_honor() {
        let tiles = vec![
            tile(Suit::Souzu, 5, 0),
            tile(Suit::Souzu, 5, 1),
            tile(Suit::Souzu, 5, 2),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        assert!(!can_trigger_structure(
            &tiles,
            &sets,
            None,
            None,
            &[],
            &[RuleModifier::RequireHonor],
        ));
    }

    #[test]
    fn require_honor_allows_trigger_when_all_sets_have_honors() {
        let tiles = vec![
            tile(Suit::Dragon, 1, 0),
            tile(Suit::Dragon, 1, 1),
            tile(Suit::Dragon, 1, 2),
        ];
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        assert!(can_trigger_structure(
            &tiles,
            &sets,
            None,
            None,
            &[],
            &[RuleModifier::RequireHonor],
        ));
    }

    #[test]
    fn require_honor_allows_trigger_when_only_one_set_has_honor() {
        let tiles = vec![
            tile(Suit::Souzu, 1, 0),
            tile(Suit::Souzu, 2, 1),
            tile(Suit::Souzu, 3, 2),
            tile(Suit::Dragon, 1, 3),
            tile(Suit::Dragon, 1, 4),
            tile(Suit::Dragon, 1, 5),
        ];
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
        ];
        assert!(can_trigger_structure(
            &tiles,
            &sets,
            None,
            None,
            &[],
            &[RuleModifier::RequireHonor],
        ));
    }

    #[test]
    fn star_tile_yaku_pool_counts_injected_chicken() {
        use crate::core::hand::validate_selection;
        let tiles = vec![
            tile(Suit::Manzu, 1, 0),
            tile(Suit::Manzu, 2, 1),
            tile(Suit::Manzu, 3, 2),
            tile(Suit::Souzu, 4, 3),
            tile(Suit::Souzu, 5, 4),
            tile(Suit::Souzu, 6, 5),
            tile(Suit::Pinzu, 7, 6),
            tile(Suit::Pinzu, 8, 7),
            tile(Suit::Pinzu, 9, 8),
            tile(Suit::Pinzu, 3, 9),
            tile(Suit::Pinzu, 3, 10),
            tile(Suit::Pinzu, 3, 11),
            tile(Suit::Wind, 2, 12),
            tile(Suit::Wind, 2, 13),
        ];
        let sets = validate_selection(&tiles).expect("chicken decomposition");
        let meta = StructureTriggerMeta {
            meld_count: sets.len() as u32,
            inject_chicken_if_no_yaku: true,
        };
        let pool = star_tile_yaku_pool(&[], Some(meta), &tiles, &sets);
        assert_eq!(pool, vec![YakuKind::ChickenHand]);
    }

    #[test]
    fn structure_remaining_tile_slots_counts_kong_bonus() {
        let tiles: Vec<Tile> = (0..14).map(|i| tile(Suit::Manzu, 1, i)).collect();
        let sets = vec![
            DetectedMeld {
                kind: MeldKind::Kong,
                tile_ids: vec![0, 1, 2, 3],
            },
            DetectedMeld {
                kind: MeldKind::Triplet,
                tile_ids: (4..14).collect(),
            },
        ];
        assert_eq!(structure_remaining_tile_slots(&tiles, &sets, 14), 1);
    }

    #[test]
    fn structure_remaining_slots_callout_labels() {
        assert_eq!(structure_remaining_slots_callout(0), "Cash In Time!");
        assert_eq!(
            structure_remaining_slots_callout(1),
            "One slot remains empty"
        );
        assert_eq!(
            structure_remaining_slots_callout(2),
            "Two slots remain empty"
        );
        assert_eq!(
            structure_remaining_slots_callout(5),
            "5 slots remain empty"
        );
    }

    #[test]
    fn structure_cannot_grow_further_at_tile_capacity() {
        let tiles: Vec<Tile> = (0..14).map(|i| tile(Suit::Manzu, 1, i)).collect();
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: (0..14).collect(),
        }];
        assert!(structure_cannot_grow_further(&tiles, &sets, 14, &[]));
    }

    #[test]
    fn structure_cannot_grow_further_with_one_slot_remaining() {
        let tiles: Vec<Tile> = (0..13).map(|i| tile(Suit::Manzu, 1, i)).collect();
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: (0..13).collect(),
        }];
        assert!(structure_cannot_grow_further(&tiles, &sets, 14, &[]));
    }

    #[test]
    fn structure_can_still_grow_with_pair_slot_remaining() {
        let tiles: Vec<Tile> = (0..12).map(|i| tile(Suit::Manzu, 1, i)).collect();
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: (0..12).collect(),
        }];
        assert!(!structure_cannot_grow_further(&tiles, &sets, 14, &[]));
    }

    #[test]
    fn must_play_five_blocks_growth_with_four_slots_remaining() {
        let rules = [RuleModifier::MustPlayFive];
        let tiles: Vec<Tile> = (0..10).map(|i| tile(Suit::Manzu, 1, i)).collect();
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: (0..10).collect(),
        }];
        assert!(!structure_cannot_grow_further(&tiles, &sets, 14, &[]));
        assert!(structure_cannot_grow_further(&tiles, &sets, 14, &rules));
    }

    #[test]
    fn must_play_five_allows_one_more_five_tile_play() {
        let rules = [RuleModifier::MustPlayFive];
        let tiles: Vec<Tile> = (0..9).map(|i| tile(Suit::Manzu, 1, i)).collect();
        let sets = vec![DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: (0..9).collect(),
        }];
        assert!(!structure_cannot_grow_further(&tiles, &sets, 14, &rules));
    }
}
