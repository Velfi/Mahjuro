//! Presentation ordering for score steps shown in the cascade UI.
//!
//! Scoring applies chip and Han updates in discovery order (meld → relic Fu →
//! yaku → relic mults, …). The UI reads clearer when **all chip lines are grouped
//! before Han lines** (then yen), so we reorder without changing the final total:
//! final Fu = base + Σ(chip deltas) and final Han = 1 + Σ(mult deltas), which
//! are commutative among steps of the same kind.

use super::{ScoreStep, StepKind, combine};

/// Rebuild `steps` with `Chips` first, then `Mult`, then `Yen`, preserving the
/// delta contributed by each step. Running chips/mult/total are recomputed from
/// `base_fu` and Han base `1.0`.
pub(crate) fn reorder_steps_fu_then_han_then_yen(steps: &mut Vec<ScoreStep>, base_fu: i32) {
    #[derive(Clone)]
    struct Delta {
        source: String,
        kind: StepKind,
        fu_delta: i32,
        han_delta: f64,
        tile_ids: Vec<u32>,
    }

    let mut deltas: Vec<Delta> = Vec::with_capacity(steps.len());
    let mut prev_c = base_fu;
    let mut prev_m = 1.0_f64;
    for s in steps.iter() {
        deltas.push(Delta {
            source: s.source.clone(),
            kind: s.kind,
            fu_delta: s.running_fu - prev_c,
            han_delta: s.running_han - prev_m,
            tile_ids: s.tile_ids.clone(),
        });
        prev_c = s.running_fu;
        prev_m = s.running_han;
    }

    deltas.sort_by_key(|d| match d.kind {
        StepKind::Fu => 0,
        StepKind::Han => 1,
        StepKind::Yen => 2,
        StepKind::Final => 3,
    });

    let mut rc = base_fu;
    let mut rm = 1.0_f64;
    steps.clear();
    for d in deltas {
        rc += d.fu_delta;
        rm += d.han_delta;
        steps.push(ScoreStep {
            source: d.source,
            kind: d.kind,
            tile_ids: d.tile_ids,
            running_fu: rc,
            running_han: rm,
            running_total: combine(rc, rm),
        });
    }
}
