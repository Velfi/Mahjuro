//! Input handler — owns the unified focus model + 3D hit dispatcher +
//! gameplay action handlers (ScoreHand, TriggerStructure, sort, discard) of
//! [`super::GameplayScene::update`]. Behaviour is identical to the inline
//! code; this is purely organisational.

use std::time::Instant;

use super::GameplayScene;
use super::cascade_hud::CascadeShowcase;
use super::focus::{
    FocusTarget, GameplayButton, focus_after_consumable_use, focus_kind, focus_kind_sfx,
    play_select_sfx, wrap_hand_tile_focus,
};
use crate::core::relic::relic_visual;
use crate::core::scoring::StepKind;
use crate::game::engine::{CommandData, GameCommand, GameEngine};
use crate::game::run::DiscardUndoSnapshot;
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::render::draw_cmd::{CameraParams, Object3d, Object3dKind};
use crate::render::table_transform::euler_xyz_rad_from_deg;
use crate::scenes::gameplay::RELIC_GLOW_LIFETIME;
use crate::scenes::journal_transition::{
    BOOK_SPINE_THICKNESS_MM, JournalDirection, JournalTransition, YAKU_JOURNAL_BOOK_PICK_ID,
    book_cover_face_extents_xy,
};
use crate::scenes::{OverlayRequest, Scene, SceneTransition, UpdateCtx, YakuJournalScene};
use crate::ui::focus_nav::{FocusDir, focus_target_at_cursor, pick_neighbor};
use crate::ui::input::{UiAction, apply_ui_actions};
use crate::ui::scene_layout::GameplayPositions;

