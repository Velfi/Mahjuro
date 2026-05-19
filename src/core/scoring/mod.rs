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
//!
//! ## Layout
//!
//! * [`pipeline`] — thin orchestration: base melds, then each layer in order.
//! * [`pre_yaku_layer`] — meld-linked chips/mults and early relic effects before Dora/yaku.
//! * [`dora_yaku_layer`] — Dora, yaku lines, structure depth mult.
//! * [`relic_mult_layer`] — post-yaku relic mults and late relic chips (one audit surface).
//! * [`effective_relic`] — Mirror Tile / Shadow Hand resolved once per score.
//! * [`presentation`] — optional regrouping of steps for the cascade (chips before mults).

mod dora_yaku_layer;
pub use dora_yaku_layer::DORA_CHIPS_PER_TILE;
mod effective_relic;
pub(crate) use effective_relic::EffectiveRelics;
mod layer_input;
mod pipeline;
mod pre_yaku_layer;
mod presentation;
mod push_steps;
mod relic_mult_layer;
mod tea_bonus;
#[cfg(test)]
mod tests;

use crate::core::hand::{DetectedMeld, MeldKind};
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

    pub scored_meld_kinds: Vec<crate::core::hand::MeldKind>,
}

pub(crate) fn meld_chip_bonus(kind: MeldKind) -> i32 {
    match kind {
        MeldKind::Pair => 18,
        MeldKind::Sequence => 28,
        MeldKind::Triplet => 50,
        MeldKind::Kong => 80,
        MeldKind::Single => 0,
    }
}

/// Grouped meld labels for logs and reports (`Pair  3m 3m · Kong  9p 9p 9p 9p`).
pub fn format_meld_groups(tiles: &[Tile], sets: &[DetectedMeld]) -> Option<String> {
    if tiles.is_empty() || sets.is_empty() {
        return None;
    }
    Some(
        sets.iter()
            .map(|s| describe_set(tiles, s))
            .collect::<Vec<_>>()
            .join(" · "),
    )
}

pub(crate) fn describe_set(tiles: &[Tile], set: &DetectedMeld) -> String {
    let label = match set.kind {
        MeldKind::Pair => "Pair",
        MeldKind::Sequence => "Sequence",
        MeldKind::Triplet => "Triplet",
        MeldKind::Kong => "Kong",
        MeldKind::Single => "Single",
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

#[inline]
pub(crate) fn tile_by_id(tiles: &[Tile], id: u32) -> Option<&Tile> {
    // Inlined linear scan: hand sizes are tiny (≤16 tiles) so a single
    // cache-warm pass through the slice beats a `HashMap` indirection.
    tiles.iter().find(|t| t.id == id)
}

#[inline]
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
