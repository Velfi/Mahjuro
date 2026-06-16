//! Shared helpers for appending lines to the live score cascade.

use super::{ScoreStep, StepKind, combine};

pub(crate) fn push_fu(
    steps: &mut Vec<ScoreStep>,
    fu: &mut i32,
    han: f64,
    source: impl Into<String>,
    delta: i32,
) {
    *fu += delta;
    steps.push(ScoreStep {
        source: source.into(),
        kind: StepKind::Fu,
        tile_ids: Vec::new(),
        running_fu: *fu,
        running_han: han,
        running_total: combine(*fu, han),
    });
}

pub(crate) fn push_han(
    steps: &mut Vec<ScoreStep>,
    fu: i32,
    han: &mut f64,
    source: impl Into<String>,
    delta: f64,
) {
    *han += delta;
    steps.push(ScoreStep {
        source: source.into(),
        kind: StepKind::Han,
        tile_ids: Vec::new(),
        running_fu: fu,
        running_han: *han,
        running_total: combine(fu, *han),
    });
}

pub(crate) fn push_yen(
    steps: &mut Vec<ScoreStep>,
    flower_yen: &mut i32,
    fu: i32,
    han: f64,
    source: impl Into<String>,
    delta: i32,
) {
    *flower_yen += delta;
    steps.push(ScoreStep {
        source: source.into(),
        kind: StepKind::Yen,
        tile_ids: Vec::new(),
        running_fu: fu,
        running_han: han,
        running_total: combine(fu, han),
    });
}
