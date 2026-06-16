//! Input handler — owns the unified focus model + 3D hit dispatcher +
//! gameplay action handlers (ScoreHand, TriggerStructure, discard) of
//! [`super::GameplayScene::update`]. Behaviour is identical to the inline
//! code; this is purely organisational.

use std::time::Instant;

use super::GameplayScene;
use super::cascade_hud::CascadeShowcase;
use super::focus::{
    FocusTarget, GameplayButton, default_hand_tile_focus, focus_after_consumable_use, focus_kind,
    focus_kind_sfx, play_select_sfx, rebuild_focus_nav,
};
use crate::core::relic::relic_visual;
use crate::game::engine::{CommandData, GameCommand, GameEngine};
use crate::game::run::DiscardUndoSnapshot;
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::draw_cmd::{CameraParams, Object3d, Object3dKind};
use crate::scenes::gameplay::RELIC_GLOW_LIFETIME;
use crate::scenes::journal_transition::{JournalDirection, JournalTransition};
use crate::scenes::{
    GuideScene, OverlayRequest, Scene, SceneTransition, UpdateCtx, WallLedgerScene,
    YakuJournalScene,
};
use crate::ui::focus_nav::{FocusDir, rect_contains};
use crate::ui::input::{UiAction, apply_ui_actions};

/// Vertical bounce (mm, before `layout.mm`) when a scoring step highlights this source.
const SCORE_WAVE_YAKU_MM: f32 = 4.0;
const SCORE_WAVE_STRUCTURE_TILE_MM: f32 = 5.0;
const RELIC_SCORE_VERTICAL_MM: f32 = 3.5;
const STRUCTURE_CALLOUT_DOWN_MM: f32 = 28.0;

#[inline]
fn released_before_hold_complete(start: Instant, now: Instant) -> bool {
    now.saturating_duration_since(start).as_secs_f32() < super::cash_in_hold_seconds()
}

fn spawn_structure_status_callout(
    scene: &mut GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    label: &str,
    fly_to_score: bool,
) {
    let (score_px, score_py) = layout.fallback_score_center();
    let source = structure_strip_callout_anchor(layout, run).unwrap_or(
        crate::render::world_space::LayoutAnchorPx {
            px: score_px,
            py: score_py + 40.0 + layout.mm(STRUCTURE_CALLOUT_DOWN_MM),
            lift_z: crate::render::score_popups::TABLE_POPUP_LIFT_Z,
        },
    );
    scene
        .score_popups
        .spawn_structure_callout(label, source, (score_px, score_py), fly_to_score);
}

/// Advances journal cover-open + zoom; pushes [`YakuJournalScene`] when the
/// forward animation completes. Returns `true` when the overlay was pushed
/// and `update()` should return immediately.
pub(super) fn tick_gameplay_journal_transition(
    scene: &mut GameplayScene,
    ctx: &mut UpdateCtx<'_>,
    now: Instant,
    dt: f32,
) -> bool {
    if scene.journal_was_open && scene.journal_transition.is_none() {
        scene.journal_was_open = false;
        scene.journal_transition = Some(JournalTransition {
            start: now,
            dir: JournalDirection::Closing,
        });
    }

    if let Some(t) = scene.journal_transition {
        scene.journal_open_amount = t.open_progress();
        scene.journal_open_target = scene.journal_open_amount;
        if t.done() {
            match t.dir {
                JournalDirection::Opening => {
                    scene.journal_transition = None;
                    scene.journal_was_open = true;
                    *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                        Scene::YakuJournal(YakuJournalScene::new()),
                    )));
                    return true;
                }
                JournalDirection::Closing => {
                    scene.journal_transition = None;
                    scene.journal_open_amount = 0.0;
                    scene.journal_open_target = 0.0;
                }
            }
        }
    } else {
        // Closed until click — no hover/focus “peek”; only `JournalTransition` opens the cover.
        scene.journal_open_target = 0.0;
        let rate = 6.0;
        let alpha = 1.0 - (-rate * dt).exp();
        scene.journal_open_amount +=
            (scene.journal_open_target - scene.journal_open_amount) * alpha;
    }
    false
}

