//! Shared tea-ceremony / rakuware tile-set helpers (chip and mult bonuses).

use std::collections::HashSet;

use crate::core::hand::{DetectedSet, SetKind};
use crate::core::tile::{Suit, Tile};

pub(crate) fn tea_harmony_chips(tiles: &[Tile]) -> Option<i32> {
    let suits: HashSet<Suit> = tiles
        .iter()
        .filter(|t| matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles))
        .map(|t| t.suit)
        .collect();
    (suits.len() >= 2).then_some(40)
}

pub(crate) fn tea_respect_chips(tiles: &[Tile]) -> Option<i32> {
    let honors = tiles
        .iter()
        .filter(|t| matches!(t.suit, Suit::Wind | Suit::Dragon))
        .count() as i32;
    (honors > 0).then_some(10 * honors)
}

pub(crate) fn tea_purity_mult(tiles: &[Tile]) -> Option<f64> {
    let numbered_suits: Vec<Suit> = tiles
        .iter()
        .filter(|t| matches!(t.suit, Suit::Bamboos | Suit::Characters | Suit::Circles))
        .map(|t| t.suit)
        .collect();
    if !numbered_suits.is_empty() && numbered_suits.iter().all(|&s| s == numbered_suits[0]) {
        Some(1.5)
    } else {
        None
    }
}

pub(crate) fn tea_tranquility_chips(sets: &[DetectedSet]) -> Option<i32> {
    sets.iter()
        .any(|s| s.kind == SetKind::Pair)
        .then_some(35)
}
