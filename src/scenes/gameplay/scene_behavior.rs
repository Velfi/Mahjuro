use super::focus::{GameplayButton, ScoreRollerBank};
use super::*;
use super::{animation_state, cascade_controller, input_handler};
use crate::core::consumable::Consumable;
use crate::core::relic::{RelicId, all_relic_defs, relic_description_live};
use crate::render::theme::color;
use crate::render::wgpu_renderer::{GpuInstance, GradientQuadInstance, TextLabel};
use crate::scenes::options;
use crate::scenes::{BackgroundId, GuideScene, OverlayRequest};
use crate::ui::controller_hints::{
    HintStyle, InlineHintIconSlot, gameplay_footer_row, is_cash_in_hint_key,
    push_screen_footer_hint,
};
use crate::ui::inspect_plaque::{
    FocusTooltipPanelParams, dora_focus_tooltip_strings, gameplay_consumable_description_full,
    hand_tile_focus_tooltip, push_focus_tooltip_panel_2d, round_wind_focus_tooltip_strings,
};
use crate::ui::score_format::format_score;
fn plinth_focus_rect_from_anchor(
    anchor: &[f32; 3],
    layout: &crate::ui::layout::LayoutResult,
    tile_count: usize,
) -> [f32; 4] {
    let spacing = layout.mm(24.0);
    let tile_w = layout.mm(22.0);
    let strip_w = if tile_count >= 2 {
        spacing + tile_w
    } else {
        tile_w
    };
    let strip_h = layout.mm(30.0);
    [
        anchor[0] - strip_w * 0.5,
        anchor[1] - strip_h * 0.5,
        strip_w,
        strip_h,
    ]
}

fn plinth_focus_rect(
    projected: Option<[f32; 4]>,
    anchor: &[f32; 3],
    layout: &crate::ui::layout::LayoutResult,
    tile_count: usize,
) -> [f32; 4] {
    if let Some(r) = projected
        && r[2] > 1.0
        && r[3] > 1.0
        && r[0].is_finite()
        && r[1].is_finite()
    {
        return r;
    }
    plinth_focus_rect_from_anchor(anchor, layout, tile_count)
}

impl SceneBehavior for GameplayScene {
    /// Borrow the in-pause-menu options overlay, if the player has opened it.
    /// Used by the main loop to sync settings (volume, tile preset)
    /// the same way it does for the standalone `OptionsScene`.
    fn pause_options_overlay(&self) -> Option<&options::OptionsScene> {
        self.pause_menu.options_overlay()
    }

    /// See [`crate::scenes::SceneBehavior::has_blocking_overlay`]. Reports
    /// `true` when any in-scene modal-like overlay is up: pause menu
    /// or the scoring cascade animation. The cascade is included
    /// because it already blocks input internally — declaring it here also
    /// kills hover tooltips on hand tiles and relics during the score reveal.
    /// (The Yaku Journal is now its own pushdown scene; when it is open,
    /// this scene isn't ticking so no separate check is needed here.)
    fn has_blocking_overlay(&self) -> bool {
        self.pause_menu.paused || !self.cascade_queue.is_empty()
    }

    fn face_button_bindings(
        &self,
        ctx: crate::ui::input::FaceBindingCtx,
    ) -> crate::ui::input::FaceButtonBindings {
        if ctx.xy_quick_action {
            crate::ui::input::FaceButtonBindings {
                west_press: Some(crate::ui::input::UiAction::WestFacePress),
                north_press: Some(crate::ui::input::UiAction::NorthFacePress),
                ..Default::default()
            }
        } else {
            crate::ui::input::FaceButtonBindings {
                west_press: Some(crate::ui::input::UiAction::FocusDiscardButton),
                north_press: Some(crate::ui::input::UiAction::FocusPlayButton),
                ..Default::default()
            }
        }
    }

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.cached_cascade_tuning = ctx.cascade_tuning.clone();
        let focus_kind_before = focus_kind(self.focus);
        {
            let _g = crate::render::cpu_profiler::scope("update.tick_basic_animations");
            animation_state::tick_basic_animations(self, &mut ctx, now, dt);
        }
        discard_animation::tick_discard_animation(self, now, ctx.cascade_tuning);
        // Cursor position is captured every frame for cursor-mode hit-test
        // and tooltip placement. The legacy `cursor_moved` guard that used
        // to drop stale controller focus on mouse motion is gone — Phase A
        // of the unified focus model (further down) overwrites `self.focus`
        // from the cursor each frame in cursor mode, which handles the
        // same race (mouse click on tile while controller focus was on a
        // consumable) without the heuristic.
        self.cursor_pos = ctx.cursor_pos;

        {
            let _g = crate::render::cpu_profiler::scope("update.tick_deal_detection");
            animation_state::tick_deal_detection(self, &mut ctx, now);
        }
        {
            let _g = crate::render::cpu_profiler::scope("update.tick_gold_change_coins");
            animation_state::tick_yen_change_coins(self, &mut ctx);
        }
        {
            let _g = crate::render::cpu_profiler::scope("update.tick_candle_and_light_ramp");
            animation_state::tick_candle_and_light_ramp(self, now, dt);
        }

        // Deferred round start: fire `apply_chamber` once the opening candle
        // light-ramp has hit full brightness. The round's hand deal, boss
        // rules, and on-round-start relic triggers (Sweepstakes coin shower,
        // DoraCrown extra dora, future hooks) all happen now instead of
        // before the scene rendered — the player sees them unfold as the
        // transition clears.
        if let Some(blind) = self.pending_chamber {
            // Keep the table empty behind the opening transition. Paths
            // that land here may have left a stale hand on the run state
            // (first-round `RunState::new` pre-draws, tutorial retry re-deals
            // before transitioning), and `apply_chamber` will do the real
            // deal once the ramp completes.
            GameEngine::prepare_pending_chamber(ctx.run);
            if self.light_ramp >= 1.0 {
                let mut engine = GameEngine::new(ctx.run, ctx.bus);
                let _ = engine.dispatch(GameCommand::ApplyChamber { blind });
                if ctx.run.onboarding_lessons_active() {
                    ctx.run.seed_onboarding_hand();
                }
                self.pending_chamber = None;
            }
        }

        onboarding_hints::sync_onboarding_step(ctx.run);
        self.boss_rule_feedback_live = ctx.run.hand_selection_blocked_by_boss();

        // Scene transition in progress — keep animations running but block
        // all input so the player can't alter game state during the fade-out.
        // Also block while `pending_chamber` is set: the scene is rendering the
        // previous round's state behind the opening transition, and any
        // input would act on that stale state instead of the round that's
        // about to start.
        if ctx.transitioning || self.pending_chamber.is_some() {
            return None;
        }

        if input_handler::tick_gameplay_journal_transition(self, &mut ctx, now, dt) {
            return None;
        }