/// Run the unified focus model, 3D-hit dispatcher, action handlers, and
/// `apply_ui_actions` cleanup from `update()`. Returns `Some(transition)`
/// when the caller should early-return from `update()`.
pub(super) fn process_focus_and_actions(
    scene: &mut GameplayScene,
    ctx: &mut UpdateCtx<'_>,
    now: Instant,
    focus_kind_before: Option<super::focus::FocusKind>,
) -> Option<SceneTransition> {
    if scene.lab_mode() {
        return process_lab_cash_in(scene, ctx, now);
    }
    let interaction = GameEngine::read_interaction(ctx.run);
    // ── Unified focus model ──────────────────────────────────────────
    //
    // `self.focus: Option<FocusTarget>` is the single source of truth
    // for "what is currently selected" across cursor, keyboard, and
    // controller. The focus rect graph it walks is built at the end
    // of the previous `draw_frame` and stored in `self.last_focus_rects`,
    // so directional navigation always uses the most recently rendered
    // on-screen positions (one frame stale, like every other projected
    // rect path in this codebase).
    //
    // Per-frame focus invariants — clamped at the top so a consumable
    // / relic / hand tile that vanished mid-frame doesn't leave a
    // dangling index pointing past the end of its collection.
    if let Some(t) = scene.focus {
        let still_valid = match t {
            FocusTarget::HandTile(i) => i < interaction.hand_len,
            FocusTarget::Consumable(i) => {
                i < interaction.consumable_capacity && i < interaction.consumable_count
            }
            FocusTarget::Relic(i) => i < interaction.relic_count,
            // YakuTablet validity is checked against the projected
            // rect graph downstream — we leave it through here so a
            // newly-disabled yaku doesn't blank the focus mid-frame
            // before the next draw rebuilds the rect list.
            FocusTarget::Button(_)
            | FocusTarget::Peg(_)
            | FocusTarget::Gold
            | FocusTarget::YakuTablet(_)
            | FocusTarget::Dora
            | FocusTarget::Ordeal
            | FocusTarget::RoundWind
            | FocusTarget::ScoreRoller(_) => true,
            FocusTarget::Journal => scene.journal_transition.is_none(),
            FocusTarget::Guidebook => scene.journal_transition.is_none(),
            FocusTarget::WallHud => true,
            FocusTarget::DiscardUndo => {
                crate::persistence::load_settings().discard_undo_enabled
                    && scene.discard_undo.is_some()
                    && scene.pending_discard_refill.is_none()
            }
        };
        if !still_valid {
            scene.focus = None;
        }
    }
    if matches!(
        scene.focus,
        Some(FocusTarget::Button(GameplayButton::Trigger))
    ) {
        let gameplay = GameEngine::read(ctx.run);
        if !gameplay.trigger_enabled && !gameplay.cash_in_blocked_until_discards_spent {
            scene.focus = None;
        }
    }
    if matches!(scene.focus, Some(FocusTarget::DiscardUndo)) {
        let ok = crate::persistence::load_settings().discard_undo_enabled
            && scene.discard_undo.is_some()
            && scene.pending_discard_refill.is_none();
        if !ok {
            scene.focus = None;
        }
    }

    // Phase A: cursor-mode sync. When the player is using the mouse,
    // hover IS focus — overwrite `self.focus` each frame from the
    // cursor's hit-test against the focus rect graph. Hand tiles get
    // the precise raycast path (`picked_hand_tile`); everything else
    // falls back to the projected 2D rect graph stored last frame.
    // This is what guarantees a mouse click on a hand tile while a
    // controller had focus on a consumable still selects the tile —
    // the cursor sync overwrites the stale focus before Confirm runs.
    let focus_rects = scene.last_focus_rects.borrow().clone();
    // Keyboard / controller: default to the first hand tile whenever nothing
    // is focused and the rack is non-empty. Cursor mode keeps hover as focus
    // (None over empty space is intentional there).
    if scene.focus.is_none()
        && ctx.input_mode != crate::ui::input::InputMode::Cursor
        && let Some(target) = default_hand_tile_focus(interaction.hand_len, &focus_rects)
    {
        scene.focus = Some(target);
    }
    if ctx.input_mode == crate::ui::input::InputMode::Cursor {
        let (cx, cy) = ctx.cursor_pos;
        let hand_rects = scene.last_hand_tile_pick_rects.borrow();
        let new_focus = if let Some(idx) =
            super::hand_layout::hand_tile_pick_at_cursor(ctx.picked_hand_tile, &hand_rects, cx, cy)
        {
            Some(FocusTarget::HandTile(idx))
        } else {
            focus_non_hand_target_at_cursor(&focus_rects, cx, cy)
        };
        scene.focus = new_focus;
    }

    // Marquee multi-select drag (cursor only): while LMB is held, the cursor's
    // hovered hand tile drives `current_slot` every frame. The pointer variant
    // always selects the contiguous linear span between the press tile and the
    // hovered tile. Keyboard/gamepad marquee uses `advance_to` via the focus
    // path below so wrap-around arcs stay available.
    if ctx.input_mode == crate::ui::input::InputMode::Cursor
        && let (Some(m), Some(FocusTarget::HandTile(idx))) = (scene.marquee.as_mut(), scene.focus)
        && let Some((added, removed)) = GameEngine::apply_marquee_selection_pointer(ctx.run, m, idx)
        && (added > 0 || removed > 0)
    {
        play_select_sfx(ctx.bus, added, removed);
    }

    // Resolve the screen-space rect for the currently focused target
    // (if any) so the spatial picker has a starting position. The
    // target may have been added or removed since last frame, in
    // which case we'll fall back to the first hand tile in the graph
    // when seeding directional input.
    let current_focus_rect = scene.focus.and_then(|t| {
        focus_rects
            .iter()
            .find_map(|(t2, r)| (*t2 == t).then_some(*r))
    });
    // Pre-collected list of Consumable targets in focus_rects, in
    // slot order. Used by the legacy `[` / `]` / LB / RB keymap below
    // to keep "shoulder buttons cycle through consumables" working as
    // a muscle-memory affordance even though spatial nav can also
    // reach them.
    let consumable_targets: Vec<FocusTarget> = focus_rects
        .iter()
        .filter_map(|(t, _)| match t {
            FocusTarget::Consumable(_) => Some(*t),
            _ => None,
        })
        .collect();
    // `[` / `]` keyboard and LB / RB shoulder: cycle inventory + optional
    // discard-undo anchor (Accessibility) without relying on spatial nav.
    let undo_hud_eligible = crate::persistence::load_settings().discard_undo_enabled
        && scene.discard_undo.is_some()
        && scene.pending_discard_refill.is_none();
    let mut hud_cycle: Vec<FocusTarget> = consumable_targets.clone();
    if undo_hud_eligible {
        hud_cycle.push(FocusTarget::DiscardUndo);
    }

    // Process actions. Directional input → spatial picker. Confirm →
    // route by self.focus variant. Cancel → clear focus AND fall
    // through so existing `clear_selection` semantics still apply.
    // Everything else flows into `actions_for_scene` for the existing
    // gameplay action handlers below (ScoreHand, etc.).
    let mut actions_for_scene: Vec<UiAction> = Vec::new();
    for &a in ctx.actions.iter() {
        if !matches!(a, UiAction::Help | UiAction::Pause)
            && ctx.run.onboarding.as_ref().is_some_and(|o| {
                o.phase == crate::game::onboarding::OnboardingPhase::Finale && !o.finale_intro_shown
            })
        {
            super::onboarding_hints::mark_finale_intro_seen(ctx.run);
        }
        // Map FocusNext/Prev → Right/Left for back-compat with the
        // existing keymap (Tab / arrows still cycle).
        let dir: Option<FocusDir> = match a {
            UiAction::FocusUp => Some(FocusDir::Up),
            UiAction::FocusDown => Some(FocusDir::Down),
            UiAction::FocusPrev => Some(FocusDir::Left),
            UiAction::FocusNext => Some(FocusDir::Right),
            _ => None,
        };
        if let Some(dir) = dir {
            // Moving focus abandons any in-progress hold-to-cash-in.
            scene.clear_cash_in_hold(ctx.bus);
            // Seed focus on first directional press from None: prefer
            // the cursor's last hit, else the first hand tile in the
            // graph, else any first entry.
            let start_rect = current_focus_rect.or_else(|| {
                focus_rects
                    .iter()
                    .find_map(|(t, r)| matches!(t, FocusTarget::HandTile(_)).then_some(*r))
            });
            if let Some(rect) = start_rect {
                rebuild_focus_nav(&mut scene.focus_nav, &focus_rects, scene.pause_menu.paused);
                let current_target = scene.focus.or_else(|| {
                    focus_rects
                        .iter()
                        .find(|(_, r)| *r == rect)
                        .map(|(t, _)| *t)
                });
                if let Some(current) = current_target
                    && let Some(next) = scene.focus_nav.pick(current, dir)
                {
                    scene.focus = Some(next);
                }
            } else if let Some((first, _)) = focus_rects.first() {
                scene.focus = Some(*first);
            }
            // Marquee: if Confirm is held and focus moved to a hand
            // tile, extend the swept range to that tile and re-apply
            // against the press-time snapshot.
            if let (Some(m), Some(FocusTarget::HandTile(idx))) =
                (scene.marquee.as_mut(), scene.focus)
                && let Some((added, removed)) = GameEngine::apply_marquee_selection(ctx.run, m, idx)
                && (added > 0 || removed > 0)
            {
                play_select_sfx(ctx.bus, added, removed);
            }
            continue;
        }

        match a {
            // `[` / `]` (keyboard) and LB / RB (gamepad): cycle consumable
            // slots, then the optional discard-undo control (Accessibility).
            UiAction::NavigateHudNext => {
                if !hud_cycle.is_empty() {
                    let cur_pos = hud_cycle.iter().position(|t| Some(*t) == scene.focus);
                    scene.focus = match cur_pos {
                        None => Some(hud_cycle[0]),
                        Some(i) if i + 1 >= hud_cycle.len() => None,
                        Some(i) => Some(hud_cycle[i + 1]),
                    };
                }
                continue;
            }
            UiAction::NavigateHudPrev => {
                if !hud_cycle.is_empty() {
                    let cur_pos = hud_cycle.iter().position(|t| Some(*t) == scene.focus);
                    scene.focus = match cur_pos {
                        None => Some(*hud_cycle.last().unwrap()),
                        Some(0) => None,
                        Some(i) => Some(hud_cycle[i - 1]),
                    };
                }
                continue;
            }
            UiAction::InvertSelection => {
                scene.marquee = None;
                let selected = ctx.run.selected_mut();
                let mut added = 0u32;
                let mut removed = 0u32;
                for s in selected.iter_mut() {
                    if *s {
                        removed += 1;
                    } else {
                        added += 1;
                    }
                    *s = !*s;
                }
                if added > 0 || removed > 0 {
                    play_select_sfx(ctx.bus, added, removed);
                }
                continue;
            }
            // Confirm: route by focused target.
            //   HandTile → toggle selection
            //   Button   → enqueue the corresponding gameplay action
            //   Consumable → use it
            //   Relic / Peg / Gold → no-op (display-only; focus
            //     exists so the player can read the tooltip from the
            //     keyboard)
            //   None → eat the press (avoids falling through to
            //     apply_ui_actions which would otherwise toggle a
            //     hand tile based on the stale focus_tile_index).
            UiAction::Confirm => {
                match scene.focus {
                    Some(FocusTarget::Relic(i))
                        if ctx.input_mode != crate::ui::input::InputMode::Cursor =>
                    {
                        // Keyboard and controller: hold Confirm to pick
                        // up the focused relic, then navigate to another
                        // relic and release to swap. Cursor LMB starts a
                        // drag via main.rs without routing through this
                        // arm, so the cursor path is a no-op here.
                        scene.held_relic_drag = Some(i);
                    }
                    Some(FocusTarget::Relic(_)) => {}
                    Some(FocusTarget::HandTile(i)) => {
                        if let Some((m, (added, removed))) =
                            GameEngine::begin_marquee_selection(ctx.run, i)
                        {
                            play_select_sfx(ctx.bus, added, removed);
                            scene.marquee = Some(m);
                        }
                    }
                    Some(FocusTarget::DiscardUndo) => {
                        actions_for_scene.push(UiAction::UndoDiscard);
                        continue;
                    }
                    Some(FocusTarget::Button(GameplayButton::Trigger)) => {
                        // Cash in is hold-to-confirm (see `cash_in_hold_started`):
                        // charging the timer commits; releasing early cancels.
                        let gameplay = GameEngine::read(ctx.run);
                        if gameplay.trigger_enabled || gameplay.cash_in_blocked_until_discards_spent
                        {
                            if scene.cash_in_hold_started.is_none() {
                                scene.cash_in_hold_started =
                                    Some(crate::ui::prompt_hold_ring::begin_hold(
                                        now,
                                        ctx.bus,
                                        gameplay.trigger_enabled,
                                    ));
                            }
                        } else {
                            // Nothing to cash in: fire immediately so the
                            // rejection feedback still plays.
                            actions_for_scene.push(UiAction::TriggerStructure);
                        }
                    }
                    Some(FocusTarget::Button(b)) => {
                        if let Some(a) = b.ui_action() {
                            actions_for_scene.push(a);
                        }
                    }
                    Some(FocusTarget::Journal) if scene.journal_transition.is_none() => {
                        scene.journal_transition = Some(JournalTransition {
                            start: now,
                            dir: JournalDirection::Opening,
                        });
                        return Some(None);
                    }
                    Some(FocusTarget::Guidebook) if scene.journal_transition.is_none() => {
                        *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Guide(
                            GuideScene::new(),
                        ))));
                        return Some(None);
                    }
                    Some(FocusTarget::WallHud) => {
                        *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                            Scene::WallLedger(WallLedgerScene::live()),
                        )));
                        return Some(None);
                    }
                    Some(FocusTarget::Consumable(i)) => {
                        let outcome = {
                            let mut engine = GameEngine::new(ctx.run, ctx.bus);
                            engine.dispatch(GameCommand::UseConsumable { index: i })
                        };
                        match outcome.data {
                            CommandData::UseConsumable {
                                result:
                                    crate::game::run::ConsumableUseResult::Zodiac { yaku, new_level },
                            } => {
                                log::info!("Used Zodiac → {} now level {}", yaku.name(), new_level,);
                                let label = format!("{} Lvl.{}", yaku.name(), new_level);
                                let src = ctx.cursor_pos;
                                scene.score_popups.spawn(
                                    label,
                                    crate::render::world_space::LayoutAnchorPx {
                                        px: src.0,
                                        py: src.1,
                                        lift_z: crate::render::score_popups::TABLE_POPUP_LIFT_Z,
                                    },
                                    src,
                                    None,
                                    crate::core::scoring::StepKind::Yen,
                                    new_level as f32,
                                    crate::render::score_popups::PopupMotionTiming::shipping_default(),
                                );
                                scene.particles.emit(
                                    src.0,
                                    src.1,
                                    24,
                                    crate::render::theme::color::RELIC_GOLD,
                                    0.9,
                                );
                                ctx.bus
                                    .push(crate::game::event_bus::GameEvent::ZodiacLevelUp);
                            }
                            CommandData::UseConsumable {
                                result: crate::game::run::ConsumableUseResult::Talisman { kind },
                            } => {
                                if let Some(enh) = kind.enhancement() {
                                    log::info!(
                                        "Used {} — every tile in hand stamped with {:?}",
                                        kind.name(),
                                        enh,
                                    );
                                } else {
                                    log::info!("Used {}", kind.name());
                                }
                            }
                            _ => {}
                        }
                        if outcome.rejection.is_none()
                            && matches!(outcome.data, CommandData::UseConsumable { .. })
                        {
                            scene.clear_discard_undo();
                        }
                        let post = GameEngine::read_interaction(ctx.run);
                        scene.focus = focus_after_consumable_use(
                            i,
                            post.consumable_count,
                            post.hand_len,
                            &focus_rects,
                        );
                    }
                    Some(FocusTarget::Peg(_))
                    | Some(FocusTarget::Gold)
                    | Some(FocusTarget::YakuTablet(_))
                    | Some(FocusTarget::Dora)
                    | Some(FocusTarget::Ordeal)
                    | Some(FocusTarget::RoundWind)
                    | Some(FocusTarget::ScoreRoller(_))
                    | Some(FocusTarget::Journal)
                    | Some(FocusTarget::Guidebook) => {}
                    None => {}
                }
                continue;
            }
            UiAction::ConfirmRelease => {
                let released_early_cash_in = scene
                    .cash_in_hold_started
                    .is_some_and(|start| released_before_hold_complete(start, now));
                scene.clear_cash_in_hold(ctx.bus);
                if released_early_cash_in {
                    scene.trigger_cash_in_hold_tooltip(ctx.run, now);
                }
                if let Some(from_idx) = scene.held_relic_drag.take()
                    && let Some(FocusTarget::Relic(to_idx)) = scene.focus
                {
                    let _ = GameEngine::swap_active_relics(ctx.run, from_idx, to_idx);
                }
                // Commit the marquee multi-select. The snapshot was applied
                // every focus tick; releasing simply drops the bookkeeping
                // so the next Confirm starts fresh.
                scene.marquee = None;
                continue;
            }
            // Cancel: snap focus to the hand AND let the existing
            // clear_selection path run via apply_ui_actions.
            UiAction::Cancel => {
                scene.held_relic_drag = None;
                scene.marquee = None;
                scene.clear_cash_in_hold(ctx.bus);
                scene.focus = default_hand_tile_focus(interaction.hand_len, &focus_rects);
                actions_for_scene.push(a);
                continue;
            }
            // X / Y as focus moves (when "X and Y quick action" is OFF):
            // place focus on Play / Discard so the player can read its
            // tooltip and confirm with A. We only honour the request if
            // the corresponding button is actually present in the focus
            // graph this frame.
            UiAction::FocusPlayButton => {
                if focus_rects
                    .iter()
                    .any(|(t, _)| matches!(t, FocusTarget::Button(GameplayButton::Play)))
                {
                    scene.focus = Some(FocusTarget::Button(GameplayButton::Play));
                }
                continue;
            }
            UiAction::FocusDiscardButton => {
                if focus_rects
                    .iter()
                    .any(|(t, _)| matches!(t, FocusTarget::Button(GameplayButton::Discard)))
                {
                    scene.focus = Some(FocusTarget::Button(GameplayButton::Discard));
                }
                continue;
            }
            UiAction::WestFacePress => {
                if ctx.run.onboarding_discard_allowed() {
                    actions_for_scene.push(UiAction::CommitDiscard);
                } else if ctx.run.onboarding_lessons_active() {
                    scene.reject_tutorial_gated_action(ctx.bus);
                } else {
                    ctx.bus.push(crate::game::event_bus::GameEvent::UiSound(
                        crate::sfx_id::SfxId::InvalidAction,
                    ));
                }
                continue;
            }
            UiAction::NorthFacePress => {
                actions_for_scene.push(UiAction::ScoreHand);
                continue;
            }
            UiAction::WestFaceRelease => {
                continue;
            }
            // Cash in is hold-to-confirm for keyboard (T) and gamepad triggers.
            // The instant mouse path lives in the 3D-pick dispatcher below.
            UiAction::TriggerStructure => {
                let gameplay = GameEngine::read(ctx.run);
                if gameplay.trigger_enabled || gameplay.cash_in_blocked_until_discards_spent {
                    if scene.cash_in_hold_started.is_none() {
                        scene.cash_in_hold_started = Some(crate::ui::prompt_hold_ring::begin_hold(
                            now,
                            ctx.bus,
                            gameplay.trigger_enabled,
                        ));
                    }
                } else {
                    // Nothing to cash in: fire immediately so the rejection
                    // feedback (shake) still plays.
                    actions_for_scene.push(UiAction::TriggerStructure);
                }
                continue;
            }
            UiAction::TriggerStructureRelease => {
                let released_early_cash_in = scene
                    .cash_in_hold_started
                    .is_some_and(|start| released_before_hold_complete(start, now));
                scene.clear_cash_in_hold(ctx.bus);
                if released_early_cash_in {
                    scene.trigger_cash_in_hold_tooltip(ctx.run, now);
                }
                continue;
            }
            _ => {}
        }
        actions_for_scene.push(a);
    }

    // 3D-hit dispatcher: when the catch-all `GAMEPLAY_3D_HIT_ID`
    // button fires, route the click based on what the renderer's
    // raycast picker actually hit this frame. The picker is the
    // single source of truth for which 3D action object the cursor
    // is over — we synthesise the same `UiAction`s that the keyboard
    // shortcuts enqueue and append them to `actions_for_scene`, so
    // the rest of the update flow (cascade-active gating, focus
    // highlighting, animation triggers, etc.) is identical for
    // mouse and keyboard. Yaku tablets are hover-only and don't
    // contribute clicks.
    for &cid in ctx.button_clicks {
        if cid == super::UNDO_DISCARD_CLICK_ID {
            if crate::persistence::load_settings().discard_undo_enabled
                && scene.discard_undo.is_some()
                && scene.pending_discard_refill.is_none()
            {
                actions_for_scene.push(UiAction::UndoDiscard);
            }
            continue;
        }
        if cid == super::WALL_HUD_CLICK_ID {
            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::WallLedger(
                WallLedgerScene::live(),
            ))));
            return Some(None);
        }
        if cid != super::GAMEPLAY_3D_HIT_ID {
            continue;
        }
        use crate::render::wgpu_renderer::GameplayPick;
        let gameplay = GameEngine::read(ctx.run);
        let cash_in_enabled = gameplay.trigger_enabled;
        if matches!(ctx.picked_gameplay_object, Some(GameplayPick::JournalBook))
            && scene.journal_transition.is_none()
        {
            scene.journal_transition = Some(JournalTransition {
                start: now,
                dir: JournalDirection::Opening,
            });
            return Some(None);
        }
        if matches!(ctx.picked_gameplay_object, Some(GameplayPick::GuideBook))
            && scene.journal_transition.is_none()
        {
            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Guide(
                GuideScene::new(),
            ))));
            return Some(None);
        }
        let (cx, cy) = ctx.cursor_pos;
        let focus_rects = scene.last_focus_rects.borrow();
        if play_button_hit_at_cursor(&focus_rects, cx, cy) {
            actions_for_scene.push(UiAction::ScoreHand);
            continue;
        }
        let action = match ctx.picked_gameplay_object {
            Some(GameplayPick::CashInButton) if cash_in_enabled => Some(UiAction::TriggerStructure),
            Some(GameplayPick::DiscardBowl) => Some(UiAction::CommitDiscard),
            _ => None,
        };
        if let Some(a) = action {
            actions_for_scene.push(a);
        }
    }

    if actions_for_scene
        .iter()
        .any(|a| matches!(a, UiAction::DebugToggleAxes))
    {
        scene.debug_show_axes = !scene.debug_show_axes;
        log::info!(
            "[debug] world-axes overlay {}",
            if scene.debug_show_axes { "ON" } else { "OFF" }
        );
    }

    // Normal input handling when no cascade is active.
    for a in &actions_for_scene {
        match a {
            UiAction::ScoreHand => {
                let gameplay = GameEngine::read(ctx.run);
                let played_chips_before = GameEngine::structure_played_meld_fu(ctx.run);
                let round_before = gameplay.round_score;
                let score_before = gameplay.round_score;
                let cascade_showcase = if gameplay.selected_count == 0 {
                    None
                } else {
                    let selected_tiles: Vec<_> = ctx
                        .run
                        .hand()
                        .iter()
                        .zip(interaction.selected.iter())
                        .filter(|&(_, &sel)| sel)
                        .map(|(t, _)| *t)
                        .collect();
                    GameEngine::validate_with_wildcards(ctx.run, &selected_tiles).map(
                        |(sets, scoring_tiles)| {
                            let sets = ctx.run.pick_best_decomposition(
                                sets,
                                &scoring_tiles,
                                &selected_tiles,
                            );
                            let mut tiles = GameplayScene::display_tiles(
                                gameplay.structure_tiles.iter().copied(),
                                ctx.run,
                            );
                            tiles.extend(GameplayScene::display_tiles(scoring_tiles, ctx.run));
                            let mut all_sets = gameplay.structure_sets.clone();
                            all_sets.extend(sets);
                            CascadeShowcase {
                                tiles,
                                sets: all_sets,
                            }
                        },
                    )
                };
                use crate::core::structure::{
                    structure_remaining_slots_callout, structure_remaining_tile_slots,
                };
                use crate::game::game_mode::HAND_SIZE;
                let remaining_before = structure_remaining_tile_slots(
                    &gameplay.structure_tiles,
                    &gameplay.structure_sets,
                    HAND_SIZE,
                );
                let outcome = {
                    let mut engine = GameEngine::new(ctx.run, ctx.bus);
                    engine.dispatch(GameCommand::CommitSelection)
                };
                let step = match outcome.data {
                    CommandData::CommitSelection { step } => step,
                    _ => 0,
                };
                if step > 0 {
                    scene.clear_discard_undo();
                }
                let gained = outcome.after.round_score.saturating_sub(round_before);
                log::info!(
                    "[score] Commit: step={} gained={} breakdown_steps={} base_steps={}",
                    step,
                    gained,
                    ctx.run
                        .last_breakdown
                        .as_ref()
                        .map(|b| b.steps.len())
                        .unwrap_or(0),
                    ctx.run
                        .last_breakdown
                        .as_ref()
                        .map(|b| b.base_steps.len())
                        .unwrap_or(0),
                );

                if matches!(
                    outcome.rejection,
                    Some(crate::game::engine::CommandRejection::InvalidSelection)
                ) {
                    scene.trigger_invalid_meld_flash(
                        ctx.run,
                        &interaction.hand,
                        &interaction.selected,
                    );
                    ctx.run.onboarding_notify_invalid_play();
                    ctx.anim
                        .shake(crate::render::animation::ENTITY_HAND_STRIP, 8.0, 200);
                    if let Some(msg) = ctx.run.play_rejection_callout() {
                        spawn_structure_status_callout(scene, ctx.layout, ctx.run, msg, false);
                    }
                } else {
                    scene.invalid_meld_flash_at = None;
                    scene.invalid_meld_flash_slots.clear();
                    scene.clear_boss_rule_feedback();
                    ctx.run.onboarding_clear_invalid_meld_hint();
                    if gained > 0 {
                        ctx.anim.pulse(ENTITY_SCORE_PANEL);
                        scene.begin_scoring_cascade(ctx, score_before, gained, cascade_showcase);
                    } else if step > 0 {
                        ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                        let played_chips_after = GameEngine::structure_played_meld_fu(ctx.run);
                        let d = played_chips_after.saturating_sub(played_chips_before);
                        if d > 0 {
                            let gameplay_after = GameEngine::read(ctx.run);
                            let remaining = structure_remaining_tile_slots(
                                &gameplay_after.structure_tiles,
                                &gameplay_after.structure_sets,
                                HAND_SIZE,
                            );
                            if remaining == 0 && remaining_before > 0 {
                                scene.final_tiles_fov_pop_at = Some(Instant::now());
                            }
                            let callout = structure_remaining_slots_callout(remaining);
                            spawn_structure_status_callout(
                                scene,
                                ctx.layout,
                                ctx.run,
                                &callout,
                                remaining > 0,
                            );
                        }
                    }
                }
            }
            UiAction::TriggerStructure => {
                // Instant path (mouse pick on the Cash In tablet). Keyboard /
                // gamepad route through `cash_in_hold_started` instead.
                execute_cash_in(scene, ctx);
            }
            UiAction::CommitDiscard => {
                if !ctx.run.onboarding_discard_allowed() {
                    if ctx.run.onboarding_lessons_active() {
                        scene.reject_tutorial_gated_action(ctx.bus);
                    } else {
                        ctx.bus.push(crate::game::event_bus::GameEvent::UiSound(
                            crate::sfx_id::SfxId::InvalidAction,
                        ));
                    }
                    continue;
                }
                let gameplay = GameEngine::read(ctx.run);
                let snap_before = if gameplay.selected_count > 0 && gameplay.discards_remaining > 0
                {
                    Some(DiscardUndoSnapshot::capture(ctx.run))
                } else {
                    None
                };
                let mut pre_discard_tiles: Vec<crate::core::tile::Tile> = Vec::new();
                let mut pre_discard_indices: Vec<usize> = Vec::new();
                if gameplay.selected_count > 0 && gameplay.discards_remaining > 0 {
                    let interaction = GameEngine::read_interaction(ctx.run);
                    for (i, &sel) in ctx.run.selected_slice().iter().enumerate() {
                        if sel {
                            pre_discard_indices.push(i);
                            pre_discard_tiles.push(interaction.hand[i]);
                        }
                    }
                }
                // Remove the tiles immediately, but defer the auto-draw
                // until the discard river animation has finished.
                let outcome = {
                    let mut engine = GameEngine::new(ctx.run, ctx.bus);
                    engine.dispatch(GameCommand::DiscardSelectionNoRefill)
                };
                let discarded = match outcome.data {
                    CommandData::DiscardSelection { count } => count,
                    _ => 0,
                };
                if discarded > 0 {
                    if let Some(s) = snap_before {
                        scene.discard_undo = Some(s);
                    }
                    super::discard_animation::begin_discard_batch(
                        scene,
                        ctx.layout,
                        ctx.room_gltf_height_scale,
                        &pre_discard_indices,
                        &pre_discard_tiles,
                        now,
                    );
                    ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                    let fallback =
                        std::time::Duration::from_millis(ctx.cascade_tuning.discard_refill_cap_ms);
                    let anim_dur = scene
                        .active_discard_anim
                        .as_ref()
                        .map(|b| b.total_duration(ctx.cascade_tuning))
                        .unwrap_or(fallback);
                    scene.pending_discard_refill = Some(now + anim_dur.max(fallback));
                }
            }
            UiAction::UndoDiscard => {
                if crate::persistence::load_settings().discard_undo_enabled
                    && scene.pending_discard_refill.is_none()
                    && let Some(snap) = scene.discard_undo.take()
                {
                    scene.active_discard_anim = None;
                    scene.river_settled_tiles.clear();
                    scene.river_sink_batch = None;
                    ctx.run.apply_discard_undo(snap, Some(ctx.bus));
                    ctx.bus.push(crate::game::event_bus::GameEvent::UiSound(
                        crate::sfx_id::SfxId::TilePlace,
                    ));
                    ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                }
            }
            _ => {}
        }
    }
    // Complete (or invalidate) an in-progress hold-to-cash-in. Mirrors the
    // shop's hold-to-sell: the action fires once the timer crosses the
    // threshold without waiting for release; an early release cancels it
    // (handled above). Progress freezes while cash-in is blocked.
    let trigger_enabled = GameEngine::read(ctx.run).trigger_enabled;
    scene.tick_cash_in_hold_anchor(now, trigger_enabled);
    if let Some(start) = scene.cash_in_hold_started {
        if trigger_enabled
            && now.saturating_duration_since(start).as_secs_f32() >= super::cash_in_hold_seconds()
        {
            ctx.run.onboarding_notify_hold_success();
            scene.clear_cash_in_hold(ctx.bus);
            execute_cash_in(scene, ctx);
        }
    }

    // Let apply_ui_actions handle toggle-select, cancel, and focus movement.
    let non_handled: Vec<_> = actions_for_scene
        .iter()
        .filter(|a| {
            !matches!(
                a,
                UiAction::ScoreHand
                    | UiAction::TriggerStructure
                    | UiAction::CommitDiscard
                    | UiAction::UndoDiscard
            )
        })
        .copied()
        .collect();
    apply_ui_actions(
        &non_handled,
        ctx.run,
        ctx.bus,
        ctx.anim,
        ctx.focus_tile_index,
    );
    let focus_kind_after = focus_kind(scene.focus);
    if focus_kind_after != focus_kind_before
        && let Some(sfx) = focus_kind_after.and_then(focus_kind_sfx)
    {
        ctx.bus
            .push(crate::game::event_bus::GameEvent::UiSound(sfx));
    }
    None
}

