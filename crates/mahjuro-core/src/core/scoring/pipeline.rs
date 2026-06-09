use crate::core::hand::{DetectedMeld, MeldKind};
use crate::core::relic::{RelicId, ScoreContext};
use crate::core::rules::RuleModifier;
use crate::core::tile::Tile;

use super::effective_relic::EffectiveRelics;
use super::presentation::reorder_steps_chips_then_mult_then_yen;
use super::{
    ScoreBreakdown, ScoreStep, StepKind, combine, describe_set, fmt_mult, tile_by_id,
    tile_is_debuffed,
};

pub fn score_sets_with_original(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
    original_tiles: &[Tile],
) -> ScoreBreakdown {
    score_sets_inner(tiles, sets, ctx, rules, Some(original_tiles))
}

#[cfg(test)]
pub fn score_sets(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
) -> ScoreBreakdown {
    score_sets_inner(tiles, sets, ctx, rules, None)
}

fn score_sets_inner(
    tiles: &[Tile],
    sets: &[DetectedMeld],
    ctx: &ScoreContext<'_>,
    rules: &[RuleModifier],
    original_tiles: Option<&[Tile]>,
) -> ScoreBreakdown {
    let mut steps: Vec<ScoreStep> = Vec::new();
    let mut base_steps: Vec<ScoreStep> = Vec::new();
    let mut chips: i32;
    let mut mult: f64 = 1.0;
    let mut flower_yen: i32 = 0;

    let honor_triple = rules.contains(&RuleModifier::HonorTripleScore);
    let no_seq_bonus = rules.contains(&RuleModifier::NoSequenceBonus);
    let pairs_zero = rules.contains(&RuleModifier::PairsScoreZero);
    let sequences_halved = rules.contains(&RuleModifier::SequencesHalved);
    let censor_repeats = rules.contains(&RuleModifier::CensorRepeats);

    let eff = EffectiveRelics::from_context(ctx);

    let mut base_chips: i32 = 0;
    for s in sets {
        let mut meld_contrib = 0;
        for &tid in &s.tile_ids {
            if let Some(t) = tile_by_id(tiles, tid) {
                meld_contrib += if tile_is_debuffed(t, ctx.tiles.debuffs) {
                    0
                } else {
                    t.point_value() as i32
                };
            }
        }
        if pairs_zero && s.kind == MeldKind::Pair {
            meld_contrib = 0;
        }
        if sequences_halved && s.kind == MeldKind::Sequence {
            meld_contrib /= 2;
        }
        base_chips += meld_contrib;
        base_steps.push(ScoreStep {
            source: describe_set(tiles, s),
            kind: StepKind::Chips,
            tile_ids: s.tile_ids.clone(),
            running_chips: base_chips,
            running_mult: 1.0,
            running_total: combine(base_chips, 1.0),
        });
    }
    chips = base_chips;

    let has_triplet_boost = eff.has(ctx.relic.roster, RelicId::TripletBoost);
    let layer_input = super::layer_input::ScoringLayerInput {
        ctx,
        tiles,
        sets,
        eff,
    };
    super::pre_yaku_layer::apply_pre_yaku_scoring(
        &layer_input,
        super::layer_input::ScoringLayerOut {
            chips: &mut chips,
            mult: &mut mult,
            steps: &mut steps,
        },
        super::layer_input::PreYakuLayerOpts {
            has_triplet_boost,
            flower_yen: &mut flower_yen,
        },
    );

    let detected_yaku = super::dora_yaku_layer::apply_dora_yaku_and_structure(
        &layer_input,
        super::layer_input::ScoringLayerOut {
            chips: &mut chips,
            mult: &mut mult,
            steps: &mut steps,
        },
        super::layer_input::DoraYakuLayerOpts {
            censor_repeats,
            original_tiles,
        },
    );

    super::relic_mult_layer::apply_post_yaku_relic_modifiers(
        &layer_input,
        super::layer_input::ScoringLayerOut {
            chips: &mut chips,
            mult: &mut mult,
            steps: &mut steps,
        },
        super::layer_input::PostYakuRelicLayerOpts {
            honor_triple,
            no_seq_bonus,
            has_triplet_boost,
            detected_yaku: &detected_yaku,
        },
    );

    reorder_steps_chips_then_mult_then_yen(&mut steps, base_chips);

    let final_chips = chips;
    let final_mult = mult;
    let total = combine(final_chips, final_mult);
    steps.push(ScoreStep {
        source: format!("{} × {}", final_chips, fmt_mult(final_mult)),
        kind: StepKind::Final,
        tile_ids: Vec::new(),
        running_chips: final_chips,
        running_mult: final_mult,
        running_total: total,
    });

    ScoreBreakdown {
        base_chips,
        base_points: base_chips,
        base_steps,
        steps,
        detected_yaku,
        final_chips,
        final_mult,
        total,
        flower_yen,
        scored_meld_kinds: sets.iter().map(|s| s.kind).collect(),
    }
}
