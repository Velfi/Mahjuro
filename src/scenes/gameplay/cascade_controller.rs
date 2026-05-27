//! Cascade controller — owns the active scoring cascade arm of
//! [`super::GameplayScene::update`], including dora chime scheduling,
//! reveal-edge popups, and final-beat effects. Behaviour is identical to
//! the inline code; this is purely organisational.

use std::time::Instant;

use super::GameplayScene;
use super::cascade_hud::{CascadeHandoffStage, CascadeHudState};
use crate::core::scoring::StepKind;
use crate::game::engine::GameEngine;
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::scenes::{SceneTransition, UpdateCtx};
use crate::ui::input::UiAction;

/// Advance the active scoring cascade. Returns `Some(transition)` when
/// the caller should early-return from `update()` (cascade still
/// blocking input or skipped/finished with another queued).
pub(super) fn tick_active_cascade(
    scene: &mut GameplayScene,
    ctx: &mut UpdateCtx<'_>,
    now: Instant,
) -> Option<SceneTransition> {
    // If a cascade is running, advance it and block most input.
    if let Some((cascade, cascade_showcase_opt)) = scene.cascade_queue.front_mut() {
        let cascade_showcase = cascade_showcase_opt.clone();
        // Fire any dora chimes whose scheduled time has arrived. Entries
        // are pushed in ascending time order, so we drain from the front
        // while the head is due.
        while scene.pending_dora_chimes.first().is_some_and(|t| *t <= now) {
            scene.pending_dora_chimes.remove(0);
            ctx.bus.push(crate::game::event_bus::GameEvent::UiSound(
                crate::audio::SfxId::DoraScored,
            ));
        }
        cascade.update(now);
        let frame = cascade.frame(now);
        scene.displayed_score = frame.displayed_score;
        scene.score_reel.set_score(scene.displayed_score, now);

        // Snapshot the cascade's read-out so the draw path can render the
        // chips/×/mult trio under the plaque and (during HandOff) tween
        // the merged total up into the reel.
        scene.cascade_hud = Some(CascadeHudState {
            chips: frame.displayed_chips,
            mult: frame.displayed_mult,
            total: frame.handoff_total,
            merge_t: frame.handoff_merge_t,
            flight_t: frame.handoff_flight_t,
        });

        // Hand-off stage edges → one-shot sounds. Fire on each transition.
        let next_stage = match (frame.handoff_merge_t, frame.handoff_flight_t) {
            (None, None) => CascadeHandoffStage::Pre,
            (Some(_), None) => CascadeHandoffStage::Merging,
            (Some(_), Some(t)) if t < 1.0 => CascadeHandoffStage::Flying,
            (Some(_), Some(_)) => CascadeHandoffStage::Landed,
            (None, Some(_)) => CascadeHandoffStage::Flying,
        };
        if next_stage != scene.cascade_handoff_stage {
            let sfx = match (scene.cascade_handoff_stage, next_stage) {
                (CascadeHandoffStage::Pre, CascadeHandoffStage::Merging) => {
                    Some(crate::audio::SfxId::CascadeMerge)
                }
                (CascadeHandoffStage::Merging, CascadeHandoffStage::Flying) => {
                    Some(crate::audio::SfxId::CascadeLaunch)
                }
                (CascadeHandoffStage::Flying, CascadeHandoffStage::Landed) => {
                    Some(crate::audio::SfxId::CascadeLand)
                }
                _ => None,
            };
            if let Some(sfx) = sfx {
                ctx.bus
                    .push(crate::game::event_bus::GameEvent::UiSound(sfx));
            }
            scene.cascade_handoff_stage = next_stage;
        }

        // Pulse the score panel on each reveal beat.
        if frame.reveal_ordinal.is_some() {
            ctx.anim.pulse(ENTITY_SCORE_PANEL);
        }

        // Reveal-edge effects: fire once per visible beat on the frame the
        // cascade transitions onto a new step. Drives both the relic
        // glow overlay and the per-step audio beat.
        if let Some(ordinal) = frame.reveal_ordinal
            && scene.last_revealed_step != Some(ordinal)
        {
            scene.last_revealed_step = Some(ordinal);
            log::info!(
                "[score] cascade reveal edge: ordinal={} base_steps={} steps={}",
                ordinal,
                cascade.breakdown.base_steps.len(),
                cascade.breakdown.steps.len(),
            );
            ctx.bus
                .push(crate::game::event_bus::GameEvent::ScoreStepRevealed { index: ordinal });
            let step = if ordinal < cascade.breakdown.base_steps.len() {
                cascade.breakdown.base_steps.get(ordinal)
            } else {
                cascade
                    .breakdown
                    .steps
                    .get(ordinal - cascade.breakdown.base_steps.len())
            };
            if let Some(step) = step {
                // Dora step just revealed: schedule one chime per
                // matching tile, spaced so multiple dora play as a
                // rolling ding-ding rather than a stacked hit.
                if step.source.starts_with("Dora") && !step.tile_ids.is_empty() {
                    const DORA_CHIME_SPACING_MS: u64 = 180;
                    for (i, _) in step.tile_ids.iter().enumerate() {
                        let offset =
                            std::time::Duration::from_millis((i as u64) * DORA_CHIME_SPACING_MS);
                        scene.pending_dora_chimes.push(now + offset);
                    }
                }
                if let Some(rid) = crate::core::relic::relic_by_name(&step.source) {
                    scene.relic_glow_starts.insert(rid, now);
                }
                let (chip_delta, mult_delta) = if ordinal < cascade.breakdown.base_steps.len() {
                    if ordinal > 0 {
                        let prev = &cascade.breakdown.base_steps[ordinal - 1];
                        (
                            step.running_chips - prev.running_chips,
                            step.running_mult - prev.running_mult,
                        )
                    } else {
                        (step.running_chips, step.running_mult - 1.0)
                    }
                } else {
                    let idx = ordinal - cascade.breakdown.base_steps.len();
                    if idx > 0 {
                        let prev = &cascade.breakdown.steps[idx - 1];
                        (
                            step.running_chips - prev.running_chips,
                            step.running_mult - prev.running_mult,
                        )
                    } else {
                        (
                            step.running_chips - cascade.breakdown.base_chips,
                            step.running_mult - 1.0,
                        )
                    }
                };
                let popup_label = match step.kind {
                    StepKind::Chips if chip_delta != 0 => Some(format!("{chip_delta:+}")),
                    StepKind::Mult if mult_delta.abs() > 0.001 => {
                        Some(format!("{mult_delta:+.1}x"))
                    }
                    StepKind::Yen => Some(step.source.clone()),
                    StepKind::Final => Some(format!("={}", step.running_total)),
                    _ => None,
                };
                if let Some(label) = popup_label {
                    let source_xy = GameplayScene::popup_source_xy(
                        step,
                        ctx.layout,
                        ctx.run,
                        cascade_showcase.as_ref(),
                        ctx.room_gltf_height_scale,
                    );
                    let counter =
                        super::score_counter::score_counter_layout(ctx.layout, &scene.positions);
                    // Chips popups stream into the left accumulator
                    // token, Mult popups into the right — mirroring
                    // the token geometry set up in the draw path
                    // around the `CascadeToken` placements. Gold and
                    // Final still land on the reel.
                    let reel_xy = (counter.reel.px, counter.reel.py);
                    let reel_lift = counter.reel.lift_z;
                    let (dest_xy, dest_lift) = match step.kind {
                        StepKind::Chips | StepKind::Mult => {
                            let tokens = GameplayScene::cascade_token_layout(ctx.layout);
                            let xy = if step.kind == StepKind::Chips {
                                tokens.chips_center
                            } else {
                                tokens.mult_center
                            };
                            (xy, Some(24.0))
                        }
                        _ => (reel_xy, Some(reel_lift)),
                    };
                    let magnitude = chip_delta.abs().max(1) as f32 + mult_delta.abs() as f32;
                    log::info!(
                        "[popup spawn] kind={:?} label={:?} src=({:.0},{:.0}) dest=({:.0},{:.0}) source_name={:?}",
                        step.kind,
                        label,
                        source_xy.0,
                        source_xy.1,
                        dest_xy.0,
                        dest_xy.1,
                        step.source,
                    );
                    scene
                        .score_popups
                        .spawn(label, source_xy, dest_xy, dest_lift, step.kind, magnitude);
                    let gameplay = GameEngine::read(ctx.run);
                    if step.tile_ids.iter().any(|&tid| {
                        ctx.run
                            .hand()
                            .iter()
                            .chain(gameplay.structure_tiles.iter())
                            .find(|tile| tile.id == tid)
                            .is_some_and(|tile| {
                                ctx.run
                                    .tile_debuffs
                                    .iter()
                                    .any(|debuff| debuff.matches(tile))
                            })
                    }) {
                        scene.score_popups.spawn_debuff_x(source_xy, magnitude);
                    }
                }
                // Avoid piling exact numeric deltas into the
                // play space while the cascade is in motion.
            }
        }

        // Final-beat edge: fire once when the cascade transitions into
        // its ShowTotal phase. Used by the audio dispatcher to play the
        // closing sting on the final number landing, and now also drives
        // the screen shake + gold flash crescendo so big hands feel
        // *bigger* than small ones.
        if cascade.is_in_total() && !scene.cascade_final_emitted {
            scene.cascade_final_emitted = true;
            ctx.bus
                .push(crate::game::event_bus::GameEvent::ScoreCascadeFinal {
                    earned: cascade.earned,
                });
            // Screen shake amplitude scales with the magnitude of this
            // hand — log curve so a 200-point hand still gets a tiny
            // kick and a 50,000-point hand really thumps.
            let earned = cascade.earned.max(1) as f32;
            let amp = (earned.log2() * 1.6).clamp(3.0, 18.0);
            ctx.anim
                .shake(crate::render::animation::ENTITY_HAND_STRIP, amp, 350);
            ctx.anim.shake(ENTITY_SCORE_PANEL, amp * 0.7, 350);
            // Trigger the gold flash overlay; the draw() loop reads this
            // timestamp and renders a fading gold quad over the scene.
            scene.gold_flash_at = Some(now);
        }
        // Stronger score-pop whenever the displayed value actually
        // advances — this is what makes scoring *feel* like the number
        // is climbing on the cartouche.
        if scene.displayed_score != scene.prev_displayed_score {
            ctx.anim.score_pop(ENTITY_SCORE_PANEL);
            scene.prev_displayed_score = scene.displayed_score;
        }

        let frame_active = frame.active;
        if !frame_active {
            // Cascade finished — pop it and advance to the next if queued.
            scene.displayed_score = GameEngine::read(ctx.run).round_score;
            scene.score_reel.set_score(scene.displayed_score, now);
            scene.cascade_queue.pop_front();
            scene.last_revealed_step = None;
            scene.cascade_final_emitted = false;
            scene.cascade_hud = None;
            scene.cascade_handoff_stage = CascadeHandoffStage::Pre;
            scene.pending_dora_chimes.clear();
            scene.score_popups.clear();
            // If more cascades are queued, stay in cascade mode.
            if !scene.cascade_queue.is_empty() {
                return Some(None);
            }
        } else {
            // Allow skip on a fresh user press during cascade. Filter out
            // button-release events (ConfirmRelease) — those are tail
            // events from the gesture that *started* the cascade, not
            // a new skip request.
            let skip_request = ctx
                .actions
                .iter()
                .any(|a| !matches!(a, UiAction::ConfirmRelease));
            if skip_request {
                // Skip the current cascade and advance to next.
                scene.displayed_score = GameEngine::read(ctx.run).round_score;
                scene.score_reel.set_score(scene.displayed_score, now);
                scene.cascade_queue.pop_front();
                scene.last_revealed_step = None;
                scene.cascade_final_emitted = false;
                scene.cascade_hud = None;
                scene.cascade_handoff_stage = CascadeHandoffStage::Pre;
                scene.pending_dora_chimes.clear();
                scene.score_popups.clear();
                if !scene.cascade_queue.is_empty() {
                    return Some(None);
                }
            }
            return Some(None);
        }
    }
    None
}