/// Run the cash-in (TriggerStructure) command and its scoring feedback. Shared
/// by the instant mouse path and the hold-to-cash-in completion.
fn execute_cash_in(scene: &mut GameplayScene, ctx: &mut UpdateCtx<'_>) {
    let score_before = GameEngine::read(ctx.run).round_score;
    let gameplay = GameEngine::read(ctx.run);
    let cascade_showcase = Some(CascadeShowcase {
        tiles: GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), ctx.run),
        sets: gameplay.structure_sets.clone(),
    });
    let outcome = {
        let mut engine = GameEngine::new(ctx.run, ctx.bus);
        engine.dispatch(GameCommand::TriggerStructure)
    };
    let earned = match outcome.data {
        CommandData::TriggerStructure { earned } => earned,
        _ => 0,
    };
    if earned > 0 {
        scene.clear_discard_undo();
    }
    let gained = outcome.after.round_score.saturating_sub(score_before);
    log::info!(
        "[score] TriggerStructure: earned={} gained={} breakdown_steps={} base_steps={}",
        earned,
        gained,
        ctx.run
            .last_breakdown
            .as_ref()
            .map(|b| b.steps.len())
            .unwrap_or(0),
        ctx.run
            .last_breakdown
            .as_ref()
            .map(|b| b.base_steps.len())
            .unwrap_or(0),
    );
    if earned == 0 {
        ctx.anim
            .shake(crate::render::animation::ENTITY_HAND_STRIP, 6.0, 160);
    } else {
        ctx.anim.pulse(ENTITY_SCORE_PANEL);
        scene.begin_scoring_cascade(ctx, score_before, gained, cascade_showcase);
    }
}

