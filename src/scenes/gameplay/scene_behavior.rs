use super::*;
use super::{animation_state, cascade_controller, input_handler};
use crate::core::consumable::Consumable;
use crate::core::relic::{RelicId, all_relic_defs, relic_description_live};
use crate::render::theme::color;
use crate::scenes::options;
use crate::scenes::{BackgroundId, GuideScene, OverlayRequest};
use crate::ui::inspect_plaque::{
    FocusTooltipPanelParams, dora_focus_tooltip_strings, gameplay_consumable_description_full,
    hand_tile_focus_tooltip, push_focus_tooltip_panel_2d, round_wind_focus_tooltip_strings,
};
use crate::ui::ordeal_icons::ordeal_icon_source;

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
            let _g = crate::render::cpu_profiler::scope("update.tick_wind_and_deal_detection");
            animation_state::tick_wind_and_deal_detection(self, &mut ctx, now);
        }
        {
            let _g = crate::render::cpu_profiler::scope("update.tick_gold_change_coins");
            animation_state::tick_gold_change_coins(self, &mut ctx);
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
        // transition clears. The post-deal wind gust that follows the
        // deal is what sweeps the remaining curtain away, so timing-wise
        // this lands at the end of the fade-in.
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
            // The pause menu's "Guide" entry sets a one-shot flag and
            // closes itself; drain the flag to push the Guide as an overlay.
            if self.pause_menu.take_guide_request() {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::Guide(
                    GuideScene::new(),
                ))));
                return None;
            }
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
                            src,
                            src,
                            None,
                            crate::core::scoring::StepKind::Gold,
                            new_level as f32,
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
        if let Some(deadline) = self.pending_refill {
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
                self.pending_refill = None;
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
        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
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

        let current_chamber = gameplay.run_number;
        // TODO there should be a const for this
        let total_chambers = 21;
        let chamber_name = gameplay.chamber.name();

        // The window title is recomputed unconditionally so the OS chrome
        // tracks the current run state even when the glossary takes over
        // the screen.
        let window_title = format!(
            "Mahjuro — {current_chamber}/{total_chambers} {chamber_name} {} / {}  Gold: {}  Hands: {}  Discards: {}",
            if !self.cascade_queue.is_empty() {
                self.displayed_score
            } else {
                gameplay.round_score
            },
            gameplay.target_score,
            gameplay.gold,
            gameplay.plays_remaining,
            gameplay.discards_remaining,
        );

        // Score-panel overlay quads (plays/discard pips) — empty; fans on the
        // table show remaining plays/discards. Kept as a vec for the HUD merge.
        let score_panel_quads = build_instances_from_layout(
            (
                layout.score_panel.x,
                layout.score_panel.y,
                layout.score_panel.w,
                layout.score_panel.h,
            ),
            (
                layout.modifier_strip.x,
                layout.modifier_strip.y,
                layout.modifier_strip.w,
                layout.modifier_strip.h,
            ),
            ctx.anim.transform_for(ENTITY_SCORE_PANEL).scale,
            gameplay.plays_remaining,
            gameplay.plays_max,
            gameplay.discards_remaining,
            gameplay.discards_max,
        );

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
        let showcase_present = has_structure || cascade_showcase_ref.is_some();
        let (yaku_panel_h, structure_tag_h, structure_meld_h) =
            super::glb_anchors::gameplay_hud_strip_heights(
                layout.window_h,
                layout_scale,
                showcase_present,
            );
        let structure_strip_h = (structure_tag_h + structure_meld_h).max(1.0);
        let glb_anchors = match super::glb_anchors::resolve_gameplay_glb_anchors(
            layout,
            interaction.hand_len,
            layout.window_w,
            layout.window_h,
            &scene_camera,
            env_h,
            structure_strip_h,
            yaku_panel_h,
        ) {
            Ok(anchors) => anchors,
            Err(e) => {
                log::error!("{e:#}");
                return super::glb_anchors::gameplay_glb_error_frame(layout, &e.to_string());
            }
        };
        let hand_slots = glb_anchors.hand_slots.clone();

        // Boss payload for the dedicated boss plinth inspect target.
        let boss_title_text = gameplay.ordeal_ofuda_title.clone();
        let boss_rule_text = gameplay.ordeal_ofuda_rule_text.clone();

        // Modifier strip: cascade / sets (full width). Relics shown as row below score panel.
        let cascade_frame = self
            .cascade_queue
            .front()
            .map(|(cascade, _)| cascade.frame(now));

        // Active relics are 3D medallions in a horizontal tray (`build_relic_tray_and_wind`).
        // No 2D relic badge strip here — `RelicIcon` GPU path stays empty.
        let relic_icons: Vec<crate::render::wgpu_renderer::RelicIcon> = Vec::new();

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
        let discard_btn_rect = glb_anchors.discard_btn_rect;
        let play_btn_rect = glb_anchors.play_btn_rect;
        let trigger_btn_rect = glb_anchors.cash_in_btn_rect;

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
        let discard_undo_rect: Option<[f32; 4]> = if !paused
            && !ctx.modal_active
            && self.cascade_queue.is_empty()
            && self.journal_transition.is_none()
            && crate::persistence::load_settings().discard_undo_enabled
            && self.discard_undo.is_some()
            && self.pending_refill.is_none()
            && let Some(bowl_rect) = ctx.proj.bowl_rect
        {
            let zscale = (layout.window_w.min(layout.window_h)) / 600.0;
            let gap = (6.0 * zscale).max(4.0);
            let btn_h = (28.0 * zscale).max(22.0);
            let btn_w = (88.0 * zscale).max(72.0);
            let bx = bowl_rect[0];
            let by = bowl_rect[1] + bowl_rect[3] + gap;
            Some([bx, by, btn_w, btn_h])
        } else {
            None
        };
        let btn_rects = [discard_btn_rect, play_btn_rect, trigger_btn_rect];
        let play_enabled = selection_valid && gameplay.plays_remaining > 0;
        let discard_enabled = selected_count > 0 && gameplay.discards_remaining > 0;
        input_handler::push_action_button_focus_rects(
            &btn_rects,
            has_structure,
            &mut focus_rect_graph,
        );
        let action_row = input_handler::build_glb_action_pick_proxies(
            &glb_anchors,
            self.journal_open_amount,
            has_structure,
        );
        let input_handler::ActionRowOutputs {
            wood_tablet_placements,
            discard_bowl_placement,
            bronze_mirror_placement,
            journal_book,
        } = action_row;

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
        let dora_rect: [f32; 4] = plinth_focus_rect_from_anchor(
            &glb_anchors.tile_plinth_poses[0].anchor,
            layout,
            dora_tile_count,
        );
        let round_wind_tile_count = 1 + usize::from(gameplay.bonus_round_wind_rank.is_some());
        let round_wind_rect: [f32; 4] = plinth_focus_rect_from_anchor(
            &glb_anchors.tile_plinth_poses[1].anchor,
            layout,
            round_wind_tile_count,
        );
        let boss_plinth_rect: Option<[f32; 4]> = (!boss_title_text.is_empty()).then(|| {
            plinth_focus_rect_from_anchor(&glb_anchors.tile_plinth_poses[2].anchor, layout, 1)
        });
        let ordeal_icon_rect: Option<[f32; 4]> = boss_plinth_rect.map(|rect| {
            let icon_size = layout.mm(20.0).min(rect[2] * 0.70).min(rect[3] * 0.70);
            let anchor_x = rect[0] + rect[2] * 0.5;
            let anchor_y = rect[1] + rect[3] * 0.5;
            [
                anchor_x - icon_size * 0.5,
                anchor_y - icon_size * 0.5,
                icon_size,
                icon_size,
            ]
        });

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

        // Hint: compute tile indices that would complete a meld with current selection.
        // `suggest_completions` runs full backtracking validation per unselected
        // tile, so memoize against the inputs that affect its result (hand uids
        // + selection bitmask). Same hand+selection across frames → reuse.
        let hint_indices = if interaction.hints_enabled
            && !interaction.selected_indices.is_empty()
            && self.cascade_queue.is_empty()
        {
            let selection_mask: u32 = interaction
                .selected_indices
                .iter()
                .fold(0u32, |acc, &i| acc | (1u32 << i.min(31)));
            let mut cache = self.suggest_hint_cache.borrow_mut();
            if !cache.matches(&interaction.hand, selection_mask) {
                cache.hand_uids.clear();
                cache
                    .hand_uids
                    .extend(interaction.hand.iter().map(|t| t.id));
                cache.selection_mask = selection_mask;
                cache.hints = suggest_completions(&interaction.hand, &interaction.selected_indices);
            }
            cache.hints.clone()
        } else {
            vec![]
        };
        // Phase 8: the `?` glossary badge has been removed from the
        // gameplay HUD. The glossary is now reachable from the pause menu's
        // "Glossary" entry. The keyboard `Help` action shortcut still works
        // as a hidden affordance for power users.

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
            crate::scenes::options::OptionsDrawHint::pause_overlay(&ctx),
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

        let animation_state::CandleAndLightBuffers { spot_lights } = {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_candles_and_spotlights");
            animation_state::build_candles_and_spotlights(
                self,
                layout,
                &gameplay,
                glb_anchors.hand_world_slots.as_slice(),
                &hint_indices,
                &glb_anchors.tile_plinth_poses,
                ctx.progress.dora_enabled(),
            )
        };

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
        let _ = relic_icons; // gameplay no longer renders 2D relic icons.
        let mut frame = UiFrame::new();
        let fov_pop_offset = self.final_tiles_fov_pop_offset_deg(now);
        scene_camera.fovy_deg = (scene_camera.fovy_deg - fov_pop_offset).max(35.0);
        frame.camera_override = Some(scene_camera);
        frame.gameplay_action_picks = Some(crate::render::draw_cmd::GameplayActionPickProxies {
            bowl: discard_bowl_placement.clone(),
            mirror: bronze_mirror_placement.clone(),
            journal: journal_book.clone(),
            cash_in_tablet: wood_tablet_placements.first().cloned(),
        });
        frame.background(BackgroundId::Black);
        if !vis.hide_environment {
            frame.gameplay_environment();
        }
        let room_glb_lights = crate::render::gameplay_glb::gameplay_glb_has_embedded_lights();
        frame.scene_lighting.embedded_gltf_punctual = room_glb_lights;
        frame.scene_lighting.room_glb_brdf = room_glb_lights;
        if room_glb_lights && !vis.hide_candle_lights {
            let lamp_flicker = self.light_ramp * self.candle_wind_dim;
            frame.scene_lighting.punctual =
                crate::render::gameplay_glb::gameplay_embedded_point_lights_runtime(
                    layout.window_w,
                    layout.window_h,
                    env_h,
                    &ctx.shop_env_lighting,
                    self.candle_time,
                    lamp_flicker,
                )
                .into_iter()
                .map(crate::render::draw_cmd::ScenePunctualLight::InverseSquare)
                .collect();
            frame.scene_lighting.spot_lights =
                crate::render::gameplay_glb::gameplay_embedded_spot_lights_runtime(
                    layout.window_w,
                    layout.window_h,
                    env_h,
                    &ctx.shop_env_lighting,
                );
            let glb_flames = crate::render::gameplay_glb::gameplay_gltf_candle_flame_emitters(
                layout.window_h,
                env_h,
                lamp_flicker,
            );
            frame.candle_light_count = glb_flames.len() as u32;
            frame.flame_height_world = crate::render::flame_volume::shop_gltf_flame_height_world(
                crate::render::room_glb::room_env_world_scale(layout.window_h, env_h),
            );
            frame.procedural_flame_emitters = glb_flames;
        }

        let gold_label_rect = if vis.hide_gold_label {
            [0.0, 0.0, 0.0, 0.0]
        } else {
            crate::render::gold_display::push_gold_amount_label(
                &mut frame,
                layout.window_w,
                layout.window_h,
                gameplay.gold,
                (gold_pose.anchor[0], gold_pose.anchor[1]),
            )
        };
        if !vis.hide_wall_hud {
            crate::render::wall_display::push_wall_remaining_hud(
                &mut frame,
                layout.window_w,
                layout.window_h,
                gameplay.tiles_left,
            );
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
            // Dora face set for highlighting matching hand tiles. Empty
            // before dora unlocks (level 4) so the marker only appears
            let mut hand_placements: Vec<crate::render::draw_cmd::ShowcaseTilePlacement> =
                Vec::with_capacity(hand.len());
            let (invalid_flash, invalid_elapsed) = self.invalid_meld_flash_phase(now);
            for (i, &tile) in hand.iter().enumerate() {
                let tile = Self::display_tile(tile, run);
                let is_selected = interaction.selected.get(i).copied().unwrap_or(false);
                let is_focused = focus == i;
                let is_hinted = hint_indices.contains(&i);
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
                let (cx, cy, lift, size_px) = (
                    px + slide_x_px + reject_shake_x,
                    py + pop_offset,
                    lift_z,
                    sw * hand_scale_mul,
                );
                hand_placements.push(crate::render::draw_cmd::ShowcaseTilePlacement {
                    tile,
                    center_pos: [cx, cy, lift],
                    rotation: hand_rot,
                    scale: slide_y_frac.max(0.05),
                    size_px,
                    brightness: if is_invalid_flash { 1.12 } else { 1.0 },
                    // Suppress gold selection rim on straggler tiles so red reads clearly.
                    selected: is_selected && !is_invalid_flash,
                    hovered: is_focused && !is_invalid_flash,
                    outline: (is_selected || is_focused) && !is_invalid_flash,
                    glow: is_hinted || is_invalid_flash || is_selected,
                    glow_color: if is_invalid_flash {
                        Some([1.00, 0.14, 0.08, 0.72 + 0.28 * invalid_flash])
                    } else {
                        None
                    },
                    pick_id: Some(i),
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
        // their destinations use `score_counter_layout`.
        let score_counter = super::score_counter::score_counter_layout(layout, &self.positions);
        // Debug visibility: `hide_score_readout` gates the 2D score line.
        // Anchor it from the plaque's *actual* left edge instead of the raw
        // score-panel bounds: perspective projection pulls taller / higher
        // objects inward on screen, so a naive "some pixels left of
        // score_panel.x" anchor can still drift back over the wood plaque
        // and obscure the plaque text.
        // Counter fans — upright bone-stick tallies standing in front of
        // the action objects. Draws fan (jade tips) stands in front of the
        // bronze mirror; discards fan (amber tips) stands in front of the
        // discard river. Each stick = one remaining action; the fan thins
        // from the outermost stick inward as the count drops, so the
        // upright core stays intact and the consumption reads as a spent
        // stick rather than a re-deal.
        {
            let stick_len = layout.mm(28.0);
            let stick_wide = layout.mm(4.0);
            let stick_thickness = layout.mm(1.5);
            let spread_deg = 60.0;
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
                        tip_color: crate::render::theme::color::JADE,
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
                        tip_color: crate::render::theme::color::AMBER,
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
            const FLASH_MS: f32 = 400.0;
            let elapsed_ms = now.saturating_duration_since(t0).as_secs_f32() * 1000.0;
            if elapsed_ms < FLASH_MS {
                let t = elapsed_ms / FLASH_MS;
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
            let cap = score_panel_quads.len().saturating_add(hud_quads.len());
            let mut merged = Vec::with_capacity(cap);
            merged.extend(score_panel_quads);
            merged.append(&mut hud_quads);
            frame.quads(merged);
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
        if !vis.hide_boss_icon
            && let (Some(ordeal_kind), Some(icon_rect)) = (gameplay.ordeal_kind, ordeal_icon_rect)
        {
            let (boss_glow, boss_wiggle) = self.boss_rule_feedback(now, boss_blocks_selection);
            if boss_glow > 0.0 {
                let pad = icon_rect[2].max(icon_rect[3]) * 0.42;
                hud_quads.push(GpuInstance {
                    rect: [
                        icon_rect[0] - pad + boss_wiggle,
                        icon_rect[1] - pad,
                        icon_rect[2] + pad * 2.0,
                        icon_rect[3] + pad * 2.0,
                    ],
                    color: [1.0, 0.48, 0.10, 0.18 + 0.42 * boss_glow],
                    user: 0,
                });
            }
            frame.image_quads(std::iter::once(crate::render::draw_cmd::ImageQuad {
                inst: GpuInstance {
                    rect: [
                        icon_rect[0] + boss_wiggle,
                        icon_rect[1],
                        icon_rect[2],
                        icon_rect[3],
                    ],
                    color: color::alpha(color::CHAMPAGNE, 0.98),
                    user: 0,
                },
                source: ordeal_icon_source(ordeal_kind),
            }));
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
        frame.scene_lighting.spot_lights.extend(spot_lights);
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
                out.push(TextLabel {
                    rect,
                    text: copy.into(),
                    color: color::CHAMPAGNE,
                    font_px: Some(fs),
                    align: crate::render::wgpu_renderer::TextAlign::Center,
                    no_glossary: true,
                    ..Default::default()
                });
            };
            if let Some(rect) = ctx.proj.bowl_rect {
                push_centered(&mut hud_text, rect, "Discard");
            }
            if let Some(rect) = ctx.proj.mirror_rect {
                push_centered(&mut hud_text, rect, "Play");
            }
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
                        no_glossary: true,
                        ..Default::default()
                    });
                }
            }
            let settings = crate::persistence::load_settings();
            let show_discard_legend = super::action_prompts::gameplay_west_north_legend_active(
                ctx.input_mode,
                settings.xy_quick_action,
                self.focus,
                discard_enabled,
            );
            let show_play_legend = super::action_prompts::gameplay_west_north_legend_active(
                ctx.input_mode,
                settings.xy_quick_action,
                self.focus,
                play_enabled,
            );
            super::action_prompts::push_gameplay_action_prompts(
                &mut frame,
                &ctx,
                super::action_prompts::GameplayActionPromptInput {
                    discard_btn_rect,
                    play_btn_rect,
                    trigger_btn_rect,
                    cash_in_enabled: gameplay.trigger_enabled,
                    show_discard_legend,
                    show_play_legend,
                    hud_text: &mut hud_text,
                },
            );
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
                    no_glossary: true,
                    ..Default::default()
                });
            }
        }

        let hud_text = if !vis.hide_score_readout {
            use crate::render::theme::typography;
            let sp = layout.score_panel;
            let zc = (layout.window_w / 900.0).max(0.55);
            let ww = layout.window_w;
            let wh = layout.window_h;

            // The Cassowary score strip is only ~9% of window height (`layout.rs`).
            // Basing font caps on that rect forces a microscopic ceiling; pin H6 from
            // `window_h` and rasterize inside a wide, tall enough rect centered on
            // the strip (see `font-scaling.md`).
            let main_fs = typography::size(typography::H6, wh);
            let label_w = (ww * 0.52).max(sp.w + 36.0 * zc).min(ww - 16.0).max(160.0);
            let label_h = (main_fs * 1.48).clamp(64.0, wh * 0.22);
            let anchor_x = sp.x + sp.w * 0.5;
            let anchor_y = sp.y + sp.h * super::score_counter::readout_2d::ANCHOR_Y_FRAC;
            let mut main_rect = [
                anchor_x - label_w * 0.5,
                anchor_y - label_h * 0.5,
                label_w,
                label_h,
            ];
            main_rect[0] = main_rect[0].clamp(8.0, (ww - main_rect[2] - 8.0).max(8.0));
            main_rect[1] = main_rect[1].max(6.0);

            let live_score = if !self.cascade_queue.is_empty() {
                self.displayed_score
            } else {
                gameplay.round_score
            };
            let main_line = format!("{live_score} / {}", gameplay.target_score);

            let mut v = Vec::with_capacity(1 + hud_text.len());
            v.push(TextLabel {
                rect: main_rect,
                text: main_line,
                color: color::CHAMPAGNE,
                font_px: Some(main_fs),
                align: crate::render::wgpu_renderer::TextAlign::Center,
                no_glossary: true,
                bold: true,
                ..Default::default()
            });
            v.append(&mut hud_text);
            v
        } else {
            hud_text
        };
        frame.texts(hud_text);

        if let Some(undo_rect) = discard_undo_rect {
            focus_rect_graph.push((FocusTarget::DiscardUndo, undo_rect));
        }

        // Append the deferred focus rect entries (hand tiles, relics,
        // pegs, gold) before the centralized focus ring so the lookup
        // can find them. The button-bar and consumable strip already
        // pushed their entries inline above.
        for (i, rect) in ctx.proj.hand_rects.iter() {
            focus_rect_graph.push((FocusTarget::HandTile(*i), *rect));
        }
        if ctx.proj.hand_rects.is_empty() {
            for (i, slot) in hand_slots.iter().enumerate() {
                focus_rect_graph.push((FocusTarget::HandTile(i), [slot.0, slot.1, slot.2, slot.3]));
            }
        }
        for (i, r) in ctx.proj.relic_rects.iter().enumerate() {
            if r[2] > 1.0 && r[3] > 1.0 {
                focus_rect_graph.push((FocusTarget::Relic(i), *r));
            }
        }
        if let Some(r) = ctx.proj.peg_rects[0]
            && r[2] > 1.0
            && r[3] > 1.0
        {
            focus_rect_graph.push((FocusTarget::Peg(PegKind::Hands), r));
        }
        if let Some(r) = ctx.proj.peg_rects[1]
            && r[2] > 1.0
            && r[3] > 1.0
        {
            focus_rect_graph.push((FocusTarget::Peg(PegKind::Discards), r));
        }
        // Anchor the gold focus rect to the actual 3D coin pile (when
        // there is gold to display). The pile rect was computed up at
        // the top of `draw_frame` so the focus ring and
        // physical pile draw all share one source of truth.
        focus_rect_graph.push((FocusTarget::Gold, gold_label_rect));
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
                                run.gold,
                                Some((&run.relics, i)),
                                Some(run.ghost_hand_preview_chips()),
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
                        let desc = format!("You have {remaining} {label} remaining.");
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
                                title: "Gold",
                                desc: "Your current treasure",
                                cta: &format!("{}g", gameplay.gold),
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
                                desc: "Confirm to restore your previous hand and wall before the last discard. Clears when you play, sort, use a consumable, or discard again.",
                                cta: "",
                                accent_color: color::CHAMPAGNE,
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
            if let Some(rect) = ctx.proj.bowl_rect {
                buttons.push(ButtonDef::scene(
                    (rect[0], rect[1], rect[2], rect[3]),
                    GAMEPLAY_3D_HIT_ID,
                ));
            }
            if let Some(rect) = ctx.proj.mirror_rect {
                buttons.push(ButtonDef::scene(
                    (rect[0], rect[1], rect[2], rect[3]),
                    GAMEPLAY_3D_HIT_ID,
                ));
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
        if !self.pause_menu.paused && ctx.picked_gameplay_object.is_some() {
            buttons.push(ButtonDef::scene(
                (0.0, 0.0, layout.window_w, layout.window_h),
                GAMEPLAY_3D_HIT_ID,
            ));
        }
        if !self.pause_menu.paused && self.cascade_queue.is_empty() {
            onboarding_hints::push_lessons_banner(&mut frame, &ctx, ctx.run);
            onboarding_hints::push_finale_intro_banner(&mut frame, &ctx, ctx.run);
        }

        frame.buttons = buttons;
        frame.window_title = window_title;
        frame.debug_axes = self.debug_show_axes;

        // Stash the focus rect graph for the next frame's `update()` to
        // hit-test the cursor and run spatial navigation against.
        *self.last_focus_rects.borrow_mut() = focus_rect_graph;

        // Cheap invariant check — catches future migration mistakes that
        // accidentally push two `HandTileFaces` markers (or zero of one)
        // into the cmds list, which would silently break tile-face z
        // order. Compiled out of release builds.
        debug_assert_marker_uniqueness(&frame);

        insert_structure_before_hand(frame, structure_showcase, vis)
    }
}
