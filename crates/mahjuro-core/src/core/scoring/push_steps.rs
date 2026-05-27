//! Shared helpers for appending lines to the live score cascade.

use super::{ScoreStep, StepKind, combine};

pub(crate) fn push_chips(
    steps: &mut Vec<ScoreStep>,
    chips: &mut i32,
    mult: f64,
    source: impl Into<String>,
    delta: i32,
) {
    *chips += delta;
    steps.push(ScoreStep {
        source: source.into(),
        kind: StepKind::Chips,
        tile_ids: Vec::new(),
        running_chips: *chips,
        running_mult: mult,
        running_total: combine(*chips, mult),
    });
}

pub(crate) fn push_mult(
    steps: &mut Vec<ScoreStep>,
    chips: i32,
    mult: &mut f64,
    source: impl Into<String>,
    delta: f64,
) {
    *mult += delta;
    steps.push(ScoreStep {
        source: source.into(),
        kind: StepKind::Mult,
        tile_ids: Vec::new(),
        running_chips: chips,
        running_mult: *mult,
        running_total: combine(chips, *mult),
    });
}

pub(crate) fn push_yen(
    steps: &mut Vec<ScoreStep>,
    flower_yen: &mut i32,
    chips: i32,
    mult: f64,
    source: impl Into<String>,
    delta: i32,
) {
    *flower_yen += delta;
    steps.push(ScoreStep {
        source: source.into(),
        kind: StepKind::Yen,
        tile_ids: Vec::new(),
        running_chips: chips,
        running_mult: mult,
        running_total: combine(chips, mult),
    });
}