/// Cascade Lab: only the authored 3D cash-in control — no table focus/hover.
fn process_lab_cash_in(
    scene: &mut GameplayScene,
    ctx: &mut UpdateCtx<'_>,
    _now: Instant,
) -> Option<SceneTransition> {
    use crate::render::wgpu_renderer::GameplayPick;

    for &cid in ctx.button_clicks {
        if cid != super::GAMEPLAY_3D_HIT_ID {
            continue;
        }
        let cash_in_enabled = GameEngine::read(ctx.run).trigger_enabled;
        if matches!(ctx.picked_gameplay_object, Some(GameplayPick::CashInButton)) && cash_in_enabled
        {
            scene.lab_cash_in(ctx);
        }
    }
    None
}

pub(super) fn build_relic_tray(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    _scene_camera: Option<&CameraParams>,
    env_height_scale: f32,
    glb_relic_poses: &[crate::render::gameplay_glb::GameplayMarkerPose],
) -> Vec<Object3d> {
    use crate::core::relic::{RelicId, all_relic_defs};
    use crate::render::table_transform::{compose_rotation_euler, rot_euler_xyz_rad};
    use crate::render::world_space::pixel_to_world;

    let mut relic_objects: Vec<Object3d> = Vec::new();
    let active_ids = GameEngine::active_relics(run);
    let lab_tray = scene.lab_mode();
    if active_ids.is_empty() && !lab_tray {
        return relic_objects;
    }
    if glb_relic_poses.is_empty() {
        return relic_objects;
    }

    let w = layout.window_w;
    let h = layout.window_h;
    let stride_world = if glb_relic_poses.len() >= 2 {
        let a0 = pixel_to_world(
            w,
            h,
            glb_relic_poses[0].anchor[0],
            glb_relic_poses[0].anchor[1],
            glb_relic_poses[0].anchor[2],
        );
        let a1 = pixel_to_world(
            w,
            h,
            glb_relic_poses[1].anchor[0],
            glb_relic_poses[1].anchor[1],
            glb_relic_poses[1].anchor[2],
        );
        let d = a1 - a0;
        d.truncate().length().max(layout.mm(8.0))
    } else {
        w.min(h) * 0.048 * 1.1
    };
    let face = stride_world / 1.1;
    let slot_count = if lab_tray {
        glb_relic_poses.len()
    } else {
        active_ids.len().min(glb_relic_poses.len())
    };
    let defs = all_relic_defs();

    // Mesh geometry (see `build_relic_mesh`): the disc lies in local
    // XZ with radius 0.5, and thickness runs along local ±Y with
    // half-height 0.5. Extents are full-width scalars on each local
    // axis, so `[face, thick, face]` scales the disc to face-width
    // and the cylinder height to the (much smaller) badge thickness.
    for i in 0..slot_count {
        let pose = glb_relic_poses[i];
        let slot_face = face * pose.uniform_author_scale(h, env_height_scale);
        let rotation = crate::render::gameplay_glb::rotate_marker_pose_x_180(pose.rotation_rad);

        if let Some(&rid) = active_ids.get(i) {
            let visual = relic_visual(rid);
            let thick = slot_face * 0.06 * visual.thickness_scale;

            // Color tracks the relic's rarity tier.
            let rarity = defs
                .iter()
                .find(|d| d.id == rid)
                .map(|d| d.rarity)
                .unwrap_or(crate::core::relic::Rarity::Common);
            let color = crate::render::theme::color::rarity(rarity.tier());

            // Activation glow: fast-attack / smooth-decay envelope.
            let (glow, wiggle_deg, vertical_wave) =
                if let Some(start) = scene.relic_glow_starts.get(&rid) {
                    let now_for_glow = Instant::now();
                    let age = now_for_glow.saturating_duration_since(*start).as_secs_f32();
                    let life = RELIC_GLOW_LIFETIME.as_secs_f32();
                    if age >= life {
                        (0.0, 0.0, 0.0)
                    } else {
                        let t = (age / life).clamp(0.0, 1.0);
                        let attack_end = 0.12_f32;
                        let glow = if t < attack_end {
                            (t / attack_end).clamp(0.0, 1.0)
                        } else {
                            let decay_t = (t - attack_end) / (1.0 - attack_end);
                            (1.0 - decay_t).max(0.0).powi(2)
                        };
                        let wiggle = glow * 12.0 * (age * 22.0).sin();
                        let vertical = glow * (age * 18.0).sin();
                        (glow, wiggle, vertical)
                    }
                } else {
                    (0.0, 0.0, 0.0)
                };

            let mut rotation = rotation;
            if wiggle_deg != 0.0 {
                rotation = compose_rotation_euler(
                    rot_euler_xyz_rad(rotation[0], rotation[1], rotation[2]),
                    [0.0, wiggle_deg, 0.0],
                );
            }
            relic_objects.push(Object3d {
                pos: {
                    let mut anchor = pose.anchor;
                    if vertical_wave != 0.0 {
                        anchor[2] += layout.mm(RELIC_SCORE_VERTICAL_MM * vertical_wave);
                    }
                    anchor
                },
                extents: [slot_face, thick, slot_face],
                rotation,
                color,
                kind: Object3dKind::Relic {
                    relic_id: rid,
                    glow,
                    silhouette: false,
                    debuffed: run.relics.is_debuffed(rid),
                },
                hover_target: 0.0,
                anim_id: 0,
            });
        } else if lab_tray {
            // Cascade Lab: keep one Object3d per physical tray slot so
            // projected `relic_rects` indices match slot indices and empty
            // slots get click targets.
            let visual = relic_visual(RelicId::PairPower);
            let thick = slot_face * 0.06 * visual.thickness_scale;
            relic_objects.push(Object3d {
                pos: pose.anchor,
                extents: [slot_face, thick, slot_face],
                rotation,
                color: [0.06, 0.10, 0.08, 0.42],
                kind: Object3dKind::Relic {
                    relic_id: RelicId::PairPower,
                    glow: 0.0,
                    silhouette: true,
                    debuffed: false,
                },
                hover_target: 0.0,
                anim_id: 0,
            });
        }
    }

    relic_objects
}

