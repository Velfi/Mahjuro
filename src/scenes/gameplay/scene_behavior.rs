use super::*;
use super::{animation_state, cascade_controller, input_handler};
use crate::core::consumable::Consumable;
use crate::core::relic::{all_relic_defs, relic_description_live};
use crate::render::table_transform::{mat4_to_euler_xyz_rad, rot_euler_xyz_rad};
use crate::render::theme::color;
use crate::scenes::options;
use crate::scenes::{BackgroundId, MeldGuideScene, OverlayRequest};
use crate::ui::inspect_plaque::{
    FocusTooltipPanelParams, dora_focus_tooltip_strings, round_wind_focus_tooltip_strings,
    gameplay_consumable_description_full, hand_tile_focus_tooltip, push_focus_tooltip_panel_2d,
};

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

    fn update(&mut self, mut ctx: UpdateCtx<'_>) -> SceneTransition {
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let focus_kind_before = focus_kind(self.focus);
        {
            let _g = crate::render::cpu_profiler::scope("update.tick_basic_animations");
            animation_state::tick_basic_animations(self, &mut ctx, now, dt);
        }
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

        // Deferred round start: fire `apply_blind` once the opening candle
        // light-ramp has hit full brightness. The round's hand deal, boss
        // rules, and on-round-start relic triggers (Sweepstakes coin shower,
        // DoraCrown extra dora, future hooks) all happen now instead of
        // before the scene rendered — the player sees them unfold as the
        // transition clears. The post-deal wind gust that follows the
        // deal is what sweeps the remaining curtain away, so timing-wise
        // this lands at the end of the fade-in.
        if let Some(blind) = self.pending_blind {
            // Keep the felt empty behind the opening transition. Paths
            // that land here may have left a stale hand on the run state
            // (first-round `RunState::new` pre-draws, tutorial retry re-deals
            // before transitioning), and `apply_blind` will do the real
            // deal once the ramp completes.
            GameEngine::prepare_pending_blind(ctx.run);
            if self.light_ramp >= 1.0 {
                let mut engine = GameEngine::new(ctx.run, ctx.bus);
                let _ = engine.dispatch(GameCommand::ApplyBlind { blind });
                if ctx.run.onboarding_lessons_active() {
                    ctx.run.seed_onboarding_hand();
                }
                self.pending_blind = None;
            }
        }

        onboarding_hints::sync_onboarding_step(ctx.run);

        // Scene transition in progress — keep animations running but block
        // all input so the player can't alter game state during the fade-out.
        // Also block while `pending_blind` is set: the scene is rendering the
        // previous round's state behind the opening transition, and any
        // input would act on that stale state instead of the round that's
        // about to start.
        if ctx.transitioning || self.pending_blind.is_some() {
            return None;
        }

        if input_handler::tick_gameplay_journal_transition(self, &mut ctx, now, dt) {
            return None;
        }

        // Help action opens the Meld Guide scene (replaces the old glossary overlay).
        for &cid in ctx.button_clicks {
            if cid == HELP_BADGE_ID {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::MeldGuide(
                    MeldGuideScene::new(),
                ))));
                return None;
            }
        }
        for a in ctx.actions {
            if matches!(a, UiAction::Help) {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::MeldGuide(
                    MeldGuideScene::new(),
                ))));
                return None;
            }
        }

        // Pause menu handling — drives the menu while paused and intercepts
        // the open-on-Pause shortcut. Returns immediately if either applies.
        if let Some(t) = self.pause_menu.handle(&mut ctx) {
            // The pause menu's "Meld Guide" entry sets a one-shot flag and
            // closes itself; drain the flag to push the Meld Guide as an overlay.
            if self.pause_menu.take_meld_guide_request() {
                *ctx.overlay_request = Some(OverlayRequest::Push(Box::new(Scene::MeldGuide(
                    MeldGuideScene::new(),
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

        // If a discard is waiting for its departure animation to play out,
        // hold input until the deadline passes, then auto-draw replacements.
        if let Some(deadline) = self.pending_refill {
            if now >= deadline {
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

        // The window title is recomputed unconditionally so the OS chrome
        // tracks the current run state even when the glossary takes over
        // the screen.
        let window_title = format!(
            "Mahjuro — {} Round {}  {} / {}  Gold: {}  Hands: {}  Discards: {}",
            gameplay.blind.name(),
            gameplay.run_number,
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

        let ts = ctx.anim.transform_for(ENTITY_SCORE_PANEL);

        // Score-panel cartouche + modifier strip backplane quads. Returned
        // as a vec because they need to land in the persistent-HUD section
        // of the frame, which is built further down.
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
            ts.scale,
            gameplay.plays_remaining,
            gameplay.plays_max,
            gameplay.discards_remaining,
            gameplay.discards_max,
        );

        let hand_slots = hand_slots_for_count(layout, interaction.hand_len);

        // Score header is split into two lines so it doesn't get auto-shrunk
        // into the cartouche. The cartouche is only ~38% of the score panel
        // width and the rasterizer's width-based fallback would otherwise
        // squeeze a 100-char single-line string to the 8px floor.
        let score_text_top = format!(
            "{}  ·  R{}  ·  / {}",
            gameplay.blind_label, gameplay.run_number, gameplay.target_score,
        );
        // Boss-rule ofuda payload: derived independently from `run` so the
        // hanging paper always reflects the active boss rule, regardless of
        // whether a cascade is currently animating in the modifier strip.
        let ofuda_title_text = gameplay.boss_ofuda_title.clone();
        let ofuda_rule_text = gameplay.boss_ofuda_rule_text.clone();

        // Modifier strip: cascade / sets (full width). Relics shown as row below score panel.
        let cascade_frame = self
            .cascade_queue
            .front()
            .map(|(cascade, _)| cascade.frame(now));
        // Cascade chips/mult bone tokens — populated only while a cascade is
        // active. The numerical readout that used to sit on top of these
        // tokens (and the idle "Select tiles to play" / hand-preview line)
        // is now handled by the floating 3D `ScorePopupSystem`, so no 2D
        // labels are pushed for the modifier strip anymore.
        let mut cascade_token_placements: Vec<Object3d> = Vec::new();
        if let Some(frame) = cascade_frame.as_ref() {
            // Geometry (chips pill on the left, mult pill on the right of the
            // modifier strip) lives in `cascade_token_layout` so the popup
            // streaming destinations stay locked to the drawn tokens.
            let tokens = Self::cascade_token_layout(layout);
            let extents = [
                tokens.pill_w * 0.5,
                (tokens.pill_h * 0.6 * 0.5).max(4.0),
                tokens.pill_h * 0.5,
            ];

            // Pulse envelope: fast pop-in then settle. Active token grows ~12%.
            let pulse_strength = (1.0 - frame.phase_t * 1.6).clamp(0.0, 1.0);
            let pulse_for = |axis: StepKind| {
                if frame.pulse_axis == Some(axis) {
                    1.0 + 0.12 * pulse_strength
                } else {
                    1.0
                }
            };

            for (center, axis, token_kind) in [
                (
                    tokens.chips_center,
                    StepKind::Chips,
                    crate::render::draw_cmd::CascadeTokenKind::Chips,
                ),
                (
                    tokens.mult_center,
                    StepKind::Mult,
                    crate::render::draw_cmd::CascadeTokenKind::Mult,
                ),
            ] {
                let pulse_t = ((pulse_for(axis) - 1.0) / 0.12).clamp(0.0, 1.0);
                cascade_token_placements.push(Object3d {
                    pos: [center.0, center.1, 24.0],
                    extents,
                    rotation: [0.0, 0.0, 0.0],
                    color: [1.0, 1.0, 1.0, 1.0],
                    kind: Object3dKind::CascadeToken {
                        kind: token_kind,
                        pulse: pulse_t,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }

            let _ = &frame.latest_step;
        }

        // Active relics are 3D medallions in a horizontal tray (`build_relic_tray_and_wind`).
        // No 2D relic badge strip here — `RelicIcon` GPU path stays empty.
        let relic_icons: Vec<crate::render::wgpu_renderer::RelicIcon> = Vec::new();

        // Bottom button bar: discard bowl, bronze mirror, journal — see
        // [`action_bar_layout`] for spacing, lift, and rects.
        let layout_scale = (layout.window_w.min(layout.window_h)) / 600.0;
        let selected_count = gameplay.selected_count;
        let selection_valid = GameEngine::selection_is_valid(run);

        // Bowl + mirror: own row below the hand tile slots, above the journal
        // (discard left, play right within the centered playfield). Click rects
        // match diameter.
        //
        // Vertical order: structure strip, yaku tablets, hand rack, bowl/mirror
        // row, then journal.
        let has_structure = gameplay.has_structure;
        let cascade_showcase_ref = self.cascade_queue.front().and_then(|(_, sc)| sc.as_ref());
        let showcase_present = has_structure || cascade_showcase_ref.is_some();
        let hud_layout =
            compute_gameplay_hud_layout(layout, &hand_slots, has_structure, showcase_present);
        let yaku_panel_h = hud_layout.yaku_panel_h;
        let structure_tag_h = hud_layout.structure_tag_h;
        let structure_meld_h = hud_layout.structure_meld_h;
        let structure_strip_top = hud_layout.structure_strip_top;
        let yaku_row_y = hud_layout.yaku_row_y;
        let ab = hud_layout.action_bar;
        let ActionBarLayout {
            scale,
            container_w,
            container_x,
            journal_btn_rect,
            journal_btn_cx,
            discard_btn_rect,
            play_btn_rect,
            trigger_btn_rect,
            action_hud_table_lift,
            ..
        } = ab;
        let action_world_z_py = action_hud_world_z_py_nudge(layout_scale);

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
        let mut hud_quads: Vec<GpuInstance> = Vec::new();
        let mut hud_text: Vec<TextLabel> = Vec::new();
        let mut inspect_tooltip_quads: Vec<GpuInstance> = Vec::new();
        let mut inspect_tooltip_texts: Vec<TextLabel> = Vec::new();
        let mut structure_showcase: Vec<ShowcaseTilePlacement> = Vec::new();
        let mut structure_pile_tokens: Vec<Object3d> = Vec::new();
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
            structure_pile_tokens: yaku_structure_pile_tokens,
            cam_euler,
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
                container_w,
                container_x,
                structure_strip_top,
                structure_tag_h,
                structure_meld_h,
                yaku_panel_h,
                yaku_row_y,
                trigger_btn_rect,
            )
        };
        // Merge the yaku-panel structure showcase + preview pile pushes
        // into the outer accumulators so the rest of the draw_frame logic
        // (which pushes its own placements) can append to them.
        structure_showcase.extend(yaku_structure_showcase);
        structure_pile_tokens.extend(yaku_structure_pile_tokens);

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
        let btn_rects = [
            discard_btn_rect,
            play_btn_rect,
            trigger_btn_rect,
        ];
        let play_enabled = selection_valid && gameplay.plays_remaining > 0;
        let discard_enabled = selected_count > 0 && gameplay.discards_remaining > 0;
        let input_handler::ActionRowOutputs {
            mut wood_tablet_placements,
            discard_bowl_placement,
            bronze_mirror_placement,
            journal_book,
        } = {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_action_row_and_journal");
            input_handler::build_action_row_and_journal(
                self,
                layout,
                run,
                &ctx,
                &gameplay,
                &btn_rects,
                journal_btn_rect,
                journal_btn_cx,
                action_world_z_py,
                action_hud_table_lift,
                cam_euler,
                has_structure,
                play_enabled,
                discard_enabled,
                now,
                &mut focus_rect_graph,
            )
        };
        let _ = &mut wood_tablet_placements;

        // Score-panel text fits inside the narrow centered cartouche painted
        // by `build_instances_from_layout`. Cartouche geometry mirrors that
        // function: 38% width × 78% height of the score-panel region, applied
        // to the same scale-pop transform so the text grows with the
        // cartouche on score changes.
        let sp = layout.score_panel;
        let cart_w = sp.w * 0.38;
        let cart_h = sp.h * 0.78;
        let cart_x = sp.x + (sp.w - cart_w) * 0.5;
        let cart_y = sp.y + (sp.h - cart_h) * 0.5;

        // Dora indicator screen rect. Pre-computed up here so the focus
        // rect graph entry can both use it.
        // Prefer the renderer's projected plinth rect (one frame stale,
        // tracks the actual on-screen quad as the camera or arrange-mode
        // overrides shift it). Falls back to a screen-position estimate on
        // the first frame before the projection cache has populated.
        let dora_rect: [f32; 4] = ctx.proj.dora_plinth_rect.unwrap_or_else(|| {
            let dora_x = self.positions.dora.nx * layout.window_w;
            let dora_y = self.positions.dora.ny * layout.window_h;
            let dora_w = layout.mm(48.0);
            let dora_h = layout.mm(34.0);
            [dora_x - dora_w * 0.5, dora_y - dora_h * 0.5, dora_w, dora_h]
        });
        let round_wind_rect: [f32; 4] = ctx.proj.round_wind_plinth_rect.unwrap_or_else(|| {
            let rw_x = self.positions.round_wind.nx * layout.window_w;
            let rw_y = self.positions.round_wind.ny * layout.window_h;
            let rw_w = layout.mm(48.0);
            let rw_h = layout.mm(34.0);
            [rw_x - rw_w * 0.5, rw_y - rw_h * 0.5, rw_w, rw_h]
        });

        let gold_anchor = crate::render::gold_display::gameplay_gold_pile_anchor(
            layout,
            &self.positions.coin_pile,
        );
        let (ctx_x, ctx_y, ctx_w, ctx_h) =
            crate::render::animation::apply_transform_rect(cart_x, cart_y, cart_w, cart_h, ts);
        // Two-line stack inside the cartouche: top = blind/round/score (bot line
        // reserved on plaque for the score reel). Gold is the floating label + coin pile.
        // font_px so they render at the same readable size regardless of how
        // long the strings are. The cartouche header text + the modifier
        // strip cascade/idle text are kept in their own dedicated buffers
        // so the final assembly can place them between the score-panel
        // backplane quads and the rest of the HUD body.
        // Score header text is engraved directly onto the hanging plaque's
        // +Z face via the per-instance decal pipeline (see the plaque draw
        // path in `wgpu_renderer.rs` and `rasterize_plaque_decal` in
        // `decal.rs`). The two-line payload travels in `plaque_top_text` /
        // Plaque bottom band is left blank for the score reel — no 2D overlay
        // text is emitted for the header anymore, so the smoke composite can
        // drift over the wood face without text floating on top of it.
        let _ = (ctx_x, ctx_y, ctx_w, ctx_h, &score_text_top);
        // Cash-in / play labels are engraved on the wood tablets (per-instance decals).
        // Discard river + play mirror use centered text in their projected rects in the
        // persistent HUD pass (see `hud_text` just before `frame.texts(hud_text)`).

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

        let input_handler::ConsumableDishBuild {
            talisman_dish_placements,
            ribbon_dish_placements,
            talisman_dish_strip,
        } = {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_consumable_dish");
            input_handler::build_consumable_dish(
                self,
                layout,
                &ctx,
                &interaction,
                paused,
                &mut focus_rect_graph,
                &mut buttons,
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
            if let Some(affinity) = onboarding_hints::lessons_hint_indices(ctx.run) {
                affinity
            } else {
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
                    cache.hints =
                        suggest_completions(&interaction.hand, &interaction.selected_indices);
                }
                cache.hints.clone()
            }
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
            scale,
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

        let animation_state::CandleAndLightBuffers {
            flame_emitters,
            point_lights,
            spot_lights,
            candle_placements,
        } = {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_candles_and_spotlights");
            animation_state::build_candles_and_spotlights(
                self,
                layout,
                run,
                &gameplay,
                &hand_slots,
                &hint_indices,
                discard_bowl_placement.as_ref(),
                bronze_mirror_placement.as_ref(),
                ctx.debug_visibility.hide_candles,
                ctx.progress.dora_enabled(),
            )
        };

        // The 3D table + tiles + candles ARE the UI. Selection feedback is
        // now a true 3D gold-metal outline shell drawn by the renderer's
        // tile_outline_pipeline (which catches candlelight), so no 2D
        // selection overlay is added here.

        let relic_objects= {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_relic_tray");
            input_handler::build_relic_tray(self, layout, run)
        };

        // ── Frame assembly ──────────────────────────────────────────────
        //
        // Now push every layer into a fresh `UiFrame` in canonical order.
        let _ = relic_icons; // gameplay no longer renders 2D relic icons.
        let mut frame = UiFrame::new();
        let fov_pop_offset = self.final_tiles_fov_pop_offset_deg(now);
        {
            // Build camera from positions (multipliers on the cs-scaled defaults).
            let h = layout.window_h;
            let cs = h / 2104.0_f32;
            let gp = &self.positions;
            let mut camera = crate::render::draw_cmd::CameraParams {
                eye: [
                    0.0,
                    -2104.0 * cs * gp.camera_eye_y_mul,
                    1157.2 * cs * gp.camera_eye_z_mul,
                ],
                target: [
                    0.0,
                    -39.6 * cs * gp.camera_target_y_mul,
                    105.2 * cs * gp.camera_target_z_mul,
                ],
                up: [0.0, 0.0, 1.0],
                fovy_deg: gp.camera_fovy_deg,
                clip_near: None,
                clip_far: None,
            };
            camera.fovy_deg = (camera.fovy_deg - fov_pop_offset).max(35.0);
            frame.camera_override = Some(camera);
        }
        frame.background(BackgroundId::Black);
        frame.table();

        let gold_label_rect = crate::render::gold_display::push_gold_amount_label(
            &mut frame,
            layout.window_w,
            layout.window_h,
            gameplay.gold,
            (gold_anchor[0], gold_anchor[1]),
        );
        crate::render::wall_display::push_wall_remaining_hud(
            &mut frame,
            layout.window_w,
            layout.window_h,
            gameplay.tiles_left,
        );

        // Build hand tile placements for the showcase pipeline.
        // Each slot becomes one ShowcaseTilePlacement; the renderer draws,
        // picks, and projects them with no separate hand-tile GPU path.
        {
            let _g = crate::render::cpu_profiler::scope("draw_frame.hand_tile_placements");
            use crate::ui::layout::HAND_TILE_MESH_Y_FRAC;
            // Rx(+π/2): rotates face normal from +Z → -Y (toward camera at large -Y).
            // Rz(π): flips the tile's long axis so the top faces up (without Rz the top faces down).
            // The -22° lean tilts the top slightly toward the player.
            const HAND_TILE_RX: f32 =
                std::f32::consts::FRAC_PI_2 - 22.0_f32 * std::f32::consts::PI / 180.0;
            const HAND_TILE_RZ: f32 = std::f32::consts::PI;
            let hand = &interaction.hand;
            // Dora face set for highlighting matching hand tiles. Empty
            // before dora unlocks (level 4) so the marker only appears
            let mut hand_placements: Vec<crate::render::draw_cmd::ShowcaseTilePlacement> =
                Vec::with_capacity(hand.len());
            for (i, &tile) in hand.iter().enumerate() {
                let Some(&(sx, sy, sw, sh)) = hand_slots.get(i) else {
                    continue;
                };
                let tile = Self::display_tile(tile, run);
                let is_selected = interaction.selected.get(i).copied().unwrap_or(false);
                let is_focused = focus == i;
                let is_hinted = hint_indices.contains(&i);
                // Pop-in: slide_y 0→1, offset pixels downward (large py = nearer player).
                let slide_y_frac = self.hand_slide_y.get(i).copied().unwrap_or(1.0);
                let pop_offset = (1.0 - slide_y_frac) * sh * 0.3;
                let slide_x_px = self.hand_slide_x.get(i).copied().unwrap_or(0.0);
                let cx =
                    sx + sw * 0.5 + slide_x_px + self.positions.hand_strip.nx * layout.window_w;
                let cy = sy
                    + sh * HAND_TILE_MESH_Y_FRAC
                    + pop_offset
                    + self.positions.hand_strip.ny * layout.window_h;
                // Tile standing upright: center is at half the long dimension above the table.
                // Chinese tile: 30mm long, half = 15mm. layout.mm() converts mm → world units.
                let lift = layout.mm(15.0) + layout.mm(self.positions.hand_strip.lift_mm);
                hand_placements.push(crate::render::draw_cmd::ShowcaseTilePlacement {
                    tile,
                    center_pos: [cx, cy, lift],
                    rotation: [
                        HAND_TILE_RX + self.positions.hand_strip.rx_deg.to_radians(),
                        self.positions.hand_strip.ry_deg.to_radians(),
                        HAND_TILE_RZ + self.positions.hand_strip.rz_deg.to_radians(),
                    ],
                    scale: slide_y_frac.max(0.05),
                    size_px: sw,
                    brightness: 1.0,
                    selected: is_selected,
                    hovered: is_focused,
                    outline: is_selected || is_focused,
                    glow: is_selected || is_hinted,
                    glow_color: None,
                    pick_id: Some(i),
                });
            }
            if !hand_placements.is_empty() {
                frame.showcase_tile_batch(hand_placements);
            }
        }
        if !candle_placements.is_empty() {
            frame.object3d_batch(candle_placements.clone());
        }
        if !relic_objects.is_empty() {
            frame.object3d_batch(relic_objects);
        }
        // PERSISTENT HUD: hanging plaque + ofuda (3D wood/paper) → score
        // panel pip indicators → score header text → modifier strip text →
        // yaku card bodies + button bar quads + zodiac slots + particles +
        // help badge → button labels + zodiac labels + help text.
        // Persistent quads first, then text on top of them — exactly the
        // behaviour the legacy flush had, just scoped to the persistent
        // layer instead of mixing with hover content.
        //
        // The wooden plaque replaces the legacy slate-blue cartouche.
        // Positioned in pixel space matching the score panel rect, with a
        // modest world-Y lift so it reads as hanging above the table. The
        // header text is engraved directly onto the +Z face via a
        // per-instance decal — see the plaque draw path in
        // `wgpu_renderer.rs` and `rasterize_plaque_decal` in `decal.rs`.
        let sp = layout.score_panel;
        let plaque_thickness = 8.0_f32;
        // Lift is proportional to window height so the plaque tracks the
        // camera (which also scales with `h` — see `eye_height = h * 0.55`
        // in the renderer). A fixed world-unit lift drifts downward as the
        // window grows because the table grows around a constant lift.
        let plaque_lift = layout.mm(self.positions.plaque.lift_mm);
        // Push the plaque deeper into the scene (more negative world_z) so
        // it reads as hanging at the back of the room rather than right
        // above the player. pixel_y → world_z is a direct mapping in the
        // renderer's `pixel_to_world`, so subtracting from pixel_y here
        // moves the plaque back along the table's depth axis.
        let plaque_back_offset = layout.window_h * self.positions.plaque.ny;
        // Debug visibility: gated at the call site (rather than post-filtered)
        // because the status placard below also uses `DrawCmd::Plaque(_)` and
        // a cmd-level `retain` couldn't tell them apart.
        if !ctx.debug_visibility.hide_blind_plaque {
            frame.object3d(Object3d {
                pos: [
                    sp.x + sp.w * 0.5,
                    sp.y + sp.h * 0.5 - plaque_back_offset,
                    plaque_lift,
                ],
                extents: [sp.w * 0.95, sp.h * 1.8, plaque_thickness],
                rotation: mat4_to_euler_xyz_rad(
                    glam::Mat4::from_rotation_x((-65.0_f32).to_radians())
                        * rot_euler_xyz_rad(cam_euler[0], cam_euler[1], cam_euler[2]),
                ),
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::BeveledSlab,
                    // Top line is replaced by a blank: the floating score
                    // reel occupies that band of the plaque face.
                    material: crate::render::primitive::MaterialSpec::lacquered_wood_flat()
                        .with_decal(crate::render::primitive::plaque_decal("\n")),
                    pick_id: None,
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: None,
            });
        }
        // Ofuda only appears on boss blinds (where there's a rule to show).
        // Anchor it from the plaque's *actual* left edge instead of the raw
        // score-panel bounds: perspective projection pulls taller / higher
        // objects inward on screen, so a naive "some pixels left of
        // score_panel.x" anchor can still drift back over the wood plaque
        // and obscure the plaque text.
        if !ofuda_title_text.is_empty() {
            let sp = layout.score_panel;
            let ms_rect = layout.modifier_strip;
            // Keep the gameplay ofuda slimmer than the shrine-screen paper:
            // the main plaque also needs room for score line, so a
            // full-width warning card feels crowding here. Width set so the
            // wrapped rule body reads at table distance — too narrow and the
            // body shrinks to unreadable per-glyph sizes.
            let ofuda_w = ms_rect.w * 0.34;
            let ofuda_h = ms_rect.h * 1.55;
            let scale_c = (layout.window_w / 600.0).max(0.5);
            let plaque_w = sp.w * 0.95;
            let plaque_left = sp.x + (sp.w - plaque_w) * 0.5;
            // Give the paper extra berth from the plaque itself, not just
            // from the candle footprint. That keeps the plaque text readable
            // even after the taller paper projects inward toward center.
            let plaque_gap = 26.0 * scale_c;
            let candle_clearance = 120.0 * scale_c;
            let right_edge = (plaque_left - plaque_gap).min(sp.x - candle_clearance);
            let min_left_margin = ofuda_w * 0.5 + 12.0;
            let ofuda_cx = (right_edge - ofuda_w * 0.5).max(min_left_margin);
            // Push it up the back wall: smaller pixel-y → farther into z
            // (recessed against the wall behind the table) and a taller
            // lift so the paper hangs above the score-plaque elevation
            // rather than beside it. The mesh is now upright (no
            // toward-camera tilt — see ofuda_tilt_x in the renderer), so
            // raising it visually moves it up the back wall on screen.
            let ofuda_cy = sp.y - sp.h * 0.68;
            let ofuda_lift = layout.window_h * 0.45;
            let ofuda_p = &self.positions.ofuda;
            frame.object3d(Object3d {
                pos: [
                    ofuda_cx + ofuda_p.nx * layout.window_w,
                    ofuda_cy + ofuda_p.ny * layout.window_h,
                    ofuda_lift + layout.mm(ofuda_p.lift_mm),
                ],
                extents: [ofuda_w, ofuda_h, layout.mm(3.0)],
                // Placement rotation applied centrally via
                // `committed_arrange_rotations`.
                rotation: cam_euler,
                color: [1.0, 1.0, 1.0, 1.0],
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::Ofuda,
                    material: crate::render::primitive::MaterialSpec::plain().with_decal(
                        crate::render::primitive::DecalSpec {
                            text: format!("{}\n{}", ofuda_title_text, ofuda_rule_text),
                            layout: crate::render::primitive::DecalLayout::TitleRule {
                                target_short_edge: crate::render::decal::OFUDA_DECAL_LONG_EDGE,
                            },
                        },
                    ),
                    pick_id: None,
                    shadow_caster: false,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: Some("gameplay.score_panel.ofuda"),
            });
        }
        // Counter fans — upright bone-stick tallies standing in front of
        // the action objects. Draws fan (jade tips) stands in front of the
        // bronze mirror; discards fan (amber tips) stands in front of the
        // discard river. Each stick = one remaining action; the fan thins
        // from the outermost stick inward as the count drops, so the
        // upright core stays intact and the consumption reads as a spent
        // stick rather than a re-deal.
        let _ = score_panel_quads;
        let _ = (sp, plaque_back_offset, plaque_lift); // formerly anchored from the plaque
        {
            let stick_len = layout.mm(28.0);
            let stick_wide = layout.mm(4.0);
            let stick_thickness = layout.mm(1.5);
            let spread_deg = 60.0;
            // Push each fan just toward-the-camera of its anchor so the
            // sticks stand on the table surface in front of (not inside)
            // the mirror/river.
            let fan_forward_px = layout.mm(30.0);
            if let Some(mirror) = bronze_mirror_placement.as_ref() {
                let fan = &self.positions.counter_draws_fan;
                let fx = mirror.pos[0] + fan.nx * layout.window_w;
                let fy = mirror.pos[1] + fan_forward_px + fan.ny * layout.window_h;
                let flift = mirror.pos[2] + layout.mm(fan.lift_mm);
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
                        rotation_y_deg: fan.ry_deg,
                        kind: crate::render::draw_cmd::TallyFanKind::Draws,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
            if let Some(bowl) = discard_bowl_placement.as_ref() {
                let fan = &self.positions.counter_discards_fan;
                let fx = bowl.pos[0] + fan.nx * layout.window_w;
                let fy = bowl.pos[1] + fan_forward_px + fan.ny * layout.window_h;
                let flift = bowl.pos[2] + layout.mm(fan.lift_mm);
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
                        rotation_y_deg: fan.ry_deg,
                        kind: crate::render::draw_cmd::TallyFanKind::Discards,
                    },
                    hover_target: 0.0,
                    anim_id: 0,
                    arrange_name: None,
                });
            }
        }
        // (Score header text is now engraved on the plaque mesh as a
        // per-instance decal — no overlay TextLabels are pushed here.)
        // Phase 6: cascade scoring tokens (engraved bone, chips + mult)
        // pop in during a scoring cascade. Pushed before the cascade text
        // labels so the numbers read on top of the wood.
        if !cascade_token_placements.is_empty() {
            frame.object3d_batch(cascade_token_placements);
        }
        // Score reel — odometer digits floating in front of the plaque.
        // Always visible; anchor matches the plaque center but lifted a bit
        // further forward so it reads in front of the wood.
        {
            let reel_lift = plaque_lift * 1.08;
            let reel_px = sp.x + sp.w * 0.5;
            // Sit the reel in the top half of the plaque face. The plaque
            // decal leaves the top line blank for the reel and renders the
            // bottom band left for the reel; anchoring the
            // reel a quarter-height above center lines it up with the
            // vacated top band instead of painting over the bottom line.
            let reel_py = sp.y + sp.h * 0.25;
            // Reel uses world-yaw 0 here; plaque tilt is handled on the plaque mesh.
            let reel_placements = self.score_reel.placements(
                now,
                crate::render::world_space::PlacementAnchor {
                    anchor: crate::render::world_space::LayoutAnchorPx {
                        px: reel_px,
                        py: reel_py,
                        lift_z: reel_lift,
                    },
                    rot_y: 0.0,
                    scale: sp.h / 200.0,
                },
                Some(gameplay.target_score as u64),
            );
            if !reel_placements.is_empty() {
                frame.object3d_batch(reel_placements);
            }
        }
        // Floating extruded-glyph score popups (per-step "+50" / "×3").
        if self.score_popups.is_active() {
            let popup_scale = layout.window_w.min(layout.window_h) / 1080.0;
            let placements = self.score_popups.placements(now, popup_scale);
            frame.object3d_batch(placements);
        }
        // Cascade HUD: chips × mult counter under the plaque. During the
        // hand-off tween the trio merges into `= TOTAL` and physically
        // flies up into the score reel.
        if let Some(hud) = self.cascade_hud {
            let reel_px = sp.x + sp.w * 0.5;
            let reel_py = sp.y + sp.h * 0.25;
            let reel_lift = plaque_lift * 1.08;
            // Pad anchor: centered under the plaque, a little below its
            // bottom edge so it reads as a separate readout rather than
            // competing with the plaque text.
            let pad_px = sp.x + sp.w * 0.5;
            let pad_py = sp.y + sp.h * 1.05;
            let pad_lift = plaque_lift * 0.6;
            let glyph_scale = (layout.window_w.min(layout.window_h) / 1080.0) * 180.0;
            let placements = build_cascade_hud_placements(
                &hud,
                crate::render::world_space::LayoutAnchorPx {
                    px: pad_px,
                    py: pad_py,
                    lift_z: pad_lift,
                },
                crate::render::world_space::LayoutAnchorPx {
                    px: reel_px,
                    py: reel_py,
                    lift_z: reel_lift,
                },
                glyph_scale,
                sp.w,
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
        frame.quads(hud_quads);
        // Committed structure melds + tier tokens: inserted before the hand
        // `ShowcaseTileBatch` at end of `draw_frame` so they sit behind the rack (depth order).
        // Phase 3: bone yaku tablets (decal names on mesh).
        if !yaku_tablet_placements.is_empty() {
            frame.object3d_batch(yaku_tablet_placements);
        }
        // Phase 4: wood sort/trigger + lacquered bowl + bronze mirror.
        // Journal book is drawn later — last among gameplay props — so zoom depth cannot lose to bowl/mirror/etc.
        if !wood_tablet_placements.is_empty() {
            frame.object3d_batch(wood_tablet_placements);
        }
        if let Some(bowl) = discard_bowl_placement {
            frame.object3d(bowl);
        }
        if let Some(mirror) = bronze_mirror_placement {
            frame.object3d(mirror);
        }
        // Talisman/zodiac dish on the right side of the table. Glazed
        // porcelain with an aged-cream tint so the shader's crazing
        // pattern lands on it — reads as a well-loved temple ceramic
        // rather than a fresh kiln piece.
        if let Some((sx, sy, sw, sh)) = talisman_dish_strip {
            let dish_pad_x = sw * 0.10;
            let dish_pad_y = sh * 0.40;
            let td = &self.positions.talisman_dish;
            frame.object3d(Object3d {
                pos: [
                    sx + sw * 0.5 + td.nx * layout.window_w,
                    sy + sh * 0.5 + td.ny * layout.window_h,
                    layout.mm(td.lift_mm) + layout.mm(18.0) * 0.5,
                ],
                // Porcelain dish ~18mm tall — gives the bowl profile
                // enough vertical depth that the curved silhouette
                // reads as ceramic dishware rather than a flat coaster.
                extents: [
                    sw + dish_pad_x * 2.0,
                    layout.mm(18.0),
                    sh + dish_pad_y * 2.0,
                ],
                rotation: [std::f32::consts::FRAC_PI_2, 0.0, 0.0],
                // Aged cream — same tint as the gold dish so the two
                // ceramic surfaces read as a matched set across the table.
                color: color::PORCELAIN_AGED,
                kind: Object3dKind::Primitive {
                    shape: crate::render::primitive::MeshId::PorcelainDish,
                    material: crate::render::primitive::MaterialSpec::porcelain(),
                    pick_id: Some(PICK_CONSUMABLE_DISH),
                    shadow_caster: true,
                    silhouette: false,
                },
                hover_target: 0.0,
                anim_id: 0,
                arrange_name: Some("gameplay.talisman_dish"),
            });
            if !ribbon_dish_placements.is_empty() {
                frame.object3d_batch(ribbon_dish_placements);
            }
            if !talisman_dish_placements.is_empty() {
                frame.object3d_batch(talisman_dish_placements);
            }
        }

        {
            let _g = crate::render::cpu_profiler::scope("draw_frame.build_ambient_table_objects");
            animation_state::build_ambient_table_objects(
                self,
                layout,
                &gameplay,
                ctx.progress.dora_enabled(),
                &mut frame,
            );
        }

        // Flying coin animations (gold changes).
        {
            let flying = self.flying_coins.placements();
            if !flying.is_empty() {
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
        if let Some(book) = journal_book {
            frame.object3d(book);
        }

        frame.procedural_flame_emitters = flame_emitters;
        if !frame.procedural_flame_emitters.is_empty() {
            // One `DrawCmd::Flame` triggers the volume batch (same path as shop).
            frame.flames(std::iter::once(crate::render::wgpu_renderer::GpuInstance {
                rect: [0.0, 0.0, 1.0, 1.0],
                color: [0.0, 0.0, 1.0, 0.0],
                user: 0,
            }));
        }

        // Play mirror + discard river: labels centered in projected rects (not cursor hover tooltips).
        if !paused
            && !ctx.modal_active
            && self.cascade_queue.is_empty()
            && self.journal_transition.is_none()
        {
            use crate::render::theme::typography;
            let body_px = typography::size(typography::H36, layout.window_h);
            let push_centered = |out: &mut Vec<TextLabel>, rect: [f32; 4], copy: &'static str| {
                if rect[2] <= 1.0 || rect[3] <= 1.0 {
                    return;
                }
                let cap = body_px.min(rect[3] * 0.24).min(rect[2] * 0.14);
                let fs = typography::tier_at_most(cap, layout.window_h);
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
                push_centered(&mut hud_text, rect, "Discard tiles");
            }
            if let Some(rect) = ctx.proj.mirror_rect {
                push_centered(&mut hud_text, rect, "Score hand");
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
                    cash_in_enabled: has_structure,
                    show_discard_legend,
                    show_play_legend,
                    hud_text: &mut hud_text,
                },
            );
            if let Some(undo_rect) = discard_undo_rect {
                let is_focus = matches!(self.focus, Some(FocusTarget::DiscardUndo));
                let fs = typography::tier_at_most(body_px.min(undo_rect[3] * 0.55), layout.window_h);
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
                                let rare =
                                    def.map(|d| format!("{:?}", d.rarity)).unwrap_or_default();
                                let desc = relic_description_live(
                                    rid,
                                    &run.relic_counters,
                                    run.total_score_earned,
                                    Some((&run.relics, i)),
                                    Some(run.ghost_hand_preview_chips()),
                                );
                                push_focus_tooltip_panel_2d(
                                    &mut inspect_tooltip_quads,
                                    &mut inspect_tooltip_texts,
                                    FocusTooltipPanelParams {
                                        window_w: layout.window_w,
                                        window_h: layout.window_h,
                                        anchor_rect: Some(rect),
                                        title: &name,
                                        desc: &desc,
                                        cta: &format!("Tier · {rare}"),
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
                                    cta: "D-pad Up: Discard · [ ] / LB RB: HUD cycle",
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
                    scale,
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

        frame.candle_light_count = candle_placements.len() as u32;
        frame.flame_height_world = crate::render::flame_volume::flame_height_world(&layout);
        frame.scene_lighting.set_smooth_points(point_lights);
        frame.scene_lighting.spot_lights = spot_lights;
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

        // Arrange-mode hint: the relic tray's horizontal position is clamped
        // between `relic_col_top_ny` and `relic_col_bottom_ny` (historical
        // field names — both are x-fractions). If the user's `nx` lands
        // outside the band the tray pins to the wall and nudges appear to
        // be ignored. Expose the band so the renderer can draw it while
        // this placement is selected.
        {
            let gp = &self.positions;
            frame
                .arrange_clamps
                .push(crate::render::draw_cmd::ArrangeClamp {
                    name: "gameplay.relic_col".to_string(),
                    axis: crate::render::draw_cmd::ClampAxis::Horizontal,
                    lo_frac: gp.relic_col_top_ny,
                    hi_frac: gp.relic_col_bottom_ny,
                    center_frac: gp.relic_col.nx,
                });
        }

        // Stash the focus rect graph for the next frame's `update()` to
        // hit-test the cursor and run spatial navigation against.
        *self.last_focus_rects.borrow_mut() = focus_rect_graph;

        // Cheap invariant check — catches future migration mistakes that
        // accidentally push two `HandTileFaces` markers (or zero of one)
        // into the cmds list, which would silently break tile-face z
        // order. Compiled out of release builds.
        debug_assert_marker_uniqueness(&frame);

        insert_structure_before_hand(frame, structure_showcase, structure_pile_tokens)
    }
}

impl SceneBehavior for Box<GameplayScene> {
    fn pause_options_overlay(&self) -> Option<&options::OptionsScene> {
        (**self).pause_options_overlay()
    }

    fn has_blocking_overlay(&self) -> bool {
        (**self).has_blocking_overlay()
    }

    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        (**self).update(ctx)
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        (**self).draw_frame(ctx)
    }
}
