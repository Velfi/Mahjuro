//! Score hands as **chips × mult**, Balatro-style.
//!
//! Each scored hand accumulates two parallel running totals:
//!
//! * **Chips** — additive. Tiles contribute their rank value (honors flat 10),
//!   melds add a flat bonus, and chip-flavored relics pile on more.
//! * **Mult** — multiplicative axis, but built additively (`+N mult`) so it
//!   stacks fast and predictably. Yaku and "explosive" relics live here.
//!
//! Final score = `final_chips × final_mult` (floored).
//!
//! The cascade UI walks the `steps` in order, updating both running totals.
//! The last visible beat is the multiplication itself.

mod pipeline;
#[cfg(test)]
mod tests;

use crate::core::hand::{DetectedSet, SetKind};
use crate::core::tile::Tile;
use crate::core::yaku::YakuKind;

#[cfg(test)]
pub use pipeline::score_sets;
pub use pipeline::score_sets_with_original;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    Chips,
    Mult,
    Gold,
    Final,
}

#[derive(Clone, Debug)]
pub struct ScoreStep {
    pub source: String,
    pub kind: StepKind,
    pub tile_ids: Vec<u32>,

    pub running_chips: i32,

    pub running_mult: f64,
    pub running_total: u64,
}

#[derive(Clone, Debug)]
pub struct ScoreBreakdown {
    pub base_chips: i32,
    pub base_points: i32,
    pub base_steps: Vec<ScoreStep>,
    pub steps: Vec<ScoreStep>,
    pub detected_yaku: Vec<YakuKind>,

    pub final_chips: i32,

    pub final_mult: f64,
    pub total: u64,
    pub flower_gold: i32,

    pub scored_set_kinds: Vec<crate::core::hand::SetKind>,
}

pub(crate) fn meld_chip_bonus(kind: SetKind) -> i32 {
    match kind {
        SetKind::Pair => 18,
        SetKind::Sequence => 28,
        SetKind::Triplet => 50,
        SetKind::Kong => 80,
    }
}

pub(crate) fn describe_set(tiles: &[Tile], set: &DetectedSet) -> String {
    let label = match set.kind {
        SetKind::Pair => "Pair",
        SetKind::Sequence => "Sequence",
        SetKind::Triplet => "Triplet",
        SetKind::Kong => "Kong",
    };
    let faces = set
        .tile_ids
        .iter()
        .filter_map(|id| tile_by_id(tiles, *id))
        .map(Tile::label)
        .collect::<Vec<_>>()
        .join(" ");
    format!("{label}  {faces}")
}

pub(crate) fn tile_by_id(tiles: &[Tile], id: u32) -> Option<&Tile> {
    tiles.iter().find(|t| t.id == id)
}

pub(crate) fn tile_is_debuffed(tile: &Tile, debuffs: &[crate::core::debuff::TileDebuff]) -> bool {
    debuffs.iter().any(|debuff| debuff.matches(tile))
}

pub(crate) fn combine(chips: i32, mult: f64) -> u64 {
    ((chips.max(0) as f64) * mult)
        .floor()
        .clamp(0.0, u64::MAX as f64) as u64
}

pub(crate) fn fmt_mult(m: f64) -> String {
    if (m - m.round()).abs() < 1e-6 {
        format!("×{}", m.round() as i64)
    } else {
        format!("×{m:.1}")
    }
}
