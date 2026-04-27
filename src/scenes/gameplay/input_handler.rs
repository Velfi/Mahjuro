//! Input handler — owns the unified focus model + 3D hit dispatcher +
//! gameplay action handlers (ScoreHand, TriggerStructure, sort, discard) of
//! [`super::GameplayScene::update`]. Behaviour is identical to the inline
//! code; this is purely organisational.

use std::time::Instant;

use super::GameplayScene;
use super::cascade_hud::CascadeShowcase;
use super::focus::{
    FocusTarget, GameplayButton, focus_kind, focus_kind_sfx, play_select_sfx, wrap_hand_tile_focus,
};
use crate::core::scoring::StepKind;
use crate::game::engine::{CommandData, GameCommand, GameEngine};
use crate::render::animation::ENTITY_SCORE_PANEL;
use crate::scenes::{OverlayRequest, Scene, SceneTransition, UpdateCtx, YakuJournalScene};
use crate::ui::focus_nav::{FocusDir, focus_target_at_cursor, pick_neighbor};
use crate::ui::input::{UiAction, apply_ui_actions};

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
            | FocusTarget::Dora => true,
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
        let has_structure = gameplay.uses_structure_bank && gameplay.has_structure;
        if !has_structure {
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

    // Process actions. Directional input → spatial picker. Confirm →
    // route by self.focus variant. Cancel → clear focus AND fall
    // through so existing `clear_selection` semantics still apply.
    // Everything else flows into `actions_for_scene` for the existing
    // gameplay action handlers below (ScoreHand, SortBySuit, etc.).
    let mut actions_for_scene: Vec<UiAction> = Vec::new();
    for &a in ctx.actions.iter() {
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
                    (Some(FocusTarget::Button(GameplayButton::Play)), FocusDir::Left) => focus_rects
                        .iter()
                        .find(|(t, _)| matches!(t, FocusTarget::Button(GameplayButton::Discard)))
                        .map(|(t, _)| *t),
                    // RIGHT from Discard → Play (commit melds)
                    (Some(FocusTarget::Button(GameplayButton::Discard)), FocusDir::Right) => {
                        focus_rects
                            .iter()
                            .find(|(t, _)| {
                                matches!(t, FocusTarget::Button(GameplayButton::Play))
                            })
                            .map(|(t, _)| *t)
                    }
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
                && let Some((added, removed)) =
                    GameEngine::apply_marquee_selection(ctx.run, m, idx)
                && (added > 0 || removed > 0)
            {
                play_select_sfx(ctx.bus, added, removed);
            }
            continue;
        }

        match a {
            // Legacy "shoulder buttons cycle consumables" affordance.
            // Steps through `Consumable` targets in order; wraps back
            // to `None` after the last so the player can exit the
            // strip without a separate keybind.
            UiAction::NavigateHudNext if !consumable_targets.is_empty() => {
                let cur_pos = consumable_targets
                    .iter()
                    .position(|t| Some(*t) == scene.focus);
                scene.focus = match cur_pos {
                    None => Some(consumable_targets[0]),
                    Some(i) if i + 1 >= consumable_targets.len() => None,
                    Some(i) => Some(consumable_targets[i + 1]),
                };
                continue;
            }
            UiAction::NavigateHudPrev => {
                if !consumable_targets.is_empty() {
                    let cur_pos = consumable_targets
                        .iter()
                        .position(|t| Some(*t) == scene.focus);
                    scene.focus = match cur_pos {
                        None => Some(*consumable_targets.last().unwrap()),
                        Some(0) => None,
                        Some(i) => Some(consumable_targets[i - 1]),
                    };
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
                        *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(
                            Scene::YakuJournal(YakuJournalScene::new()),
                        )));
                        return Some(None);
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
                                    crate::game::run::ConsumableUseResult::Zodiac {
                                        yaku,
                                        new_level,
                                    },
                            } => {
                                log::info!(
                                    "Used Zodiac → {} now level {}",
                                    yaku.name(),
                                    new_level,
                                );
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
                                    [0.95, 0.78, 0.25, 1.0],
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
                        // Clear focus so the next press doesn't double-
                        // fire on whatever consumable shifted into the
                        // freed slot.
                        scene.focus = None;
                    }
                    Some(FocusTarget::Peg(_))
                    | Some(FocusTarget::Gold)
                    | Some(FocusTarget::YakuTablet(_))
                    | Some(FocusTarget::Dora) => {}
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
            // Cancel: clear focus AND let the existing
            // clear_selection path run via apply_ui_actions.
            UiAction::Cancel => {
                scene.held_relic_drag = None;
                scene.marquee = None;
                scene.focus = None;
                actions_for_scene.push(a);
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
        if cid != super::GAMEPLAY_3D_HIT_ID {
            continue;
        }
        use crate::render::wgpu_renderer::GameplayPick;
        let gameplay = GameEngine::read(ctx.run);
        let has_structure = gameplay.uses_structure_bank && gameplay.has_structure;
        let journal_tablet_i: usize = if has_structure { 3 } else { 2 };
        if matches!(
            ctx.picked_gameplay_object,
            Some(GameplayPick::WoodTablet(i)) if i == journal_tablet_i
        ) {
            *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::YakuJournal(
                YakuJournalScene::new(),
            ))));
            return Some(None);
        }
        let action = match ctx.picked_gameplay_object {
            Some(GameplayPick::WoodTablet(0)) => Some(UiAction::SortBySuit),
            Some(GameplayPick::WoodTablet(1)) => Some(UiAction::SortByRank),
            Some(GameplayPick::WoodTablet(2)) if has_structure => {
                Some(UiAction::TriggerStructure)
            }
            Some(GameplayPick::BronzeMirror) => Some(UiAction::ScoreHand),
            Some(GameplayPick::DiscardBowl) => Some(UiAction::CommitDiscard),
            _ => None,
        };
        if let Some(a) = action {
            actions_for_scene.push(a);
        }
    }

    // Clear any previous frame's departures.
    scene.pending_departures.clear();

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
                let had_selection = gameplay.selected_count > 0;
                let bank_before = GameEngine::structure_banked_meld_chips(ctx.run);
                let round_before = gameplay.round_score;
                let score_before = gameplay.round_score;
                let cascade_showcase = if gameplay.selected_count == 0 {
                    None
                } else {
                    let selected_tiles: Vec<_> = ctx
                        .run
                        .hand
                        .iter()
                        .zip(interaction.selected.iter())
                        .filter(|&(_, &sel)| sel)
                        .map(|(t, _)| *t)
                        .collect();
                    GameEngine::validate_with_wildcards(ctx.run, &selected_tiles).map(
                        |(sets, scoring_tiles)| {
                            if gameplay.uses_structure_bank {
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
                            } else {
                                CascadeShowcase {
                                    tiles: GameplayScene::display_tiles(scoring_tiles, ctx.run),
                                    sets,
                                }
                            }
                        },
                    )
                };
                let structure_was_complete = gameplay.structure_complete;
                let outcome = {
                    let mut engine = GameEngine::new(ctx.run, ctx.bus);
                    engine.dispatch(GameCommand::PlaySelection)
                };
                let step = match outcome.data {
                    CommandData::PlaySelection { step } => step,
                    _ => 0,
                };
                let gained = outcome.after.round_score.saturating_sub(round_before);
                log::info!(
                    "[score] Commit: step={} gained={} structure_bank={} breakdown_steps={} base_steps={}",
                    step,
                    gained,
                    gameplay.uses_structure_bank,
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

                if step == 0 && had_selection {
                    ctx.anim
                        .shake(crate::render::animation::ENTITY_HAND_STRIP, 8.0, 200);
                } else if gained > 0 {
                    ctx.anim.pulse(ENTITY_SCORE_PANEL);
                    scene.begin_scoring_cascade(ctx, score_before, gained, cascade_showcase);
                } else if step > 0 {
                    ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                    let bank_after = GameEngine::structure_banked_meld_chips(ctx.run);
                    let d = bank_after.saturating_sub(bank_before);
                    if d > 0 {
                        let structure_is_complete = GameEngine::read(ctx.run).structure_complete;
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
            UiAction::TriggerStructure => {
                if !GameEngine::read(ctx.run).uses_structure_bank {
                    // Classic mode: plays score on commit; no cash-in action.
                } else {
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
            }
            UiAction::SortBySuit => {
                let mut engine = GameEngine::new(ctx.run, ctx.bus);
                let _ = engine.dispatch(GameCommand::SortHandBySuit);
                ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
            }
            UiAction::SortByRank => {
                let mut engine = GameEngine::new(ctx.run, ctx.bus);
                let _ = engine.dispatch(GameCommand::SortHandByRank);
                ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
            }
            UiAction::CommitDiscard => {
                let gameplay = GameEngine::read(ctx.run);
                // Capture selected indices BEFORE discard so we can animate them departing.
                if gameplay.selected_count > 0 && gameplay.discards_remaining > 0 {
                    let selected_indices: Vec<usize> = ctx
                        .run
                        .selected
                        .iter()
                        .enumerate()
                        .filter(|&(_, &s)| s)
                        .map(|(i, _)| i)
                        .collect();
                    scene.pending_departures = selected_indices;
                }
                // Remove the tiles immediately, but defer the auto-draw
                // until the departure animation has had time to play.
                let outcome = {
                    let mut engine = GameEngine::new(ctx.run, ctx.bus);
                    engine.dispatch(GameCommand::DiscardSelectionNoRefill)
                };
                let discarded = match outcome.data {
                    CommandData::DiscardSelection { count } => count,
                    _ => 0,
                };
                if discarded > 0 {
                    ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                    let depart_lifetime =
                        std::time::Duration::from_millis(ctx.cascade_tuning.depart_lifetime_ms);
                    scene.pending_refill = Some(now + depart_lifetime);
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

/// Build the relic tray (3D enamel medallions across the top of the
/// screen), the relic-focused hover ring + tooltip, the display-only
/// focus tooltips for pegs/gold/yaku tablets/dora, and the post-deal
/// smoke breath / debug `B` wind gust impulses. Behaviour is a verbatim
/// lift of the inline draw_frame chunk; relocated for organisation.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_relic_tray_and_wind(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    gameplay: &crate::game::engine::GameplayReadModel,
    ctx: &crate::scenes::DrawCtx<'_>,
    now: Instant,
    hand_slots: &[(f32, f32, f32, f32)],
    is_chicken_hand: bool,
    visible_preview_kinds: &[crate::core::yaku::YakuKind],
    coin_pile_rect: Option<[f32; 4]>,
    dora_rect: [f32; 4],
    hover_quads: &mut Vec<crate::render::wgpu_renderer::GpuInstance>,
    hover_text: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
) -> (Vec<crate::render::draw_cmd::Object3d>, Vec<crate::render::draw_cmd::WindGust>) {
    use crate::core::relic::relic_visual;
    use crate::render::draw_cmd::{CameraParams, Object3d, Object3dKind};
    use crate::render::table_transform::rot_rx_ry_rz_deg;
    use crate::render::theme::typography;
    use crate::render::wgpu_renderer::{GpuInstance, TextAlign, TextLabel};
    use crate::ui::widget::{self, TextStyle};
    use super::focus::PegKind;
    use super::tooltip::{push_tooltip, relic_tooltip_copy_detail, yaku_card_shape_text};
    use super::DEBUG_WIND_DURATION;
    use super::RELIC_GLOW_LIFETIME;

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
            let thick = face * 0.12 * visual.thickness_scale;

            // Color tracks the relic's rarity tier.
            let rarity = defs
                .iter()
                .find(|d| d.id == rid)
                .map(|d| d.rarity)
                .unwrap_or(crate::core::relic::Rarity::Common);
            let tier: u8 = match rarity {
                crate::core::relic::Rarity::Common => 0,
                crate::core::relic::Rarity::Uncommon => 1,
                crate::core::relic::Rarity::Rare => 2,
                crate::core::relic::Rarity::Legendary => 3,
            };
            let color = crate::render::theme::color::rarity(tier);

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
                rotation: rot_rx_ry_rz_deg(face_pitch_deg, wiggle_deg, 0.0),
                color,
                kind: Object3dKind::Relic {
                    relic_id: rid,
                    glow,
                    silhouette: false,
                    pick_id: None,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }
    }

    // Focus / hover detection: in cursor mode, the Phase A sync in
    // `update()` already wrote `FocusTarget::Relic(i)` whenever the
    // cursor was over a projected relic rect; in keyboard / controller
    // mode the player navigates focus there with directional input.
    // Either way, the relic tooltip and outline show whenever
    // `self.focus` is `Some(Relic(i))`.
    let hovered_relic_idx: Option<usize> = match scene.focus {
        Some(FocusTarget::Relic(i)) if i < ctx.proj.relic_rects.len() => Some(i),
        _ => None,
    };
    if let Some(hi) = hovered_relic_idx
        && let (Some(rect), Some(rid)) = (
            ctx.proj.relic_rects.get(hi),
            relic_objects.get(hi).and_then(|o| match o.kind {
                Object3dKind::Relic { relic_id, .. } => Some(relic_id),
                _ => None,
            }),
        )
    {
        // Gold rim drawn around the projected screen rect — cheap
        // 2D outline that hugs the visible 3D box.
        let [rx, ry, rw, rh] = *rect;
        let t = (rh * 0.04).clamp(2.0, 4.0);
        let rim = crate::render::theme::color::CHAMPAGNE;
        hover_quads.push(GpuInstance {
            rect: [rx - t, ry - t, rw + t * 2.0, t],
            color: rim,
        });
        hover_quads.push(GpuInstance {
            rect: [rx - t, ry + rh, rw + t * 2.0, t],
            color: rim,
        });
        hover_quads.push(GpuInstance {
            rect: [rx - t, ry, t, rh],
            color: rim,
        });
        hover_quads.push(GpuInstance {
            rect: [rx + rw, ry, t, rh],
            color: rim,
        });

        // Tooltip: name + description in a small dark panel anchored
        // above the hovered relic.
        use crate::core::relic::all_relic_defs;
        let defs = all_relic_defs();
        if let Some(def) = defs.iter().find(|d| d.id == rid) {
            let mut live_desc = GameEngine::relic_live_description(ctx.run, rid);
            if let Some(copy_detail) = relic_tooltip_copy_detail(rid, hi, run) {
                live_desc.push_str("\n\n");
                live_desc.push_str(&copy_detail);
            }
            let pad = 18.0_f32;
            let tip_w = 440.0_f32;
            let title_h = 38.0_f32;
            // Pre-wrap the description so the tooltip box can grow
            // tall enough to fit every line.
            let body_style = TextStyle {
                tier: typography::BODY,
                color: crate::render::theme::color::PARCHMENT,
                padding: 0.0,
                align: TextAlign::Left,
            };
            let body_line_h = typography::size(body_style.tier, layout.window_h, ctx.ui_scale);
            let body_step = body_line_h * 1.6;
            let body_box = body_line_h * 1.8;
            let body_inner_w = tip_w - pad * 2.0;
            let wrapped_lines = widget::wrap_text(&live_desc, body_inner_w, body_line_h);
            let body_h = (wrapped_lines.len() as f32 * body_step).max(body_box);
            let tip_h = pad * 2.0 + title_h + body_h;
            let mut tip_x = rx + rw * 0.5 - tip_w * 0.5;
            let mut tip_y = ry - tip_h - 8.0;
            // Clamp to window so the tooltip stays visible.
            tip_x = tip_x.clamp(8.0, layout.window_w - tip_w - 8.0);
            if tip_y < 8.0 {
                tip_y = ry + rh + 8.0;
            }
            if tip_y + tip_h > layout.window_h - 8.0 {
                tip_y = (layout.window_h - tip_h - 8.0).max(8.0);
            }
            let bg =
                crate::render::theme::color::alpha(crate::render::theme::color::MIDNIGHT, 0.96);
            hover_quads.push(GpuInstance {
                rect: [tip_x, tip_y, tip_w, tip_h],
                color: bg,
            });
            // Gold border (4 thin quads).
            let bt = 1.5_f32;
            let border = crate::render::theme::color::BRASS;
            hover_quads.push(GpuInstance {
                rect: [tip_x, tip_y, tip_w, bt],
                color: border,
            });
            hover_quads.push(GpuInstance {
                rect: [tip_x, tip_y + tip_h - bt, tip_w, bt],
                color: border,
            });
            hover_quads.push(GpuInstance {
                rect: [tip_x, tip_y + bt, bt, tip_h - bt * 2.0],
                color: border,
            });
            hover_quads.push(GpuInstance {
                rect: [tip_x + tip_w - bt, tip_y + bt, bt, tip_h - bt * 2.0],
                color: border,
            });
            hover_text.push(TextLabel {
                rect: [tip_x + pad, tip_y + pad, tip_w - pad * 2.0, title_h],
                text: def.name.to_string(),
                color: crate::render::theme::color::CHAMPAGNE,
                ..Default::default()
            });
            widget::push_text_block(
                hover_text,
                [
                    tip_x + pad,
                    tip_y + pad + title_h,
                    tip_w - pad * 2.0,
                    body_h,
                ],
                &live_desc,
                body_style,
                layout.window_h,
                ctx.ui_scale,
            );
        }
    }
    // Display-only focus tooltips: when the player navigates focus
    // onto a counter peg or the gold counter, surface a small info
    // panel with the current count. These are anchored to whatever
    // rect the focus rect graph published for the same target.
    match scene.focus {
        Some(FocusTarget::Peg(kind)) => {
            let gameplay = GameEngine::read(run);
            let rect_idx = match kind {
                PegKind::Hands => 0,
                PegKind::Discards => 1,
            };
            if let Some(r) = ctx.proj.peg_rects[rect_idx] {
                let (title, body) = match kind {
                    PegKind::Hands => (
                        "Hands Remaining".to_string(),
                        format!(
                            "{} of {} plays left this round. Each Play Hand consumes one peg.",
                            gameplay.plays_remaining, gameplay.plays_max,
                        ),
                    ),
                    PegKind::Discards => (
                        "Discards Remaining".to_string(),
                        format!(
                            "{} of {} discards left this round. Each Discard consumes one peg.",
                            gameplay.discards_remaining, gameplay.discards_max,
                        ),
                    ),
                };
                push_tooltip(
                    hover_quads,
                    hover_text,
                    (r[0] + r[2] * 0.5, r[1]),
                    crate::ui::layout::ViewportCtx {
                        window_w: layout.window_w,
                        window_h: layout.window_h,
                        ui_scale: ctx.ui_scale,
                    },
                    &title,
                    &body,
                );
            }
        }
        Some(FocusTarget::Gold) => {
            let gameplay = GameEngine::read(run);
            // Anchor the tooltip just above the coin pile so the
            // hover label points at the actual gold rather than the
            // unrelated score-panel cartouche. When there's no gold
            // (no pile drawn) we fall back to skipping the tooltip
            // entirely — `FocusTarget::Gold` is only reachable from
            // the focus rect graph, which is also gated on a
            // populated `coin_pile_rect`.
            if let Some(rect) = coin_pile_rect {
                push_tooltip(
                    hover_quads,
                    hover_text,
                    (rect[0] + rect[2] * 0.5, rect[1]),
                    crate::ui::layout::ViewportCtx {
                        window_w: layout.window_w,
                        window_h: layout.window_h,
                        ui_scale: ctx.ui_scale,
                    },
                    "Gold",
                    &format!(
                        "${}. Earned from clearing blinds. Spend in the shop on relics, ribbons, talismans, and pack rerolls.",
                        gameplay.gold,
                    ),
                );
            }
        }
        Some(FocusTarget::YakuTablet(i)) => {
            // Mirror the cursor-hover yaku tooltip path: same title +
            // body, same anchor (just above the projected tablet
            // rect). Falls back to the visible_previews entry rather
            // than re-running the preview pipeline.
            let (title, body) = if is_chicken_hand && i == 0 {
                (
                    "\u{1F414} Chicken Hand".to_string(),
                    "A valid hand with no yaku. Scores base chips \u{00D7} 1 mult. \
                     Build toward a yaku to multiply your score."
                        .to_string(),
                )
            } else if let Some(yk) = visible_preview_kinds.get(i).copied() {
                (
                    format!(
                        "{}  (+{} mult, +{} chips)",
                        yk.name(),
                        yk.mult_bonus(),
                        yk.chip_bonus()
                    ),
                    yaku_card_shape_text(yk).to_string(),
                )
            } else {
                ("".to_string(), "".to_string())
            };
            if !title.is_empty() {
                let (ax, ay) = match ctx.proj.yaku_tablet_rects.get(i).copied() {
                    Some([px, py, pw, _ph]) if pw > 0.0 && px.is_finite() && py.is_finite() => {
                        (px + pw * 0.5, py)
                    }
                    _ => (layout.window_w * 0.5, layout.window_h * 0.5),
                };
                push_tooltip(
                    hover_quads,
                    hover_text,
                    (ax, ay),
                    crate::ui::layout::ViewportCtx {
                        window_w: layout.window_w,
                        window_h: layout.window_h,
                        ui_scale: ctx.ui_scale,
                    },
                    &title,
                    &body,
                );
            }
        }
        Some(FocusTarget::Dora) => {
            let per_dora = if gameplay.has_dora_crown { 35 } else { 25 };
            let dora_faces = gameplay.dora_faces.clone();
            let body = if dora_faces.is_empty() {
                format!("Each dora in a scored set: +{per_dora} chips.")
            } else {
                let names: Vec<String> = dora_faces
                    .iter()
                    .map(|&(suit, rank)| {
                        crate::core::tile::Tile::new(suit, rank, 0).full_name()
                    })
                    .collect();
                let label = if names.len() == 1 { "Dora" } else { "Doras" };
                format!(
                    "{label}: {}. Each in a scored set: +{per_dora} chips.",
                    names.join(", ")
                )
            };
            push_tooltip(
                hover_quads,
                hover_text,
                (dora_rect[0] + dora_rect[2] * 0.5, dora_rect[1]),
                crate::ui::layout::ViewportCtx {
                    window_w: layout.window_w,
                    window_h: layout.window_h,
                    ui_scale: ctx.ui_scale,
                },
                "Dora",
                &body,
            );
        }
        _ => {}
    }

    // The glossary-suppression branch from the legacy path is gone:
    // when the glossary is open we early-return a dedicated frame at
    // the very top of `draw_frame`, so by the time we reach this
    // point the glossary is *not* open and every variable below uses
    // its real value directly.

    // Post-deal smoke breath. `wind_delay_secs` after the most
    // recent deal we exhale a soft sweep of impulses across the hand
    // strip — a few evenly spaced points pushed back-and-up — so the
    // smoke that built up while the tiles were sliding in drifts off
    // toward the back of the table. The strength follows a 4t(1-t)
    // bell so the breath fades in and out instead of snapping on.
    let mut wind_gusts: Vec<crate::render::draw_cmd::WindGust> = Vec::new();
    let wind_delay = scene.wind_delay_secs;
    let wind_duration = scene.wind_duration_secs.max(0.001);
    {
        if let Some(deal_at) = scene.last_deal_at {
            let elapsed = now.saturating_duration_since(deal_at).as_secs_f32();
            if elapsed >= wind_delay && elapsed < wind_delay + wind_duration {
                let t = (elapsed - wind_delay) / wind_duration;
                let envelope = (4.0 * t * (1.0 - t)).clamp(0.0, 1.0);
                if !hand_slots.is_empty() {
                    // Sweep a 2D grid of impulses across the screen so
                    // the breath covers both the hand strip *and* the
                    // table interior above it. Horizontal axis: full
                    // window width with edge overshoot, so corner smoke
                    // gets shoved off-stage. Vertical axis: starts just
                    // below the hand strip (where smoke pools above the
                    // tile faces) and extends upward into the table all
                    // the way to the candle row, with each row lifted a
                    // bit higher off the table than the last so the
                    // sweep catches smoke pooling higher up as well.
                    let sw = hand_slots[0].2;
                    let sy = hand_slots[0].1;
                    // 6×4 = 24 impulses keeps us under MAX_INJECTIONS
                    // with headroom for tile motion impulses and the
                    // cursor puffs that share the same per-frame
                    // budget on the fluid sim.
                    const COLS: usize = 6;
                    const ROWS: usize = 4;
                    let win_w = layout.window_w;
                    let win_h = layout.window_h;
                    let x_pad = win_w * 0.12;
                    let span_min = -x_pad;
                    let span_max = win_w + x_pad;
                    // Vertical span: from a touch under the hand strip
                    // up to the back of the playable table area (~22%
                    // of window height from the top, where the candles
                    // and dish sit). Anything further back is outside
                    // the smoke grid anyway.
                    let y_bottom = sy + sw * 0.5;
                    let y_top = win_h * 0.22;
                    let radius =
                        ((win_w / COLS as f32) * 1.55).max((sy - y_top) / ROWS as f32 * 1.6);
                    for r in 0..ROWS {
                        // 0..1 across rows, 0 = nearest the player
                        let rf = (r as f32 + 0.5) / ROWS as f32;
                        let cy = y_bottom + (y_top - y_bottom) * rf;
                        // Lift higher for back rows so the gust also
                        // reaches smoke that has drifted upward, not
                        // just the table surface.
                        let lift = 18.0 + 32.0 * rf;
                        // Back rows fade slightly so the sweep reads
                        // as a directional breath rolling forward, not
                        // a uniform wall of wind.
                        let row_strength = 1.0 - 0.25 * rf;
                        for c in 0..COLS {
                            let f = (c as f32 + 0.5) / COLS as f32;
                            let cx = span_min + (span_max - span_min) * f;
                            let edge_bias = (f - 0.5) * 2.0; // -1..1
                            // Velocity tuned against the curtain density
                            // below: previous values (28 lateral / -55 z)
                            // were too gentle to push the curtain off-
                            // grid before the overlay finished fading,
                            // leaving the round draped in residual smoke
                            // that took the natural dissipation many
                            // seconds to clear. The debug `B` gust uses
                            // 1400 lateral / -120 z; we sit well below
                            // that so this still reads as a soft breath
                            // rather than a hurricane, but well above the
                            // old values so the field actually clears.
                            let lateral = 220.0 * edge_bias * envelope * row_strength;
                            wind_gusts.push(crate::render::draw_cmd::WindGust {
                                center_px: (cx, cy),
                                lift,
                                velocity: [
                                    lateral,
                                    -180.0 * envelope * row_strength,
                                    (6.0 + 4.0 * rf) * envelope * row_strength,
                                ],
                                radius,
                                density: -0.10 * envelope * row_strength,
                            });
                        }
                    }
                }
            }
        }
    }

    // Debug: while the `B` gust timer is live, sweep a strong lateral
    // gust across the *same* grid the opening smoke curtain and the
    // post-deal breath use, so the wind footprint matches the smoke
    // footprint exactly — every cell that can hold smoke gets a
    // negative-density impulse with a big lateral velocity, instead of
    // only the four candle bases. The shape envelope is a 4t(1-t) bell
    // so the gust ramps in/out instead of popping. Velocity is
    // intentionally large compared to the post-deal sweep (~28 lateral)
    // — this is a "did the wiring work?" hammer, not a subtle ambient
    // effect, and the radius * 3.0 falloff inside
    // `wgpu_renderer::flame_anchors` still picks the grid impulses up
    // to bend the flames as the gust rolls across the candle row.
    if let Some(debug_at) = scene.debug_wind_at {
        let elapsed = now.saturating_duration_since(debug_at).as_secs_f32();
        if elapsed < DEBUG_WIND_DURATION {
            let t = (elapsed / DEBUG_WIND_DURATION).clamp(0.0, 1.0);
            let envelope = (4.0 * t * (1.0 - t)).clamp(0.0, 1.0);
            // Mirror the curtain grid in `initial_smoke_fill_active`
            // above: same COLS/ROWS, same x_pad, same y_top/y_bottom,
            // same radius formula. If you change one, change both.
            const COLS: usize = 6;
            const ROWS: usize = 4;
            let win_w = layout.window_w;
            let win_h = layout.window_h;
            let x_pad = win_w * 0.15;
            let span_min = -x_pad;
            let span_max = win_w + x_pad;
            let y_top = win_h * 0.22;
            let y_bottom = if !hand_slots.is_empty() {
                hand_slots[0].1 + hand_slots[0].3
            } else {
                win_h * 0.85
            };
            let radius = ((span_max - span_min) / COLS as f32 * 0.95)
                .max((y_bottom - y_top) / ROWS as f32 * 1.6);
            for r in 0..ROWS {
                let rf = (r as f32 + 0.5) / ROWS as f32;
                let cy = y_bottom + (y_top - y_bottom) * rf;
                let lift = 22.0 + 30.0 * rf;
                for c in 0..COLS {
                    let f = (c as f32 + 0.5) / COLS as f32;
                    let cx = span_min + (span_max - span_min) * f;
                    wind_gusts.push(crate::render::draw_cmd::WindGust {
                        center_px: (cx, cy),
                        lift,
                        velocity: [1400.0 * envelope, -120.0 * envelope, 0.0],
                        radius,
                        density: -0.04 * envelope,
                    });
                }
            }
        }
    }

    (relic_objects, wind_gusts)
}

/// Build the consumable inventory dish (Zodiacs + Talismans) — brass dish,
/// pendant placements, focus rects, click buttons, hover tooltip. Behaviour
/// is a verbatim lift of the inline `draw_frame` chunk; relocated for
/// organisation.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_consumable_dish(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    ctx: &crate::scenes::DrawCtx<'_>,
    interaction: &crate::game::engine::GameplayInteractionReadModel,
    paused: bool,
    focus_rect_graph: &mut Vec<(FocusTarget, [f32; 4])>,
    buttons: &mut Vec<crate::scenes::ButtonDef>,
    hover_quads: &mut Vec<crate::render::wgpu_renderer::GpuInstance>,
    hover_text: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
) -> (
    Vec<crate::render::draw_cmd::Object3d>,
    Vec<crate::render::draw_cmd::Object3d>,
    Option<(f32, f32, f32, f32)>,
) {
    use super::tooltip::push_tooltip;
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
    // consumable inventory now lives on a brass `DishExplicit` with
    // `TalismanBatch` pendants for each filled slot. The text labels
    // and click handlers stay at the same screen positions so hover +
    // input plumbing is unchanged.
    let consumables = &interaction.consumables;
    let consumable_capacity = interaction.consumable_capacity;
    let mut talisman_dish_placements: Vec<Object3d> = Vec::new();
    let mut ribbon_dish_placements: Vec<Object3d> = Vec::new();
    let mut talisman_dish_strip: Option<(f32, f32, f32, f32)> = None;
    if consumable_capacity > 0 {
        // The brass dish is a 3D object — use base resolution scale
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
        // The brass dish gets projected through the gameplay camera
        // to a different on-screen position than its raw pixel anchor.
        // We look up the previous frame's projected dish rect and
        // remap each per-slot rect through the same affine transform
        // (treating the dish as roughly planar). This keeps the
        // tooltip hit-test, focus ring, click target, and tooltip
        // anchor in lockstep with the visible pendant.
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
                        let proj_rect = ctx
                            .proj
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
                    crate::core::consumable::Consumable::Talisman(tk) => match tk {
                        crate::core::talisman::TalismanKind::Jade => [0.42, 0.82, 0.55, 1.0],
                        crate::core::talisman::TalismanKind::Pearl => [0.94, 0.95, 0.98, 1.0],
                        crate::core::talisman::TalismanKind::Gilded => [0.96, 0.78, 0.30, 1.0],
                        crate::core::talisman::TalismanKind::Polychrome => {
                            [0.82, 0.55, 0.95, 1.0]
                        }
                        crate::core::talisman::TalismanKind::Kiln => [0.85, 0.35, 0.18, 1.0],
                        crate::core::talisman::TalismanKind::Bamboo => [0.06, 0.55, 0.28, 1.0], // emerald
                        crate::core::talisman::TalismanKind::Dots => [0.08, 0.22, 0.78, 1.0], // sapphire
                        crate::core::talisman::TalismanKind::Characters => {
                            [0.82, 0.08, 0.18, 1.0]
                        } // ruby
                        crate::core::talisman::TalismanKind::Honors => [0.78, 0.64, 0.28, 1.0],
                        crate::core::talisman::TalismanKind::Wildflower => {
                            [0.92, 0.48, 0.62, 1.0]
                        }
                        crate::core::talisman::TalismanKind::Conformity => {
                            [0.62, 0.60, 0.68, 1.0]
                        }
                    },
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
                        // Ribbon thickness = width × 0.15 (set by the
                        // renderer); bump width to mm(12) so the silk
                        // reads as ~1.8mm thick at table scale.
                        let ribbon_w = layout.mm(12.0);
                        ribbon_dish_placements.push(Object3d {
                            pos: [zx + slot_w * 0.5, zy, pendant_y],
                            extents: [ribbon_w, slot_h * 0.85, ribbon_w * 0.15],
                            rotation: crate::render::table_transform::rot_rz_ry_rx_deg(
                                -90.0, 0.0, 0.0,
                            ),
                            color: [1.0, 1.0, 1.0, 1.0],
                            kind: Object3dKind::ZodiacRibbon { kind: Some(z) },
                            hover_target: 0.0,
                            anim_id: 0,
                            arrange_name: None,
                        });
                    }
                    crate::core::consumable::Consumable::Talisman(tk) => {
                        let talisman_half_height = slot_w * 0.55 * 0.5;
                        let anchor = crate::ui::placement::PlacementAnchor::new(
                            [
                                zx + slot_w * 0.5,
                                zy + slot_h * 0.5,
                                pendant_y + talisman_half_height,
                            ],
                            crate::render::table_transform::rot_rz_ry_rx_deg(0.0, 0.0, 90.0),
                            &scene.positions.consumable_dish_talisman,
                            "gameplay.consumable_dish.talisman",
                            layout,
                        );
                        talisman_dish_placements.push(Object3d {
                            pos: anchor.pos,
                            extents: [slot_h * 0.85, slot_h * 0.85 * 1.4, slot_h * 0.85 * 0.25],
                            rotation: anchor.rotation,
                            color: pendant_color,
                            kind: Object3dKind::Talisman { kind: tk },
                            hover_target: 0.0,
                            anim_id: 0,
                            arrange_name: Some(anchor.arrange_name),
                        });
                    }
                }
                // The persistent on-slot labels (name + sub) are
                // gone — the brass dish + colored pendant are the
                // visual representation, and the hover tooltip below
                // supplies the full name/description on demand.
                let (tooltip_title, tooltip_body) = match item {
                    crate::core::consumable::Consumable::Zodiac(z) => {
                        let level = GameEngine::read_yaku_progress(run).level_of(z.yaku());
                        (
                            format!("{} (Zodiac)", z.name()),
                            format!(
                                "Click or press to use. Permanently raises {} from level {} to {} for the rest of the run (+0.5 mult, +20 chips per level).",
                                z.yaku().name(),
                                level,
                                level + 1,
                            ),
                        )
                    }
                    crate::core::consumable::Consumable::Talisman(t) => (
                        format!("{} (Talisman)", t.name()),
                        format!("Click or press to use. {}", t.description()),
                    ),
                };
                if !paused {
                    buttons.push(crate::scenes::ButtonDef::scene(
                        (fx, fy, fw, fh),
                        ZODIAC_USE_BASE + slot_idx as u32,
                    ));
                }
                // Tooltip is now driven by `self.focus`. In cursor
                // mode the Phase A sync in `update()` writes
                // `FocusTarget::Consumable(i)` whenever the cursor is
                // over a slot rect; in keyboard / controller mode the
                // player navigates here with spatial nav.
                if scene.focus == Some(FocusTarget::Consumable(slot_idx)) {
                    push_tooltip(
                        hover_quads,
                        hover_text,
                        (fx + fw * 0.5, fy),
                        crate::ui::layout::ViewportCtx {
                            window_w: layout.window_w,
                            window_h: layout.window_h,
                            ui_scale: ctx.ui_scale,
                        },
                        &tooltip_title,
                        &tooltip_body,
                    );
                }
            }
        }
    }

    (
        talisman_dish_placements,
        ribbon_dish_placements,
        talisman_dish_strip,
    )
}