        // Help action opens the Guide scene (replaces the old glossary overlay).
        for &cid in ctx.button_clicks {
            if cid == HELP_BADGE_ID {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Guide(
                    GuideScene::new(),
                ))));
                return None;
            }
        }
        for a in ctx.actions {
            if matches!(a, UiAction::Help) {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Guide(
                    GuideScene::new(),
                ))));
                return None;
            }
        }

        // Pause menu handling — drives the menu while paused and intercepts
        // the open-on-Pause shortcut. Returns immediately if either applies.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            if self.pause_menu.take_credits_request() {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Credits(
                    crate::scenes::CreditsScene::overlay(),
                ))));
                return None;
            }
            return t;
        }

        if let Some(t) = cascade_controller::tick_active_cascade(self, &mut ctx, now) {
            return t;
        }

        // Evict expired relic glow entries so the map doesn't grow.
        self.relic_glow_starts
            .retain(|_, start| now.saturating_duration_since(*start) < RELIC_GLOW_LIFETIME);

        // Drain non-scoring relic activations from the run state. These are
        // pushed by run.rs for effects that fire outside the scoring cascade
        // (round-end gold, discard triggers, draw bonuses, consumable
        // interactions, etc.). Each activation starts a glow+wiggle and
        // pushes a bus event for audio.
        for rid in GameEngine::drain_relic_activations(ctx.run) {
            self.relic_glow_starts.insert(rid, now);
            ctx.bus
                .push(crate::game::event_bus::GameEvent::RelicActivated(rid));
        }

        // Zodiac inventory: use a card between plays to raise its yaku level for
        // the run. Click ids are `ZODIAC_USE_BASE + slot_idx`. Blocked while a
        // cascade is running (handled by the early return above).
        for &cid in ctx.button_clicks {
            if (ZODIAC_USE_BASE..ZODIAC_USE_BASE + 16).contains(&cid) {
                let idx = (cid - ZODIAC_USE_BASE) as usize;
                let outcome = {
                    let mut engine = GameEngine::new(ctx.run, ctx.bus);
                    engine.dispatch(GameCommand::UseConsumable { index: idx })
                };
                match outcome.data {
                    CommandData::UseConsumable {
                        result: crate::game::run::ConsumableUseResult::Zodiac { yaku, new_level },
                    } => {
                        log::info!("Used Zodiac → {} now level {}", yaku.name(), new_level);
                        let label = format!("{} Lvl.{}", yaku.name(), new_level);
                        let src = ctx.cursor_pos;
                        self.score_popups.spawn(
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
                        self.particles
                            .emit(src.0, src.1, 24, color::RELIC_GOLD, 0.9);
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
                                enh
                            );
                        } else {
                            log::info!("Used {}", kind.name());
                        }
                    }
                    CommandData::UseConsumable {
                        result: crate::game::run::ConsumableUseResult::Memorial { kind },
                    } => {
                        log::info!("Used memorial {}", kind.name());
                    }
                    _ => {}
                }
                if outcome.rejection.is_none()
                    && matches!(outcome.data, CommandData::UseConsumable { .. })
                {
                    self.clear_discard_undo();
                }
            }
        }

        // Catch any score change that didn't come through the cascade path
        // (round resets, instant-grant relic bonuses). Same effect: pop the panel.
        if self.displayed_score != self.prev_displayed_score {
            ctx.anim.score_pop(ENTITY_SCORE_PANEL);
            self.prev_displayed_score = self.displayed_score;
        }

        // If a discard is waiting for the river animation to finish,
        // hold input until the animation completes and the fallback deadline
        // passes, then auto-draw replacements.
        if let Some(deadline) = self.pending_discard_refill {
            let anim_done = !discard_animation::discard_animation_active(self);
            if anim_done && now >= deadline {
                let outcome = {
                    let mut engine = GameEngine::new(ctx.run, ctx.bus);
                    engine.dispatch(GameCommand::RefillHand)
                };
                if matches!(outcome.data, CommandData::RefillHand)
                    && outcome.before != outcome.after
                {
                    ctx.anim.pulse(crate::render::animation::ENTITY_HAND_STRIP);
                }
                self.pending_discard_refill = None;
            } else {
                return None;
            }
        }

        if let Some(t) = {
            let _g = crate::render::cpu_profiler::scope("update.process_focus_and_actions");
            input_handler::process_focus_and_actions(self, &mut ctx, now, focus_kind_before)
        } {
            return t;
        }

        self.tick_staging_zone(ctx.run, dt);

        // Terminal predicates can become true without another discard refill (dead hand
        // with plays/discards exhausted). Reconcile once per idle frame like the bot loop.
        if !self.is_animating() && self.pending_chamber.is_none() {
            ctx.run.resolve_round_end(ctx.bus);
        }

        None
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let vis = ctx.debug_visibility;
        let layout = ctx.layout;
        let run = ctx.run;
        let gameplay = GameEngine::read(run);
        let interaction = GameEngine::read_interaction(run);
        // Hand-strip focus highlight is driven by the unified `self.focus`
        // model — only render the ring when focus is actually on a hand
        // tile. `usize::MAX` is the renderer's "no highlighted tile"
        // sentinel, so navigating focus onto a button / relic / peg
        // correctly removes the hand-strip ring.
        let focus = match self.focus {
            Some(FocusTarget::HandTile(i)) => i.min(interaction.hand_len.saturating_sub(1)),
            _ => usize::MAX,
        };
        let now = Instant::now();
        let live_score = if !self.cascade_queue.is_empty() {
            self.displayed_score
        } else {
            gameplay.round_score
        };

        let current_chamber = gameplay.run_number;
        let total_chambers = crate::core::chamber_target::TOTAL_CHAMBERS;
        let chamber_name = gameplay.chamber.name();

        // The window title is recomputed unconditionally so the OS chrome
        // tracks the current run state even when the glossary takes over
        // the screen.
        let window_title = format!(
            "Mahjuro — {chamber_name} {current_chamber}/{total_chambers} - Score {} / {}  Target-  Yen: {}  Plays: {}  Discards: {}",
            format_score(live_score),
            format_score(gameplay.target_score as u64),
            gameplay.yen,
            gameplay.plays_remaining,
            gameplay.discards_remaining,
        );

        // Score-panel overlay quads (plays/discard pips) — empty; fans on the
        // table show remaining plays/discards.

        let env_h = ctx.room_gltf_height_scale;
        let glb_load = crate::render::gameplay_glb::gameplay_glb_load_state();
        if !matches!(
            glb_load,
            crate::render::gameplay_glb::GameplayGlbLoadState::Ready
        ) {
            let msg = match glb_load {
                crate::render::gameplay_glb::GameplayGlbLoadState::Invalid(m) => m,
                crate::render::gameplay_glb::GameplayGlbLoadState::Missing => {
                    "gameplay.glb is not embedded".to_string()
                }
                crate::render::gameplay_glb::GameplayGlbLoadState::Ready => unreachable!(),
            };
            return super::glb_anchors::gameplay_glb_error_frame(layout, &msg);
        }
        let mut scene_camera =
            match crate::render::gameplay_glb::require_gameplay_camera(layout.window_h, env_h) {
                Ok(cam) => cam,
                Err(e) => {
                    log::error!("{e:#}");
                    return super::glb_anchors::gameplay_glb_error_frame(layout, &e.to_string());
                }
            };
        let layout_scale = (layout.window_w.min(layout.window_h)) / 600.0;
        let has_structure = gameplay.has_structure;
        let cascade_showcase_ref = self.cascade_queue.front().and_then(|(_, sc)| sc.as_ref());
        let glb_anchors = match super::glb_anchors::resolve_gameplay_glb_anchors(
            layout,
            interaction.hand_len,
            layout.window_w,
            layout.window_h,
            &scene_camera,
            env_h,
        ) {
            Ok(anchors) => anchors,
            Err(e) => {
                log::error!("{e:#}");
                return super::glb_anchors::gameplay_glb_error_frame(layout, &e.to_string());
            }
        };
        let hand_slots_fallback = glb_anchors.hand_slots.clone();

        // Boss payload for the dedicated boss plinth inspect target.
        let boss_title_text = gameplay.ordeal_ofuda_title.clone();
        let boss_rule_text = gameplay.ordeal_ofuda_rule_text.clone();

        // Modifier strip: cascade / sets (full width). Relics shown as row below score panel.
        let cascade_frame = self
            .cascade_queue
            .front()
            .map(|(cascade, _)| cascade.frame(now));

        // Active relics are 3D medallions in a horizontal tray (`build_relic_tray_and_wind`).

        // Bottom button bar: discard bowl, bronze mirror, journal — rects from GLB markers.
        let selected_count = gameplay.selected_count;
        let selection_valid = GameEngine::selection_is_valid(run);
        let boss_blocks_selection = self.boss_rule_feedback_live;

        // Bowl + mirror: own row below the hand tile slots, above the journal
        // (discard left, play right within the centered playfield). Click rects
        // match diameter.
        //
        // Vertical order: structure strip, yaku tablets, hand rack, bowl/mirror
        // row, then journal.
        let discard_btn_rect;
        let play_btn_rect;
        let trigger_btn_rect;

        // ── Frame accumulators ───────────────────────────────────────────
        //
        // The migrated draw_frame separates HUD content into layers and
        // pushes them into the final `UiFrame` at the end of this function
        // in canonical order:
        //
        //   1. PERSISTENT HUD (`hud_quads` + optional `hud_text`) — particles,
        //      etc. Lives between the 3D backdrop and the `HandTileFaces`
        //      marker so tile faces read on top of HUD panels they overlap.
        //
        //   2. Focus rings (`hover_quads`) — brass frame for keyboard /
        //      controller focus.
        //
        //   3. PAUSE OVERLAY (`pause_quads` + `pause_text`) — built only
        //      when the pause menu is open; sits above focus rings.
        //
        //   4. ONBOARDING OVERLAY (`onboarding_hints`) — post-tonemap
        //      `overlay_quads` + text; pushed last so lessons / finale
        //      prompts sit above the gameplay HUD.
        //
        let mut hud_quads: Vec<GpuInstance> = Vec::new();
        let mut hud_text: Vec<TextLabel> = Vec::new();
        let mut inspect_tooltip_quads: Vec<GpuInstance> = Vec::new();
        let mut inspect_tooltip_texts: Vec<TextLabel> = Vec::new();
        let mut structure_showcase: Vec<ShowcaseTilePlacement> = Vec::new();
        // Brass focus-ring quads (keyboard/controller focus).
        let mut hover_quads: Vec<GpuInstance> = Vec::new();

        // Focus rect graph: every focusable HUD element pushes its
        // screen-space rect here as it's laid out below. Stashed in
        // `self.last_focus_rects` at the end of `draw_frame` so the next
        // frame's `update()` can hit-test the cursor and run spatial
        // navigation against the freshest on-screen positions.
        let mut focus_rect_graph: Vec<(FocusTarget, [f32; 4])> = Vec::new();

        let input_handler::YakuPanelOutputs {
            yaku_preview_effective_tiles: _yaku_preview_effective_tiles,
            yaku_preview_sets: _yaku_preview_sets,
            yaku_tablet_placements,
            structure_showcase: yaku_structure_showcase,
        } = {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_yaku_panel_and_tablets");
            input_handler::build_yaku_panel_and_tablets(
                self,
                layout,
                run,
                &ctx,
                &gameplay,
                &interaction,
                cascade_showcase_ref,
                cascade_frame.as_ref(),
                has_structure,
                layout_scale,
                glb_anchors.structure_marker_poses,
                glb_anchors.yaku_marker_poses,
            )
        };
        structure_showcase.extend(yaku_structure_showcase);

        let paused = self.pause_menu.paused;

        let cash_in_blocked = gameplay.cash_in_blocked_until_discards_spent;
        let cash_in_visible = gameplay.trigger_enabled || cash_in_blocked;
        let _cash_in_enabled = gameplay.trigger_enabled;
        let play_enabled = selection_valid && gameplay.plays_remaining > 0;
        let discard_enabled = selected_count > 0 && gameplay.discards_remaining > 0;
        let action_row = input_handler::build_glb_action_pick_proxies(
            &glb_anchors,
            self.journal_open_amount,
            has_structure,
        );
        let input_handler::ActionRowOutputs {
            wood_tablet_placements,
            discard_bowl_placement,
            mut bronze_mirror_placement,
            journal_book,
            guidebook,
        } = action_row;
        if let Some(mirror) = bronze_mirror_placement.as_mut()
            && let crate::render::draw_cmd::Object3dKind::Mirror { valid_play_glow } =
                &mut mirror.kind
        {
            *valid_play_glow = if !paused
                && !ctx.modal_active
                && self.cascade_queue.is_empty()
                && play_enabled
                && selected_count > 0
            {
                0.88 + 0.12 * (self.candle_time * 4.5).sin()
            } else {
                0.0
            };
        }

        // Dora indicator screen rect. Pre-computed up here so the focus
        // rect graph entry can both use it.
        // Prefer the renderer's projected dora tile rect (one frame stale,
        // tracks the actual on-screen tile as camera/arrange overrides shift
        // it). Falls back to a screen-position estimate before projection
        // cache has populated.
        let dora_tile_count = if self.pending_chamber.is_some() {
            0
        } else {
            gameplay.dora_indicator_tiles.len().min(2)
        };
        let dora_rect: [f32; 4] = plinth_focus_rect(
            ctx.proj.dora_tile_rect,
            &glb_anchors.tile_plinth_poses[0].anchor,
            layout,
            dora_tile_count,
        );
        let round_wind_tile_count = 1 + usize::from(gameplay.bonus_round_wind_rank.is_some());
        let round_wind_rect: [f32; 4] = plinth_focus_rect(
            ctx.proj.round_wind_tile_rect,
            &glb_anchors.tile_plinth_poses[1].anchor,
            layout,
            round_wind_tile_count,
        );
        let gold_pose = glb_anchors.gold_pose;
        // Cash-in / play labels are engraved on the wood tablets (per-instance decals).
        // Discard river + play mirror use centered text in their projected rects in the
        // persistent HUD pass (see score readout + `hud_text` before `frame.texts`).

        // The 3D action objects (cash-in tablet + discard bowl) no longer go
        // through `frame.buttons`.
        // Their click routing is driven by `pick_gameplay_object` in
        // `main.rs`'s `MouseInput` handler — clicks land on whichever 3D
        // object the cursor is *actually* over per raycast, not whichever
        // 2D rect happens to overlap the cursor. This avoids the
        // perspective-distortion issues that plagued the projected-rect
        // approach. Keyboard nav (button_focus) still works because the
        // `update()` path enqueues UiActions directly.
        //
        // While paused, no gameplay buttons should be clickable — the
        // pause overlay swallows all input via its own buttons plus a
        // fullscreen blocker.
        let mut buttons: Vec<ButtonDef> = Vec::new();
        let _ = paused;

        let input_handler::ConsumableBuild {
            placements: consumable_placements,
        } = {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_consumable_spawns");
            input_handler::build_consumable_spawns(
                layout,
                &ctx,
                &interaction,
                paused,
                &mut focus_rect_graph,
                &mut buttons,
                &glb_anchors.consumable_poses,
            )
        };

        // Particle instances. Pushed into the persistent HUD layer (under
        // hand tile faces) to preserve the legacy z-order — score-cascade
        // bursts visually peek out *around* the tiles rather than over
        // them. Move to `hover_quads` if a future design wants particles
        // to fly over tile faces.
        for (rect, color) in self.particles.instances() {
            hud_quads.push(GpuInstance {
                rect,
                color,
                user: 0,
            });
        }

        // Phase 8: the `?` glossary badge has been removed from the gameplay
        // HUD. Open the Guide book on the table, or press Select / View / −
        // (`Help`) for the full reference.

        // Pause overlay — built into its own dedicated layer so it lands
        // ABOVE the hover layer in canonical push order. Reuses the
        // existing dual-vec `PauseMenu::draw` API with fresh local
        // accumulators (the pause menu has no internal interleaving
        // hazards — it's a dim panel + buttons + text where text-on-top is
        // the desired contract).
        let mut pause_quads: Vec<GpuInstance> = Vec::new();
        let mut pause_text: Vec<TextLabel> = Vec::new();
        self.pause_menu.draw(
            crate::ui::layout::ViewportCtx {
                window_w: layout.window_w,
                window_h: layout.window_h,
            },
            layout_scale,
            crate::scenes::options::options_scroll_fade_backdrop(true),
            &mut pause_quads,
            &mut pause_text,
            &mut buttons,
        );

        // Fullscreen click-blocker behind the pause menu's own buttons.
        // Buttons are hit-tested in vec order with first-match-wins, and the
        // pause menu just appended its buttons above, so this blocker (added
        // last) only catches clicks that miss every pause-menu button. It
        // uses an unused Scene id so the gameplay scene treats the click as
        // a no-op instead of toggling tile selection or starting a drag.
        if paused {
            buttons.push(ButtonDef::scene(
                (0.0, 0.0, layout.window_w, layout.window_h),
                u32::MAX,
            ));
        }
        // The glossary overlay path has its own early-return at the top of
        // this function, so it does not appear here.

        // The 3D table + tiles + candles ARE the UI. Selection feedback is
        // now a true 3D gold-metal outline shell drawn by the renderer's
        // tile_outline_pipeline (which catches candlelight), so no 2D
        // selection overlay is added here.

        let relic_objects = {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_relic_tray");
            input_handler::build_relic_tray(
                self,
                layout,
                run,
                Some(&scene_camera),
                env_h,
                &glb_anchors.relic_poses,
            )
        };

        // ── Frame assembly ──────────────────────────────────────────────
        //
        // Now push every layer into a fresh `UiFrame` in canonical order.
        let mut frame = UiFrame::new();
        let fov_pop_offset = self.final_tiles_fov_pop_offset_deg(now);
        // Keep authored GLB framing at rest; only apply temporary pop animation.
        scene_camera.fovy_deg = (scene_camera.fovy_deg - fov_pop_offset).max(1.0);
        frame.camera_override = Some(scene_camera);
        (discard_btn_rect, play_btn_rect, trigger_btn_rect) =
            super::glb_anchors::reproject_action_button_rects(
                layout.window_w,
                layout.window_h,
                &scene_camera,
                env_h,
                layout_scale,
                &glb_anchors,
            );
        let play_hit_rect: (f32, f32, f32, f32) =
            crate::render::gameplay_glb::gameplay_play_button_hit_rect([
                play_btn_rect.0,
                play_btn_rect.1,
                play_btn_rect.2,
                play_btn_rect.3,
            ])
            .into();
        let meld_count = gameplay.structure_sets.len();
        let (cash_in_glow, cash_in_wiggle_amp) = if !paused && cash_in_visible && meld_count > 0 {
            self.structure_cash_in_feedback(meld_count, gameplay.structure_complete)
        } else {
            (0.0, 0.0)
        };
        let (cash_in_wiggle_x, cash_in_wiggle_y) = if self.cash_in_hold_in_progress() {
            (self.cash_in_hold_vibration_px(), 0.0)
        } else {
            (0.0, self.structure_cash_in_wiggle_px(cash_in_wiggle_amp))
        };
        let btn_rects = [discard_btn_rect, play_hit_rect, trigger_btn_rect];
        input_handler::push_action_button_focus_rects(
            &btn_rects,
            cash_in_visible,
            &mut focus_rect_graph,
        );
        let tally_stick_len = layout.mm(28.0);
        let tally_stick_wide = layout.mm(4.0);
        let tally_stick_thickness = layout.mm(1.5);
        const TALLY_SPREAD_DEG: f32 = 60.0;
        let hand_focus_slots = super::glb_anchors::reproject_hand_focus_slots(
            layout,
            interaction.hand_len,
            layout.window_w,
            layout.window_h,
            &scene_camera,
            env_h,
        )
        .unwrap_or(hand_slots_fallback);
        let (discard_tally_focus_rect, play_tally_focus_rect) =
            super::glb_anchors::reproject_tally_focus_rects(
                layout.window_w,
                layout.window_h,
                &scene_camera,
                &glb_anchors,
                tally_stick_len,
                tally_stick_wide,
                tally_stick_thickness,
                TALLY_SPREAD_DEG,
            );
        let hand_slots = hand_focus_slots;
        if !vis.hide_journal && self.journal_transition.is_none() {
            let journal_rect = crate::render::gameplay_glb::gameplay_journal_book_screen_rect(
                layout.window_w,
                layout.window_h,
                &scene_camera,
                &glb_anchors.journal_pick,
            );
            if journal_rect[2] > 1.0 && journal_rect[3] > 1.0 {
                focus_rect_graph.push((FocusTarget::Journal, journal_rect));
            }
            let guidebook_rect = crate::render::gameplay_glb::gameplay_journal_book_screen_rect(
                layout.window_w,
                layout.window_h,
                &scene_camera,
                &glb_anchors.guidebook_pick,
            );
            if guidebook_rect[2] > 1.0 && guidebook_rect[3] > 1.0 {
                focus_rect_graph.push((FocusTarget::Guidebook, guidebook_rect));
            }
        }
        let hand_slot_w = layout.hand_slot_w;
        let tile_size_px = hand_slot_w * (22.0 / crate::ui::layout::TILE_WIDTH_MM);
        let (boss_ordeal_obj, ordeal_icon_rect, boss_ordeal_glow, boss_ordeal_wiggle) = if !vis
            .hide_boss_icon
            && let Some(kind) = gameplay.ordeal_kind
        {
            let (boss_glow, boss_wiggle) = self.boss_rule_feedback(now, boss_blocks_selection);
            let obj = crate::render::gameplay_glb::gameplay_boss_ordeal_object3d(
                &glb_anchors.tile_plinth_poses[2],
                layout.window_w,
                layout.window_h,
                env_h,
                &scene_camera,
                tile_size_px,
                kind,
                boss_glow,
            );
            let rect = crate::render::gameplay_glb::gameplay_boss_ordeal_screen_rect(
                &glb_anchors.tile_plinth_poses[2],
                layout.window_w,
                layout.window_h,
                env_h,
                &scene_camera,
                tile_size_px,
            );
            (Some(obj), Some(rect), boss_glow, boss_wiggle)
        } else {
            (None, None, 0.0, 0.0)
        };
        let discard_undo_rect = if !paused
            && !ctx.modal_active
            && self.cascade_queue.is_empty()
            && self.journal_transition.is_none()
            && crate::persistence::load_settings().discard_undo_enabled
            && self.discard_undo.is_some()
            && self.pending_discard_refill.is_none()
            && discard_btn_rect.2 > 1.0
            && discard_btn_rect.3 > 1.0
        {
            let zscale = (layout.window_w.min(layout.window_h)) / 600.0;
            let gap = (6.0 * zscale).max(4.0);
            let btn_h = (28.0 * zscale).max(22.0);
            let btn_w = (88.0 * zscale).max(72.0);
            let bx = discard_btn_rect.0;
            let by = discard_btn_rect.1 + discard_btn_rect.3 + gap;
            Some([bx, by, btn_w, btn_h])
        } else {
            None
        };
        frame.gameplay_cash_in_button_visible = cash_in_visible;
        frame.gameplay_cash_in_glow = cash_in_glow;
        frame.gameplay_cash_in_wiggle_x = cash_in_wiggle_x;
        frame.gameplay_cash_in_wiggle = cash_in_wiggle_y;
        frame.gameplay_cash_in_blocked = cash_in_blocked;
        frame.gameplay_action_picks = Some(crate::render::draw_cmd::GameplayActionPickProxies {
            bowl: discard_bowl_placement.clone(),
            mirror: bronze_mirror_placement.clone(),
            journal: journal_book.clone(),
            guidebook: guidebook.clone(),
            cash_in_tablet: None,
        });
        frame.background(BackgroundId::Black);
        frame.gameplay_score_roller_values = Some((live_score, gameplay.target_score as u64));
        if !cash_in_blocked && cash_in_glow > 0.0 {
            let trigger_rect = [
                trigger_btn_rect.0,
                trigger_btn_rect.1,
                trigger_btn_rect.2,
                trigger_btn_rect.3,
            ];
            if trigger_rect[2] > 1.0 && trigger_rect[3] > 1.0 {
                let pad = trigger_rect[2].max(trigger_rect[3]) * 0.55;
                let alpha = 0.10 + 0.22 * cash_in_glow;
                frame.gradient_quads([GradientQuadInstance {
                    rect: [
                        trigger_rect[0] - pad + cash_in_wiggle_x,
                        trigger_rect[1] - pad + cash_in_wiggle_y,
                        trigger_rect[2] + pad * 2.0,
                        trigger_rect[3] + pad * 2.0,
                    ],
                    color: [0.96, 0.82, 0.52, alpha],
                    feather: [0.62, 1.0, 0.0, 0.0],
                }]);
            }
        }
        if !vis.hide_environment {
            frame.gameplay_environment();
        }
        let room_glb_lights = crate::render::gameplay_glb::gameplay_glb_has_embedded_lights();
        frame.scene_lighting.embedded_gltf_punctual = room_glb_lights;
        frame.scene_lighting.room_glb_brdf = room_glb_lights;
        if room_glb_lights && !vis.hide_candle_lights {
            let lamp_flicker = self.light_ramp;
            let (punctual, nodes) = crate::render::room_gltf_punctual::tagged_to_scene_punctual(
                crate::render::gameplay_glb::gameplay_embedded_point_lights_runtime_tagged(
                    layout.window_w,
                    layout.window_h,
                    env_h,
                    &ctx.room_env_for("gameplay").0,
                    self.candle_time,
                    lamp_flicker,
                    ctx.flame_tuning.candle_flicker_amp,
                ),
            );
            frame.scene_lighting.punctual = punctual;
            frame.scene_lighting.punctual_gltf_nodes = nodes;
            frame.scene_lighting.set_gltf_embedded_spot_lights(
                crate::render::gameplay_glb::gameplay_embedded_spot_lights_runtime(
                    layout.window_w,
                    layout.window_h,
                    env_h,
                    &ctx.room_env_for("gameplay").0,
                ),
            );
            let glb_flames = crate::render::gameplay_glb::gameplay_gltf_candle_flame_emitters(
                layout.window_h,
                env_h,
                lamp_flicker,
                &ctx.flame_tuning,
            );
            frame.candle_light_count = glb_flames.len() as u32;
            frame.flame_height_world = ctx.flame_tuning.flame_height_world(
                crate::render::room_glb::room_env_world_scale(layout.window_h, env_h),
                crate::render::flame_volume::SHOP_GLTF_CANDLE_HEIGHT_DOC_M,
            );
            frame.procedural_flame_emitters = glb_flames;
        }

        let yen_label_rect = if vis.hide_yen_label {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            let gold_label_center = super::glb_anchors::player_gold_label_screen_center(
                layout.window_w,
                layout.window_h,
                env_h,
                &scene_camera,
                (gold_pose.anchor[0], gold_pose.anchor[1]),
            );
            crate::render::yen_display::push_yen_amount_label(
                &mut frame,
                layout.window_w,
                layout.window_h,
                gameplay.yen,
                gold_label_center,
            )
        };
        if !vis.hide_wall_hud {
            let wall_layout = crate::render::wall_display::wall_hud_layout(
                layout.window_w,
                layout.window_h,
                gameplay.tiles_left,
            );
            let wr = wall_layout.block_rect;
            focus_rect_graph.push((FocusTarget::WallHud, wr));
            if !self.pause_menu.paused {
                buttons.push(ButtonDef::scene(
                    (wr[0], wr[1], wr[2], wr[3]),
                    super::WALL_HUD_CLICK_ID,
                ));
            }
        }

        // Build hand tile placements for the showcase pipeline.
        // Each slot becomes one ShowcaseTilePlacement; the renderer draws,
        // picks, and projects them with no separate hand-tile GPU path.
        {
            let _g = crate::render::cpu_profiler::scope("draw_frame.hand_tile_placements");
            let hand = &interaction.hand;
            let hand_world_slots = glb_anchors.hand_world_slots.as_slice();
            let hand_scale_mul =
                glb_anchors.hand_marker_poses[0].uniform_author_scale(layout.window_h, env_h);
            // Hand tile placements (dora gold overlay comes from renderer merge rects).
            let meld_preview = crate::persistence::load_settings().structure_meld_preview;
            let mut hand_placements: Vec<crate::render::draw_cmd::ShowcaseTilePlacement> =
                Vec::with_capacity(hand.len());
            let staging_preview_anchors = if meld_preview {
                input_handler::staging_preview_anchors_for_groups(
                    &glb_anchors.structure_marker_poses,
                    layout,
                    layout_scale,
                    env_h,
                    &gameplay.structure_sets,
                    &self.staging_layout.meld_index_groups,
                )
            } else {
                rustc_hash::FxHashMap::default()
            };
            let (invalid_flash, invalid_elapsed) = self.invalid_meld_flash_phase(now);
            for (i, &tile) in hand.iter().enumerate() {
                let tile = Self::display_tile(tile, run);
                let is_selected = interaction.selected.get(i).copied().unwrap_or(false);
                let is_focused = focus == i;
                let is_invalid_flash =
                    invalid_flash > 0.0 && self.invalid_meld_flash_slots.contains(&i);
                // Pop-in: slide_y 0→1, offset pixels downward (large py = nearer player).
                let slide_y_frac = self.hand_slide_y.get(i).copied().unwrap_or(1.0);
                let slide_x_px = self.hand_slide_x.get(i).copied().unwrap_or(0.0);
                // Side-to-side "no" shake — fast horizontal oscillation that decays out.
                let reject_shake_x = if is_invalid_flash {
                    let decay =
                        1.0 - (invalid_elapsed / super::INVALID_MELD_FLASH_SECS).clamp(0.0, 1.0);
                    13.0 * decay * (invalid_elapsed * 34.0).sin()
                } else {
                    0.0
                };
                let Some(&([px, py, lift_z], sw, hand_rot)) = hand_world_slots.get(i) else {
                    continue;
                };
                let sh = hand_slots.get(i).map(|(_, _, _, h)| *h).unwrap_or(sw);
                let pop_offset = (1.0 - slide_y_frac) * sh * 0.3;
                let size_px = super::hand_layout::hand_tile_size_from_slot_width(
                    sw,
                    hand.len(),
                    hand_scale_mul,
                );
                
                let staging_lift = self.staging_layout.staging_lift_z.get(i).copied().unwrap_or(0.0);
                let _staging_offset = self.staging_layout.staging_offset_slots.get(i).copied().unwrap_or(0.0);
                
                if meld_preview && staging_lift > 0.01 {
                    // Ghost on the hand rail: selection shell + hit target stay here.
                    hand_placements.push(crate::render::draw_cmd::ShowcaseTilePlacement {
                        tile,
                        center_pos: [px + slide_x_px, py + pop_offset, lift_z],
                        rotation: hand_rot,
                        scale: slide_y_frac.max(0.05),
                        size_px,
                        brightness: 0.15,
                        opacity: 1.0,
                        selected: is_selected && !is_invalid_flash,
                        hovered: is_focused && !is_invalid_flash,
                        outline: (is_selected || is_focused) && !is_invalid_flash,
                        glow: is_selected && !self.staging_layout.is_valid_meld,
                        glow_color: None,
                        outline_sel: None,
                        pick_id: Some(i),
                        overlay_rect_group: None,
                    });
                }
                
                let hand_x = px + slide_x_px + reject_shake_x;
                let hand_y = py + pop_offset;
                let (cx, cy, lift, preview_rot, preview_size_px) =
                    if meld_preview && staging_lift > 0.01 {
                        if let Some(target) = staging_preview_anchors.get(&i) {
                            let t = staging_lift;
                            let cx = hand_x + (target.center_pos[0] - hand_x) * t;
                            let cy = hand_y + (target.center_pos[1] - hand_y) * t;
                            let lift = lift_z + (target.center_pos[2] - lift_z) * t;
                            let rot = [
                                hand_rot[0] + (target.rotation[0] - hand_rot[0]) * t,
                                hand_rot[1] + (target.rotation[1] - hand_rot[1]) * t,
                                hand_rot[2] + (target.rotation[2] - hand_rot[2]) * t,
                            ];
                            let preview_size =
                                size_px + (target.size_px - size_px) * t;
                            (cx, cy, lift, rot, preview_size)
                        } else {
                            (
                                hand_x,
                                hand_y,
                                lift_z,
                                hand_rot,
                                size_px,
                            )
                        }
                    } else {
                        (hand_x, hand_y, lift_z, hand_rot, size_px)
                    };
                
                let is_valid_group = self.staging_layout.valid_meld_tiles.get(i).copied().unwrap_or(false);
                let capacity_error = is_valid_group && self.staging_layout.has_capacity_error;
                let is_staged_preview = meld_preview && staging_lift > 0.01;
                let preview_opacity = if is_staged_preview {
                    0.55 + 0.12 * staging_lift
                } else {
                    1.0
                };
                let preview_brightness = if is_invalid_flash { 1.12 } else { 1.0 };
                let preview_scale = if is_staged_preview {
                    slide_y_frac.max(0.05) * 0.96
                } else {
                    slide_y_frac.max(0.05)
                };
                
                hand_placements.push(crate::render::draw_cmd::ShowcaseTilePlacement {
                    tile,
                    center_pos: [cx, cy, lift],
                    rotation: preview_rot,
                    scale: preview_scale,
                    size_px: preview_size_px,
                    brightness: preview_brightness,
                    opacity: preview_opacity,
                    selected: if is_staged_preview { false } else { is_selected && !is_invalid_flash },
                    hovered: if is_staged_preview { false } else { is_focused && !is_invalid_flash },
                    outline: if is_staged_preview { false } else { (is_selected || is_focused) && !is_invalid_flash },
                    glow: if is_staged_preview {
                        false
                    } else {
                        is_invalid_flash || is_selected || is_valid_group
                    },
                    glow_color: if is_invalid_flash || capacity_error {
                        Some([1.00, 0.14, 0.08, 0.72 + 0.28 * invalid_flash])
                    } else if is_valid_group && !is_staged_preview {
                        Some([1.0, 0.9, 0.3, 0.8])
                    } else {
                        None
                    },
                    outline_sel: None,
                    pick_id: if is_staged_preview { None } else { Some(i) },
                    overlay_rect_group: None,
                });
            }
            if !vis.hide_hand_tiles && !hand_placements.is_empty() {
                frame.showcase_tile_batch(hand_placements);
            }
        }
        if !vis.hide_relics && !relic_objects.is_empty() {
            frame.object3d_batch(relic_objects);
        }
        // PERSISTENT HUD: score-panel overlay quads (none
        // today) → modifier strip → tally fans → cascade tokens / popups /
        // cascade HUD (3D) → gold flash → undo → `frame.quads` → texts.
        //
        // The score readout itself is **2D** (`TextLabel`s in the persistent
        // text pass). Cascade popups and hand-off glyphs still use 3D meshes;
        let score_cascade = match super::score_counter::resolve_score_cascade_layout(
            layout,
            &self.positions,
            layout.window_w,
            layout.window_h,
            &scene_camera,
            env_h,
        ) {
            Ok(cascade) => cascade,
            Err(e) => {
                log::error!("score cascade layout: {e:#}");
                return super::glb_anchors::gameplay_glb_error_frame(layout, &e.to_string());
            }
        };
        let score_counter = score_cascade.counter;
        // Debug visibility: `hide_score_readout` gates the 2D score line.
        // Anchor it from the plaque's *actual* left edge instead of the raw
        // score-frame bounds: perspective projection pulls taller / higher
        // objects inward on screen, so a naive screen anchor can still drift
        // back over the wood plaque and obscure the plaque text.
        // Counter fans — upright tally sticks standing in front of the action
        // objects. Play fan (green) stands in front of the bronze mirror;
        // discard fan (orange) stands in front of the discard river. Each
        // stick = one remaining action; the fan thins
        // from the outermost stick inward as the count drops, so the
        // upright core stays intact and the consumption reads as a spent
        // stick rather than a re-deal.
        {
            let stick_len = tally_stick_len;
            let stick_wide = tally_stick_wide;
            let stick_thickness = tally_stick_thickness;
            let spread_deg = TALLY_SPREAD_DEG;
            // Push each fan just toward-the-camera of its anchor so the
            // sticks stand on the table surface in front of (not inside)
            // the mirror/river.
            let [fx, fy, flift] = glb_anchors.play_tally_pose.anchor;
            let play_rot = glb_anchors.play_tally_pose.rotation_deg();
            if !vis.hide_play_tally_fan {
                frame.object3d(Object3d {
                    pos: [fx, fy, flift],
                    extents: [1.0, 1.0, 1.0],
                    rotation: [0.0, 0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::TallyFan {
                        stick_len,
                        stick_wide,
                        stick_thickness,
                        count: gameplay.plays_remaining,
                        max_count: gameplay.plays_max,
                        spread_deg,
                        base_color: crate::render::theme::color::tally_stick::PLAY,
                        tip_color: crate::render::theme::color::tally_stick::PLAY_TIP,
                        rotation_y_deg: play_rot[1],
                        placement_rot_deg: play_rot,
                        kind: crate::render::draw_cmd::TallyFanKind::Draws,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                });
            }
            let [fx, fy, flift] = glb_anchors.discard_tally_pose.anchor;
            let discard_rot = glb_anchors.discard_tally_pose.rotation_deg();
            if !vis.hide_discard_tally_fan {
                frame.object3d(Object3d {
                    pos: [fx, fy, flift],
                    extents: [1.0, 1.0, 1.0],
                    rotation: [0.0, 0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::TallyFan {
                        stick_len,
                        stick_wide,
                        stick_thickness,
                        count: gameplay.discards_remaining,
                        max_count: gameplay.discards_max,
                        spread_deg,
                        base_color: crate::render::theme::color::tally_stick::DISCARD,
                        tip_color: crate::render::theme::color::tally_stick::DISCARD_TIP,
                        rotation_y_deg: discard_rot[1],
                        placement_rot_deg: discard_rot,
                        kind: crate::render::draw_cmd::TallyFanKind::Discards,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                });
            }
        }
        // Floating extruded-glyph score popups (per-step "+50" / "×3").
        if !vis.hide_score_popups && self.score_popups.is_active() {
            let popup_scale = layout.window_w.min(layout.window_h) / 1080.0;
            let placements = self.score_popups.placements(now, popup_scale);
            frame.object3d_batch(placements);
        }
        // Cascade HUD: chips × mult counter under the score panel. During the
        // hand-off tween the trio merges into `= TOTAL` and flies up toward
        // the readout anchor (`score_counter.reel`).
        if !vis.hide_cascade_hud
            && let Some(hud) = self.cascade_hud
        {
            let placements = build_cascade_hud_placements(
                &hud,
                score_counter.cascade_pad,
                score_counter.reel,
                score_counter.glyph_scale,
                score_counter.plaque_w,
            );
            if !placements.is_empty() {
                frame.object3d_batch(placements);
            }
        }
        // Gold-flash overlay: a fullscreen quad that fades from peak alpha
        // to zero over ~400ms, fired when the cascade lands on its final
        // beat. Tints the entire screen warm so the closing crescendo
        // really *lands* visually. Drawn before the modifier-strip text
        // so 2D HUD labels stay readable through the flash.
        if let Some(t0) = self.gold_flash_at {
            let elapsed_ms = now.saturating_duration_since(t0).as_secs_f32() * 1000.0;
            if elapsed_ms < super::GOLD_FLASH_SECS * 1000.0 {
                let t = elapsed_ms / (super::GOLD_FLASH_SECS * 1000.0);
                // Ease-out-cubic decay: 1 → 0
                let env = (1.0 - t).powi(3);
                let alpha = 0.22 * env;
                frame.quad(GpuInstance {
                    rect: [0.0, 0.0, layout.window_w, layout.window_h],
                    color: [1.0, 0.85, 0.45, alpha],
                    user: 0,
                });
            }
        }
        if let Some(undo_rect) = discard_undo_rect {
            let is_focus = matches!(self.focus, Some(FocusTarget::DiscardUndo));
            let bg = if is_focus {
                color::WALNUT_SOFT
            } else {
                color::WALNUT_RAISED
            };
            hud_quads.push(GpuInstance {
                rect: undo_rect,
                color: bg,
                user: 0,
            });
            buttons.push(ButtonDef::scene(
                (undo_rect[0], undo_rect[1], undo_rect[2], undo_rect[3]),
                super::UNDO_DISCARD_CLICK_ID,
            ));
        }
        {
            frame.quads(std::mem::take(&mut hud_quads));
        }
        // Committed structure melds + tier tokens: inserted before the hand
        // `ShowcaseTileBatch` at end of `draw_frame` so they sit behind the rack (depth order).
        // Phase 3: bone yaku tablets (decal names on mesh).
        if !vis.hide_yaku_tablets && !yaku_tablet_placements.is_empty() {
            frame.object3d_batch(yaku_tablet_placements);
        }
        // `gameplay.glb` marker empties skip env draw; spawn procedural meshes at those anchors.
        if !vis.hide_discard_bowl
            && let Some(ref bowl) = discard_bowl_placement
        {
            frame.object3d(bowl.clone());
        }
        if !vis.hide_play_mirror
            && let Some(ref mirror) = bronze_mirror_placement
        {
            frame.object3d(mirror.clone());
        }
        if !vis.hide_journal
            && let Some(ref book) = journal_book
        {
            frame.object3d(book.clone());
        }
        if !vis.hide_journal
            && let Some(ref book) = guidebook
        {
            frame.object3d(book.clone());
        }
        if !vis.hide_wood_tablets && !wood_tablet_placements.is_empty() {
            frame.object3d_batch(wood_tablet_placements);
        }
        let bowl_model = ctx.proj.bowl_model.or_else(|| {
            discard_bowl_placement.as_ref().map(|bowl| {
                discard_animation::bowl_model_matrix(layout.window_w, layout.window_h, bowl)
            })
        });
        // Discard river tiles (sinking pile, settled, in-flight) — after the
        // river mesh so tiles read as resting on / flying into the water.
        {
            let mut discard_placements = discard_animation::sinking_placements(
                self,
                now,
                &self.cached_cascade_tuning,
                bowl_model,
                layout,
                layout.window_w,
                layout.window_h,
                run,
            );
            discard_placements.extend(discard_animation::settled_placements(
                self,
                bowl_model,
                layout,
                layout.window_w,
                layout.window_h,
                run,
            ));
            discard_placements.extend(discard_animation::in_flight_placements(
                self,
                now,
                &self.cached_cascade_tuning,
                bowl_model,
                layout,
                layout.window_w,
                layout.window_h,
                run,
            ));
            if !vis.hide_discard_tiles && !discard_placements.is_empty() {
                frame.showcase_tile_batch(discard_placements);
            }
        }
        if !vis.hide_consumables && !consumable_placements.is_empty() {
            frame.object3d_batch(consumable_placements);
        }

        {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_ambient_table_objects");
            if let Err(e) = animation_state::build_ambient_table_objects(
                self,
                layout,
                &gameplay,
                ctx.progress.dora_enabled(),
                &mut frame,
                &glb_anchors.tile_plinth_poses,
                glb_anchors.gold_pose,
                &scene_camera,
                layout.window_w,
                layout.window_h,
                env_h,
                vis,
            ) {
                log::error!("{e:#}");
                return super::glb_anchors::gameplay_glb_error_frame(layout, &e.to_string());
            }
        }
        if let (Some(obj), Some(icon_rect)) = (boss_ordeal_obj, ordeal_icon_rect) {
            if boss_ordeal_glow > 0.0 {
                let pad = icon_rect[2].max(icon_rect[3]) * 0.42;
                frame.quad(GpuInstance {
                    rect: [
                        icon_rect[0] - pad + boss_ordeal_wiggle,
                        icon_rect[1] - pad,
                        icon_rect[2] + pad * 2.0,
                        icon_rect[3] + pad * 2.0,
                    ],
                    color: [1.0, 0.48, 0.10, 0.18 + 0.42 * boss_ordeal_glow],
                    user: 0,
                });
            }
            frame.object3d_batch(vec![obj]);
        }

        // Flying coin animations (gold changes).
        {
            let flying = self.flying_coins.placements();
            if !vis.hide_flying_coins && !flying.is_empty() {
                frame.object3d_batch(flying);
            }
        }

        // Journal zoom: darken HUD briefly so the scaled book does not z-fight tiles/props behind it.
        if let Some(t) = self.journal_transition {
            let zp = t.zoom_progress();
            if zp > 0.001 {
                let smoothed = zp * zp * (3.0 - 2.0 * zp);
                let a = smoothed * 0.72;
                frame.quad(GpuInstance {
                    rect: [0.0, 0.0, layout.window_w, layout.window_h],
                    color: [0.03, 0.04, 0.06, a],
                    user: 0,
                });
            }
        }
        if !frame.procedural_flame_emitters.is_empty() {
            // One `DrawCmd::Flame` triggers the volume batch (same path as shop).
            frame.flame_batch();
        }

        // Play mirror + discard river: labels centered in projected rects (not cursor hover tooltips).
        if !paused
            && !ctx.modal_active
            && self.cascade_queue.is_empty()
            && self.journal_transition.is_none()
            && self.pending_chamber.is_none()
        {
            use crate::render::theme::typography;
            let body_px = typography::size(typography::H36, layout.window_h);
            let push_centered = |out: &mut Vec<TextLabel>, rect: [f32; 4], copy: &'static str| {
                if rect[2] <= 1.0 || rect[3] <= 1.0 {
                    return;
                }
                let fs = typography::size(typography::H28, layout.window_h);
                let label = TextLabel {
                    rect,
                    text: copy.into(),
                    color: color::CHAMPAGNE,
                    font_px: Some(fs),
                    align: crate::render::wgpu_renderer::TextAlign::Center,
                    ..Default::default()
                };
                let d = (fs * 0.055).clamp(1.0, 2.5);
                let outline = [0.0, 0.0, 0.0, color::CHAMPAGNE[3].min(0.95)];
                for (dx, dy) in [
                    (-d, 0.0),
                    (d, 0.0),
                    (0.0, -d),
                    (0.0, d),
                    (-d, -d),
                    (d, -d),
                    (-d, d),
                    (d, d),
                ] {
                    let mut stroke = label.clone();
                    stroke.rect[0] += dx;
                    stroke.rect[1] += dy;
                    stroke.color = outline;
                    out.push(stroke);
                }
                out.push(label);
            };
            let discard_rect = [
                discard_btn_rect.0,
                discard_btn_rect.1,
                discard_btn_rect.2,
                discard_btn_rect.3,
            ];
            let play_rect = [
                play_btn_rect.0,
                play_btn_rect.1,
                play_btn_rect.2,
                play_btn_rect.3,
            ];
            push_centered(&mut hud_text, discard_rect, "Discard");
            push_centered(&mut hud_text, play_rect, "Play");
            {
                use crate::core::relic::RelicId;
                let ids = GameEngine::active_relics(run);
                let preview = run.ghost_hand_preview_chips();
                for (i, &rid) in ids.iter().enumerate() {
                    if rid != RelicId::GhostHand {
                        continue;
                    }
                    let Some(rect) = ctx.proj.relic_rects.get(i).copied() else {
                        continue;
                    };
                    if rect[2] <= 1.0 || rect[3] <= 1.0 {
                        continue;
                    }
                    let cap = body_px.min(rect[3] * 0.22).min(rect[2] * 0.35);
                    let fs = typography::tier_at_most(cap, layout.window_h);
                    let label_h = (fs * 1.25).min(rect[3] * 0.45);
                    let chip_rect = [
                        rect[0],
                        rect[1] + rect[3] - label_h * 0.95,
                        rect[2],
                        label_h,
                    ];
                    hud_text.push(TextLabel {
                        rect: chip_rect,
                        text: format!("+{preview}"),
                        color: color::CHAMPAGNE,
                        font_px: Some(fs),
                        align: crate::render::wgpu_renderer::TextAlign::Center,
                        ..Default::default()
                    });
                }
            }
            if let Some(undo_rect) = discard_undo_rect {
                let is_focus = matches!(self.focus, Some(FocusTarget::DiscardUndo));
                let fs =
                    typography::tier_at_most(body_px.min(undo_rect[3] * 0.55), layout.window_h);
                hud_text.push(TextLabel {
                    rect: undo_rect,
                    text: "Undo".into(),
                    color: if is_focus {
                        color::CHAMPAGNE
                    } else {
                        color::STONE
                    },
                    font_px: Some(fs),
                    align: crate::render::wgpu_renderer::TextAlign::Center,
                    ..Default::default()
                });
            }
        }

        let hud_text = hud_text;
        frame.texts(hud_text);

        if let Some(undo_rect) = discard_undo_rect {
            focus_rect_graph.push((FocusTarget::DiscardUndo, undo_rect));
        }

        // Append the deferred focus rect entries (hand tiles, relics,
        // pegs, gold) before the centralized focus ring so the lookup
        // can find them. The button-bar and consumable strip already
        // pushed their entries inline above.
        // Hand rack: tile-sized projected bounds (same source as mouse picking).
        let hand_scale_mul = glb_anchors.hand_marker_poses[0]
            .uniform_author_scale(layout.window_h, env_h);
        for i in 0..interaction.hand_len {
            let slot = hand_slots
                .get(i)
                .copied()
                .unwrap_or((0.0, 0.0, layout.hand_slot_w, layout.hand_slot_h));
            let rect = super::hand_layout::hand_tile_screen_rect(
                i,
                slot,
                interaction.hand_len,
                slot.2,
                hand_scale_mul,
                &ctx.proj.hand_rects,
            );
            focus_rect_graph.push((FocusTarget::HandTile(i), rect));
        }
        for (i, r) in ctx.proj.relic_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 {
                focus_rect_graph.push((FocusTarget::Relic(i), *r));
            }
        }
        if !vis.hide_discard_tally_fan
            && let Some(r) = discard_tally_focus_rect
        {
            focus_rect_graph.push((FocusTarget::Peg(PegKind::Discards), r));
        }
        if !vis.hide_play_tally_fan
            && let Some(r) = play_tally_focus_rect
        {
            focus_rect_graph.push((FocusTarget::Peg(PegKind::Hands), r));
        }
        // Anchor the gold focus rect to the actual 3D coin pile (when
        // there is gold to display). The pile rect was computed up at
        // the top of `draw_frame` so the focus ring and
        // physical pile draw all share one source of truth.
        focus_rect_graph.push((FocusTarget::Gold, yen_label_rect));
        // Yaku tablets — push the projected rects into the focus graph
        // so spatial nav can land on them. We use the projected rects
        // (one frame stale) to match where the player actually sees the
        // tablets after camera projection; on the very first frame they
        // may be missing, in which case the tablet is briefly skipped.
        for (i, r) in ctx.proj.yaku_tablet_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 && r[0].is_finite() && r[1].is_finite() {
                focus_rect_graph.push((FocusTarget::YakuTablet(i), *r));
            }
        }
        // Dora indicator — display-only focus target so a controller
        // player can read what the brass plinth represents.
        focus_rect_graph.push((FocusTarget::Dora, dora_rect));
        if let Some(rect) = ordeal_icon_rect {
            focus_rect_graph.push((FocusTarget::Ordeal, rect));
        }
        focus_rect_graph.push((FocusTarget::RoundWind, round_wind_rect));
        if let Some((score_rect, target_rect)) =
            super::score_counter::resolve_score_roller_bank_focus_rects(
                layout.window_w,
                layout.window_h,
                &scene_camera,
                env_h,
            )
        {
            if score_rect[2] > 1.0 && score_rect[3] > 1.0 {
                focus_rect_graph.push((
                    FocusTarget::ScoreRoller(ScoreRollerBank::Score),
                    score_rect,
                ));
            }
            if target_rect[2] > 1.0 && target_rect[3] > 1.0 {
                focus_rect_graph.push((
                    FocusTarget::ScoreRoller(ScoreRollerBank::Target),
                    target_rect,
                ));
            }
        }

        // Focus inspect: [`crate::ui::tooltip`] frame + wrapped text (shop uses the same helper).
        if !self.pause_menu.paused
            && !ctx.modal_active
            && self.cascade_queue.is_empty()
            && self.journal_transition.is_none()
        {
            if let Some(target) = self.focus
                && let Some(rect) = focus_rect_graph
                    .iter()
                    .find_map(|(t, r)| (*t == target).then_some(*r))
            {
                match target {
                    FocusTarget::Relic(i) => {
                        let ids = GameEngine::active_relics(run);
                        if let Some(&rid) = ids.get(i) {
                            let def = all_relic_defs().iter().find(|d| d.id == rid);
                            let name = def
                                .map(|d| d.name.to_string())
                                .unwrap_or_else(|| "Relic".into());
                            let rare = def.map(|d| format!("{:?}", d.rarity)).unwrap_or_default();
                            let desc = relic_description_live(
                                rid,
                                &run.relic_counters,
                                run.yen,
                                Some((&run.relics, i)),
                                Some(run.ghost_hand_preview_chips()),
                                Some(run.wing),
                            );
                            let cta = match rid {
                                RelicId::MirrorTile => {
                                    "[ ] reorder · copy = relic to the right".to_string()
                                }
                                RelicId::ShadowHand => {
                                    "Copy = leftmost relic (not Shadow Hand)".to_string()
                                }
                                _ => format!("Tier · {rare}"),
                            };
                            push_focus_tooltip_panel_2d(
                                &mut inspect_tooltip_quads,
                                &mut inspect_tooltip_texts,
                                FocusTooltipPanelParams {
                                    window_w: layout.window_w,
                                    window_h: layout.window_h,
                                    anchor_rect: Some(rect),
                                    title: &name,
                                    desc: &desc,
                                    cta: cta.as_str(),
                                    accent_color: color::STONE,
                                    hover_is_owned: false,
                                    skip_title_block: false,
                                    avoid_rect: None,
                                },
                            );
                        }
                    }
                    FocusTarget::Consumable(i) => {
                        if let Some(&c) = interaction.consumables.get(i) {
                            let kind = match c {
                                Consumable::Zodiac(_) => "Ribbon",
                                Consumable::Talisman(_) => "Talisman",
                                Consumable::Memorial(_) => "Remnant",
                            };
                            let title = format!("{} · {}", kind, c.name());
                            let desc = gameplay_consumable_description_full(c);
                            push_focus_tooltip_panel_2d(
                                &mut inspect_tooltip_quads,
                                &mut inspect_tooltip_texts,
                                FocusTooltipPanelParams {
                                    window_w: layout.window_w,
                                    window_h: layout.window_h,
                                    anchor_rect: Some(rect),
                                    title: &title,
                                    desc: &desc,
                                    cta: "",
                                    accent_color: color::GOLD,
                                    hover_is_owned: false,
                                    skip_title_block: false,
                                    avoid_rect: None,
                                },
                            );
                        }
                    }
                    FocusTarget::HandTile(i) => {
                        if let Some(&tile) = interaction.hand.get(i) {
                            let tile = Self::display_tile(tile, run);
                            let (title, desc) = hand_tile_focus_tooltip(
                                &tile,
                                &gameplay.dora_faces,
                                &run.tile_debuffs,
                                interaction.selected.get(i).copied().unwrap_or(false),
                            );
                            let hand_scale_mul = glb_anchors.hand_marker_poses[0]
                                .uniform_author_scale(layout.window_h, env_h);
                            let slot = hand_slots
                                .get(i)
                                .copied()
                                .unwrap_or((rect[0], rect[1], rect[2], rect[3]));
                            let tooltip_rect = super::hand_layout::hand_tile_tooltip_rect(
                                i,
                                slot,
                                interaction.hand_len,
                                slot.2,
                                hand_scale_mul,
                                &ctx.proj.hand_rects,
                            );
                            push_focus_tooltip_panel_2d(
                                &mut inspect_tooltip_quads,
                                &mut inspect_tooltip_texts,
                                FocusTooltipPanelParams {
                                    window_w: layout.window_w,
                                    window_h: layout.window_h,
                                    anchor_rect: Some(tooltip_rect),
                                    title: &title,
                                    desc: &desc,
                                    cta: "",
                                    accent_color: color::BRASS,
                                    hover_is_owned: false,
                                    skip_title_block: false,
                                    avoid_rect: None,
                                },
                            );
                        }
                    }
                    FocusTarget::Dora => {
                        let (title, cta, desc) = dora_focus_tooltip_strings(
                            ctx.progress.dora_enabled(),
                            &gameplay.dora_faces,
                        );
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: &title,
                                desc: &desc,
                                cta: &cta,
                                accent_color: color::GOLD,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::RoundWind => {
                        let (title, cta, desc) = round_wind_focus_tooltip_strings(
                            gameplay.round_wind_rank,
                            gameplay.bonus_round_wind_rank,
                        );
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: &title,
                                desc: &desc,
                                cta: &cta,
                                accent_color: color::BRASS,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::WallHud => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Wall Ledger",
                                desc: "Full tile supply for this round — vivid tiles remain in the wall.",
                                cta: "Open",
                                accent_color: color::CHAMPAGNE,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::Ordeal => {
                        if !boss_title_text.is_empty() {
                            push_focus_tooltip_panel_2d(
                                &mut inspect_tooltip_quads,
                                &mut inspect_tooltip_texts,
                                FocusTooltipPanelParams {
                                    window_w: layout.window_w,
                                    window_h: layout.window_h,
                                    anchor_rect: Some(rect),
                                    title: &boss_title_text,
                                    desc: &boss_rule_text,
                                    cta: "Ordeal Rule",
                                    accent_color: color::RUBY,
                                    hover_is_owned: false,
                                    skip_title_block: false,
                                    avoid_rect: None,
                                },
                            );
                        }
                    }
                    FocusTarget::Peg(kind) => {
                        let (title, remaining, label, accent) = match kind {
                            PegKind::Hands => (
                                "Plays",
                                gameplay.plays_remaining,
                                if gameplay.plays_remaining == 1 {
                                    "play"
                                } else {
                                    "plays"
                                },
                                color::JADE,
                            ),
                            PegKind::Discards => (
                                "Discards",
                                gameplay.discards_remaining,
                                if gameplay.discards_remaining == 1 {
                                    "discard"
                                } else {
                                    "discards"
                                },
                                color::AMBER,
                            ),
                        };
                        let desc = format!("You have {remaining} {label} remaining.\n\nYou can {label} multiple {} at once.", (if label == "discard" { "tiles" } else { "melds" }));
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title,
                                desc: &desc,
                                cta: "",
                                accent_color: accent,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::Gold => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Yen",
                                desc: "Your wealth in yen",
                                cta: &format!("¥{}", gameplay.yen),
                                accent_color: color::GOLD,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::DiscardUndo => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Undo discard",
                                desc: "Confirm to restore your previous hand and wall before the last discard. Clears when you play, use a consumable, or discard again.",
                                cta: "",
                                accent_color: color::CHAMPAGNE,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::Button(GameplayButton::Discard) => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Discard",
                                desc: "Confirm to discard the selected tiles from your hand.",
                                cta: "",
                                accent_color: color::CHAMPAGNE,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::Button(GameplayButton::Play) => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Play",
                                desc: "Confirm to play the selected meld into your structure.",
                                cta: "",
                                accent_color: color::CHAMPAGNE,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::Button(GameplayButton::Trigger) => {
                        let (title, desc, accent) = if cash_in_blocked {
                            let boss = if boss_title_text.is_empty() {
                                "The boss".to_string()
                            } else {
                                boss_title_text.clone()
                            };
                            let discards = gameplay.discards_remaining;
                            let discard_word = if discards == 1 { "discard" } else { "discards" };
                            let desc = if boss_rule_text.is_empty() {
                                format!(
                                    "{boss} is blocking cash-in until you use all discards ({discards} {discard_word} left)."
                                )
                            } else {
                                format!("{boss_rule_text} ({discards} {discard_word} left).")
                            };
                            ("Cash In".to_string(), desc, color::RUBY)
                        } else {
                            (
                                "Cash In".to_string(),
                                "Confirm to score the melds in your structure and end the round."
                                    .to_string(),
                                color::CHAMPAGNE,
                            )
                        };
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: title.as_str(),
                                desc: desc.as_str(),
                                cta: if cash_in_blocked { "Ordeal Rule" } else { "" },
                                accent_color: accent,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::Journal => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Yaku",
                                desc: "Confirm to open the yaku journal and browse scored hands.",
                                cta: "",
                                accent_color: color::CHAMPAGNE,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::Guidebook => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Guide",
                                desc: "Confirm to open the guide and browse tiles, melds, and rules.",
                                cta: "",
                                accent_color: color::CHAMPAGNE,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::ScoreRoller(ScoreRollerBank::Score) => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Round score",
                                desc: "Points you've earned this round.",
                                cta: &format_score(live_score),
                                accent_color: color::CHAMPAGNE,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    FocusTarget::ScoreRoller(ScoreRollerBank::Target) => {
                        push_focus_tooltip_panel_2d(
                            &mut inspect_tooltip_quads,
                            &mut inspect_tooltip_texts,
                            FocusTooltipPanelParams {
                                window_w: layout.window_w,
                                window_h: layout.window_h,
                                anchor_rect: Some(rect),
                                title: "Blind target",
                                desc: "Score needed to clear this round.",
                                cta: &format_score(gameplay.target_score as u64),
                                accent_color: color::BRASS,
                                hover_is_owned: false,
                                skip_title_block: false,
                                avoid_rect: None,
                            },
                        );
                    }
                    _ => {}
                }
            }
            if !inspect_tooltip_quads.is_empty() || !inspect_tooltip_texts.is_empty() {
                frame.overlay_quads(inspect_tooltip_quads);
                frame.texts(inspect_tooltip_texts);
            }
        }

        // Centralized focus ring: a single brass frame around whatever
        // `self.focus` is currently pointing at, in the `hover_quads`
        // batch so it sits above all HUD elements. Hand tiles get their
        // focus indicator via `ShowcaseTilePlacement.outline`, so we
        // suppress the 2D ring for HandTile to avoid double-ringing.
        if let Some(target) = self.focus
            && !matches!(target, FocusTarget::HandTile(_))
        {
            let rect_lookup = focus_rect_graph
                .iter()
                .find_map(|(t, r)| (*t == target).then_some(*r));
            if let Some(rect) = rect_lookup {
                push_focus_ring(
                    rect,
                    layout_scale,
                    layout.window_w,
                    layout.window_h,
                    &mut hover_quads,
                );
            }
        }

        // Focus-ring layer (brass frame only). Pushed *after* `hand_tile_faces`.
        if !ctx.modal_active {
            frame.quads(hover_quads);
        }

        // PAUSE OVERLAY: dim panel + buttons + text built earlier into
        // its own buffers. Sits above the hover layer so the pause menu
        // always visually wins.
        frame.quads(pause_quads);
        frame.texts(pause_text);

        // Projected rects for the discard river + play mirror: hit-test order
        // before the fullscreen 3D catch-all (same id — dispatcher uses
        // `picked_gameplay_object`). Labels render centered in these rects in the
        // persistent HUD text pass above.
        if !self.pause_menu.paused {
            if discard_btn_rect.2 > 1.0 && discard_btn_rect.3 > 1.0 {
                buttons.push(ButtonDef::scene(
                    (
                        discard_btn_rect.0,
                        discard_btn_rect.1,
                        discard_btn_rect.2,
                        discard_btn_rect.3,
                    ),
                    GAMEPLAY_3D_HIT_ID,
                ));
            }
            if play_hit_rect.2 > 1.0 && play_hit_rect.3 > 1.0 {
                buttons.push(ButtonDef::scene(
                    (
                        play_hit_rect.0,
                        play_hit_rect.1,
                        play_hit_rect.2,
                        play_hit_rect.3,
                    ),
                    GAMEPLAY_3D_HIT_ID,
                ));
            }
            if cash_in_visible {
                let (bx, by, bw, bh) = trigger_btn_rect;
                buttons.push(ButtonDef::scene((bx, by, bw, bh), GAMEPLAY_3D_HIT_ID));
            }
        }
        // Catch-all 3D-hit dispatcher: a full-screen `ButtonDef::scene`
        // pushed last so it only wins the first-hit search if no other
        // (smaller) button matched the cursor first. The matching click
        // routes through `update()` via `ctx.picked_gameplay_object`.
        //
        // Only push it when the cursor is *actually* over a 3D pickable
        // this frame. Otherwise the fullscreen rect intercepts clicks
        // meant for hand tiles (which aren't buttons — they're routed
        // through `pointer_slot` in `main.rs`'s `MouseInput` handler) and
        // silently drops them, since `picked_gameplay_object` is `None`
        // over a tile and the dispatch loop produces no action.
        //
        // Also suppressed while paused — the pause overlay already pushed
        // its own buttons + fullscreen blocker into `buttons` earlier in
        // this function, and the catch-all would otherwise intercept
        // clicks meant for the pause menu. Crucially we do *not* clear
        // `buttons` here: that would also wipe the pause-menu buttons we
        // just added, leaving the pause overlay completely unclickable.
        use crate::render::wgpu_renderer::GameplayPick;
        let push_3d_hit = if self.lab_mode {
            gameplay.trigger_enabled
                && matches!(ctx.picked_gameplay_object, Some(GameplayPick::CashInButton))
        } else {
            ctx.picked_gameplay_object.is_some()
        };
        if !self.pause_menu.paused && push_3d_hit {
            buttons.push(ButtonDef::scene(
                (0.0, 0.0, layout.window_w, layout.window_h),
                GAMEPLAY_3D_HIT_ID,
            ));
        }
        if !self.pause_menu.paused && self.cascade_queue.is_empty() {
            onboarding_hints::push_lessons_banner(
                &mut frame,
                &ctx,
                ctx.run,
                self.tutorial_panel_wiggle_x(now),
            );
            onboarding_hints::push_finale_intro_banner(&mut frame, &ctx, ctx.run);
        }

        if !self.pause_menu.paused
            && !ctx.modal_active
            && self.cascade_queue.is_empty()
        {
            let settings = crate::persistence::load_settings();
            // Single combined footer: available table actions (discard / play /
            // cash in) plus the guide hint, so the controls never collide with
            // or hide each other.
            let show_discard = super::action_prompts::gameplay_west_north_legend_active(
                ctx.input_mode,
                settings.xy_quick_action,
                self.focus,
                discard_enabled,
            );
            let show_play = super::action_prompts::gameplay_west_north_legend_active(
                ctx.input_mode,
                settings.xy_quick_action,
                self.focus,
                play_enabled,
            );
            let show_cash_in =
                gameplay.trigger_enabled || self.cash_in_hold_in_progress();
            let icon_slots = push_screen_footer_hint(
                &mut frame,
                &ctx,
                gameplay_footer_row(ctx.input_mode, show_discard, show_play, show_cash_in),
                HintStyle::standard(layout.window_w, layout.window_h),
            );

            // Hold-to-cash-in progress ring around the footer cash-in glyph (mirrors
            // the shop's hold-to-sell / hold-to-buy rings).
            if show_cash_in
                && let Some(InlineHintIconSlot { icon_rect, .. }) =
                    icon_slots.iter().find(|s| is_cash_in_hint_key(s.key))
            {
                let cash_in_invalid =
                    self.cash_in_hold_started.is_some() && !gameplay.trigger_enabled;
                let progress = self
                    .cash_in_hold_progress(now, gameplay.trigger_enabled)
                    .unwrap_or(0.0);
                let [ix, iy, icon_px, _] = *icon_rect;
                let cx = ix + icon_px * 0.5;
                let cy = iy + icon_px * 0.5;
                let r = icon_px * 0.58;
                let thickness = (icon_px * 0.12).max(3.5);
                frame.arc_ring_quads([crate::ui::prompt_hold_ring::hold_prompt_ring(
                    cx,
                    cy,
                    r,
                    thickness,
                    progress,
                    cash_in_invalid,
                )]);
            }

            if !vis.hide_wall_hud {
                crate::render::wall_display::push_wall_remaining_hud(
                    &mut frame,
                    layout.window_w,
                    layout.window_h,
                    gameplay.tiles_left,
                );
            }
        }

        frame.buttons = buttons;
        frame.window_title = window_title;
        frame.debug_axes = self.debug_show_axes;

        if self.pause_menu.paused {
            self.pause_menu
                .stash_focus_nav_debug(&mut ctx, layout.window_w, layout.window_h);
        } else {
            ctx.stash_focus_nav_graph(
                &focus_rect_graph,
                &super::focus::gameplay_nav_edges(&focus_rect_graph),
                self.focus,
                self.focus_nav.memory(),
                |t| format!("{t:?}"),
            );
        }

        // Stash the focus rect graph for the next frame's `update()` to
        // hit-test the cursor and run spatial navigation against.
        *self.last_focus_rects.borrow_mut() = focus_rect_graph;

        // Cheap invariant check — catches future migration miseasons that
        // accidentally push two `HandTileFaces` markers (or zero of one)
        // into the cmds list, which would silently break tile-face z
        // order. Compiled out of release builds.
        debug_assert_marker_uniqueness(&frame);

        insert_structure_before_hand(frame, structure_showcase, vis)
    }
}
