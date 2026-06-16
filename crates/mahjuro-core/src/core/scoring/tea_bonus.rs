//! Shared tea-ceremony / rakuware tile-set helpers (chip and Han bonuses).

use rustc_hash::FxHashSet;

use crate::core::hand::{DetectedMeld, MeldKind};
use crate::core::tile::{Suit, Tile};

pub(crate) fn tea_harmony_fu(tiles: &[Tile]) -> Option<i32> {
    let suits: FxHashSet<Suit> = tiles
        .iter()
        .filter(|t| matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu))
        .map(|t| t.suit)
        .collect();
    (suits.len() >= 2).then_some(60)
}

pub(crate) fn tea_respect_fu(tiles: &[Tile]) -> Option<i32> {
    let honors = tiles
        .iter()
        .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
        .count() as i32;
    (honors > 0).then_some(15 * honors)
}

pub(crate) fn tea_purity_han(tiles: &[Tile]) -> Option<f64> {
    let numbered_suits: Vec<Suit> = tiles
        .iter()
        .filter(|t| matches!(t.suit, Suit::Souzu | Suit::Manzu | Suit::Pinzu))
        .map(|t| t.suit)
        .collect();
    if !numbered_suits.is_empty() && numbered_suits.iter().all(|&s| s == numbered_suits[0]) {
        Some(2.5)
    } else {
        None
    }
}

pub(crate) fn tea_tranquility_fu(sets: &[DetectedMeld]) -> Option<i32> {
    sets.iter().any(|s| s.kind == MeldKind::Pair).then_some(55)
}