/// Screen-space center of the `idx`-th active relic in the horizontal tray.
/// Must stay in sync with [`build_relic_tray_and_wind`] (stride / clamp math).
pub(super) fn relic_tray_screen_center_xy(
    positions: &GameplayPositions,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    idx: usize,
) -> Option<(f32, f32)> {
    let active_ids = GameEngine::active_relics(run);
    if idx >= active_ids.len() {
        return None;
    }
    let gp = positions;
    let w = layout.window_w;
    let h = layout.window_h;
    let tray_cx = gp.relic_col.nx * w;
    let tray_cy = gp.relic_col.ny * h;
    let tray_left = gp.relic_col_top_ny * w;
    let tray_right = gp.relic_col_bottom_ny * w;
    let face_base = layout.mm(gp.relic_cell_height_mm);
    let n = active_ids.len() as f32;
    let avail_w = (tray_right - tray_left).max(face_base);
    let stride_ideal = face_base * 1.1;
    let stride = if n * stride_ideal > avail_w {
        (avail_w / n).max(face_base * 0.5)
    } else {
        stride_ideal
    };
    let total_w = stride * n;
    let start_x = (tray_cx - total_w * 0.5 + stride * 0.5)
        .clamp(tray_left + stride * 0.5, tray_right - stride * 0.5);
    let x = start_x + idx as f32 * stride;
    Some((x, tray_cy))
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
            | FocusTarget::Boss
            | FocusTarget::RoundWind => true,
            FocusTarget::DiscardUndo => {
                crate::persistence::load_settings().discard_undo_enabled
                    && scene.discard_undo.is_some()
                    && scene.pending_refill.is_none()
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
        let has_structure = gameplay.has_structure;
        if !has_structure {
            scene.focus = None;
        }
    }
    if matches!(scene.focus, Some(FocusTarget::DiscardUndo)) {
        let ok = crate::persistence::load_settings().discard_undo_enabled
            && scene.discard_undo.is_some()
            && scene.pending_refill.is_none();
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
        && scene.pending_refill.is_none();
    let mut hud_cycle: Vec<FocusTarget> = consumable_targets.clone();
    if undo_hud_eligible {
        hud_cycle.push(FocusTarget::DiscardUndo);
    }

    // Process actions. Directional input → spatial picker. Confirm →
    // route by self.focus variant. Cancel → clear focus AND fall
    // through so existing `clear_selection` semantics still apply.
    // Everything else flows into `actions_for_scene` for the existing
    // gameplay action handlers below (ScoreHand, SortBySuit, etc.).
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
                // Navigation overrides for the two-row action bar:
                let overridden = match (scene.focus, dir) {
                    // LEFT from Play (commit melds) → Discard
                    (Some(FocusTarget::Button(GameplayButton::Play)), FocusDir::Left) => {
                        focus_rects
                            .iter()
                            .find(|(t, _)| {
                                matches!(t, FocusTarget::Button(GameplayButton::Discard))
                            })
                            .map(|(t, _)| *t)
                    }
                    // RIGHT from Discard → Play (commit melds)
                    (Some(FocusTarget::Button(GameplayButton::Discard)), FocusDir::Right) => {
                        focus_rects
                            .iter()
                            .find(|(t, _)| matches!(t, FocusTarget::Button(GameplayButton::Play)))
                            .map(|(t, _)| *t)
                    }
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
                if let Some(next) = overridden.or(hand_wrap).or(spatial) {
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
                    Some(FocusTarget::Button(GameplayButton::Journal)) => {
                        if scene.journal_transition.is_none() {
                            scene.journal_transition = Some(JournalTransition {
                                start: now,
                                dir: JournalDirection::Opening,
                            });
                        }
                        return Some(None);
                    }
                    Some(FocusTarget::DiscardUndo) => {
                        actions_for_scene.push(UiAction::UndoDiscard);
                        continue;
                    }
                    Some(FocusTarget::Button(b)) => {
                        if let Some(a) = b.ui_action() {
                            actions_for_scene.push(a);
                        }
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
                                    src,
                                    src,
                                    None,
                                    crate::core::scoring::StepKind::Gold,
                                    new_level as f32,
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
                        let remaining = GameEngine::read_interaction(ctx.run).consumable_count;
                        scene.focus = focus_after_consumable_use(i, remaining, &focus_rects);
                    }
                    Some(FocusTarget::Peg(_))
                    | Some(FocusTarget::Gold)
                    | Some(FocusTarget::YakuTablet(_))
                    | Some(FocusTarget::Dora)
                    | Some(FocusTarget::Boss)
                    | Some(FocusTarget::RoundWind) => {}
                    None => {}
                }
                continue;
            }
            UiAction::ConfirmRelease => {
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
                scene.focus = focus_rects
                    .iter()
                    .find_map(|(t, _)| matches!(t, FocusTarget::HandTile(_)).then_some(*t));
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
                } else {
                    ctx.bus.push(crate::game::event_bus::GameEvent::UiSound(
                        crate::audio::SfxId::InvalidAction,
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
                && scene.pending_refill.is_none()
            {
                actions_for_scene.push(UiAction::UndoDiscard);
            }
            continue;
        }
        if cid != super::GAMEPLAY_3D_HIT_ID {
            continue;
        }
        use crate::render::wgpu_renderer::GameplayPick;
        let gameplay = GameEngine::read(ctx.run);
        let has_structure = gameplay.has_structure;
        if matches!(ctx.picked_gameplay_object, Some(GameplayPick::JournalBook))
            && scene.journal_transition.is_none()
        {
            scene.journal_transition = Some(JournalTransition {
                start: now,
                dir: JournalDirection::Opening,
            });
            return Some(None);
        }
        let action = match ctx.picked_gameplay_object {
            Some(GameplayPick::WoodTablet(0)) if has_structure => Some(UiAction::TriggerStructure),
            Some(GameplayPick::BronzeMirror) => Some(UiAction::ScoreHand),
            Some(GameplayPick::DiscardBowl) => Some(UiAction::CommitDiscard),
            _ => None,
        };
        if let Some(a) = action {
            actions_for_scene.push(a);
        }
    }

    // Debug: `B` blows a strong gust of wind at the candle row so the
    // flame's wind reaction is observable on demand. Stamps a timer the
    // draw step reads to emit the actual `WindGust` impulses.
    if actions_for_scene
        .iter()
        .any(|a| matches!(a, UiAction::DebugBlowWind))
    {
        scene.debug_wind_at = Some(now);
        log::info!("[debug] candle wind gust triggered");
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
                    ctx.anim
                        .shake(crate::render::animation::ENTITY_HAND_STRIP, 8.0, 200);
                } else {
                    scene.invalid_meld_flash_at = None;
                    scene.invalid_meld_flash_slots.clear();
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
                                (px, py),
                                (px, py),
                                None,
                                StepKind::Chips,
                                d as f32,
                            );
                        }
                    }
                }
            }
            UiAction::TriggerStructure => {
                let score_before = GameEngine::read(ctx.run).round_score;
                let gameplay = GameEngine::read(ctx.run);
                let cascade_showcase = Some(CascadeShowcase {
                    tiles: GameplayScene::display_tiles(
                        gameplay.structure_tiles.iter().copied(),
                        ctx.run,
                    ),
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
            UiAction::SortBySuit => {
                scene.clear_discard_undo();
                let mut engine = GameEngine::new(ctx.run, ctx.bus);
                let _ = engine.dispatch(GameCommand::SortHandBySuit);
                ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
            }
            UiAction::SortByRank => {
                scene.clear_discard_undo();
                let mut engine = GameEngine::new(ctx.run, ctx.bus);
                let _ = engine.dispatch(GameCommand::SortHandByRank);
                ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
            }
            UiAction::CommitDiscard => {
                if !ctx.run.onboarding_discard_allowed() {
                    ctx.bus.push(crate::game::event_bus::GameEvent::UiSound(
                        crate::audio::SfxId::InvalidAction,
                    ));
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
                let mut pre_discard_hand_slots: Vec<(f32, f32, f32, f32)> = Vec::new();
                if gameplay.selected_count > 0 && gameplay.discards_remaining > 0 {
                    let interaction = GameEngine::read_interaction(ctx.run);
                    pre_discard_hand_slots =
                        super::hand_layout::hand_slots_for_count(ctx.layout, interaction.hand_len);
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
                        &pre_discard_hand_slots,
                        gameplay.has_structure,
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
                        .map(|b| b.total_duration(&ctx.cascade_tuning))
                        .unwrap_or(fallback);
                    scene.pending_refill = Some(now + anim_dur.max(fallback));
                }
            }
            UiAction::UndoDiscard => {
                if crate::persistence::load_settings().discard_undo_enabled
                    && scene.pending_refill.is_none()
                    && let Some(snap) = scene.discard_undo.take()
                {
                    scene.active_discard_anim = None;
                    scene.river_settled_tiles.clear();
                    scene.river_sink_batch = None;
                    ctx.run.apply_discard_undo(snap, Some(ctx.bus));
                    ctx.bus.push(crate::game::event_bus::GameEvent::UiSound(
                        crate::audio::SfxId::TilePlace,
                    ));
                    ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                }
            }
            _ => {}
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
                    | UiAction::SortBySuit
                    | UiAction::SortByRank
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

/// Build the relic tray
pub(super) fn build_relic_tray(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
) -> Vec<Object3d> {
    // ── Relic tray (horizontal row across the top of the screen) ────
    // Each active relic renders as a face-on enamel medallion using the
    // showcase path — same mesh/material as the collection screen.
    // The row is centered on `relic_col.nx, relic_col.ny` and clamped
    // within `[relic_col_top_ny, relic_col_bottom_ny]` horizontally
    // (re-used as left/right edge clamps for the tray).
    let mut relic_objects: Vec<Object3d> = Vec::new();
    let active_ids = GameEngine::active_relics(run);
    if !active_ids.is_empty() {
        use crate::core::relic::all_relic_defs;
        let defs = all_relic_defs();
        let gp = &scene.positions;
        let tray_cx = gp.relic_col.nx * layout.window_w;
        let tray_cy = gp.relic_col.ny * layout.window_h;
        let tray_left = gp.relic_col_top_ny * layout.window_w;
        let tray_right = gp.relic_col_bottom_ny * layout.window_w;
        // Badge face size and stride. If the row would overflow the
        // clamp, tighten the stride so everything fits.
        let face_base = layout.mm(gp.relic_cell_height_mm);
        let n = active_ids.len() as f32;
        let avail_w = (tray_right - tray_left).max(face_base);
        let stride_ideal = face_base * 1.1;
        let stride = if n * stride_ideal > avail_w {
            (avail_w / n).max(face_base * 0.5)
        } else {
            stride_ideal
        };
        let face = stride / 1.1;
        let total_w = stride * n;
        let start_x = (tray_cx - total_w * 0.5 + stride * 0.5)
            .clamp(tray_left + stride * 0.5, tray_right - stride * 0.5);

        // Camera-facing pitch. The relic mesh's face-normal is local +Y.
        // After Rz=0, Ry=wiggle, Rx=α: face normal = Rx(α)·(0,1,0) =
        // (0, cos α, sin α). We want that to equal -look/|look| so the
        // face points at the eye. The gameplay camera looks forward-and-
        // down (look_y > 0, look_z < 0), so cos α < 0 and sin α > 0 —
        // α lands in the second quadrant (~100-120°).
        let cam = CameraParams::default_table_camera(layout.window_h);
        let look = [
            cam.target[0] - cam.eye[0],
            cam.target[1] - cam.eye[1],
            cam.target[2] - cam.eye[2],
        ];
        let face_pitch_deg = {
            let ly = look[1];
            let lz = look[2];
            let len = (ly * ly + lz * lz).sqrt().max(1e-6);
            let cos_a = -ly / len;
            let sin_a = -lz / len;
            sin_a.atan2(cos_a).to_degrees()
        };

        // Mesh geometry (see `build_relic_mesh`): the disc lies in local
        // XZ with radius 0.5, and thickness runs along local ±Y with
        // half-height 0.5. Extents are full-width scalars on each local
        // axis, so `[face, thick, face]` scales the disc to face-width
        // and the cylinder height to the (much smaller) badge thickness.
        for (i, &rid) in active_ids.iter().enumerate() {
            let visual = relic_visual(rid);
            let thick = face * 0.06 * visual.thickness_scale;

            // Color tracks the relic's rarity tier.
            let rarity = defs
                .iter()
                .find(|d| d.id == rid)
                .map(|d| d.rarity)
                .unwrap_or(crate::core::relic::Rarity::Common);
            let color = crate::render::theme::color::rarity(rarity.tier());

            // Activation glow: fast-attack / smooth-decay envelope.
            let (glow, wiggle_deg) = if let Some(start) = scene.relic_glow_starts.get(&rid) {
                let now_for_glow = Instant::now();
                let age = now_for_glow.saturating_duration_since(*start).as_secs_f32();
                let life = RELIC_GLOW_LIFETIME.as_secs_f32();
                if age >= life {
                    (0.0, 0.0)
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
                    (glow, wiggle)
                }
            } else {
                (0.0, 0.0)
            };

            let px = start_x + i as f32 * stride;
            relic_objects.push(Object3d {
                pos: [px, tray_cy, face * 0.45],
                extents: [face, thick, face],
                rotation: euler_xyz_rad_from_deg(face_pitch_deg, wiggle_deg, 0.0),
                color,
                kind: Object3dKind::Relic {
                    relic_id: rid,
                    glow,
                    silhouette: false,
                    debuffed: run.relics.is_debuffed(rid),
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
            });
        }
    }

    relic_objects
}

/// Outputs of [`build_consumable_dish`].
pub(super) struct ConsumableDishBuild {
    pub talisman_dish_placements: Vec<crate::render::draw_cmd::Object3d>,
    pub ribbon_dish_placements: Vec<crate::render::draw_cmd::Object3d>,
    pub talisman_dish_strip: Option<(f32, f32, f32, f32)>,
}

/// Build the consumable inventory dish (Zodiacs + Talismans) — brass dish,
/// pendant placements, focus rects, click buttons.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_consumable_dish(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    ctx: &crate::scenes::DrawCtx<'_>,
    interaction: &crate::game::engine::GameplayInteractionReadModel,
    paused: bool,
    focus_rect_graph: &mut Vec<(FocusTarget, [f32; 4])>,
    buttons: &mut Vec<crate::scenes::ButtonDef>,
) -> ConsumableDishBuild {
    use super::PICK_CONSUMABLE_DISH;
    use super::ZODIAC_USE_BASE;
    use crate::render::draw_cmd::{Object3d, Object3dKind};

    // ── Consumable inventory bar (Zodiacs + Talismans) ───────────────
    //
    // Sits in the top-right corner of the screen, away from the score
    // cartouche and relic dish. Each slot is a clickable badge for one
    // consumable — Zodiacs level a yaku for the run, Talismans stamp
    // their enhancement onto every tile in the current hand at once.
    //
    // Phase 5: the flat slot backgrounds + gold rims are gone; the
    // consumable inventory now lives on a porcelain dish with
    // `TalismanBatch` pendants for each filled slot. The text labels
    // and click handlers stay at the same screen positions so hover +
    // input plumbing is unchanged.
    let consumables = &interaction.consumables;
    let consumable_capacity = interaction.consumable_capacity;
    let mut talisman_dish_placements: Vec<Object3d> = Vec::new();
    let mut ribbon_dish_placements: Vec<Object3d> = Vec::new();
    let mut talisman_dish_strip: Option<(f32, f32, f32, f32)> = None;
    if consumable_capacity > 0 {
        // The porcelain dish is a 3D object — use base resolution scale
        // so its placement stays stable regardless of UI scale.
        let zscale = (layout.window_w.min(layout.window_h)) / 600.0;
        let slot_w = (140.0 * zscale).max(120.0);
        let slot_h = (56.0 * zscale).max(48.0);
        let gap = (6.0 * zscale).max(3.0);
        let total_w = (slot_w * consumable_capacity as f32
            + gap * (consumable_capacity as f32 - 1.0))
            .min(layout.window_w * 0.65);
        // Mockup: TALISMANS right:2% top:18% width:16% height:16%.
        // Anchor the strip to right:2%, clamped so it doesn't go off-screen.
        let strip_x = (layout.window_w * 0.98 - total_w).max(4.0);
        let strip_y = layout.window_h * 0.18;
        talisman_dish_strip = Some((strip_x, strip_y, total_w, slot_h));

        // ── Projection-aware slot rects ──────────────────────────────
        // The porcelain dish gets projected through the gameplay camera
        // to a different on-screen position than its raw pixel anchor.
        // We look up the previous frame's projected dish rect and
        // remap each per-slot rect through the same affine transform
        // (treating the dish as roughly planar). This keeps the focus
        // ring and click target in lockstep with the visible pendant.
        //
        // Must match the dish-padding values used at the
        // `frame.dish_explicit(...)` push site below.
        let dish_pad_x_ratio = 0.10_f32;
        let dish_pad_y_ratio = 0.40_f32;
        let dish_pad_x = total_w * dish_pad_x_ratio;
        let dish_pad_y = slot_h * dish_pad_y_ratio;
        let orig_dish_x = strip_x - dish_pad_x;
        let orig_dish_y = strip_y - dish_pad_y;
        let orig_dish_w = total_w + dish_pad_x * 2.0;
        let orig_dish_h = slot_h + dish_pad_y * 2.0;
        let projected_dish = ctx
            .proj
            .aux_dish_rects
            .iter()
            .find_map(|(pid, r)| (*pid == Some(PICK_CONSUMABLE_DISH)).then_some(*r));
        let slot_screen_rect = |slot_idx: usize| -> (f32, f32, f32, f32) {
            let raw_x = strip_x + slot_idx as f32 * (slot_w + gap);
            let raw_y = strip_y;
            if let Some([pdx, pdy, pdw, pdh]) = projected_dish
                && pdw > 0.0
                && pdh > 0.0
            {
                let tx0 = (raw_x - orig_dish_x) / orig_dish_w;
                let tx1 = (raw_x + slot_w - orig_dish_x) / orig_dish_w;
                let ty0 = (raw_y - orig_dish_y) / orig_dish_h;
                let ty1 = (raw_y + slot_h - orig_dish_y) / orig_dish_h;
                let psx0 = pdx + pdw * tx0;
                let psx1 = pdx + pdw * tx1;
                let psy0 = pdy + pdh * ty0;
                let psy1 = pdy + pdh * ty1;
                return (psx0, psy0, psx1 - psx0, psy1 - psy0);
            }
            // First-frame fallback (no projection data yet) — use the
            // raw pixel rect. One frame of misalignment, then the
            // projected path takes over.
            (raw_x, raw_y, slot_w, slot_h)
        };

        // Pendants must track the dish's arrange-mode nudge, otherwise
        // they render at the raw strip anchor while the dish drifts off
        // to its tweaked position — talismans float next to the tray
        // instead of sitting on it.
        let td = &scene.positions.talisman_dish;
        let dish_nudge_x = td.nx * layout.window_w;
        let dish_nudge_y = td.ny * layout.window_h;
        // Pendants are drawn in slot order into two separate batches
        // (ribbons, then talismans), so the nth talisman placement maps
        // to `proj.talisman_rects[n]` and the nth ribbon to
        // `proj.ribbon_rects[n]`. The renderer's projected rects already
        // fold in the pendant's full rotation (base 90° Z + the
        // consumable_dish_talisman placement tilt), so using them gives
        // a focus rect that tracks the rotated silhouette instead of an
        // axis-aligned stand-in.
        let mut talisman_draw_i: usize = 0;
        let mut ribbon_draw_i: usize = 0;
        for slot_idx in 0..consumable_capacity {
            // Pendant placement still uses the raw pixel anchors
            // (those get re-projected by the renderer for rendering).
            // The 2D overlays use the projected slot rect derived
            // from the dish.
            let zx = strip_x + slot_idx as f32 * (slot_w + gap) + dish_nudge_x;
            let zy = strip_y + dish_nudge_y;
            let (slot_sx, slot_sy, slot_sw, slot_sh) = slot_screen_rect(slot_idx);
            // Shrink the focus rect to match the visual item extents
            // so the hover/click region hugs the pendant, not the
            // full inventory slot rectangle.
            let Some(&slot_item) = consumables.get(slot_idx) else {
                // Empty slot — not selectable. Skip the focus rect so
                // cursor hover, spatial nav, and shoulder-button
                // cycling all pass over it.
                continue;
            };
            let (fx, fy, fw, fh) = {
                match slot_item {
                    crate::core::consumable::Consumable::Talisman(_) => {
                        let proj_rect = ctx
                            .proj
                            .talisman_rects
                            .get(talisman_draw_i)
                            .copied()
                            .filter(|r| {
                                r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite()
                            });
                        talisman_draw_i += 1;
                        if let Some(r) = proj_rect {
                            (r[0], r[1], r[2], r[3])
                        } else {
                            // First-frame fallback (no projection data yet).
                            // Talisman visual is 55% × 85% of the slot.
                            // Bias upward — the pendant sits on top of the
                            // dish, in the upper portion of the slot rect.
                            let fw = slot_sw * 0.55;
                            let fh = slot_sh * 0.85;
                            (
                                slot_sx + (slot_sw - fw) * 0.5,
                                slot_sy + (slot_sh - fh) * 0.20,
                                fw,
                                fh,
                            )
                        }
                    }
                    crate::core::consumable::Consumable::Zodiac(_) => {
                        let proj_rect =
                            ctx.proj
                                .ribbon_rects
                                .get(ribbon_draw_i)
                                .copied()
                                .filter(|r| {
                                    r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite()
                                });
                        ribbon_draw_i += 1;
                        if let Some(r) = proj_rect {
                            (r[0], r[1], r[2], r[3])
                        } else {
                            // First-frame fallback — ribbon is narrow,
                            // use ~30% width, 85% height.
                            let fw = slot_sw * 0.30;
                            let fh = slot_sh * 0.85;
                            (
                                slot_sx + (slot_sw - fw) * 0.5,
                                slot_sy + (slot_sh - fh) * 0.20,
                                fw,
                                fh,
                            )
                        }
                    }
                }
            };
            focus_rect_graph.push((FocusTarget::Consumable(slot_idx), [fx, fy, fw, fh]));
            {
                let item = slot_item;
                // Physical pendant on the dish — color encodes the
                // consumable type. Zodiacs read jade-green, talismans
                // pick up the talisman's enhancement family color.
                let pendant_color = match item {
                    crate::core::consumable::Consumable::Zodiac(_) => [0.45, 0.78, 0.55, 1.0],
                    crate::core::consumable::Consumable::Talisman(tk) => tk.accent_color(),
                };
                // Rest the pendant on the dish's rim. The dish is
                // centered at `mm(td.lift_mm)` with full rim extent
                // `mm(10.0)` (rotated so the rim height becomes world
                // Z), so the top surface sits at `mm(td.lift_mm + 5.0)`.
                // Add a small extra lift so the pendant reads as
                // resting *on* the rim rather than clipping into it.
                let pendant_y = layout.mm(td.lift_mm + 5.0) + 2.0;
                match item {
                    crate::core::consumable::Consumable::Zodiac(z) => {
                        // Length is the natural dimension here — the ribbon
                        // fills most of the slot height — and width / depth
                        // come from the canonical 3:1 aspect via the helper.
                        // `pos` is the mesh centroid; lift sits half a length
                        // below the finial that rests on the dish rim.
                        let ribbon_length = slot_h * 0.85;
                        let ribbon_world =
                            crate::render::ribbon_mesh::ribbon_display_length(ribbon_length);
                        ribbon_dish_placements.push(
                            crate::render::ribbon_mesh::zodiac_ribbon_object3d(
                                crate::render::ribbon_mesh::ZodiacRibbonSpec {
                                    pos: [zx + slot_w * 0.5, zy, pendant_y - ribbon_world * 0.5],
                                    length: ribbon_length,
                                    rotation:
                                        crate::render::table_transform::euler_xyz_rad_from_deg(
                                            -90.0, 0.0, 0.0,
                                        ),
                                    color: [1.0, 1.0, 1.0, 1.0],
                                    kind: Some(z),
                                    hover_target: 0.0,
                                    anim_id: 0,
                                    placement_rot_deg: [0.0, 0.0, 0.0],
                                },
                            ),
                        );
                    }
                    crate::core::consumable::Consumable::Talisman(tk) => {
                        let talisman_half_height = slot_w * 0.55;
                        let anchor = crate::ui::placement::PlacementAnchor::new(
                            [
                                zx + slot_w * 0.5,
                                zy + slot_h * 0.5,
                                pendant_y + talisman_half_height,
                            ],
                            crate::render::table_transform::rot_fixed_axes_deg(0.0, 0.0, 90.0),
                            &scene.positions.consumable_dish_talisman,
                            layout,
                        );
                        let tscale = slot_h * 1.56;
                        talisman_dish_placements.push(Object3d {
                            pos: anchor.pos,
                            extents: [tscale, tscale * 1.28, tscale * 0.22],
                            rotation: anchor.object3d_rotation(),
                            color: pendant_color,
                            kind: Object3dKind::Talisman { kind: tk },
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
        }
    }

    ConsumableDishBuild {
        talisman_dish_placements,
        ribbon_dish_placements,
        talisman_dish_strip,
    }
}

/// Outputs of the action row + journal book builder.
pub(super) struct ActionRowOutputs {
    pub(super) wood_tablet_placements: Vec<crate::render::draw_cmd::Object3d>,
    pub(super) discard_bowl_placement: Option<crate::render::draw_cmd::Object3d>,
    pub(super) bronze_mirror_placement: Option<crate::render::draw_cmd::Object3d>,
    pub(super) journal_book: Option<crate::render::draw_cmd::Object3d>,
}

/// Build the action-row 3D objects (discard bowl, bronze mirror, optional
/// cash-in tablet) plus the Yaku Journal book.
/// Pushes button focus rects into `focus_rect_graph`.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_action_row_and_journal(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    ctx: &crate::scenes::DrawCtx<'_>,
    gameplay: &crate::game::engine::GameplayReadModel,
    btn_rects: &[(f32, f32, f32, f32); 3],
    journal_btn_rect: (f32, f32, f32, f32),
    journal_btn_cx: f32,
    action_world_z_py: f32,
    action_hud_table_lift: f32,
    cam_euler: [f32; 3],
    has_structure: bool,
    play_enabled: bool,
    discard_enabled: bool,
    now: Instant,
    focus_rect_graph: &mut Vec<(FocusTarget, [f32; 4])>,
) -> ActionRowOutputs {
    use super::focus::ALL_BUTTONS;
    use crate::render::draw_cmd::{Object3d, Object3dKind};
    use crate::render::table_transform::rot_euler_xyz_rad;
    use crate::render::world_space::LayoutAnchorPx;
    // Phase 4: action row is now physical objects.
    //   - Discard / Play → bowl + mirror (row below hand, above journal;
    //                      mirror play left, discard bowl right)
    // The flat slate-blue button background quads are gone; only the
    // focus-highlight border remains as a 2D affordance for keyboard
    // navigation.
    let mut wood_tablet_placements: Vec<Object3d> = Vec::new();
    let mut discard_bowl_placement: Option<Object3d> = None;
    let mut bronze_mirror_placement: Option<Object3d> = None;
    // `action_hud_table_lift`: third component of [`crate::render::draw_cmd::WorldSurfaceAnchor`]
    // (height above felt); set in [`action_bar_layout::compute_action_bar`].
    for (i, &(bx, by, bw, bh)) in btn_rects.iter().enumerate() {
        if i == 2 && !has_structure {
            continue;
        }
        // Register this button in the focus rect graph so the unified
        // focus model can navigate to it spatially.
        // Anchor the focus hit-test on the renderer's projected mesh
        // rect for this button — the layout pixel rect doesn't
        // account for camera tilt + perspective so it falls out of
        // sync with where the player actually sees the object. On
        // the very first frame the projected rect may be missing
        // (renderer hasn't drawn yet); skip the entry then, the
        // focus picker tolerates absent targets and the next frame
        // repopulates.
        let proj = match ALL_BUTTONS[i] {
            GameplayButton::Discard => ctx.proj.bowl_rect,
            GameplayButton::Play => ctx.proj.mirror_rect,
            GameplayButton::Trigger => ctx.proj.wood_tablet_rects.first().copied(),
            // Pushed separately alongside the journal placement
            // block further down — its slot index in the wood
            // tablet rect vec isn't known until then.
            GameplayButton::Journal => None,
        };
        if let Some(rect) = proj {
            focus_rect_graph.push((FocusTarget::Button(ALL_BUTTONS[i]), rect));
        }
        // The hover state for the 3D action objects comes from the
        // renderer's raycast picker against precomputed local AABBs —
        // no projected screen rects, no per-frame intersection of
        // input pixel rects with camera-distorted AABBs. The picker
        // is one frame stale, like every other 3D pick path.
        let pick = ctx.picked_gameplay_object;
        // The bowl + mirror animate their tilt envelope from the
        // *hover* flag below, so we want it to also light up when the
        // unified focus model lands on them via keyboard nav — not
        // just on cursor hover. Hand tablets keep their existing
        // pick-only behavior since they have no tilt animation.
        let focused_btn = match scene.focus {
            Some(FocusTarget::Button(b)) => Some(b),
            _ => None,
        };
        let hovered = match i {
            0 => {
                matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::DiscardBowl),
                ) || focused_btn == Some(GameplayButton::Discard)
            }
            1 => {
                matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::BronzeMirror),
                ) || focused_btn == Some(GameplayButton::Play)
            }
            2 => {
                matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(0)),
                ) || focused_btn == Some(GameplayButton::Trigger)
            }
            _ => false,
        };
        // The per-button focus highlight is gone — the unified focus
        // model draws a single brass ring around the focused target
        // at the end of `draw_frame` via `push_focus_ring`.
        let center_px = bx + bw * 0.5;
        let center_py = by + bh * 0.5;
        let action_anchor = LayoutAnchorPx {
            px: center_px,
            py: center_py + action_world_z_py,
            lift_z: action_hud_table_lift,
        };
        match i {
            0 => {
                // Discard bowl — right side of the discard/play row under the rack.
                // The synthesized
                // `discard_btn_rect` above is already a square sized to
                // the desired bowl diameter and centered at the
                // (bowl_cx, bowl_cy) anchor, so we just read the rect
                // back here without re-applying any nudges.
                //
                // The `hover` field is a binary 0/1 *target*, not a
                // pre-eased value: the renderer keeps a per-bowl
                // `bowl_hover_anim` envelope that exponentially eases
                // toward this target each frame, driving both the
                // existing lift and the tilt-toward-camera rotation
                // applied to the bowl's model matrix in
                // `wgpu_renderer.rs`. Tilt direction reverses
                // automatically when the target flips back to 0.
                let target = if hovered && discard_enabled { 1.0 } else { 0.0 };
                let diam = bw.min(bh);
                // Stretch along the river's flow axis (local X) and
                // keep Z narrower so the meandering stream reads as
                // a stream rather than a pool. Y stays uniform so
                // the hover lift envelope matches the old bowl.
                let river_len = diam * 1.9;
                let river_width = diam * 1.1;
                let pos = {
                    let t = action_anchor.to_draw_cmd_triple();
                    let bowl = &scene.positions.bowl;
                    [
                        t[0] + bowl.nx * layout.window_w,
                        t[1] + bowl.ny * layout.window_h,
                        t[2] + layout.mm(bowl.lift_mm),
                    ]
                };
                discard_bowl_placement = Some(Object3d {
                    pos,
                    extents: [river_len, diam, river_width],
                    rotation: [std::f32::consts::FRAC_PI_2, 0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Bowl,
                    hover_target: target,
                    anim_id: 1,
                });
                // Gold "Discard tiles" label superimposed on the river
                // when it's the active selection (cursor hover or
                // keyboard focus). Anchored purely on the renderer's
                // projected mesh rect — no layout-rect fallback, so
                // on the very first frame after a scene transition
                // the label briefly doesn't appear.
            }
            1 => {
                // Bronze mirror — left side of that same row (paired with the bowl).
                // Same square `play_btn_rect` convention as the bowl,
                // and the same binary-target → renderer-eased envelope
                // pattern. The renderer's `mirror_hover_anim` field
                // handles the smoothed tilt + reverse-on-unhover.
                let target = if hovered && play_enabled { 1.0 } else { 0.0 };
                let diam = bw.min(bh);
                let pos = {
                    let t = action_anchor.to_draw_cmd_triple();
                    let mirror = &scene.positions.mirror;
                    [
                        t[0] + mirror.nx * layout.window_w,
                        t[1] + mirror.ny * layout.window_h,
                        t[2] + layout.mm(mirror.lift_mm),
                    ]
                };
                bronze_mirror_placement = Some(Object3d {
                    pos,
                    extents: [diam, diam, diam],
                    rotation: [0.0, 0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Mirror {
                        rotation_x_deg: 90.0,
                        rotation_z_deg: 0.0,
                    },
                    hover_target: target,
                    anim_id: 2,
                });
                // Gold "Play hand" label superimposed on the mirror
                // when it's the active selection. Same projected-mesh
                // anchoring as the river label above (no layout-rect
                // fallback).
            }
            2 => {
                let tablet_thickness = (bh * 0.35).max(8.0);
                let structure_full = gameplay.structure_complete;
                let wiggle_deg =
                    if structure_full && !GameEngine::auto_cash_in_on_full_structure(run) {
                        scene.trigger_tablet_wiggle_deg(now)
                    } else {
                        0.0
                    };
                let _tablet_idx = wood_tablet_placements.len();
                let wiggle = glam::Mat4::from_rotation_z(wiggle_deg.to_radians());
                let tp = &scene.positions.tablet_cash_in;
                let anchor = action_anchor.to_draw_cmd_triple();
                let cam_m = rot_euler_xyz_rad(cam_euler[0], cam_euler[1], cam_euler[2]);
                wood_tablet_placements.push(Object3d {
                    pos: [
                        anchor[0] + tp.nx * layout.window_w,
                        anchor[1] + tp.ny * layout.window_h,
                        anchor[2] + layout.mm(tp.lift_mm),
                    ],
                    extents: [bw, tablet_thickness, bh],
                    rotation: crate::render::table_transform::compose_rotation_euler(
                        wiggle * cam_m,
                        tp.rotation_deg(),
                    ),
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::WoodTablet {
                        label: std::borrow::Cow::Borrowed("Cash in"),
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                });
            }
            _ => {}
        }
    }

    // Yaku Journal — same leather-bound book mesh + zoom transition as the shop.
    let (_, rby, _, rbh) = journal_btn_rect;
    let book_cy = rby + rbh * 0.5;
    let book_cx = journal_btn_cx;
    let w = layout.window_w;
    let h = layout.window_h;
    let journal_anchor = LayoutAnchorPx {
        px: book_cx,
        py: book_cy + action_world_z_py,
        lift_z: action_hud_table_lift,
    }
    .to_draw_cmd_triple();
    let tp = &scene.positions.tablet_journal;
    let journal_base_x = journal_anchor[0] + tp.nx * w;
    let journal_base_y = journal_anchor[1] + tp.ny * h;
    let journal_base_z = journal_anchor[2] + layout.mm(tp.lift_mm);

    let (journal_zoom, journal_pos) = match scene.journal_transition {
        Some(t) => {
            let z = t.zoom_progress();
            let smoothed = z * z * (3.0 - 2.0 * z);
            let zoom = 1.0 + smoothed * 7.0;
            let cx = w * 0.5;
            let cy = h * 0.5;
            let pos = [
                journal_base_x + (cx - journal_base_x) * smoothed,
                journal_base_y + (cy - journal_base_y) * smoothed,
                journal_base_z,
            ];
            (zoom, pos)
        }
        None => (1.0, [journal_base_x, journal_base_y, journal_base_z]),
    };

    let (face_w, face_h) = book_cover_face_extents_xy(w, journal_zoom);
    let journal_book = Some(Object3d {
        pos: journal_pos,
        extents: [
            face_w,
            layout.mm(BOOK_SPINE_THICKNESS_MM) * journal_zoom,
            face_h,
        ],
        rotation: crate::render::table_transform::compose_rotation_euler(
            rot_euler_xyz_rad(cam_euler[0], cam_euler[1], cam_euler[2]),
            tp.rotation_deg(),
        ),
        color: [1.0, 1.0, 1.0, 1.0],
        kind: Object3dKind::Book {
            spine_label: std::borrow::Cow::Borrowed("Journal"),
            pick_id: Some(YAKU_JOURNAL_BOOK_PICK_ID),
            open_amount: scene.journal_open_amount,
        },
        hover_target: 0.0,
        anim_id: 0,
    });

    if let Some(rect) = ctx
        .proj
        .aux_dish_rects
        .iter()
        .find_map(|(pid, r)| (*pid == Some(YAKU_JOURNAL_BOOK_PICK_ID)).then_some(*r))
    {
        focus_rect_graph.push((FocusTarget::Button(GameplayButton::Journal), rect));
    }

    ActionRowOutputs {
        wood_tablet_placements,
        discard_bowl_placement,
        bronze_mirror_placement,
        journal_book,
    }
}

/// Outputs of the yaku panel + structure showcase + yaku tablet builder.
pub(super) struct YakuPanelOutputs {
    pub(super) yaku_preview_effective_tiles: Vec<crate::core::tile::Tile>,
    pub(super) yaku_preview_sets: Vec<crate::core::hand::DetectedMeld>,
    pub(super) yaku_tablet_placements: Vec<crate::render::draw_cmd::Object3d>,
    pub(super) structure_showcase: Vec<crate::render::draw_cmd::ShowcaseTilePlacement>,
    pub(super) structure_pile_tokens: Vec<crate::render::draw_cmd::Object3d>,
    pub(super) cam_euler: [f32; 3],
}

/// Build the yaku progress panel (previews, structure showcase tiles,
/// preview piles) and the yaku tablet placements.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_yaku_panel_and_tablets(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    ctx: &crate::scenes::DrawCtx<'_>,
    gameplay: &crate::game::engine::GameplayReadModel,
    interaction: &crate::game::engine::GameplayInteractionReadModel,
    cascade_showcase_ref: Option<&CascadeShowcase>,
    cascade_frame: Option<&crate::game::cascade::CascadeFrame>,
    has_structure: bool,
    layout_scale: f32,
    container_w: f32,
    container_x: f32,
    structure_strip_top: f32,
    structure_tag_h: f32,
    structure_meld_h: f32,
    yaku_panel_h: f32,
    yaku_row_y: f32,
    trigger_btn_rect: (f32, f32, f32, f32),
) -> YakuPanelOutputs {
    use super::cascade_hud::{
        PreviewPilePlacement, push_structure_preview_pile, structure_preview_chip_stack_count,
        structure_preview_mult_stack_count,
    };
    use crate::core::yaku::yaku_preview;
    use crate::render::draw_cmd::{
        CameraParams, CascadeTokenKind, Object3d, Object3dKind, ShowcaseTilePlacement,
        camera_facing_euler_xyz_rad,
    };

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

    if selected_tiles_for_yaku.is_empty() {
        yaku_preview_original_tiles =
            GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), run);
        yaku_preview_effective_tiles =
            GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), run);
        yaku_preview_sets = gameplay.structure_sets.clone();
    } else if let Some((selected_sets, selected_scoring_tiles)) = wildcard_result.as_ref() {
        yaku_preview_original_tiles =
            GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), run);
        yaku_preview_original_tiles.extend(GameplayScene::display_tiles(
            selected_tiles_for_yaku.iter().copied(),
            run,
        ));
        yaku_preview_effective_tiles =
            GameplayScene::display_tiles(gameplay.structure_tiles.iter().copied(), run);
        yaku_preview_effective_tiles.extend(GameplayScene::display_tiles(
            selected_scoring_tiles.iter().copied(),
            run,
        ));
        yaku_preview_sets = gameplay.structure_sets.clone();
        yaku_preview_sets.extend(selected_sets.iter().cloned());
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
    let mut structure_pile_tokens: Vec<Object3d> = Vec::new();

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
        // Same pose as the hand strip: upright with face toward the camera.
        const STRUCTURE_TILE_RX: f32 =
            std::f32::consts::FRAC_PI_2 - 22.0_f32 * std::f32::consts::PI / 180.0;
        const STRUCTURE_TILE_RZ: f32 = std::f32::consts::PI;
        let structure_base_lift_mm = 15.0;

        let pad = (8.0 * layout_scale).max(6.0);
        // Reserve a gutter on the right for the structure cash-in
        // preview stacks only while the live structure is still present.
        let preview_pill_w = (22.0 * layout_scale).max(18.0);
        let preview_gap_x = (8.0 * layout_scale).max(5.0);
        let preview_lane_w = if has_structure && cascade_showcase_ref.is_none() {
            preview_pill_w * 2.6 + preview_gap_x + pad
        } else {
            0.0
        };
        let intra_gap = (3.0 * layout_scale).max(2.0);
        let inter_gap = (10.0 * layout_scale).max(7.0);
        let total_tiles: usize = showcase.sets.iter().map(|s| s.tile_ids.len()).sum();
        let intra_count: usize = showcase
            .sets
            .iter()
            .map(|s| s.tile_ids.len().saturating_sub(1))
            .sum();
        let inter_count = showcase.sets.len().saturating_sub(1);
        let available_w = container_w
            - pad * 2.0
            - preview_lane_w
            - intra_count as f32 * intra_gap
            - inter_count as f32 * inter_gap;
        let n_t = total_tiles.max(1);
        let tile_size = (available_w / n_t as f32).clamp(22.0, (44.0 * layout_scale).max(28.0));
        let meld_top = structure_strip_top + structure_tag_h;
        let center_py = meld_top + structure_meld_h * 0.5;
        let mut x_cursor = container_x + pad;
        let active_tile_ids = cascade_frame
            .as_ref()
            .map(|frame| frame.highlight_tile_ids.as_slice())
            .unwrap_or(&[]);
        let pulse_t = cascade_frame
            .as_ref()
            .map(|frame| frame.phase_t)
            .unwrap_or(0.0);
        for (mi, set) in showcase.sets.iter().enumerate() {
            for (ti, &tid) in set.tile_ids.iter().enumerate() {
                let Some(tile) = showcase.tiles.iter().find(|t| t.id == tid).copied() else {
                    continue;
                };
                let px = x_cursor + tile_size * 0.5;
                let mut lift = ti as f32 * 1.2 + mi as f32 * 0.15;
                let pulse = active_tile_ids
                    .iter()
                    .position(|id| *id == tid)
                    .map(|pulse_idx| {
                        let delay = (pulse_idx as f32 * 0.18).min(0.7);
                        let local_t =
                            ((pulse_t - delay) / (1.0 - delay).max(0.001)).clamp(0.0, 1.0);
                        (local_t * std::f32::consts::PI).sin().max(0.0)
                    })
                    .unwrap_or(0.0);
                let scale = 1.0 + 0.16 * pulse;
                let brightness = 1.0 + 0.35 * pulse;
                lift += 6.0 * pulse;
                structure_showcase.push(ShowcaseTilePlacement {
                    tile,
                    center_pos: [px, center_py, layout.mm(structure_base_lift_mm + lift)],
                    rotation: [STRUCTURE_TILE_RX, 0.0, STRUCTURE_TILE_RZ],
                    scale,
                    size_px: tile_size,
                    brightness,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: false,
                    glow_color: None,
                    pick_id: None,
                    overlay_rect_group: None,
                });
                x_cursor += tile_size + intra_gap;
            }
            if mi + 1 < showcase.sets.len() {
                x_cursor += inter_gap - intra_gap;
            }
        }

        if has_structure && cascade_showcase_ref.is_none() {
            let trigger_preview = GameEngine::preview_manual_trigger_breakdown(run);
            let preview_chips = trigger_preview
                .as_ref()
                .map(|breakdown| breakdown.final_chips.max(0))
                .unwrap_or_else(|| GameEngine::structure_banked_meld_chips(run).max(0));
            let preview_mult = trigger_preview
                .as_ref()
                .map(|breakdown| breakdown.final_mult.max(1.0))
                .unwrap_or_else(|| {
                    1.0 + crate::core::structure::structure_depth_mult_bonus(
                        gameplay.structure_sets.len() as u32,
                    )
                });
            let chip_stack_count = structure_preview_chip_stack_count(preview_chips);
            let mult_stack_count = structure_preview_mult_stack_count(preview_mult);
            let (tr_x, tr_y, _tr_w, tr_h) = trigger_btn_rect;
            let pill_w = (22.0 * layout_scale).max(18.0);
            let pill_h = (16.0 * layout_scale).max(12.0);
            let t_th = (pill_h * 0.35).max(4.0);
            let gap_x = (8.0 * layout_scale).max(5.0);
            let col_cx = (tr_x - gap_x - pill_w * 1.1).max(pill_w * 0.5 + 4.0);
            let base_cy = tr_y + tr_h * 0.5;
            let base_lift = 5.0;
            let token_extents = [pill_w * 0.5, t_th, pill_h * 0.5];
            push_structure_preview_pile(
                &mut structure_pile_tokens,
                CascadeTokenKind::Chips,
                chip_stack_count,
                PreviewPilePlacement {
                    center_x: col_cx - pill_w * 0.55,
                    center_y: base_cy,
                    base_lift,
                    extents: token_extents,
                },
                0.35,
                0x5C71_0000_u64 ^ preview_chips.max(0) as u64,
            );
            push_structure_preview_pile(
                &mut structure_pile_tokens,
                CascadeTokenKind::Mult,
                mult_stack_count,
                PreviewPilePlacement {
                    center_x: col_cx + pill_w * 0.55 + gap_x,
                    center_y: base_cy,
                    base_lift,
                    extents: token_extents,
                },
                0.35,
                0xA17E_0000_u64 ^ mult_stack_count as u64,
            );
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
    let is_chicken_hand = visible_previews.is_empty()
        && !yaku_preview_sets.is_empty()
        && crate::core::yaku::is_complete_winning_hand(
            &yaku_preview_effective_tiles,
            &yaku_preview_sets,
        );

    // Phase 3: yaku selectors are now physical bone tablets sitting in
    // a row in front of the hand. The flat slate-blue card quads + the
    // progress-fill bar are gone — replaced by `YakuTabletBatch` that
    // the renderer dispatches through the lit-mesh pipeline. The 2D
    // text labels stay as a screen-space overlay until the engraved
    // decal pass lands; hover tracking still uses the original screen
    // rect (the cards live in the same pixel region as before).
    let cam_euler = {
        let cam = CameraParams::default_table_camera(layout.window_h);
        camera_facing_euler_xyz_rad(cam.eye, cam.target)
    };
    let yaku_tablet_rot = cam_euler;
    let yaku_tablet_px_dx = scene.positions.yaku_tablet.nx * layout.window_w;
    let yaku_tablet_px_dy = scene.positions.yaku_tablet.ny * layout.window_h;
    let yaku_tablet_lift_dz = layout.mm(scene.positions.yaku_tablet.lift_mm);
    let mut yaku_tablet_placements: Vec<Object3d> = Vec::new();
    if !visible_previews.is_empty() || is_chicken_hand {
        let panel_h = yaku_panel_h;
        let panel_y = yaku_row_y;
        let panel_w = container_w;
        let panel_x = container_x;
        let tablet_count = if is_chicken_hand {
            1
        } else {
            visible_previews.len()
        };
        let n = tablet_count as f32;
        let card_gap = 6.0 * layout_scale;
        // Cap individual card width so a lone active yaku doesn't stretch
        // across the entire container, which reads as a UI bug.
        let natural_card_w = (panel_w - card_gap * 2.0) / 3.0;
        let card_w = ((panel_w - card_gap * (n - 1.0)) / n).min(natural_card_w);
        // Tablets are flat-on-table dominoes: extents[0] is width
        // (matches card width), extents[1] is the thickness above the
        // wood, extents[2] is depth (matches card height into the
        // scene).
        let tablet_thickness = (8.0 * layout_scale).max(6.0);
        if is_chicken_hand {
            // Chicken hand: valid meld but no yaku fired. Show a single
            // tablet with a chicken emoji so the player knows the hand
            // is legal (just worth very little).
            let center_px = panel_x + card_w * 0.5;
            let center_py = panel_y + panel_h * 0.5;
            let hovered_now = matches!(
                ctx.picked_gameplay_object,
                Some(crate::render::wgpu_renderer::GameplayPick::YakuTablet(0))
            );
            yaku_tablet_placements.push(Object3d {
                pos: [
                    center_px + yaku_tablet_px_dx,
                    center_py + yaku_tablet_px_dy,
                    yaku_tablet_lift_dz,
                ],
                extents: [card_w, tablet_thickness, panel_h],
                rotation: yaku_tablet_rot,
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::YakuTablet {
                    label: std::borrow::Cow::Borrowed("\u{1F414} Chicken Hand"),
                    active: true,
                    hover: if hovered_now { 1.0 } else { 0.0 },
                },
                hover_target: 0.0,
                anim_id: 0,
            });
        } else {
            for (i, p) in visible_previews.iter().enumerate() {
                let cx = panel_x + i as f32 * (card_w + card_gap);
                let cy = panel_y;
                let center_px = cx + card_w * 0.5;
                let center_py = cy + panel_h * 0.5;
                // Hover state comes from the renderer's raycast picker
                // (precomputed local AABB × per-frame model matrix), not
                // from screen-rect hit-testing the projected AABB. The
                // picker is one frame stale, like every other 3D pick
                // path in the engine.
                let hovered_now = matches!(
                    ctx.picked_gameplay_object,
                    Some(crate::render::wgpu_renderer::GameplayPick::YakuTablet(j))
                        if j == i,
                );
                let yaku_discovered = ctx
                    .progress
                    .yaku_times_scored
                    .get(&p.kind)
                    .copied()
                    .unwrap_or(0)
                    >= 1;
                let tablet_label: std::borrow::Cow<'static, str> = if yaku_discovered {
                    std::borrow::Cow::Borrowed(p.kind.name())
                } else {
                    std::borrow::Cow::Borrowed("???")
                };
                yaku_tablet_placements.push(Object3d {
                    pos: [
                        center_px + yaku_tablet_px_dx,
                        center_py + yaku_tablet_px_dy,
                        yaku_tablet_lift_dz,
                    ],
                    extents: [card_w, tablet_thickness, panel_h],
                    rotation: yaku_tablet_rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::YakuTablet {
                        label: tablet_label,
                        active: p.active,
                        hover: if hovered_now { 1.0 } else { 0.0 },
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                });
            }
        }
    }

    YakuPanelOutputs {
        yaku_preview_effective_tiles,
        yaku_preview_sets,
        yaku_tablet_placements,
        structure_showcase,
        structure_pile_tokens,
        cam_euler,
    }
}
