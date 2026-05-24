//! Dora chips and yaku chip/mult lines — the “standard” scoring
//! block between early relics ([`super::pre_yaku_layer`]) and post-yaku relics
//! ([`super::relic_mult_layer`]).

use crate::core::relic::RelicId;
use crate::core::yaku::{YakuKind, detect_yaku_with_wind};

use super::layer_input::{DoraYakuLayerOpts, ScoringLayerInput, ScoringLayerOut};
use super::push_steps::{push_chips, push_mult};
use super::{ScoreStep, StepKind, combine, tile_is_debuffed};

/// Chips added per matching tile when a face is on the dora plinth.
pub const DORA_CHIPS_PER_TILE: i32 = 100;

/// Extra chips per dora tile from each Dora Crown copy.
pub const DORA_CROWN_CHIPS_PER_TILE: i32 = 50;

/// Apply Dora and scored yaku. Returns the yaku list used for the breakdown.
pub(crate) fn apply_dora_yaku_and_structure(
    input: &ScoringLayerInput<'_>,
    out: ScoringLayerOut<'_>,
    opts: DoraYakuLayerOpts<'_>,
) -> Vec<YakuKind> {
    let ScoringLayerInput {
        ctx,
        tiles,
        sets,
        eff,
    } = *input;
    let ScoringLayerOut { chips, mult, steps } = out;
    let DoraYakuLayerOpts {
        censor_repeats,
        original_tiles,
    } = opts;
    if !ctx.pattern.dora_faces.is_empty() {
        let dora_count = tiles
            .iter()
            .filter(|t| !tile_is_debuffed(t, ctx.tiles.debuffs))
            .filter(|t| ctx.pattern.dora_faces.contains(&(t.suit, t.rank)))
            .count() as i32;
        if dora_count > 0 {
            let delta = DORA_CHIPS_PER_TILE * dora_count;
            *chips += delta;
            steps.push(ScoreStep {
                source: format!("Dora ×{dora_count}"),
                kind: StepKind::Chips,
                tile_ids: tiles
                    .iter()
                    .filter(|t| !tile_is_debuffed(t, ctx.tiles.debuffs))
                    .filter(|t| ctx.pattern.dora_faces.contains(&(t.suit, t.rank)))
                    .map(|t| t.id)
                    .collect(),
                running_chips: *chips,
                running_mult: *mult,
                running_total: combine(*chips, *mult),
            });
            for _ in 0..eff.count(ctx.relic.roster, RelicId::DoraCrown) {
                push_chips(
                    steps,
                    chips,
                    *mult,
                    format!("Dora Crown ×{dora_count}"),
                    DORA_CROWN_CHIPS_PER_TILE * dora_count,
                );
            }
        }
    }

    let all_yaku = detect_yaku_with_wind(
        tiles,
        sets,
        ctx.round.round_wind,
        ctx.round.bonus_round_wind,
        original_tiles,
    );
    let mut detected_yaku: Vec<YakuKind> = if ctx.pattern.available_yaku.is_empty() {
        all_yaku
    } else {
        all_yaku
            .into_iter()
            .filter(|y| {
                // Secret pattern: Kokushi Musō is omitted from `available_yaku`
                // until the first cash-in, but the hand must still score when valid.
                if *y == YakuKind::KokushiMusou {
                    return true;
                }
                ctx.pattern.available_yaku.contains(y)
            })
            .collect()
    };
    if let Some(st) = &ctx.structure
        && st.inject_chicken_if_no_yaku
        && detected_yaku.is_empty()
    {
        detected_yaku.push(YakuKind::ChickenHand);
    }
    let level_of = |y: YakuKind| -> u32 {
        ctx.pattern
            .yaku_levels
            .as_ref()
            .map(|m| m.level_of(y))
            .unwrap_or(1)
    };
    for yaku in &detected_yaku {
        let level = level_of(*yaku);
        let mut mult_bonus = yaku.mult_bonus_at(level);
        let mut chip_bonus = yaku.chip_bonus_at(level);
        if censor_repeats && ctx.round.played_yaku_this_round.contains(yaku) {
            chip_bonus = (chip_bonus as f64 * 0.5).floor() as i32;
            mult_bonus *= 0.5;
        }
        if chip_bonus != 0 {
            push_chips(steps, chips, *mult, yaku.name(), chip_bonus);
        }
        push_mult(steps, *chips, mult, yaku.name(), mult_bonus);
    }

    detected_yaku
}