/// Outputs of [`build_consumable_spawns`].
pub(super) struct ConsumableBuild {
    pub placements: Vec<crate::render::draw_cmd::Object3d>,
}

/// Spawn owned consumables at `player_consumables` GLB markers (dish mesh is static env).
pub(super) fn build_consumable_spawns(
    layout: &crate::ui::layout::LayoutResult,
    ctx: &crate::scenes::DrawCtx<'_>,
    interaction: &crate::game::engine::GameplayInteractionReadModel,
    paused: bool,
    focus_rect_graph: &mut Vec<(FocusTarget, [f32; 4])>,
    buttons: &mut Vec<crate::scenes::ButtonDef>,
    glb_consumable_poses: &[crate::render::gameplay_glb::GameplayMarkerPose],
) -> ConsumableBuild {
    use super::ZODIAC_USE_BASE;
    use crate::render::draw_cmd::{Object3d, Object3dKind};
    use crate::render::table_transform::{compose_rotation_euler, rot_euler_xyz_rad};

    let consumables = &interaction.consumables;
    let consumable_capacity = interaction.consumable_capacity;
    let mut placements: Vec<Object3d> = Vec::new();
    if consumable_capacity == 0 {
        return ConsumableBuild { placements };
    }

    let zscale = (layout.window_w.min(layout.window_h)) / 600.0;
    let base_slot_w = (140.0 * zscale).max(120.0);
    let base_slot_h = (56.0 * zscale).max(48.0);
    let gameplay_talisman_scale = 0.5;
    let ribbon_mesh_rot = crate::render::table_transform::euler_xyz_rad_from_deg(-90.0, 0.0, 0.0);
    let talisman_mesh_rot = crate::render::table_transform::euler_xyz_rad_from_deg(90.0, 0.0, 0.0);

    let mut talisman_draw_i: usize = 0;
    let mut ribbon_draw_i: usize = 0;
    for (slot_idx, &pose) in glb_consumable_poses
        .iter()
        .enumerate()
        .take(consumable_capacity)
    {
        let Some(&slot_item) = consumables.get(slot_idx) else {
            continue;
        };
        let author_scale = pose.uniform_author_scale(layout.window_h, ctx.room_gltf_height_scale);
        let slot_w = base_slot_w * author_scale;
        let slot_h = base_slot_h * author_scale;
        let slot_sx = pose.anchor[0] - slot_w * 0.5;
        let slot_sy = pose.anchor[1] - slot_h * 0.5;

        let (fx, fy, fw, fh) = match slot_item {
            crate::core::consumable::Consumable::Talisman(_)
            | crate::core::consumable::Consumable::Memorial(_) => {
                let proj_rect = ctx
                    .proj
                    .talisman_rects
                    .get(talisman_draw_i)
                    .copied()
                    .filter(|r| r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite());
                talisman_draw_i += 1;
                if let Some(r) = proj_rect {
                    (r[0], r[1], r[2], r[3])
                } else {
                    let fw =
                        crate::render::consumable_prop_scale::owned_talisman_tablet_extent(slot_w)
                            * gameplay_talisman_scale;
                    let fh = fw.min(slot_h);
                    (
                        slot_sx + (slot_w - fw) * 0.5,
                        slot_sy + (slot_h - fh) * 0.20,
                        fw,
                        fh,
                    )
                }
            }
            crate::core::consumable::Consumable::Zodiac(_) => {
                let proj_rect = ctx
                    .proj
                    .ribbon_rects
                    .get(ribbon_draw_i)
                    .copied()
                    .filter(|r| r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite());
                ribbon_draw_i += 1;
                if let Some(r) = proj_rect {
                    (r[0], r[1], r[2], r[3])
                } else {
                    let ribbon_len =
                        crate::render::consumable_prop_scale::owned_ribbon_length(slot_w, slot_h);
                    let fw = ribbon_len / 3.0;
                    let fh = ribbon_len.min(slot_h);
                    (
                        slot_sx + (slot_w - fw) * 0.5,
                        slot_sy + (slot_h - fh) * 0.20,
                        fw,
                        fh,
                    )
                }
            }
        };
        focus_rect_graph.push((FocusTarget::Consumable(slot_idx), [fx, fy, fw, fh]));

        let color = match slot_item {
            crate::core::consumable::Consumable::Zodiac(_) => [0.45, 0.78, 0.55, 1.0],
            crate::core::consumable::Consumable::Talisman(tk) => tk.accent_color(),
            crate::core::consumable::Consumable::Memorial(mk) => mk.accent_color(),
        };

        match slot_item {
            crate::core::consumable::Consumable::Zodiac(z) => {
                let ribbon_length =
                    crate::render::consumable_prop_scale::owned_ribbon_length(slot_w, slot_h);
                placements.push(crate::render::ribbon_mesh::zodiac_ribbon_object3d(
                    crate::render::ribbon_mesh::ZodiacRibbonSpec {
                        pos: pose.anchor,
                        length: ribbon_length,
                        rotation: ribbon_mesh_rot,
                        color: [1.0, 1.0, 1.0, 1.0],
                        kind: Some(z),
                        hover_target: 0.0,
                        anim_id: 0,
                        placement_rot_deg: pose.rotation_deg(),
                    },
                ));
            }
            crate::core::consumable::Consumable::Talisman(tk) => {
                let tablet_extent =
                    crate::render::consumable_prop_scale::owned_talisman_tablet_extent(slot_w)
                        * gameplay_talisman_scale;
                placements.push(Object3d {
                    pos: pose.anchor,
                    extents: crate::render::talisman_mesh::talisman_object_extents(tablet_extent),
                    rotation: compose_rotation_euler(
                        rot_euler_xyz_rad(
                            talisman_mesh_rot[0],
                            talisman_mesh_rot[1],
                            talisman_mesh_rot[2],
                        ),
                        pose.rotation_deg(),
                    ),
                    color,
                    kind: Object3dKind::Talisman { kind: tk },
                    hover_target: 0.0,
                    anim_id: 0,
                });
            }
            crate::core::consumable::Consumable::Memorial(mk) => {
                let tablet_extent =
                    crate::render::consumable_prop_scale::owned_talisman_tablet_extent(slot_w)
                        * gameplay_talisman_scale;
                placements.push(Object3d {
                    pos: pose.anchor,
                    extents: crate::render::talisman_mesh::talisman_object_extents(tablet_extent),
                    rotation: compose_rotation_euler(
                        rot_euler_xyz_rad(
                            talisman_mesh_rot[0],
                            talisman_mesh_rot[1],
                            talisman_mesh_rot[2],
                        ),
                        pose.rotation_deg(),
                    ),
                    color,
                    kind: Object3dKind::MemorialTalisman { kind: mk },
                    hover_target: 0.0,
                    anim_id: 0,
                });
            }
        }

        if !paused {
            buttons.push(crate::scenes::ButtonDef::scene(
                (fx, fy, fw, fh),
                ZODIAC_USE_BASE + slot_idx as u32,
            ));
        }
    }

    ConsumableBuild { placements }
}

