//! Structure scoring: commit melds from hand into a held area, then trigger to score.

use crate::core::hand::DetectedMeld;
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
pub fn played_meld_chips(tiles: &[Tile], sets: &[DetectedMeld]) -> i32 {
    sets.iter()
        .flat_map(|s| s.tile_ids.iter())
        .filter_map(|id| tiles.iter().find(|t| t.id == *id))
        .map(|t| t.point_value() as i32)
        .sum()
}

pub fn is_winning_structure_shape(tiles: &[Tile], sets: &[DetectedMeld]) -> bool {
    is_complete_winning_hand(tiles, sets)
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
}
