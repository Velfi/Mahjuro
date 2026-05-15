//! Presentation ordering for score steps shown in the cascade UI.
//!
//! Scoring applies chip and mult updates in discovery order (meld → relic chips →
//! yaku → relic mults, …). The UI reads clearer when **all chip lines are grouped
//! before mult lines** (then gold), so we reorder without changing the final total:
//! final chips = base + Σ(chip deltas) and final mult = 1 + Σ(mult deltas), which
//! are commutative among steps of the same kind.

use super::{ScoreStep, StepKind, combine};

/// Rebuild `steps` with `Chips` first, then `Mult`, then `Gold`, preserving the
/// delta contributed by each step. Running chips/mult/total are recomputed from
/// `base_chips` and mult base `1.0`.
pub(crate) fn reorder_steps_chips_then_mult_then_gold(steps: &mut Vec<ScoreStep>, base_chips: i32) {
    #[derive(Clone)]
    struct Delta {
        source: String,
        kind: StepKind,
        chip_delta: i32,
        mult_delta: f64,
        tile_ids: Vec<u32>,
    }

    let mut deltas: Vec<Delta> = Vec::with_capacity(steps.len());
    let mut prev_c = base_chips;
    let mut prev_m = 1.0_f64;
    for s in steps.iter() {
        deltas.push(Delta {
            source: s.source.clone(),
            kind: s.kind,
            chip_delta: s.running_chips - prev_c,
            mult_delta: s.running_mult - prev_m,
            tile_ids: s.tile_ids.clone(),
        });
        prev_c = s.running_chips;
        prev_m = s.running_mult;
    }

    deltas.sort_by_key(|d| match d.kind {
        StepKind::Chips => 0,
        StepKind::Mult => 1,
        StepKind::Gold => 2,
        StepKind::Final => 3,
    });

    let mut rc = base_chips;
    let mut rm = 1.0_f64;
    steps.clear();
    for d in deltas {
        rc += d.chip_delta;
        rm += d.mult_delta;
        steps.push(ScoreStep {
            source: d.source,
            kind: d.kind,
            tile_ids: d.tile_ids,
            running_chips: rc,
            running_mult: rm,
            running_total: combine(rc, rm),
        });
    }
}
