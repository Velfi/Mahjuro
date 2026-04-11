//! Structure scoring: commit melds from hand into a held area, then trigger to score.

use crate::core::hand::DetectedSet;
use crate::core::tile::Tile;
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
    /// Distinguishes manual vs auto triggers for relic tuning / UI (scoring uses other fields first).
    #[allow(dead_code)]
    pub kind: StructureTriggerKind,
    pub meld_count: u32,
    /// Multiply cumulative mult after yaku/relic phases (1.0 = no penalty).
    pub early_cashout_mult: f64,
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

/// Chip pile tier 0..=5 from plan (banked meld chips only).
pub fn chip_pile_tier(banked: i32) -> u8 {
    if banked <= 0 {
        0
    } else if banked <= 40 {
        1
    } else if banked <= 100 {
        2
    } else if banked <= 200 {
        3
    } else if banked <= 320 {
        4
    } else {
        5
    }
}

/// Mult pile tier 0..=4 from plan (melds in structure).
pub fn mult_pile_tier(meld_count: usize) -> u8 {
    match meld_count {
        0 => 0,
        1 => 1,
        2 | 3 => 2,
        4 | 5 => 3,
        _ => 4,
    }
}

/// +mult per meld after the first, capped (applied at trigger).
pub fn structure_depth_mult_bonus(meld_count: u32) -> f64 {
    let extra = meld_count.saturating_sub(1) as f64 * 0.1;
    extra.min(3.0)
}

pub fn is_winning_structure_shape(tiles: &[Tile], sets: &[DetectedSet]) -> bool {
    is_complete_winning_hand(tiles, sets)
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
) -> bool {
    if sets.is_empty() {
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

/// Early mult factor when manually triggering before a full winning shape.
pub fn early_cashout_factor(tiles: &[Tile], sets: &[DetectedSet]) -> f64 {
    if is_winning_structure_shape(tiles, sets) {
        return 1.0;
    }
    let n = tiles.len().min(14) as f64;
    (0.5 + 0.5 * (n / 14.0)).clamp(0.25, 1.0)
}