/// Keyboard-nav focus rects for discard / play action buttons (layout rects).
pub(super) fn push_action_button_focus_rects(
    btn_rects: &[(f32, f32, f32, f32); 3],
    cash_in_enabled: bool,
    focus_rect_graph: &mut Vec<(super::focus::FocusTarget, [f32; 4])>,
) {
    use super::focus::{ALL_BUTTONS, FocusTarget};
    for (i, &(bx, by, bw, bh)) in btn_rects.iter().enumerate() {
        if i == 2 && !cash_in_enabled {
            continue;
        }
        if bw <= 1.0 || bh <= 1.0 {
            continue;
        }
        focus_rect_graph.push((FocusTarget::Button(ALL_BUTTONS[i]), [bx, by, bw, bh]));
    }
}

/// Pick-ray proxies for authored `gameplay.glb` discard / play / journal meshes.
pub(super) fn build_glb_action_pick_proxies(
    anchors: &super::glb_anchors::GameplayGlbAnchors,
    journal_open_amount: f32,
    _has_structure: bool,
) -> ActionRowOutputs {
    use crate::render::draw_cmd::Object3dKind;

    let mut journal_book = anchors.journal_pick.clone();
    if let Object3dKind::Book { open_amount, .. } = &mut journal_book.kind {
        *open_amount = journal_open_amount;
    }

    ActionRowOutputs {
        wood_tablet_placements: Vec::new(),
        discard_bowl_placement: Some(anchors.discard_river_pick.clone()),
        bronze_mirror_placement: Some(anchors.play_mirror_pick.clone()),
        journal_book: Some(journal_book),
        guidebook: Some(anchors.guidebook_pick.clone()),
    }
}

/// Outputs of the action row + journal book builder.
pub(super) struct ActionRowOutputs {
    pub(super) wood_tablet_placements: Vec<crate::render::draw_cmd::Object3d>,
    pub(super) discard_bowl_placement: Option<crate::render::draw_cmd::Object3d>,
    pub(super) bronze_mirror_placement: Option<crate::render::draw_cmd::Object3d>,
    pub(super) journal_book: Option<crate::render::draw_cmd::Object3d>,
    pub(super) guidebook: Option<crate::render::draw_cmd::Object3d>,
}

/// Outputs of the yaku panel + structure showcase + yaku tablet builder.
pub(super) struct YakuPanelOutputs {
    pub(super) yaku_preview_effective_tiles: Vec<crate::core::tile::Tile>,
    pub(super) yaku_preview_sets: Vec<crate::core::hand::DetectedMeld>,
    pub(super) yaku_tablet_placements: Vec<crate::render::draw_cmd::Object3d>,
    pub(super) structure_showcase: Vec<crate::render::draw_cmd::ShowcaseTilePlacement>,
}

/// Median of packed object3d anchor triples (matches showcase tile `center_pos` encoding).
pub(super) fn median_layout_anchor(
    centers: &[[f32; 3]],
) -> crate::render::world_space::LayoutAnchorPx {
    debug_assert!(!centers.is_empty());
    let median = |values: &mut Vec<f32>| {
        values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = values.len() / 2;
        if values.len() % 2 == 1 {
            values[mid]
        } else {
            (values[mid - 1] + values[mid]) * 0.5
        }
    };
    let mut xs: Vec<f32> = centers.iter().map(|c| c[0]).collect();
    let mut ys: Vec<f32> = centers.iter().map(|c| c[1]).collect();
    let mut zs: Vec<f32> = centers.iter().map(|c| c[2]).collect();
    crate::render::world_space::LayoutAnchorPx {
        px: median(&mut xs),
        py: median(&mut ys),
        lift_z: median(&mut zs),
    }
}

/// Target pose for a hand tile lifted onto the structure strip (after committed melds).
pub(super) struct StagingPreviewAnchor {
    pub center_pos: [f32; 3],
    pub rotation: [f32; 3],
    pub size_px: f32,
}

fn structure_strip_gaps(layout_scale: f32) -> (f32, f32) {
    (
        (3.0 * layout_scale).max(2.0),
        (10.0 * layout_scale).max(7.0),
    )
}

/// Minimum structure-tile width before gap compression kicks in.
const STRUCTURE_TILE_MIN_PX: f32 = 11.0;

pub(super) struct StructureStripLayout {
    pub tile_size: f32,
    pub layout_span: f32,
    pub intra_gap: f32,
    pub inter_gap: f32,
}

fn structure_strip_counts(sets: &[crate::core::hand::DetectedMeld]) -> (usize, usize, usize) {
    let total_tiles: usize = sets.iter().map(|s| s.tile_ids.len()).sum();
    let intra_count: usize = sets
        .iter()
        .map(|s| s.tile_ids.len().saturating_sub(1))
        .sum();
    let inter_count = sets.len().saturating_sub(1);
    (total_tiles, intra_count, inter_count)
}

/// Tile size + gaps for the full structure strip (every committed + pending meld).
fn compute_structure_strip_layout(
    span: f32,
    layout_scale: f32,
    sets: &[crate::core::hand::DetectedMeld],
) -> StructureStripLayout {
    let layout_span = span.max(8.0);
    let max_tile = (44.0 * layout_scale).max(28.0);
    let (base_intra, base_inter) = structure_strip_gaps(layout_scale);
    let (total_tiles, intra_count, inter_count) = structure_strip_counts(sets);

    if total_tiles == 0 {
        return StructureStripLayout {
            tile_size: max_tile,
            layout_span,
            intra_gap: base_intra,
            inter_gap: base_inter,
        };
    }

    let mut intra_gap = base_intra;
    let mut inter_gap = base_inter;
    let mut tile_size;
    for _ in 0..4 {
        let gap_space = intra_count as f32 * intra_gap + inter_count as f32 * inter_gap;
        let available = (layout_span - gap_space).max(0.0);
        tile_size = (available / total_tiles as f32).min(max_tile);
        if tile_size >= STRUCTURE_TILE_MIN_PX {
            break;
        }
        let scale = (tile_size / STRUCTURE_TILE_MIN_PX).clamp(0.35, 1.0);
        intra_gap = (base_intra * scale).max(1.0);
        inter_gap = (base_inter * scale).max(2.0);
    }
    let gap_space = intra_count as f32 * intra_gap + inter_count as f32 * inter_gap;
    let available = (layout_span - gap_space).max(0.0);
    tile_size = (available / total_tiles as f32).clamp(8.0, max_tile);

    StructureStripLayout {
        tile_size,
        layout_span,
        intra_gap,
        inter_gap,
    }
}

fn structure_strip_cursor_after_committed(
    strip: &StructureStripLayout,
    committed_sets: &[crate::core::hand::DetectedMeld],
) -> f32 {
    let mut cursor = 0.0f32;
    for (mi, set) in committed_sets.iter().enumerate() {
        for _ in &set.tile_ids {
            cursor += strip.tile_size + strip.intra_gap;
        }
        if mi + 1 < committed_sets.len() {
            cursor += strip.inter_gap - strip.intra_gap;
        }
    }
    cursor
}

