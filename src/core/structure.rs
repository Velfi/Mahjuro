//! Structure scoring: commit melds from hand into a held area, then trigger to score.

use crate::core::hand::DetectedSet;
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

/// Sum of meld base chip bonuses — exposed for tier HUD.
pub fn banked_meld_chips(sets: &[DetectedSet]) -> i32 {
    use crate::core::hand::SetKind;
    sets.iter()
        .map(|s| match s.kind {
            SetKind::Pair => 18,
            SetKind::Sequence => 28,
            SetKind::Triplet => 50,
            SetKind::Kong => 80,
        })
        .sum()
}

/// +mult per meld after the first, capped (applied at trigger).
pub fn structure_depth_mult_bonus(meld_count: u32) -> f64 {
    let extra = meld_count.saturating_sub(1) as f64 * 0.1;
    extra.min(3.0)
}

pub fn is_winning_structure_shape(tiles: &[Tile], sets: &[DetectedSet]) -> bool {
    is_complete_winning_hand(tiles, sets)
}

fn structure_contains_honor(tiles: &[Tile], sets: &[DetectedSet]) -> bool {
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
    sets: &[DetectedSet],
    round_wind: Option<u8>,
    available: &[YakuKind],
) -> bool {
    let all = detect_yaku_with_wind(tiles, sets, round_wind, None);
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
    sets: &[DetectedSet],
    round_wind: Option<u8>,
    available_yaku: &[YakuKind],
    rules: &[RuleModifier],
) -> bool {
    if sets.is_empty() {
        return false;
    }
    if rules.contains(&RuleModifier::RequireHonor) && !structure_contains_honor(tiles, sets) {
        return false;
    }
    if yaku_non_empty_filtered(tiles, sets, round_wind, available_yaku) {
        return true;
    }
    if is_winning_structure_shape(tiles, sets) {
        return true;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::hand::SetKind;

    fn tile(suit: Suit, rank: u8, id: u32) -> Tile {
        Tile::new(suit, rank, id)
    }

    #[test]
    fn require_honor_blocks_trigger_when_any_set_lacks_honor() {
        let tiles = vec![
            tile(Suit::Bamboos, 5, 0),
            tile(Suit::Bamboos, 5, 1),
            tile(Suit::Bamboos, 5, 2),
        ];
        let sets = vec![DetectedSet {
            kind: SetKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        assert!(!can_trigger_structure(
            &tiles,
            &sets,
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
        let sets = vec![DetectedSet {
            kind: SetKind::Triplet,
            tile_ids: vec![0, 1, 2],
        }];
        assert!(can_trigger_structure(
            &tiles,
            &sets,
            None,
            &[],
            &[RuleModifier::RequireHonor],
        ));
    }

    #[test]
    fn require_honor_allows_trigger_when_only_one_set_has_honor() {
        let tiles = vec![
            tile(Suit::Bamboos, 1, 0),
            tile(Suit::Bamboos, 2, 1),
            tile(Suit::Bamboos, 3, 2),
            tile(Suit::Dragon, 1, 3),
            tile(Suit::Dragon, 1, 4),
            tile(Suit::Dragon, 1, 5),
        ];
        let sets = vec![
            DetectedSet {
                kind: SetKind::Sequence,
                tile_ids: vec![0, 1, 2],
            },
            DetectedSet {
                kind: SetKind::Triplet,
                tile_ids: vec![3, 4, 5],
            },
        ];
        assert!(can_trigger_structure(
            &tiles,
            &sets,
            None,
            &[],
            &[RuleModifier::RequireHonor],
        ));
    }
}
