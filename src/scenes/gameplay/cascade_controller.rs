//! Cascade controller — owns the active scoring cascade arm of
//! [`super::GameplayScene::update`], including dora chime scheduling,
//! reveal-edge popups, and final-beat effects.

use std::time::Instant;

use super::GameplayScene;
use crate::core::scoring::StepKind;
use crate::game::cascade::{YAKU_NAME_POST_PAUSE_MS, first_yaku_step};
use crate::game::engine::GameEngine;
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::score_popups::PopupMotionTiming;
use crate::scenes::{SceneTransition, UpdateCtx};
#[cfg(any(feature = "game", feature = "headless-screenshot"))]
use crate::sfx_id::SfxId;
use crate::ui::input::UiAction;

/// Advance the active scoring cascade. Returns `Some(transition)` when
/// the caller should early-return from `update()` (cascade still
/// blocking input or skipped/finished with another queued).
pub(super) fn tick_active_cascade(
    scene: &mut GameplayScene,
    ctx: &mut UpdateCtx<'_>,
    now: Instant,
) -> Option<SceneTransition> {
    if let Some((cascade, cascade_showcase_opt)) = scene.cascade_queue.front_mut() {
        let cascade_showcase = cascade_showcase_opt.clone();
        while scene.pending_dora_chimes.first().is_some_and(|t| *t <= now) {
            scene.pending_dora_chimes.remove(0);
            ctx.bus.push(crate::game::event_bus::GameEvent::UiSound(
                crate::sfx_id::SfxId::DoraScored,
            ));
        }
        cascade.update(now);
        let frame = cascade.frame(now);
        let popup_timing = popup_motion_timing(cascade.popup_timing());
        scene.displayed_score = frame.displayed_score;
        scene.score_reel.set_score(scene.displayed_score, now);

        if frame.reveal_ordinal.is_some() {
            ctx.anim.pulse(ENTITY_SCORE_PANEL);
        }

        if let Some(ordinal) = frame.reveal_ordinal
            && scene.last_revealed_step != Some(ordinal)
        {
            scene.last_revealed_step = Some(ordinal);
            ctx.bus
                .push(crate::game::event_bus::GameEvent::ScoreStepRevealed { index: ordinal });
            if ordinal >= cascade.breakdown.base_steps.len() {
                let step_index = ordinal - cascade.breakdown.base_steps.len();
                if let Some(yk) = first_yaku_step(&cascade.breakdown, step_index)
                    && cascade.mark_yaku_voiced(yk)
                {
                    #[cfg(any(feature = "game", feature = "headless-screenshot"))]
                    let voice_dur = {
                        let sfx = SfxId::for_yaku(yk);
                        ctx.audio
                            .as_mut()
                            .and_then(|audio| {
                                audio.play_sfx(sfx).or_else(|| audio.sfx_duration(sfx))
                            })
                            .unwrap_or_else(|| std::time::Duration::from_millis(700))
                    };
                    #[cfg(not(any(feature = "game", feature = "headless-screenshot")))]
                    let voice_dur = std::time::Duration::from_millis(700);
                    cascade.extend_yaku_hold(
                        now + voice_dur + std::time::Duration::from_millis(YAKU_NAME_POST_PAUSE_MS),
                    );
                }
            }
            let step = if ordinal < cascade.breakdown.base_steps.len() {
                cascade.breakdown.base_steps.get(ordinal)
            } else {
                cascade
                    .breakdown
                    .steps
                    .get(ordinal - cascade.breakdown.base_steps.len())
            };
            if let Some(step) = step {
                let step_count = cascade.breakdown.base_steps.len() + cascade.breakdown.steps.len();
                log::debug!(
                    "[score] cascade reveal {}/{}: kind={:?} source={} running_total={} chips={} mult={:.2}",
                    ordinal + 1,
                    step_count,
                    step.kind,
                    step.source,
                    step.running_total,
                    step.running_chips,
                    step.running_mult,
                );
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
                    StepKind::Yen => Some(format!("+{}", step.source.clone())),
                    StepKind::Final => Some(format!("={}", step.running_total)),
                    _ => None,
                };
                if let Some(label) = popup_label {
                    let source = GameplayScene::popup_source(
                        step,
                        ctx.layout,
                        &scene.positions,
                        ctx.run,
                        cascade_showcase.as_ref(),
                        ctx.room_gltf_height_scale,
                    );
                    let fly_dest = super::score_counter::resolve_score_popup_fly_dest(
                        ctx.layout,
                        &scene.positions,
                        ctx.room_gltf_height_scale,
                    );
                    let reel_xy = (fly_dest.px, fly_dest.py);
                    let reel_lift = Some(fly_dest.lift_z);
                    let magnitude = chip_delta.abs().max(1) as f32 + mult_delta.abs() as f32;
                    scene.score_popups.spawn(
                        label,
                        source,
                        reel_xy,
                        reel_lift,
                        step.kind,
                        magnitude,
                        popup_timing,
                    );
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
                        scene.score_popups.spawn_debuff_x(source, magnitude);
                    }
                }
            }
        }

        if cascade.is_in_total() && !scene.cascade_final_emitted {
            scene.cascade_final_emitted = true;
            ctx.bus
                .push(crate::game::event_bus::GameEvent::ScoreCascadeFinal {
                    earned: cascade.earned,
                });
            let earned = cascade.earned.max(1) as f32;
            let amp = (earned.log2() * 1.6).clamp(3.0, 18.0);
            ctx.anim
                .shake(crate::render::animation::ENTITY_HAND_STRIP, amp, 350);
            ctx.anim.shake(ENTITY_SCORE_PANEL, amp * 0.7, 350);
            scene.gold_flash_at = Some(now);
        }

        if scene.displayed_score != scene.prev_displayed_score {
            ctx.anim.score_pop(ENTITY_SCORE_PANEL);
            scene.prev_displayed_score = scene.displayed_score;
        }

        let frame_active = frame.active;
        if !frame_active {
            scene.displayed_score = GameEngine::read(ctx.run).round_score;
            scene.score_reel.set_score(scene.displayed_score, now);
            scene.cascade_queue.pop_front();
            scene.last_revealed_step = None;
            scene.cascade_final_emitted = false;
            scene.cascade_hud = None;
            scene.pending_dora_chimes.clear();
            if !scene.cascade_queue.is_empty() {
                return Some(None);
            }
        } else {
            let skip_request = ctx
                .actions
                .iter()
                .any(|a| action_skips_cascade(*a));
            if skip_request {
                scene.displayed_score = GameEngine::read(ctx.run).round_score;
                scene.score_reel.set_score(scene.displayed_score, now);
                scene.cascade_queue.pop_front();
                scene.last_revealed_step = None;
                scene.cascade_final_emitted = false;
                scene.cascade_hud = None;
                scene.pending_dora_chimes.clear();
                if !scene.cascade_queue.is_empty() {
                    return Some(None);
                }
            }
            return Some(None);
        }
    }
    None
}

/// Focus moves and cash-in holds (LT/RT, keyboard T) must not fast-forward the reveal.
fn action_skips_cascade(a: UiAction) -> bool {
    !matches!(
        a,
        UiAction::ConfirmRelease
            | UiAction::FocusNext
            | UiAction::FocusPrev
            | UiAction::FocusUp
            | UiAction::FocusDown
            | UiAction::TriggerStructure
            | UiAction::TriggerStructureRelease
    )
}

fn popup_motion_timing(t: crate::game::cascade::PopupTiming) -> PopupMotionTiming {
    PopupMotionTiming {
        pop_secs: t.pop_ms / 1000.0,
        loiter_secs: t.loiter_ms / 1000.0,
        fly_secs: t.fly_ms / 1000.0,
        overshoot: t.overshoot,
    }
}