/// Layout anchors for staging previews appended after tiles already in structure.
pub(super) fn staging_preview_anchors_for_groups(
    structure_marker_poses: &[crate::render::gameplay_glb::GameplayMarkerPose; 2],
    layout: &crate::ui::layout::LayoutResult,
    layout_scale: f32,
    env_h: f32,
    layout_sets: &[crate::core::hand::DetectedMeld],
    committed_sets: &[crate::core::hand::DetectedMeld],
    preview_groups: &[Vec<usize>],
) -> rustc_hash::FxHashMap<usize, StagingPreviewAnchor> {
    use crate::render::gameplay_glb::{
        lerp_marker_anchor, lerp_marker_rotation_rad, marker_pair_span_px,
    };

    let mut out = rustc_hash::FxHashMap::default();
    if preview_groups.is_empty() {
        return out;
    }

    let a_l = structure_marker_poses[0].anchor;
    let a_r = structure_marker_poses[1].anchor;
    let rot_l = structure_marker_poses[0].rotation_rad;
    let rot_r = structure_marker_poses[1].rotation_rad;
    let structure_scale = structure_marker_poses[0].uniform_author_scale(layout.window_h, env_h);
    let span = marker_pair_span_px(a_l, a_r);
    let strip = compute_structure_strip_layout(span, layout_scale, layout_sets);
    let size_px = strip.tile_size * structure_scale;

    let mut cursor = structure_strip_cursor_after_committed(&strip, committed_sets);
    if !committed_sets.is_empty() && !preview_groups.is_empty() {
        cursor += strip.inter_gap - strip.intra_gap;
    }

    for (gi, group) in preview_groups.iter().enumerate() {
        for (ti, &hand_idx) in group.iter().enumerate() {
            let t = if strip.layout_span > 0.0 {
                (cursor + strip.tile_size * 0.5) / strip.layout_span
            } else {
                0.5
            };
            let t = t.clamp(0.0, 1.0);
            let mut anchor = lerp_marker_anchor(a_l, a_r, t);
            let lift_mm = ti as f32 * 1.2 + gi as f32 * 0.15;
            anchor[2] += layout.mm(lift_mm);
            out.insert(
                hand_idx,
                StagingPreviewAnchor {
                    center_pos: anchor,
                    rotation: lerp_marker_rotation_rad(rot_l, rot_r, t),
                    size_px,
                },
            );
            cursor += strip.tile_size + strip.intra_gap;
        }
        if gi + 1 < preview_groups.len() {
            cursor += strip.inter_gap - strip.intra_gap;
        }
    }

    out
}

/// Median center for structure showcase tiles (matches [`build_yaku_panel_and_tablets`] layout).
pub(super) fn structure_showcase_tile_popup_center(
    structure_marker_poses: &[crate::render::gameplay_glb::GameplayMarkerPose; 2],
    layout: &crate::ui::layout::LayoutResult,
    layout_scale: f32,
    showcase: &CascadeShowcase,
    tile_ids: &[u32],
    has_structure: bool,
    cascade_showcase_active: bool,
) -> Option<crate::render::world_space::LayoutAnchorPx> {
    let a_l = structure_marker_poses[0].anchor;
    let a_r = structure_marker_poses[1].anchor;
    let span = crate::render::gameplay_glb::marker_pair_span_px(a_l, a_r);
    let _ = (has_structure, cascade_showcase_active);
    let strip = compute_structure_strip_layout(span, layout_scale, &showcase.sets);
    let mut cursor = 0.0f32;
    let mut centers: Vec<[f32; 3]> = Vec::new();
    for (mi, set) in showcase.sets.iter().enumerate() {
        for (ti, &tid) in set.tile_ids.iter().enumerate() {
            let t = if strip.layout_span > 0.0 {
                (cursor + strip.tile_size * 0.5) / strip.layout_span
            } else {
                0.5
            };
            let mut anchor =
                crate::render::gameplay_glb::lerp_marker_anchor(a_l, a_r, t.clamp(0.0, 1.0));
            if tile_ids.contains(&tid) {
                let lift_mm = ti as f32 * 1.2 + mi as f32 * 0.15;
                anchor[2] += layout.mm(lift_mm);
                centers.push(anchor);
            }
            cursor += strip.tile_size + strip.intra_gap;
        }
        if mi + 1 < showcase.sets.len() {
            cursor += strip.inter_gap - strip.intra_gap;
        }
    }
    (!centers.is_empty()).then(|| median_layout_anchor(&centers))
}

/// Spawn point for structure growth / capacity callouts — just above the
/// committed structure tile row (falls back to the strip midpoint).
pub(super) fn structure_strip_callout_anchor(
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
) -> Option<crate::render::world_space::LayoutAnchorPx> {
    use super::cascade_hud::CascadeShowcase;
    use crate::game::engine::GameEngine;

    let interaction = GameEngine::read_interaction(run);
    let env_h = crate::render::room_glb::SHOP_ENV_HEIGHT_SCALE;
    let anchors =
        super::glb_anchors::try_resolve_gameplay_glb_anchors(layout, interaction.hand_len, env_h)?;
    let layout_scale = (layout.window_w.min(layout.window_h)) / 600.0;
    let structure_marker_poses = &anchors.structure_marker_poses;
    let a_l = structure_marker_poses[0].anchor;
    let a_r = structure_marker_poses[1].anchor;
    let span = crate::render::gameplay_glb::marker_pair_span_px(a_l, a_r);
    let structure_scale = structure_marker_poses[0].uniform_author_scale(layout.window_h, env_h);
    let strip_center = crate::render::gameplay_glb::lerp_marker_anchor(a_l, a_r, 0.5);

    let gameplay = GameEngine::read(run);
    let sets = gameplay.structure_sets.clone();
    let strip = compute_structure_strip_layout(span, layout_scale, &sets);
    let tile_h = strip.tile_size * structure_scale;

    let base = if gameplay.has_structure {
        let showcase = CascadeShowcase {
            tiles: GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), run),
            sets,
        };
        let tile_ids: Vec<u32> = showcase.tiles.iter().map(|t| t.id).collect();
        structure_showcase_tile_popup_center(
            structure_marker_poses,
            layout,
            layout_scale,
            &showcase,
            &tile_ids,
            true,
            false,
        )?
    } else {
        crate::render::world_space::LayoutAnchorPx {
            px: strip_center[0],
            py: strip_center[1],
            lift_z: strip_center[2],
        }
    };

    // Table-surface lift (not TABLE_POPUP_LIFT_Z): object_popup_source_triple adds
    // glyph clearance above tile geometry. py nudges down toward the strip row.
    Some(crate::render::world_space::LayoutAnchorPx {
        px: base.px,
        py: strip_center[1] + tile_h * 0.05 + layout.mm(STRUCTURE_CALLOUT_DOWN_MM),
        lift_z: base.lift_z,
    })
}