/// Outputs of the action row + journal book builder.
pub(super) struct ActionRowOutputs {
    pub(super) wood_tablet_placements: Vec<crate::render::draw_cmd::Object3d>,
    pub(super) discard_bowl_placement: Option<crate::render::draw_cmd::Object3d>,
    pub(super) bronze_mirror_placement: Option<crate::render::draw_cmd::Object3d>,
    pub(super) journal_pick_idx: usize,
}

/// Build the action-row 3D objects (sort tablets, discard bowl, bronze
/// mirror, optional cash-in tablet) plus the Yaku Journal book tablet.
/// Pushes button focus rects into `focus_rect_graph` and hover labels
/// into `hover_text`. Behaviour is a verbatim lift of the inline
/// `draw_frame` chunk; relocated for organisation.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_action_row_and_journal(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    ctx: &crate::scenes::DrawCtx<'_>,
    gameplay: &crate::game::engine::GameplayReadModel,
    btn_rects: &[(f32, f32, f32, f32); 5],
    rank_btn_rect: (f32, f32, f32, f32),
    journal_btn_cx: f32,
    journal_btn_w: f32,
    action_world_z_py: f32,
    action_hud_table_lift: f32,
    cam_rot: glam::Mat4,
    has_structure: bool,
    play_enabled: bool,
    discard_enabled: bool,
    now: Instant,
    focus_rect_graph: &mut Vec<(FocusTarget, [f32; 4])>,
    hover_text: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
) -> ActionRowOutputs {
    use super::focus::ALL_BUTTONS;
    use crate::render::draw_cmd::{Object3d, Object3dKind};
    use crate::render::wgpu_renderer::{TextAlign, TextLabel};
    use crate::render::world_space::LayoutAnchorPx;
    use crate::ui::focus_nav::clamp_rect_to_viewport;
    // Phase 4: action row is now physical objects.
    //   - Sort by Suit / Sort by Rank → carved wood tablets
    //   - Discard / Play              → bowl + mirror (row below hand, above sort;
    //                                    mirror play left, discard bowl right)
    // The flat slate-blue button background quads are gone; only the
    // focus-highlight border remains as a 2D affordance for keyboard
    // navigation.
    let mut wood_tablet_placements: Vec<Object3d> = Vec::new();
    let mut discard_bowl_placement: Option<Object3d> = None;
    let mut bronze_mirror_placement: Option<Object3d> = None;
    // `action_hud_table_lift`: third component of [`crate::render::draw_cmd::WorldSurfaceAnchor`]
    // (height above felt); set in [`action_bar_layout::compute_action_bar`].
    for (i, &(bx, by, bw, bh)) in btn_rects.iter().enumerate() {
        if i == 4 && !has_structure {
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
            GameplayButton::SortSuit => ctx.proj.wood_tablet_rects.first().copied(),
            GameplayButton::SortRank => ctx.proj.wood_tablet_rects.get(1).copied(),
            GameplayButton::Discard => ctx.proj.bowl_rect,
            GameplayButton::Play => ctx.proj.mirror_rect,
            GameplayButton::Trigger => ctx.proj.wood_tablet_rects.get(2).copied(),
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
            0 => matches!(
                pick,
                Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(0)),
            ),
            1 => matches!(
                pick,
                Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(1)),
            ),
            2 => {
                matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::DiscardBowl),
                ) || focused_btn == Some(GameplayButton::Discard)
            }
            3 => {
                matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::BronzeMirror),
                ) || focused_btn == Some(GameplayButton::Play)
            }
            4 => {
                matches!(
                    pick,
                    Some(crate::render::wgpu_renderer::GameplayPick::WoodTablet(2)),
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
        let tablet_thickness = (bh * 0.35).max(8.0);
        match i {
            0 | 1 => {
                let (label, tp) = match i {
                    0 => ("Sort by Suit", &scene.positions.tablet_sort_suit),
                    _ => ("Sort by Rank", &scene.positions.tablet_sort_rank),
                };
                let _tablet_idx = wood_tablet_placements.len();
                let anchor = action_anchor.to_draw_cmd_triple();
                wood_tablet_placements.push(Object3d {
                    pos: [
                        anchor[0] + tp.nx * layout.window_w,
                        anchor[1] + tp.ny * layout.window_h,
                        anchor[2] + layout.mm(tp.lift_mm),
                    ],
                    extents: [bw, tablet_thickness, bh],
                    // Placement rotation applied centrally via
                    // `committed_arrange_rotations`.
                    rotation: cam_rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::WoodTablet {
                        label: label.to_string(),
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
                // Gold overlay label superimposed on the sort tablet
                // when it's the active selection. Anchored on the
                // The label is engraved directly on the wood tablet
                // via a per-instance decal texture — no 2D overlay.
            }
            2 => {
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
                    rotation: glam::Mat4::from_rotation_x(90.0_f32.to_radians()),
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Bowl,
                    hover_target: target,
                    anim_id: 1,
                    arrange_name: None,
                });
                // Gold "Discard tiles" label superimposed on the river
                // when it's the active selection (cursor hover or
                // keyboard focus). Anchored purely on the renderer's
                // projected mesh rect — no layout-rect fallback, so
                // on the very first frame after a scene transition
                // the label briefly doesn't appear.
                if let Some(r) = ctx.proj.bowl_rect.filter(|_| hovered) {
                    let label_h = (r[3] * 0.38).max(28.0);
                    let label_rect = [r[0], r[1] + r[3] * 0.5 - label_h * 0.5, r[2], label_h];
                    if let Some(clamped) =
                        clamp_rect_to_viewport(label_rect, layout.window_w, layout.window_h)
                    {
                        hover_text.push(TextLabel {
                            rect: clamped,
                            text: "Discard tiles".to_string(),
                            color: [1.0, 0.84, 0.40, 1.0],
                            align: TextAlign::Center,
                            no_glossary: true,
                            ..Default::default()
                        });
                    }
                }
            }
            3 => {
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
                    rotation: glam::Mat4::IDENTITY,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::Mirror {
                        rotation_x_deg: 90.0,
                        rotation_z_deg: 0.0,
                    },
                    hover_target: target,
                    anim_id: 2,
                    arrange_name: None,
                });
                // Gold "Play hand" label superimposed on the mirror
                // when it's the active selection. Same projected-mesh
                // anchoring as the river label above (no layout-rect
                // fallback).
                if let Some(r) = ctx.proj.mirror_rect.filter(|_| hovered) {
                    let label_h = (r[3] * 0.38).max(28.0);
                    let mirror_label = if gameplay.uses_structure_bank {
                        "Commit melds"
                    } else {
                        "Score hand"
                    };
                    let label_rect = [r[0], r[1] + r[3] * 0.5 - label_h * 0.5, r[2], label_h];
                    if let Some(clamped) =
                        clamp_rect_to_viewport(label_rect, layout.window_w, layout.window_h)
                    {
                        hover_text.push(TextLabel {
                            rect: clamped,
                            text: mirror_label.to_string(),
                            color: [1.0, 0.84, 0.40, 1.0],
                            align: TextAlign::Center,
                            no_glossary: true,
                            ..Default::default()
                        });
                    }
                }
            }
            4 => {
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
                wood_tablet_placements.push(Object3d {
                    pos: [
                        anchor[0] + tp.nx * layout.window_w,
                        anchor[1] + tp.ny * layout.window_h,
                        anchor[2] + layout.mm(tp.lift_mm),
                    ],
                    extents: [bw, tablet_thickness, bh],
                    // Placement rotation applied centrally via
                    // `committed_arrange_rotations`.
                    rotation: wiggle * cam_rot,
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::WoodTablet {
                        label: "Cash in".to_string(),
                        pick_id: None,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
                if let Some(r) = ctx.proj.wood_tablet_rects.get(2).filter(|_| hovered) {
                    let label_h = (r[3] * 0.38).max(28.0);
                    let label_rect = [r[0], r[1] + r[3] * 0.5 - label_h * 0.5, r[2], label_h];
                    if let Some(clamped) =
                        clamp_rect_to_viewport(label_rect, layout.window_w, layout.window_h)
                    {
                        hover_text.push(TextLabel {
                            rect: clamped,
                            text: "Cash in structure (T)".to_string(),
                            color: [1.0, 0.84, 0.40, 1.0],
                            align: TextAlign::Center,
                            no_glossary: true,
                            ..Default::default()
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // Yaku Journal book — an additional wood-tablet placement reusing
    // the existing wood-tablet pipeline + pick path so we don't have
    // to plumb a new mesh through the renderer just for the book.
    // Sits in the *bottom action row* to the right of the two sort
    // tablets (bowl/mirror sit in the row above). Clicking it
    // pushes the YakuJournalScene — the click is dispatched in
    // `update()` via `GameplayPick::WoodTablet(journal_pick_idx)` —
    // journal is slot 3 when the cash-in tablet is present (slot 2),
    // otherwise slot 2 when there is no committed structure to cash in.
    let journal_pick_idx = wood_tablet_placements.len();
    // The journal "lights up" on either cursor pick or keyboard focus,
    // matching how the other action buttons treat hover. The
    let (_, rby, _, rbh) = rank_btn_rect;
    let book_w = journal_btn_w;
    let book_h = rbh * 0.95;
    // Journal is the third button in the centered sort row — position
    // comes directly from action_bar_layout's centered group calculation.
    let book_cy = rby + rbh * 0.5;
    let book_cx = journal_btn_cx;
    let book_thickness = (book_h * 0.45).max(8.0);
    let journal_anchor = LayoutAnchorPx {
        px: book_cx,
        py: book_cy + action_world_z_py,
        lift_z: action_hud_table_lift,
    }
    .to_draw_cmd_triple();
    let tp = &scene.positions.tablet_journal;
    wood_tablet_placements.push(Object3d {
        pos: [
            journal_anchor[0] + tp.nx * layout.window_w,
            journal_anchor[1] + tp.ny * layout.window_h,
            journal_anchor[2] + layout.mm(tp.lift_mm),
        ],
        extents: [book_w, book_thickness, book_h],
        // Placement rotation applied centrally via
        // `committed_arrange_rotations`.
        rotation: cam_rot,
        color: [1.0, 1.0, 1.0, 1.0],
        kind: Object3dKind::WoodTablet {
            label: "Journal".to_string(),
            pick_id: None,
        },
        hover_target: 0.0,
        anim_id: 0,
        arrange_name: None,
    });
    // Anchor the Journal button's keyboard-nav focus rect on the
    // renderer's projected wood-tablet rect for the journal slot.
    // Same one-frame stale snapshot pattern as the other action
    // buttons; first-frame absence is harmless.
    if let Some(&rect) = ctx.proj.wood_tablet_rects.get(journal_pick_idx) {
        focus_rect_graph.push((FocusTarget::Button(GameplayButton::Journal), rect));
    }
    // The Journal label is engraved directly on the wood tablet
    // via a per-instance decal texture — no 2D overlay.

    ActionRowOutputs {
        wood_tablet_placements,
        discard_bowl_placement,
        bronze_mirror_placement,
        journal_pick_idx,
    }
}

/// Build the per-frame tile hover tooltip — show tile name, base/effective
/// chips, mult bonus, source breakdown, flower effect, selection state.
/// Suppressed during cascade, while paused, and when the cursor is over
/// any 2D UI element. Behaviour is a verbatim lift of the inline
/// `draw_frame` chunk; relocated for organisation.
#[allow(clippy::too_many_arguments)]
pub(super) fn build_tile_hover_tooltip(
    scene: &GameplayScene,
    layout: &crate::ui::layout::LayoutResult,
    run: &crate::game::run::RunState,
    ctx: &crate::scenes::DrawCtx<'_>,
    gameplay: &crate::game::engine::GameplayReadModel,
    interaction: &crate::game::engine::GameplayInteractionReadModel,
    hand_slots: &[(f32, f32, f32, f32)],
    scale: f32,
    hovered_yaku: bool,
    buttons: &[crate::scenes::ButtonDef],
    hover_quads: &mut Vec<crate::render::wgpu_renderer::GpuInstance>,
    hover_text: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
) {
    use crate::render::wgpu_renderer::{GpuInstance, TextLabel};
    // Tile hover tooltip — show full info for the tile under the cursor.
    // Anchored to the perspective-projected tile rect (one frame stale,
    // supplied by the renderer) so it tracks the actual visible tile
    // position rather than the flat layout slot. Falls back to the slot
    // rect on the very first frame before the renderer has projected.
    // Suppressed during cascade and while the pause menu is open.
    //
    // Also suppressed when the cursor is over any 2D UI element (a
    // button or a hovered yaku card). The 3D raycast pick happily
    // intersects a hand tile even when the cursor sits visually atop a
    // floating UI panel above the hand, which used to surface the tile
    // tooltip + ▼ pointer underneath the panel's own tooltip. Gating
    // here keeps a single tooltip on screen at a time.
    let cursor_over_ui = {
        let (cx, cy) = scene.cursor_pos;
        let in_button = buttons.iter().any(|b| {
            let (bx, by, bw, bh) = b.rect;
            cx >= bx && cx <= bx + bw && cy >= by && cy <= by + bh
        });
        in_button || hovered_yaku
    };
    if scene.cascade_queue.is_empty() && !scene.pause_menu.paused && !cursor_over_ui {
        // The tile tooltip now follows the unified focus model: it
        // shows whenever `self.focus` points at a hand tile. In
        // cursor mode the Phase A cursor sync in `update()` writes
        // `FocusTarget::HandTile(i)` whenever the cursor's raycast
        // pick lands on a tile, so this naturally collapses hover and
        // controller-focus into one path.
        let hovered_idx: Option<usize> = match scene.focus {
            Some(FocusTarget::HandTile(i)) if interaction.hand_len > 0 => {
                Some(i.min(interaction.hand_len - 1))
            }
            _ => None,
        };

        if let Some(idx) = hovered_idx
            && let Some(&raw_tile) = interaction.hand.get(idx)
        {
            let tile = GameplayScene::display_tile(raw_tile, run);
            // Resolve the anchor rect: prefer the projected rect for
            // this index, otherwise the flat slot rect.
            let anchor: (f32, f32, f32, f32) = ctx
                .proj
                .hand_rects
                .iter()
                .find(|(i, _)| *i == idx)
                .map(|(_, r)| (r[0], r[1], r[2], r[3]))
                .or_else(|| hand_slots.get(idx).copied())
                .unwrap_or((0.0, 0.0, 0.0, 0.0));
            let (ax, ay, aw, ah) = anchor;

            // ── Build the lines ───────────────────────────────
            let lines: Vec<String> = {
                // Show the tile's *effective* value: base point worth
                // plus every per-tile bonus that doesn't depend on the
                // surrounding meld structure (talisman enhancements,
                // dora, owned chip relics). The total chips line is the
                // headline; the per-source breakdown follows so the
                // player can see *why* the tile is worth what it is.
                let dora_faces = gameplay.dora_faces.clone();
                let eff = GameEngine::tile_effective_value(run, &tile, &dora_faces);
                let name = tile.full_name();
                let category = tile.category();
                let is_selected = interaction.selected.get(idx).copied().unwrap_or(false);

                let mut v: Vec<String> = Vec::new();
                v.push(name);
                if eff.bonus_chips != 0 || eff.mult_bonus != 0.0 {
                    // Effective chips (base + bonuses).
                    v.push(format!(
                        "{category} · {} pts (base {})",
                        eff.total_chips(),
                        eff.base_chips,
                    ));
                } else {
                    v.push(format!("{category} · {} pts", eff.base_chips));
                }
                if eff.mult_bonus != 0.0 {
                    v.push(format!("+{:.1} mult", eff.mult_bonus));
                }
                for (src, body) in &eff.sources {
                    v.push(format!("{src}: {body}"));
                }
                if let Some(fx) = tile.flower_effect_label() {
                    v.push(format!("flower: {fx}"));
                }
                if is_selected {
                    v.push("selected".to_string());
                }
                v
            };

            // ── Geometry ──────────────────────────────────────
            // Floors mirror the glossary tooltip in `ui::tooltip` so
            // small windows and low `ui_scale` don't squish text below
            // legible size.
            let line_h = (18.0 * scale).max(20.0);
            let pad_x = (8.0 * scale).max(8.0);
            let pad_y = (6.0 * scale).max(8.0);
            let char_px = (7.5 * scale).max(6.0);
            let widest = lines.iter().map(|s| s.chars().count()).max().unwrap_or(0) as f32;
            let tw = (widest * char_px + pad_x * 2.0).max(200.0);
            let th = line_h * lines.len() as f32 + pad_y * 2.0;

            // Position: below the anchor's bottom edge (the tile
            // face is nearest camera at the AABB bottom, so placing
            // the tooltip above the AABB would land on the face).
            // Flip above if there isn't room below.
            let mut tx = ax + (aw - tw) * 0.5;
            let mut ty = ay + ah + 6.0 * scale;
            if ty + th > layout.window_h - 4.0 {
                ty = ay - th - 6.0 * scale;
            }
            if ty < 4.0 {
                ty = 4.0;
            }
            if tx + tw > layout.window_w - 4.0 {
                tx = layout.window_w - tw - 4.0;
            }
            if tx < 4.0 {
                tx = 4.0;
            }

            // Background. Pushed into the hover layer so the
            // tooltip BG always lands ABOVE the persistent HUD
            // text labels (this is the structural fix).
            hover_quads.push(GpuInstance {
                rect: [tx, ty, tw, th],
                color: [0.06, 0.06, 0.12, 0.95],
            });
            // Gold border.
            let bc = [0.65, 0.55, 0.25, 0.85];
            let b = 1.5;
            hover_quads.push(GpuInstance {
                rect: [tx, ty, tw, b],
                color: bc,
            });
            hover_quads.push(GpuInstance {
                rect: [tx, ty + th - b, tw, b],
                color: bc,
            });
            hover_quads.push(GpuInstance {
                rect: [tx, ty, b, th],
                color: bc,
            });
            hover_quads.push(GpuInstance {
                rect: [tx + tw - b, ty, b, th],
                color: bc,
            });

            // Text lines. First line uses the suit colour as a
            // visual cue (matches the new dragon-by-rank palette);
            // subsequent lines are the standard ivory-gold.
            let suit_rgba = tile.suit_color();
            let title_color = [
                (suit_rgba[0] * 0.6 + 0.4).min(1.0),
                (suit_rgba[1] * 0.6 + 0.4).min(1.0),
                (suit_rgba[2] * 0.6 + 0.4).min(1.0),
                1.0,
            ];
            let body_color = [0.95, 0.85, 0.4, 1.0];
            for (i, line) in lines.into_iter().enumerate() {
                let color = if i == 0 { title_color } else { body_color };
                hover_text.push(TextLabel {
                    rect: [
                        tx + pad_x,
                        ty + pad_y + i as f32 * line_h,
                        tw - pad_x * 2.0,
                        line_h,
                    ],
                    text: line,
                    color,
                    ..Default::default()
                });
            }
        }
    }
}

/// Outputs of the yaku panel + structure showcase + yaku tablet builder.
pub(super) struct YakuPanelOutputs {
    pub(super) yaku_preview_effective_tiles: Vec<crate::core::tile::Tile>,
    pub(super) yaku_preview_sets: Vec<crate::core::hand::DetectedSet>,
    pub(super) is_chicken_hand: bool,
    pub(super) hovered_yaku_kind: Option<crate::core::yaku::YakuKind>,
    pub(super) yaku_tablet_placements: Vec<crate::render::draw_cmd::Object3d>,
    pub(super) structure_showcase: Vec<crate::render::draw_cmd::ShowcaseTilePlacement>,
    pub(super) structure_pile_tokens: Vec<crate::render::draw_cmd::Object3d>,
    pub(super) cam_rot: glam::Mat4,
    pub(super) visible_previews_kinds: Vec<crate::core::yaku::YakuKind>,
}

/// Build the yaku progress panel (previews, structure showcase tiles,
/// preview piles), the yaku tablet placements, and the yaku/chicken-hand
/// hover tooltips. Behaviour is a verbatim lift of the inline `draw_frame`
/// chunk; relocated for organisation.
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
    hover_quads: &mut Vec<crate::render::wgpu_renderer::GpuInstance>,
    hover_text: &mut Vec<crate::render::wgpu_renderer::TextLabel>,
) -> YakuPanelOutputs {
    use super::cascade_hud::{
        PreviewPilePlacement, push_structure_preview_pile, structure_preview_chip_stack_count,
        structure_preview_mult_stack_count,
    };
    use super::tooltip::{push_tooltip, yaku_card_shape_text};
    use crate::core::yaku::yaku_preview;
    use crate::render::draw_cmd::{
        CameraParams, CascadeTokenKind, Object3d, Object3dKind, ShowcaseTilePlacement,
        camera_facing_rotation,
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
    let wildcard_result = if selected_tiles_for_yaku.is_empty() {
        None
    } else {
        GameEngine::validate_with_wildcards(run, &selected_tiles_for_yaku)
    };
    let mut yaku_preview_original_tiles: Vec<crate::core::tile::Tile> = Vec::new();
    let mut yaku_preview_effective_tiles: Vec<crate::core::tile::Tile> = Vec::new();
    let mut yaku_preview_sets: Vec<crate::core::hand::DetectedSet> = Vec::new();

    if gameplay.uses_structure_bank {
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
    } else if let Some((selected_sets, selected_scoring_tiles)) = wildcard_result.as_ref() {
        yaku_preview_original_tiles =
            GameplayScene::display_tiles(selected_tiles_for_yaku.iter().copied(), run);
        yaku_preview_effective_tiles =
            GameplayScene::display_tiles(selected_scoring_tiles.iter().copied(), run);
        yaku_preview_sets = selected_sets.clone();
    }

    let previews = if yaku_preview_sets.is_empty() {
        Vec::new()
    } else {
        yaku_preview(
            &yaku_preview_original_tiles,
            &gameplay.available_yaku,
            round_wind_for_yaku,
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
                    center_pos: [px, center_py, 3.0 + lift],
                    rotation: [0.0, 0.0, std::f32::consts::PI],
                    scale,
                    size_px: tile_size,
                    brightness,
                    selected: false,
                    hovered: false,
                    outline: false,
                    glow: false,
                    glow_color: None,
                    pick_id: None,
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
                0xA17E_0000_u64 ^ preview_mult.to_bits(),
            );
        }
    }

    // Captured during the loop below — `(yaku_kind, anchor_x, anchor_y)`
    // for the card the cursor is currently hovering, if any. The tooltip
    // is pushed into the *hover layer* after the loop completes so it
    // draws on top of every yaku card regardless of which one captured it.
    let mut hovered_yaku: Option<(crate::core::yaku::YakuKind, f32, f32)> = None;
    let mut hovered_chicken: Option<(f32, f32)> = None;

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
    let cam_rot = {
        let cam = CameraParams::default_table_camera(layout.window_h);
        camera_facing_rotation(cam.eye, cam.target)
    };
    // Yaku-tablet placement rotation (rx/ry/rz_deg) is applied centrally
    // by the renderer via `committed_arrange_rotations`; only the
    // camera-facing orientation is baked into the base matrix here.
    let yaku_tablet_rot = cam_rot;
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
                    label: "\u{1F414} Chicken Hand".to_string(),
                    active: true,
                    hover: if hovered_now { 1.0 } else { 0.0 },
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
            if hovered_now {
                let (ax, ay) = match ctx.proj.yaku_tablet_rects.first().copied() {
                    Some([px, py, pw, _ph]) if pw > 0.0 && px.is_finite() && py.is_finite() => {
                        (px + pw * 0.5, py)
                    }
                    _ => (center_px, panel_y),
                };
                hovered_chicken = Some((ax, ay));
            }
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
                        label: p.kind.name().to_string(),
                        active: p.active,
                        hover: if hovered_now { 1.0 } else { 0.0 },
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
                // The yaku name is now engraved directly on the bone tablet
                // via a per-instance decal texture (see the renderer's tablet
                // pass), so no 2D text overlay is pushed here.

                // Hover tracking for the tooltip pass below the loop.
                // Anchor the tooltip to the *projected* on-screen rect so
                // it pops up next to the tablet the player can actually
                // see — falls back to the input pixel rect on the first
                // frame before projection data is available.
                if hovered_now {
                    let (ax, ay) = match ctx.proj.yaku_tablet_rects.get(i).copied() {
                        Some([px, py, pw, _ph])
                            if pw > 0.0 && px.is_finite() && py.is_finite() =>
                        {
                            (px + pw * 0.5, py)
                        }
                        _ => (center_px, cy),
                    };
                    hovered_yaku = Some((p.kind, ax, ay));
                }
            }
        }
    }
    // Yaku card hover tooltip — pushed into the *hover layer* so its
    // background quad lands AFTER every persistent HUD text label, which
    // is the structural fix for the legacy "tooltip BG renders under
    // parent text" bug class.
    if let Some((yk, ax, ay)) = hovered_yaku {
        let title = format!(
            "{}  (+{} mult, +{} chips)",
            yk.name(),
            yk.mult_bonus(),
            yk.chip_bonus()
        );
        let body = yaku_card_shape_text(yk).to_string();
        push_tooltip(
            hover_quads,
            hover_text,
            (ax, ay),
            crate::ui::layout::ViewportCtx {
                window_w: layout.window_w,
                window_h: layout.window_h,
                ui_scale: ctx.ui_scale,
            },
            &title,
            &body,
        );
    }
    if let Some((ax, ay)) = hovered_chicken {
        push_tooltip(
            hover_quads,
            hover_text,
            (ax, ay),
            crate::ui::layout::ViewportCtx {
                window_w: layout.window_w,
                window_h: layout.window_h,
                ui_scale: ctx.ui_scale,
            },
            "\u{1F414} Chicken Hand",
            "A valid hand with no yaku. Scores base chips \u{00D7} 1 mult. \
             Build toward a yaku to multiply your score.",
        );
    }

    let visible_previews_kinds: Vec<crate::core::yaku::YakuKind> =
        visible_previews.iter().map(|p| p.kind).collect();
    YakuPanelOutputs {
        yaku_preview_effective_tiles,
        yaku_preview_sets,
        is_chicken_hand,
        hovered_yaku_kind: hovered_yaku.map(|(yk, _, _)| yk),
        yaku_tablet_placements,
        structure_showcase,
        structure_pile_tokens,
        cam_rot,
        visible_previews_kinds,
    }
}
