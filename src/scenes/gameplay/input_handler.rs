//! Input handler — owns the unified focus model + 3D hit dispatcher +
//! gameplay action handlers (ScoreHand, TriggerStructure, discard) of
//! [`super::GameplayScene::update`]. Behaviour is identical to the inline
//! code; this is purely organisational.

use std::time::Instant;

use super::GameplayScene;
use super::cascade_hud::CascadeShowcase;
use super::focus::{
    FocusTarget, GameplayButton, default_hand_tile_focus, focus_after_consumable_use, focus_kind,
    focus_kind_sfx, play_select_sfx, relic_to_dora_focus, wrap_hand_tile_focus,
};
use crate::core::relic::relic_visual;
use crate::core::scoring::StepKind;
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
use crate::ui::focus_nav::{FocusDir, focus_target_at_cursor, pick_neighbor};
use crate::ui::input::{UiAction, apply_ui_actions};

/// Vertical bounce (mm, before `layout.mm`) when a scoring step highlights this source.
const SCORE_WAVE_YAKU_MM: f32 = 4.0;
const SCORE_WAVE_STRUCTURE_TILE_MM: f32 = 5.0;
const RELIC_SCORE_VERTICAL_MM: f32 = 3.5;

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
            | FocusTarget::RoundWind => true,
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
        let new_focus = if let Some(idx) = ctx.picked_hand_tile {
            Some(FocusTarget::HandTile(idx))
        } else {
            focus_target_at_cursor(&focus_rects, cx, cy)
        };
        scene.focus = new_focus;
    }

    // Marquee multi-select drag: while LMB is held, the cursor's hovered
    // hand tile drives `current_slot` every frame. Same logic the focus
    // path runs for keyboard/gamepad — kept here so cursor sweeps don't
    // require a Focus action to fire.
    if let (Some(m), Some(FocusTarget::HandTile(idx))) = (scene.marquee.as_mut(), scene.focus)
        && let Some((added, removed)) = GameEngine::apply_marquee_selection(ctx.run, m, idx)
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
                let spatial = pick_neighbor(rect, dir, &focus_rects);
                // Navigation overrides for the action bar (vertical links + cash-in row).
                let overridden = match (scene.focus, dir) {
                    // RIGHT from Play → Cash in when banked structure can be scored
                    (Some(FocusTarget::Button(GameplayButton::Play)), FocusDir::Right)
                        if {
                            let g = GameEngine::read(ctx.run);
                            g.trigger_enabled || g.cash_in_blocked_until_discards_spent
                        } =>
                    {
                        focus_rects
                            .iter()
                            .find(|(t, _)| {
                                matches!(t, FocusTarget::Button(GameplayButton::Trigger))
                            })
                            .map(|(t, _)| *t)
                    }
                    // LEFT from Cash in → Play
                    (Some(FocusTarget::Button(GameplayButton::Trigger)), FocusDir::Left) => {
                        focus_rects
                            .iter()
                            .find(|(t, _)| matches!(t, FocusTarget::Button(GameplayButton::Play)))
                            .map(|(t, _)| *t)
                    }
                    // DOWN from Play → journal book
                    (Some(FocusTarget::Button(GameplayButton::Play)), FocusDir::Down) => {
                        focus_rects
                            .iter()
                            .find(|(t, _)| matches!(t, FocusTarget::Journal))
                            .map(|(t, _)| *t)
                    }
                    // UP from journal / guidebook → Play
                    (Some(FocusTarget::Journal), FocusDir::Up)
                    | (Some(FocusTarget::Guidebook), FocusDir::Up) => focus_rects
                        .iter()
                        .find(|(t, _)| matches!(t, FocusTarget::Button(GameplayButton::Play)))
                        .map(|(t, _)| *t),
                    // DOWN from Discard (river) → Undo when the accessibility control is shown
                    (Some(FocusTarget::Button(GameplayButton::Discard)), FocusDir::Down) => {
                        focus_rects
                            .iter()
                            .find(|(t, _)| matches!(t, FocusTarget::DiscardUndo))
                            .map(|(t, _)| *t)
                    }
                    // UP from Undo → back to Discard
                    (Some(FocusTarget::DiscardUndo), FocusDir::Up) => focus_rects
                        .iter()
                        .find(|(t, _)| matches!(t, FocusTarget::Button(GameplayButton::Discard)))
                        .map(|(t, _)| *t),
                    _ => None,
                };
                let hand_wrap = wrap_hand_tile_focus(scene.focus, dir, &focus_rects);
                let relic_dora = relic_to_dora_focus(scene.focus, dir, &focus_rects);
                if let Some(next) = overridden.or(hand_wrap).or(relic_dora).or(spatial) {
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
                        if gameplay.trigger_enabled
                            || gameplay.cash_in_blocked_until_discards_spent
                        {
                            if scene.cash_in_hold_started.is_none() {
                                scene.cash_in_hold_started = Some(
                                    crate::ui::prompt_hold_ring::begin_hold(
                                        now,
                                        ctx.bus,
                                        gameplay.trigger_enabled,
                                    ),
                                );
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
                    | Some(FocusTarget::Journal)
                    | Some(FocusTarget::Guidebook) => {}
                    None => {}
                }
                continue;
            }
            UiAction::ConfirmRelease => {
                scene.clear_cash_in_hold(ctx.bus);
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
                        scene.cash_in_hold_started = Some(
                            crate::ui::prompt_hold_ring::begin_hold(
                                now,
                                ctx.bus,
                                gameplay.trigger_enabled,
                            ),
                        );
                    }
                } else {
                    // Nothing to cash in: fire immediately so the rejection
                    // feedback (shake) still plays.
                    actions_for_scene.push(UiAction::TriggerStructure);
                }
                continue;
            }
            UiAction::TriggerStructureRelease => {
                scene.clear_cash_in_hold(ctx.bus);
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
        let action = match ctx.picked_gameplay_object {
            Some(GameplayPick::CashInButton) if cash_in_enabled => Some(UiAction::TriggerStructure),
            Some(GameplayPick::BronzeMirror) => Some(UiAction::ScoreHand),
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
                let bank_before = GameEngine::structure_banked_meld_chips(ctx.run);
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
                let structure_was_complete = gameplay.structure_complete;
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
                        let bank_after = GameEngine::structure_banked_meld_chips(ctx.run);
                        let d = bank_after.saturating_sub(bank_before);
                        if d > 0 {
                            let structure_is_complete =
                                GameEngine::read(ctx.run).structure_complete;
                            let sp = ctx.layout.score_panel;
                            let px = sp.x + sp.w * 0.5;
                            let py = sp.y + sp.h * 0.5 + 40.0;
                            let is_final_tiles = !structure_was_complete && structure_is_complete;
                            if is_final_tiles {
                                scene.final_tiles_fov_pop_at = Some(Instant::now());
                            }
                            scene.score_popups.spawn(
                                if is_final_tiles {
                                    "The final tiles!".to_string()
                                } else {
                                    "Structure grows".to_string()
                                },
                                crate::render::world_space::LayoutAnchorPx {
                                    px,
                                    py,
                                    lift_z: crate::render::score_popups::TABLE_POPUP_LIFT_Z,
                                },
                                (px, py),
                                None,
                                StepKind::Chips,
                                d as f32,
                                crate::render::score_popups::PopupMotionTiming::shipping_default(),
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
    let intra_gap = (3.0 * layout_scale).max(2.0);
    let inter_gap = (10.0 * layout_scale).max(7.0);
    let total_tiles: usize = showcase.sets.iter().map(|s| s.tile_ids.len()).sum();
    let intra_count: usize = showcase
        .sets
        .iter()
        .map(|s| s.tile_ids.len().saturating_sub(1))
        .sum();
    let inter_count = showcase.sets.len().saturating_sub(1);
    let layout_span = span.max(8.0);
    let available_span =
        layout_span - intra_count as f32 * intra_gap - inter_count as f32 * inter_gap;
    let tile_size =
        (available_span / total_tiles.max(1) as f32).clamp(22.0, (44.0 * layout_scale).max(28.0));
    let mut cursor = 0.0f32;
    let mut centers: Vec<[f32; 3]> = Vec::new();
    for (mi, set) in showcase.sets.iter().enumerate() {
        for (ti, &tid) in set.tile_ids.iter().enumerate() {
            let t = if layout_span > 0.0 {
                (cursor + tile_size * 0.5) / layout_span
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
            cursor += tile_size + intra_gap;
        }
        if mi + 1 < showcase.sets.len() {
            cursor += inter_gap - intra_gap;
        }
    }
    (!centers.is_empty()).then(|| median_layout_anchor(&centers))
}

/// Build the yaku progress panel (previews, structure showcase tiles) and yaku tablets.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_yaku_panel_and_tablets(
    _scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    ctx: &crate::scenes::DrawCtx<'_>,
    gameplay: &crate::game::engine::GameplayReadModel,
    interaction: &crate::game::engine::GameplayInteractionReadModel,
    cascade_showcase_ref: Option<&CascadeShowcase>,
    cascade_frame: Option<&crate::game::cascade::CascadeFrame>,
    has_structure: bool,
    layout_scale: f32,
    structure_marker_poses: [crate::render::gameplay_glb::GameplayMarkerPose; 2],
    yaku_marker_poses: [crate::render::gameplay_glb::GameplayMarkerPose; 2],
) -> YakuPanelOutputs {
    use super::cascade_hud::CascadeShowcase;
    use crate::core::yaku::yaku_preview;
    use crate::render::draw_cmd::{Object3d, Object3dKind, ShowcaseTilePlacement};

    // ── Yaku progress panel (above the bottom button bar) ────────────
    // Builds one card per available yaku showing how close the current
    // selection is to qualifying. Active yaku glow gold; partial progress
    // fills a horizontal bar across the card.
    let selected_tiles_for_yaku: Vec<_> = interaction
        .hand
        .iter()
        .zip(interaction.selected.iter())
        .filter(|&(_, &sel)| sel)
        .map(|(t, _)| *t)
        .collect();
    let round_wind_for_yaku = Some(gameplay.round_wind_rank);
    let bonus_round_wind_for_yaku = run.bonus_round_wind_for_yaku();
    let wildcard_result = if selected_tiles_for_yaku.is_empty() {
        None
    } else {
        GameEngine::validate_with_wildcards(run, &selected_tiles_for_yaku)
    };
    let mut yaku_preview_original_tiles: Vec<crate::core::tile::Tile> = Vec::new();
    let mut yaku_preview_effective_tiles: Vec<crate::core::tile::Tile> = Vec::new();
    let mut yaku_preview_sets: Vec<crate::core::hand::DetectedMeld> = Vec::new();

    // Cash-in clears live structure immediately; keep the scored melds for
    // yaku tablets until the scoring cascade finishes.
    let (base_structure_tiles, base_structure_sets) = if let Some(showcase) = cascade_showcase_ref {
        (showcase.tiles.clone(), showcase.sets.clone())
    } else {
        (
            GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), run),
            gameplay.structure_sets.clone(),
        )
    };

    if selected_tiles_for_yaku.is_empty() {
        yaku_preview_original_tiles = base_structure_tiles.clone();
        yaku_preview_effective_tiles = base_structure_tiles;
        yaku_preview_sets = base_structure_sets;
    } else if let Some((selected_sets, selected_scoring_tiles)) = wildcard_result.as_ref() {
        yaku_preview_original_tiles = base_structure_tiles.clone();
        yaku_preview_original_tiles.extend(GameplayScene::display_tiles(
            selected_tiles_for_yaku.iter().copied(),
            run,
        ));
        yaku_preview_effective_tiles = base_structure_tiles;
        yaku_preview_effective_tiles.extend(GameplayScene::display_tiles(
            selected_scoring_tiles.iter().copied(),
            run,
        ));
        yaku_preview_sets = base_structure_sets;
        yaku_preview_sets.extend(selected_sets.iter().cloned());
    }

    // Banked melds alone cover cash-in preview; when the row is still empty,
    // treat structure + rack as one hand so a complete chicken shape shows its
    // tablet before every tile is played to structure.
    if yaku_preview_sets.is_empty() && selected_tiles_for_yaku.is_empty() {
        let mut combined: Vec<crate::core::tile::Tile> =
            run.structure_tiles().iter().copied().collect();
        combined.extend(interaction.hand.iter().copied());
        if combined.len() == run.mode.hand_size
            && let Some((sets, scoring_tiles)) = GameEngine::validate_with_wildcards(run, &combined)
        {
            yaku_preview_original_tiles = GameplayScene::display_tiles(combined, run);
            yaku_preview_effective_tiles =
                GameplayScene::display_tiles(scoring_tiles, run);
            yaku_preview_sets = sets;
        }
    }

    let previews = if yaku_preview_sets.is_empty() {
        Vec::new()
    } else {
        yaku_preview(
            &yaku_preview_original_tiles,
            &gameplay.available_yaku,
            round_wind_for_yaku,
            bonus_round_wind_for_yaku,
            Some((
                yaku_preview_sets.as_slice(),
                yaku_preview_effective_tiles.as_slice(),
            )),
        )
    };

    let mut structure_showcase: Vec<ShowcaseTilePlacement> = Vec::new();

    let wave_t = cascade_frame
        .as_ref()
        .map(|frame| frame.wave_t)
        .unwrap_or(0.0);
    let active_yaku = cascade_frame
        .as_ref()
        .and_then(|frame| frame.active_yaku.as_deref());

    // Structure strip / scored-hand showcase: while idle it shows the
    // committed structure, and while a cascade is active it keeps the
    // just-scored tiles visible long enough to pulse them in sequence.
    let showcase_data = cascade_showcase_ref.cloned().or_else(|| {
        has_structure.then(|| CascadeShowcase {
            tiles: GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), run),
            sets: gameplay.structure_sets.clone(),
        })
    });
    if let Some(showcase) = showcase_data {
        let a_l = structure_marker_poses[0].anchor;
        let a_r = structure_marker_poses[1].anchor;
        let rot_l = structure_marker_poses[0].rotation_rad;
        let rot_r = structure_marker_poses[1].rotation_rad;
        let structure_scale = structure_marker_poses[0]
            .uniform_author_scale(layout.window_h, ctx.room_gltf_height_scale);
        let span = crate::render::gameplay_glb::marker_pair_span_px(a_l, a_r);
        let intra_gap = (3.0 * layout_scale).max(2.0);
        let inter_gap = (10.0 * layout_scale).max(7.0);
        let total_tiles: usize = showcase.sets.iter().map(|s| s.tile_ids.len()).sum();
        let intra_count: usize = showcase
            .sets
            .iter()
            .map(|s| s.tile_ids.len().saturating_sub(1))
            .sum();
        let inter_count = showcase.sets.len().saturating_sub(1);
        let layout_span = span.max(8.0);
        let available_span =
            layout_span - intra_count as f32 * intra_gap - inter_count as f32 * inter_gap;
        let n_t = total_tiles.max(1);
        let tile_size = (available_span / n_t as f32).clamp(22.0, (44.0 * layout_scale).max(28.0));
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
                let t = if layout_span > 0.0 {
                    (cursor + tile_size * 0.5) / layout_span
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
                let brightness = 1.0 + 0.45 * pulse;
                lift_mm += SCORE_WAVE_STRUCTURE_TILE_MM * wave_t * pulse;
                anchor[2] += layout.mm(lift_mm);
                structure_showcase.push(ShowcaseTilePlacement {
                    tile,
                    center_pos: anchor,
                    rotation: crate::render::gameplay_glb::lerp_marker_rotation_rad(
                        rot_l, rot_r, t,
                    ),
                    scale,
                    size_px: tile_size * structure_scale,
                    brightness,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: pulse > 0.05,
                    glow_color: if pulse > 0.05 {
                        Some(crate::render::theme::color::score_cascade::CHIPS)
                    } else {
                        None
                    },
                    pick_id: None,
                    overlay_rect_group: None,
                });
                cursor += tile_size + intra_gap;
            }
            if mi + 1 < showcase.sets.len() {
                cursor += inter_gap - intra_gap;
            }
        }
    }

    // Phase 9: with the Yaku Journal taking over the "browse all
    // yaku" job, the in-play tablet row collapses to *only firing
    // yaku*. Players who want to study levels, bonuses, or
    // construction hints open the Journal book on the table; the
    // play area is reserved for "what just fired this turn".
    let mut visible_previews: Vec<&crate::core::yaku::YakuPreview> =
        previews.iter().filter(|p| p.active).collect();
    visible_previews.sort_by_key(|p| p.kind);

    // If the selection is a valid hand but triggers no yaku, show a
    // chicken-hand tablet so the player knows the hand is legal.
    let is_chicken_hand = !yaku_preview_sets.is_empty()
        && crate::core::yaku::would_inject_chicken_hand(
            &yaku_preview_effective_tiles,
            &yaku_preview_sets,
            round_wind_for_yaku,
            bonus_round_wind_for_yaku,
            &gameplay.available_yaku,
        );

    // Phase 3: yaku selectors are now physical bone tablets sitting in
    // a row in front of the hand. The flat slate-blue card quads + the
    // progress-fill bar are gone — replaced by `YakuTabletBatch` that
    // the renderer dispatches through the lit-mesh pipeline. The 2D
    // text labels stay as a screen-space overlay until the engraved
    // decal pass lands; hover tracking still uses the original screen
    // rect (the cards live in the same pixel region as before).
    let a_l = yaku_marker_poses[0].anchor;
    let a_r = yaku_marker_poses[1].anchor;
    let rot_l = yaku_marker_poses[0].rotation_rad;
    let rot_r = yaku_marker_poses[1].rotation_rad;
    let yaku_scale =
        yaku_marker_poses[0].uniform_author_scale(layout.window_h, ctx.room_gltf_height_scale);
    let span = crate::render::gameplay_glb::marker_pair_span_px(a_l, a_r);
    let panel_h = ((a_r[1] - a_l[1]).abs()).max((24.0 * layout_scale).max(18.0));
    let mut yaku_tablet_placements: Vec<Object3d> = Vec::new();
    if !visible_previews.is_empty() || is_chicken_hand {
        let tablet_count = if is_chicken_hand {
            1
        } else {
            visible_previews.len()
        };
        let n = tablet_count as f32;
        let card_gap = 6.0 * layout_scale;
        let natural_card_w = (span - card_gap * 2.0) / 3.0;
        let card_w = ((span - card_gap * (n - 1.0)) / n).min(natural_card_w);
        let tablet_step_t = ((card_w + card_gap) / span).clamp(0.0, 1.0);
        let tablet_thickness = (8.0 * layout_scale).max(6.0) * yaku_scale;
        let tablet_depth = panel_h * yaku_scale;
        let mut push_tablet = |i: usize,
                               label: std::borrow::Cow<'static, str>,
                               active: bool,
                               kind: Option<crate::core::yaku::YakuKind>| {
            let t = (i as f32 * tablet_step_t).clamp(0.0, 1.0);
            let mut pos = crate::render::gameplay_glb::lerp_marker_anchor(a_l, a_r, t);
            let rotation = crate::render::gameplay_glb::lerp_marker_rotation_rad(rot_l, rot_r, t);
            let yaku_wave = active_yaku.is_some_and(|name| {
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
        if is_chicken_hand {
            push_tablet(
                0,
                std::borrow::Cow::Borrowed(crate::core::yaku::YakuKind::ChickenHand.gameplay_tablet_label(
                    true,
                )),
                true,
                Some(crate::core::yaku::YakuKind::ChickenHand),
            );
        } else {
            for (i, p) in visible_previews.iter().enumerate() {
                let yaku_discovered = ctx
                    .progress
                    .yaku_times_scored
                    .get(&p.kind)
                    .copied()
                    .unwrap_or(0)
                    >= 1;
                let tablet_label = std::borrow::Cow::Borrowed(
                    p.kind.gameplay_tablet_label(yaku_discovered),
                );
                push_tablet(i, tablet_label, p.active, Some(p.kind));
            }
        }
    }

    YakuPanelOutputs {
        yaku_preview_effective_tiles,
        yaku_preview_sets,
        yaku_tablet_placements,
        structure_showcase,
    }
}