/// Build the yaku progress panel (previews, structure showcase tiles) and yaku tablets.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_yaku_panel_and_tablets(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    ctx: &crate::scenes::DrawCtx<'_>,
    gameplay: &crate::game::engine::GameplayReadModel,
    _interaction: &crate::game::engine::GameplayInteractionReadModel,
    cascade_showcase_ref: Option<&CascadeShowcase>,
    cascade_frame: Option<&crate::game::cascade::CascadeFrame>,
    cascade_scored_yaku: Option<&[crate::core::yaku::YakuKind]>,
    has_structure: bool,
    layout_scale: f32,
    structure_marker_poses: [crate::render::gameplay_glb::GameplayMarkerPose; 2],
    yaku_marker_poses: [crate::render::gameplay_glb::GameplayMarkerPose; 2],
) -> YakuPanelOutputs {
    use super::cascade_hud::CascadeShowcase;
    use crate::core::yaku::{YakuKind, YakuTabletEntry};
    use crate::render::draw_cmd::{Object3d, Object3dKind, ShowcaseTilePlacement};

    let (
        yaku_preview_sets,
        yaku_preview_effective_tiles,
        _yaku_preview_original_tiles,
        tablet_yaku,
        is_chicken_hand,
    ) = if let Some(showcase) = cascade_showcase_ref {
        let mut scored = cascade_scored_yaku
            .map(|yaku| yaku.to_vec())
            .unwrap_or_default();
        YakuKind::sort_for_tablets(&mut scored);
        (
            showcase.sets.clone(),
            showcase.tiles.clone(),
            showcase.tiles.clone(),
            scored,
            false,
        )
    } else {
        let cache = &scene.yaku_preview_cache;
        (
            cache.sets.clone(),
            cache.effective_tiles.clone(),
            cache.original_tiles.clone(),
            cache.active_yaku.clone(),
            cache.is_chicken_hand,
        )
    };

    let mut structure_showcase: Vec<ShowcaseTilePlacement> = Vec::new();

    let wave_t = cascade_frame
        .as_ref()
        .map(|frame| frame.wave_t)
        .unwrap_or(0.0);
    let active_yaku_wave = cascade_frame
        .as_ref()
        .and_then(|frame| frame.active_yaku.as_deref());

    // Structure strip / scored-hand showcase: while idle it shows committed
    // structure (plus a valid pending selection when meld preview is on), and
    // while a cascade is active it keeps the just-scored tiles visible.
    let meld_preview = crate::persistence::load_settings().structure_meld_preview;
    let selection_on_structure_strip =
        meld_preview && scene.staging_layout.is_valid_meld && !yaku_preview_sets.is_empty();
    let committed_tile_ids: rustc_hash::FxHashSet<u32> =
        gameplay.structure_tiles.iter().map(|t| t.id).collect();
    let showcase_data = cascade_showcase_ref.cloned().or_else(|| {
        if selection_on_structure_strip {
            Some(CascadeShowcase {
                tiles: yaku_preview_effective_tiles.clone(),
                sets: yaku_preview_sets.clone(),
            })
        } else if has_structure {
            Some(CascadeShowcase {
                tiles: GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), run),
                sets: gameplay.structure_sets.clone(),
            })
        } else {
            None
        }
    });
    if let Some(showcase) = showcase_data {
        let a_l = structure_marker_poses[0].anchor;
        let a_r = structure_marker_poses[1].anchor;
        let rot_l = structure_marker_poses[0].rotation_rad;
        let rot_r = structure_marker_poses[1].rotation_rad;
        let structure_scale = structure_marker_poses[0]
            .uniform_author_scale(layout.window_h, ctx.room_gltf_height_scale);
        let span = crate::render::gameplay_glb::marker_pair_span_px(a_l, a_r);
        let strip = compute_structure_strip_layout(span, layout_scale, &showcase.sets);
        let mut cursor = 0.0f32;
        let active_tile_ids = cascade_frame
            .as_ref()
            .map(|frame| frame.highlight_tile_ids.as_slice())
            .unwrap_or(&[]);
        for (mi, set) in showcase.sets.iter().enumerate() {
            for (ti, &tid) in set.tile_ids.iter().enumerate() {
                let Some(tile) = showcase.tiles.iter().find(|t| t.id == tid).copied() else {
                    continue;
                };
                let t = if strip.layout_span > 0.0 {
                    (cursor + strip.tile_size * 0.5) / strip.layout_span
                } else {
                    0.5
                };
                let t = t.clamp(0.0, 1.0);
                let mut anchor = crate::render::gameplay_glb::lerp_marker_anchor(a_l, a_r, t);
                let mut lift_mm = ti as f32 * 1.2 + mi as f32 * 0.15;
                let pulse = active_tile_ids
                    .iter()
                    .position(|id| *id == tid)
                    .map(|pulse_idx| {
                        let delay = (pulse_idx as f32 * 0.18).min(0.7);
                        let wave = wave_t * (1.0 - delay * 0.35);
                        wave.abs().clamp(0.0, 1.0)
                    })
                    .unwrap_or(0.0);
                let scale = 1.0 + 0.14 * pulse;
                let is_pending = selection_on_structure_strip && !committed_tile_ids.contains(&tid);
                let brightness = if is_pending { 0.85 } else { 1.0 + 0.45 * pulse };
                lift_mm += SCORE_WAVE_STRUCTURE_TILE_MM * wave_t * pulse;
                anchor[2] += layout.mm(lift_mm);
                structure_showcase.push(ShowcaseTilePlacement {
                    tile,
                    center_pos: anchor,
                    rotation: crate::render::gameplay_glb::lerp_marker_rotation_rad(
                        rot_l, rot_r, t,
                    ),
                    scale,
                    size_px: strip.tile_size * structure_scale,
                    brightness,
                    opacity: 1.0,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: pulse > 0.05,
                    glow_color: if pulse > 0.05 {
                        Some(crate::render::theme::color::score_cascade::FU)
                    } else {
                        None
                    },
                    outline_sel: None,
                    pick_id: None,
                    overlay_rect_group: None,
                });
                cursor += strip.tile_size + strip.intra_gap;
            }
            if mi + 1 < showcase.sets.len() {
                cursor += strip.inter_gap - strip.intra_gap;
            }
        }
    }

    // Phase 9: in-play tablet row shows only firing yaku (see Yaku Journal for the full list).
    let a_l = yaku_marker_poses[0].anchor;
    let a_r = yaku_marker_poses[1].anchor;
    let rot_l = yaku_marker_poses[0].rotation_rad;
    let rot_r = yaku_marker_poses[1].rotation_rad;
    let yaku_scale =
        yaku_marker_poses[0].uniform_author_scale(layout.window_h, ctx.room_gltf_height_scale);
    let span = crate::render::gameplay_glb::marker_pair_span_px(a_l, a_r);
    let panel_h = ((a_r[1] - a_l[1]).abs()).max((24.0 * layout_scale).max(18.0));
    let mut yaku_tablet_placements: Vec<Object3d> = Vec::new();
    let tablet_entries: Vec<YakuTabletEntry> = if is_chicken_hand {
        vec![YakuTabletEntry {
            kind: YakuKind::ChickenHand,
            count: 1,
        }]
    } else {
        let mut kinds = tablet_yaku;
        YakuKind::sort_for_tablets(&mut kinds);
        YakuKind::consolidate_for_tablets(&kinds)
    };
    let show_tablets = !tablet_entries.is_empty();
    if show_tablets {
        let tablet_count = tablet_entries.len();
        let n = tablet_count as f32;
        let card_gap = 6.0 * layout_scale;
        let natural_card_w = (span - card_gap * 2.0) / 3.0;
        let card_w = ((span - card_gap * (n - 1.0)) / n).min(natural_card_w);
        let tablet_step_t = ((card_w + card_gap) / span).clamp(0.0, 1.0);
        let tablet_thickness = (8.0 * layout_scale).max(6.0) * yaku_scale;
        let tablet_depth = panel_h * yaku_scale;
        let mut push_tablet =
            |i: usize,
             label: std::borrow::Cow<'static, str>,
             active: bool,
             kind: Option<crate::core::yaku::YakuKind>| {
                let t = (i as f32 * tablet_step_t).clamp(0.0, 1.0);
                let mut pos = crate::render::gameplay_glb::lerp_marker_anchor(a_l, a_r, t);
                let rotation =
                    crate::render::gameplay_glb::lerp_marker_rotation_rad(rot_l, rot_r, t);
                let yaku_wave = active_yaku_wave.is_some_and(|name| {
                    kind.is_some_and(|yk| yk.name() == name) || label.contains(name)
                });
                if yaku_wave {
                    pos[2] += layout.mm(SCORE_WAVE_YAKU_MM * wave_t);
                }
                let hovered_now = matches!(
                    ctx.picked_gameplay_object,
                    Some(crate::render::wgpu_renderer::GameplayPick::YakuTablet(j)) if j == i
                );
                yaku_tablet_placements.push(Object3d {
                    pos,
                    extents: [card_w, tablet_thickness, tablet_depth],
                    rotation,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::YakuTablet {
                        label,
                        active: active || yaku_wave,
                        hover: if hovered_now { 1.0 } else { 0.0 },
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                });
            };
        for (i, entry) in tablet_entries.iter().enumerate() {
            let yaku_discovered = ctx
                .progress
                .yaku_times_scored
                .get(&entry.kind)
                .copied()
                .unwrap_or(0)
                >= 1;
            let tablet_label = entry
                .kind
                .gameplay_tablet_label_with_count(entry.count, yaku_discovered);
            push_tablet(i, tablet_label, true, Some(entry.kind));
        }
    }

    YakuPanelOutputs {
        yaku_preview_effective_tiles,
        yaku_preview_sets,
        yaku_tablet_placements,
        structure_showcase,
    }
}

/// Inset play-mirror click target — matches focus rects and hover tooltip.
fn play_button_hit_at_cursor(focus_rects: &[(FocusTarget, [f32; 4])], cx: f32, cy: f32) -> bool {
    focus_rects.iter().any(|(target, rect)| {
        matches!(target, FocusTarget::Button(GameplayButton::Play)) && rect_contains(*rect, cx, cy)
    })
}

/// Cursor hit-test for HUD focus targets, excluding full-height hand slot strips.
fn focus_non_hand_target_at_cursor(
    focus_rects: &[(FocusTarget, [f32; 4])],
    cx: f32,
    cy: f32,
) -> Option<FocusTarget> {
    let mut best: Option<(FocusTarget, f32)> = None;
    for &(target, rect) in focus_rects {
        if matches!(target, FocusTarget::HandTile(_)) {
            continue;
        }
        if !rect_contains(rect, cx, cy) {
            continue;
        }
        let area = rect[2] * rect[3];
        let is_better = match best {
            None => true,
            Some((_, ba)) => area < ba,
        };
        if is_better {
            best = Some((target, area));
        }
    }
    best.map(|(t, _)| t)
}

#[cfg(test)]
mod structure_strip_tests {
    use super::*;
    use crate::core::hand::{DetectedMeld, MeldKind};

    fn triplet(ids: &[u32]) -> DetectedMeld {
        DetectedMeld {
            kind: MeldKind::Triplet,
            tile_ids: ids.to_vec(),
        }
    }

    #[test]
    fn strip_counts_span_two_melds_not_one() {
        let sets = vec![triplet(&[0, 1, 2]), triplet(&[3, 4, 5])];
        let (total, intra, inter) = structure_strip_counts(&sets);
        assert_eq!(total, 6, "two triplets = six tiles across the strip");
        assert_eq!(intra, 4);
        assert_eq!(inter, 1);
    }

    #[test]
    fn strip_layout_shrinks_tile_size_as_melds_accumulate() {
        let span = 300.0;
        let layout_scale = 1.0;
        let one_meld = compute_structure_strip_layout(span, layout_scale, &[triplet(&[0, 1, 2])]);
        let three_melds = compute_structure_strip_layout(
            span,
            layout_scale,
            &[
                triplet(&[0, 1, 2]),
                triplet(&[3, 4, 5]),
                triplet(&[6, 7, 8]),
            ],
        );
        assert!(
            three_melds.tile_size < one_meld.tile_size,
            "nine tiles must be smaller than three: {} vs {}",
            three_melds.tile_size,
            one_meld.tile_size
        );
    }
}
