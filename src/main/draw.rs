use super::*;

impl App {
    /// Process a `RoundComplete` or `GameOver` event that was held while the
    /// scoring cascade was still playing. Pushes celebration modals, plays the
    /// appropriate sting, and queues the next scene.
    pub(super) fn handle_round_end_event(&mut self, ev: GameEvent) {
        let win_size = self
            .window
            .as_ref()
            .map(|w| w.inner_size())
            .unwrap_or(PhysicalSize::new(800, 600));
        let ww = win_size.width as f32;
        let wh = win_size.height as f32;
        match ev {
            GameEvent::RoundComplete { payout, .. } => {
                if self.run.onboarding_active() {
                    self.run.gold = self.run.gold.saturating_add(payout.total as i32);
                    self.progress.tutorial_completed = true;
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                    persistence::delete_saved_run(self.active_profile);
                    self.steam
                        .unlock_achievement(crate::steam::Achievement::TutorialComplete);
                    self.pending_scene =
                        Some(Scene::TutorialSummary(TutorialSummaryScene::new(true)));
                    self.transition_alpha = 1.0;
                    return;
                }
                // First non-tutorial round cleared. Fires every round, but
                // Steam's set-achievement is idempotent so the toast only
                // shows the first time.
                self.steam
                    .unlock_achievement(crate::steam::Achievement::FirstBlindCleared);
                // Apply the gold payout now that the scoring cascade has
                // finished — kept deferred so the UI doesn't jump early.
                self.run.gold = self.run.gold.saturating_add(payout.total as i32);
                self.audio.play_sfx(audio::SfxId::RoundWin);
                // Capture the tutorial lesson *before* advancing so the
                // recap scene can show what was just learned.
                let tutorial_lesson_before = self
                    .run
                    .tutorial
                    .as_ref()
                    .filter(|t| t.is_active())
                    .map(|t| t.current_lesson);
                // Capture round_score / target_score before advance_round
                // clobbers target_score with base_target for the next blind.
                let cleared_round_score = self.run.round_score;
                let cleared_target_score = self.run.target_score;
                self.run.advance_round(&mut self.bus);

                // First-encounter tooltip: gold payout.
                if let Some(ref mut tut) = self.run.tutorial
                    && tut.is_active()
                    && payout.total > 0
                    && tut.encounter(crate::game::tutorial::FirstEncounter::GoldPayout)
                {
                    self.modals.push(Modal::new(
                        crate::game::tutorial::FirstEncounter::GoldPayout.title(),
                        crate::game::tutorial::FirstEncounter::GoldPayout.message(),
                        ModalTheme::Success,
                    ));
                }

                // After Lesson 5 (Chips x Mult), grant a free relic to
                // introduce the concept before the shop appears. This
                // bridges the gap between learning scoring and discovering
                // the relic/shop meta-loop.
                if tutorial_lesson_before == Some(5)
                    && !self
                        .run
                        .relics
                        .active
                        .contains(&crate::core::relic::RelicId::PairPower)
                {
                    self.run
                        .relics
                        .active
                        .push(crate::core::relic::RelicId::PairPower);
                    let modal = Modal::new(
                        "Relic Earned!",
                        "You found Pair Power! Relics give passive bonuses for the rest of your run. Pairs now score +30 chips and +1 mult.",
                        ModalTheme::Success,
                    );
                    self.modals.push(modal);
                }

                // Skip the "Round Complete" modal during tutorials — the
                // TutorialRecap scene already shows the round outcome and
                // rendering both causes them to overlap.
                if tutorial_lesson_before.is_none() {
                    let mut lines = vec![format!(
                        "Score: {} / {}",
                        cleared_round_score, cleared_target_score
                    )];
                    lines.push(format!("Base reward  +${}", payout.base_reward));
                    if payout.unused_play_bonus > 0 {
                        lines.push(format!("Unused plays  +${}", payout.unused_play_bonus));
                    }
                    if payout.interest > 0 {
                        lines.push(format!("Interest  +${}", payout.interest));
                    }
                    if payout.green_luck_bonus > 0 {
                        lines.push(format!("Green Luck  +${}", payout.green_luck_bonus));
                    }
                    lines.push(format!("Total  +${}", payout.total));
                    let modal =
                        Modal::new("Round Complete!", lines.join("\n"), ModalTheme::Success)
                            .with_fireworks(ww * 0.5, wh * 0.8, ww * 0.6, 5);
                    self.modals.push(modal);
                }

                if self.run.is_run_complete() {
                    self.audio.play_sfx(audio::SfxId::RoundWin);
                }
                self.pending_scene = Some(if self.run.is_run_complete() {
                    // Victory — save progress (mirrors the GameOver loss path).
                    self.progress.has_won = true;
                    self.progress.runs_completed += 1;
                    self.progress.record_score(self.run.round_score);
                    let _ = self.progress.check_level_up();
                    self.steam
                        .unlock_achievement(crate::steam::Achievement::FirstRunCompleted);
                    if self.progress.runs_completed >= 10 {
                        self.steam
                            .unlock_achievement(crate::steam::Achievement::TenRunsPlayed);
                    }
                    // Stake ladder: crediting a full victory on this
                    // (material, stake) pair unlocks the next tier for that
                    // material. Idempotent — repeat wins are no-ops. Returns
                    // `Some(stake)` exactly when a new tier unlocked.
                    let newly_unlocked_stake = self
                        .progress
                        .record_stake_victory(self.run.mode.tile_material, self.run.mode.stake);
                    if let Some(crate::core::stake::Stake::Summer) = newly_unlocked_stake {
                        self.steam
                            .unlock_achievement(crate::steam::Achievement::Stake2Unlocked);
                    }
                    self.progress
                        .run_history
                        .push(crate::core::progression::RunRecord::from_run(
                            &self.run,
                            crate::core::progression::RunOutcome::Victory,
                        ));
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                    persistence::delete_saved_run(self.active_profile);
                    Scene::GameOver(GameOverScene::victory(&self.run))
                } else if let Some(lesson) = tutorial_lesson_before {
                    // Tutorial: show a recap of the completed lesson.
                    let shop_follows = self.run.tutorial_shop_enabled();
                    Scene::TutorialRecap(TutorialRecapScene::new(lesson, shop_follows))
                } else if !self.run.tutorial_shop_enabled() {
                    Scene::Gameplay(GameplayScene::with_pending_blind(self.run.upcoming_blind))
                } else {
                    Scene::Shop(ShopScene::new(self.run.run_number, &mut self.run))
                });
                self.transition_alpha = 1.0;
            }
            GameEvent::GameOver { reason } => {
                if self.run.onboarding_active() {
                    let round_score = self.run.round_score;
                    let target_score = self.run.target_score;
                    let discards_left = self.run.discards_remaining;
                    let last = self.run.last_breakdown.as_ref();
                    let feedback = crate::game::onboarding::finale_failure_feedback(
                        round_score,
                        target_score,
                        discards_left,
                        last,
                    );
                    self.run.retry_onboarding_finale();
                    self.audio.play_sfx(audio::SfxId::GameOver);
                    let modal = Modal::new("Try Again!", &feedback, ModalTheme::Info);
                    self.modals.push(modal);
                    let retry_blind = self.run.blind;
                    self.pending_scene = Some(Scene::Gameplay(GameplayScene::with_pending_blind(
                        retry_blind,
                    )));
                    self.transition_alpha = 1.0;
                    return;
                }
                // Tutorial retry: if the tutorial is active and the player
                // hasn't reached the graduation zone, restart the current
                // blind with adaptive difficulty instead of ending the run.
                let tutorial_retry = self
                    .run
                    .tutorial
                    .as_ref()
                    .is_some_and(|t| t.is_active() && t.current_lesson < 8);
                if tutorial_retry {
                    // Capture stats before retry resets them.
                    let round_score = self.run.round_score;
                    let target_score = self.run.target_score;
                    let plays_left = self.run.plays_remaining;
                    let discards_left = self.run.discards_remaining;
                    let lesson = self
                        .run
                        .tutorial
                        .as_ref()
                        .map(|t| t.current_lesson)
                        .unwrap_or(1);

                    self.run.retry_tutorial_blind();

                    let feedback = crate::game::tutorial::failure_feedback(
                        round_score.min(u32::MAX as u64) as u32,
                        target_score,
                        plays_left,
                        discards_left,
                        lesson,
                    );
                    let modal = Modal::new("Try Again!", &feedback, ModalTheme::Success);
                    self.modals.push(modal);
                    let retry_blind = self.run.blind;
                    self.pending_scene = Some(Scene::Gameplay(GameplayScene::with_pending_blind(
                        retry_blind,
                    )));
                    self.transition_alpha = 1.0;
                    return;
                }

                // Mark tutorial as completed if the player reached graduation
                // (or finished the tutorial run regardless of outcome).
                if let Some(ref tutorial) = self.run.tutorial
                    && (tutorial.finished || tutorial.current_lesson >= 8)
                {
                    self.progress.tutorial_completed = true;
                }
                self.progress.runs_completed += 1;
                self.progress.record_score(self.run.round_score);
                let level_up = self.progress.check_level_up();
                self.progress
                    .run_history
                    .push(crate::core::progression::RunRecord::from_run(
                        &self.run,
                        crate::core::progression::RunOutcome::Defeat { reason },
                    ));
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                // Run is over — drop any saved-on-quit snapshot so the
                // player isn't offered "Continue" into a finished game.
                persistence::delete_saved_run(self.active_profile);

                if let Some(result) = level_up {
                    log::info!("Level up! Now level {}", result.new_level);
                    let mut pages = Vec::new();

                    // Relic pages.
                    let relic_defs = core::relic::all_relic_defs();
                    for rid in &result.relics {
                        if let Some(def) = relic_defs.iter().find(|d| d.id == *rid) {
                            let accent = match def.rarity {
                                core::relic::Rarity::Common => render::theme::color::rarity(0),
                                core::relic::Rarity::Uncommon => render::theme::color::rarity(1),
                                core::relic::Rarity::Rare => render::theme::color::rarity(2),
                                core::relic::Rarity::Legendary => render::theme::color::rarity(3),
                            };
                            pages.push(UnlockPage {
                                category: "New Relic".into(),
                                name: def.name.into(),
                                description: def.description.into(),
                                relic_id: Some(*rid),
                                accent_color: accent,
                            });
                        }
                    }

                    // Rule pages.
                    for rm in &result.rules {
                        pages.push(UnlockPage {
                            category: "New Rule".into(),
                            name: rm.name().into(),
                            description: rm.description().into(),
                            relic_id: None,
                            accent_color: render::theme::color::AMBER,
                        });
                    }

                    if !pages.is_empty() {
                        let modal = Modal::new(
                            format!("Level Up! — Level {}", result.new_level),
                            "",
                            ModalTheme::Success,
                        )
                        .with_pages(pages)
                        .with_fireworks(ww * 0.5, wh * 0.7, ww * 0.7, 8);
                        self.pending_post_game_over_modals.push(modal);
                        self.audio.play_sfx(audio::SfxId::LevelUp);
                    }
                }

                self.audio.play_sfx(audio::SfxId::GameOver);
                self.pending_scene = Some(Scene::GameOver(GameOverScene::new(&self.run, reason)));
                self.transition_alpha = 1.0;
            }
            _ => {}
        }
    }

    pub(super) fn draw(&mut self) {
        // Cache once up front so the borrow checker doesn't have to reason
        // about us calling `&self` methods while `self.renderer` is held
        // mutably below.
        let modal_active = self.modal_overlay_active();
        // The button-wipe below must only fire for *app-owned* overlays
        // (modals, tuning, sfx test). Scene-owned overlays like the pause
        // menu push their own clickable buttons through `frame.buttons`,
        // so wiping `active_buttons` for them would nuke the pause-menu
        // buttons themselves and clicks would land on nothing. Scenes are
        // responsible for suppressing their own non-overlay buttons while
        // their overlay is up (see e.g. `GameplayScene::draw_frame`).
        let app_overlay_wipe = self.modals.is_active()
            || self.debug.tuning_overlay.is_some()
            || self.debug.sfx_test_overlay.is_some()
            || self.debug.camera_debug_overlay.is_some()
            || self.debug.smoke_debug_overlay.is_some()
            || self.debug.volumetric_debug_overlay.is_some();
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let Some(win) = self.window.as_ref() else {
            return;
        };

        let size = win.inner_size();
        let layout = self
            .layout_engine
            .solve(size.width as f32, size.height as f32);

        let ctx = DrawCtx {
            layout: &layout,
            anim: &self.anim,
            run: &self.run,
            progress: &self.progress,
            active_profile: self.active_profile,
            game_in_progress: self.run.is_in_progress(),
            proj: renderer.projections(),
            picked_gameplay_object: self
                .input
                .as_ref()
                .and_then(|i| renderer.pick_gameplay_object(i.last_cursor.0, i.last_cursor.1)),
            picked_shop_object: self
                .input
                .as_ref()
                .and_then(|i| renderer.pick_shop_object(i.last_cursor.0, i.last_cursor.1)),
            debug_visibility: scenes::DebugVisibility {
                hide_candles: self.debug.hide_candles,
                hide_blind_plaque: self.debug.hide_blind_plaque,
            },
            ui_scale: self.gfx.ui_scale,
            modal_active,
            arrange_preview: if let Some(Some(ref state)) = self.debug.arrange_mode {
                let ww = size.width as f32;
                let wh = size.height as f32;
                Some(crate::ui::placement::ArrangePreview {
                    name: state.object_name.clone(),
                    dnx: if ww > 0.0 { state.delta_px / ww } else { 0.0 },
                    dny: if wh > 0.0 { state.delta_py / wh } else { 0.0 },
                    // Match the live preview in `sample_arrange_placement`
                    // (see HUD code in this file): convert the world-unit
                    // lift step back to mm at the canonical window.
                    d_lift_mm: state.delta_lift * crate::ui::scene_layout::HFRAC_TO_MM
                        / crate::ui::scene_layout::CANONICAL_WINDOW_W,
                    d_rx_deg: state.delta_rx_deg,
                    d_ry_deg: state.delta_ry_deg,
                    d_rz_deg: state.delta_rz_deg,
                })
            } else {
                None
            },
            shop_smoke_tuning: &self.shop_smoke_tuning,
        };
        // Build the scene's frame in canonical push-order. For migrated
        // scenes (gameplay) this calls their direct `draw_frame` impl;
        // for legacy scenes the default impl forwards through `draw()` +
        // `into_frame()`. Either way we get back a single ordered
        // `UiFrame.cmds` list whose push order is z-order.
        let mut frame: UiFrame = if let Some(top) = self.overlay_stack.last() {
            top.draw_frame(ctx)
        } else {
            self.scene.draw_frame(ctx)
        };

        let h = size.height as f32;
        self.debug.last_effective_camera = frame
            .camera_override
            .unwrap_or_else(|| CameraParams::default_table_camera(h));

        // Index of the last cmd produced by the scene itself, captured
        // BEFORE any modal/tuning/sfx/fps/tooltip overlay is appended
        // below. Used by the tooltip-overlay snapshot a few lines down so
        // glossary-hover scanning only sees scene content (not modal text
        // or fps debug labels).
        let scene_cmds_end = frame.cmds.len();

        win.set_title(&frame.window_title);
        self.active_buttons = frame
            .buttons
            .iter()
            .map(|b| ButtonDef {
                rect: b.rect,
                action: b.action,
            })
            .collect();

        // Click-safety wipe: if any modal-like overlay is up, scene buttons
        // must not be clickable through it. Overlays that want their own
        // clickable surface (e.g. `ModalQueue`'s full-screen dismiss button)
        // write to `active_buttons` *after* this point in their draw step.
        // See `App::modal_overlay_active` for the contract.
        if app_overlay_wipe {
            self.active_buttons.clear();
        }

        // Snapshot the scene's text labels and relic icons for the
        // tooltip overlay's glossary-hover scanning, by walking the
        // scene's portion of `frame.cmds` (everything pushed up to
        // `scene_cmds_end`). This works uniformly for migrated scenes
        // (which push directly into `frame.cmds`) AND legacy scenes
        // (whose `into_frame()` lands their `text_labels` / `relic_icons`
        // as `DrawCmd::Text` / `DrawCmd::RelicIcon` in the same list).
        // Walking the cmds list — instead of snapshotting separate
        // `output.text_labels` / `output.relic_icons` vecs — is what
        // makes the migration transparent to the tooltip system.
        let scene_text_labels: Vec<TextLabel> = frame.cmds[..scene_cmds_end]
            .iter()
            .filter_map(|c| {
                if let crate::render::draw_cmd::DrawCmd::Text(l) = c {
                    Some(TextLabel {
                        rect: l.rect,
                        text: l.text.clone(),
                        color: l.color,
                        font_px: l.font_px,
                        align: l.align,
                        no_glossary: l.no_glossary,
                        scroll_offset: l.scroll_offset,
                    })
                } else {
                    None
                }
            })
            .collect();
        let scene_glossary_anchors: Vec<([f32; 4], &'static str)> = frame.cmds[..scene_cmds_end]
            .iter()
            .filter_map(|c| {
                if let crate::render::draw_cmd::DrawCmd::GlossaryAnchor { rect, term } = c {
                    Some((*rect, *term))
                } else {
                    None
                }
            })
            .collect();
        // Forward the cursor position so the renderer can project it onto
        // the table plane and feed it into the volumetric smoke sim.
        frame.cursor_pos = self.input.as_ref().map(|i| i.last_cursor);

        // Apply transition alpha to everything that's part of the scene
        // (after into_frame so all scene cmds exist; before overlays are
        // appended so they fade in cleanly).
        let alpha = self.transition_alpha;
        frame.apply_alpha(alpha);

        // Overlay the shooting-star cascade effect during dramatic transitions.
        let size = win.inner_size();

        if self.transition_timer > 0.0 {
            match self.transition_kind {
                TransitionKind::ShootingStarCascade => {
                    frame.transition_progress = self.transition_timer;
                    frame.shooting_star_cascade();
                }
                TransitionKind::ForestOfTiles => {
                    crate::render::transition_fx::push_overlay_transition(
                        &mut frame,
                        crate::render::transition_fx::OverlayTransitionKind::ForestOfTiles,
                        self.transition_timer,
                        (size.width as f32, size.height as f32),
                    );
                }
                TransitionKind::GalaxyOfTiles => {
                    crate::render::transition_fx::push_overlay_transition(
                        &mut frame,
                        crate::render::transition_fx::OverlayTransitionKind::GalaxyOfTiles,
                        self.transition_timer,
                        (size.width as f32, size.height as f32),
                    );
                }
                TransitionKind::Maelstrom => {
                    crate::render::transition_fx::push_overlay_transition(
                        &mut frame,
                        crate::render::transition_fx::OverlayTransitionKind::Maelstrom,
                        self.transition_timer,
                        (size.width as f32, size.height as f32),
                    );
                }
                TransitionKind::TileWaterfall => {
                    crate::render::transition_fx::push_overlay_transition(
                        &mut frame,
                        crate::render::transition_fx::OverlayTransitionKind::TileWaterfall,
                        self.transition_timer,
                        (size.width as f32, size.height as f32),
                    );
                }
                TransitionKind::ShufflingFan => {
                    crate::render::transition_fx::push_overlay_transition(
                        &mut frame,
                        crate::render::transition_fx::OverlayTransitionKind::ShufflingFan,
                        self.transition_timer,
                        (size.width as f32, size.height as f32),
                    );
                }
                TransitionKind::Quick => {}
            }
        }

        self.modals.update();
        if let Some((modal_insts, modal_labels, modal_buttons, modal_relic_objects)) = self
            .modals
            .draw(size.width as f32, size.height as f32, self.gfx.ui_scale)
        {
            frame.quads(modal_insts);
            frame.texts(modal_labels);
            if !modal_relic_objects.is_empty() {
                // Near-orthographic camera looking down -Y at the felt so
                // pixel_to_world's (world_x, world_y, lift_z) maps cleanly
                // to screen space with Z up — matches the scene-wide axis
                // convention used by tutorial/collection relic cards.
                let h = size.height as f32;
                frame.camera_override = Some(CameraParams {
                    eye: [0.0, -h * 3.0, 0.0],
                    target: [0.0, 0.0, 0.0],
                    up: [0.0, 0.0, 1.0],
                    fovy_deg: 20.0,
                });
                frame.object3d_batch(modal_relic_objects);
            }
            // Replace scene buttons with modal buttons so only dismiss works.
            self.active_buttons = modal_buttons;
        }

        // Tuning overlay — on top of modals.
        if let Some(ref overlay) = self.debug.tuning_overlay {
            let (tuning_insts, tuning_labels) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(tuning_insts);
            frame.texts(tuning_labels);
            self.active_buttons.clear(); // Block scene buttons.
        }

        // SFX test overlay — on top of modals.
        if let Some(ref mut overlay) = self.debug.sfx_test_overlay {
            let (insts, lbls) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(insts);
            frame.texts(lbls);
            self.active_buttons.clear();
        }

        // Camera debug overlay — on top of modals.
        if let Some(ref overlay) = self.debug.camera_debug_overlay {
            // Override the scene's camera with the debug values.
            frame.camera_override = Some(overlay.to_camera_params());
            let (insts, lbls) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(insts);
            frame.texts(lbls);
            self.active_buttons.clear();
        }

        // Debug visibility overlay — on top of modals.
        if let Some(ref overlay) = self.debug.visibility_overlay {
            let (insts, lbls) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(insts);
            frame.texts(lbls);
            self.active_buttons.clear();
        }

        // Shop smoke tuning overlay — on top of modals.
        if let Some(ref overlay) = self.debug.smoke_debug_overlay {
            let (insts, lbls) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(insts);
            frame.texts(lbls);
            self.active_buttons.clear();
        }

        // Volumetric tuning overlay — on top of modals.
        if let Some(ref overlay) = self.debug.volumetric_debug_overlay {
            let (insts, lbls) =
                overlay.draw(size.width as f32, size.height as f32, self.gfx.ui_scale);
            frame.quads(insts);
            frame.texts(lbls);
            self.active_buttons.clear();
        }

        // Tooltip overlay — pushed *after* modals/tuning so it sits on top
        // of all scene/modal content. Suppressed whenever any modal-like
        // overlay is up so hover effects don't leak through to elements
        // below it. See `App::modal_overlay_active` for the contract.
        let skip_tooltips = modal_active || matches!(&self.scene, Scene::Options(_));
        if !skip_tooltips {
            let cursor = self
                .input
                .as_ref()
                .map(|i| i.last_cursor)
                .unwrap_or((0.0, 0.0));
            let ww = size.width as f32;
            let wh = size.height as f32;
            let btn_rects: Vec<(f32, f32, f32, f32)> =
                self.active_buttons.iter().map(|b| b.rect).collect();
            let owns_fortunes_favor = self
                .run
                .relics
                .has(crate::core::relic::RelicId::FortunesFavor);
            self.tooltips.update_and_draw_into(
                &mut frame,
                cursor,
                crate::ui::tooltip::TooltipSources {
                    base_labels: &scene_text_labels,
                    button_rects: &btn_rects,
                    relic_icons: &[],
                    glossary_anchors: &scene_glossary_anchors,
                },
                crate::ui::layout::ViewportCtx {
                    window_w: ww,
                    window_h: wh,
                    ui_scale: self.gfx.ui_scale,
                },
                owns_fortunes_favor,
            );
        } else {
            self.tooltips.clear();
        }

        // FPS counter overlay (debug) — pushed last so it's always on top.
        if self.debug.show_fps {
            // Use the real frame-to-frame delta captured at the top of
            // RedrawRequested. `self.last_frame.elapsed()` would only see
            // partial CPU work done so far this frame and report inflated FPS.
            let instant_fps = 1.0 / self.last_frame_dt;
            // Exponential moving average for smooth display.
            self.debug.fps_smoothed = self.debug.fps_smoothed * 0.9 + instant_fps * 0.1;
            let w = size.width as f32;
            let h = size.height as f32;
            let label_h = (h * 0.03).max(20.0);
            let label_w = label_h * 4.0;
            let margin = label_h * 0.3;
            frame.quad(GpuInstance {
                rect: [w - label_w - margin, margin, label_w, label_h],
                color: [0.0, 0.0, 0.0, 0.55],
            });
            frame.text(TextLabel {
                rect: [w - label_w - margin, margin, label_w, label_h],
                text: format!("{:.0} FPS", self.debug.fps_smoothed),
                color: [0.9, 0.9, 0.3, 1.0],
                ..Default::default()
            });
        }

        // Arrange-mode label in the lower-left — shows what's currently
        // selected (or "select an object" prompt when the mode is armed but
        // nothing is picked yet). Mirrors the FPS HUD sizing in the
        // upper-right.
        if let Some(ref inner) = self.debug.arrange_mode {
            let size = self
                .window
                .as_ref()
                .map(|w| w.inner_size())
                .unwrap_or(winit::dpi::PhysicalSize::new(1280, 720));
            let w = size.width as f32;
            let h = size.height as f32;
            let label_h = (h * 0.09).max(60.0);
            let label_w = (label_h * 16.0).min(w * 0.95);
            let margin = label_h * 0.3;
            let y = h - label_h - margin;
            let (text, color) = match inner {
                Some(state) => {
                    // Show resolved coords (current on-disk + staged delta) so
                    // the HUD matches what Enter will commit. Falls back to a
                    // delta-only string for groups (no single leaf to sample).
                    let sampled = sample_arrange_placement(&state.object_name, &self.scene);
                    let text = if let Some(p) = sampled {
                        let dnx = state.delta_px / w;
                        let dny = state.delta_py / h;
                        let d_lift_mm = state.delta_lift * crate::ui::scene_layout::HFRAC_TO_MM
                            / crate::ui::scene_layout::CANONICAL_WINDOW_W;
                        format!(
                            "Arrange: {}  nx={:.4} ny={:.4} lift={:.2}mm  rx={:+.1}° ry={:+.1}° rz={:+.1}°  [step {:.0}px/{:.0}°]",
                            state.object_name,
                            p.nx + dnx,
                            p.ny + dny,
                            p.lift_mm + d_lift_mm,
                            p.rx_deg + state.delta_rx_deg,
                            p.ry_deg + state.delta_ry_deg,
                            p.rz_deg + state.delta_rz_deg,
                            state.trans_step_px,
                            state.rot_step_deg,
                        )
                    } else {
                        format!(
                            "Arrange: {} (group)  Δpx={:+.1} Δpy={:+.1} Δz={:+.1}  Δrx={:+.1}° Δry={:+.1}° Δrz={:+.1}°  [step {:.0}px/{:.0}°]",
                            state.object_name,
                            state.delta_px,
                            state.delta_py,
                            state.delta_lift,
                            state.delta_rx_deg,
                            state.delta_ry_deg,
                            state.delta_rz_deg,
                            state.trans_step_px,
                            state.rot_step_deg,
                        )
                    };
                    (text, [0.95, 0.85, 0.35, 1.0])
                }
                None => (
                    "Arrange: click an object or press Tab".to_string(),
                    [0.8, 0.8, 0.8, 1.0],
                ),
            };
            frame.quad(GpuInstance {
                rect: [margin, y, label_w, label_h],
                color: [0.0, 0.0, 0.0, 0.6],
            });
            frame.text(TextLabel {
                rect: [margin + label_h * 0.2, y, label_w, label_h],
                text,
                color,
                ..Default::default()
            });
        }

        // Debug: drop draw cmds for hidden HUD elements so we can inspect the
        // procedural 3D scene underneath. The blind plaque, scoring placard,
        // and candles are gated at the *call site* in `gameplay.rs` (via
        // `DrawCtx::debug_visibility`) because (a) the two plaques share the
        // same `DrawCmd::Plaque(_)` variant and can't be told apart by a
        // post-process filter, and (b) skipping candle pushes also skips the
        // attached `PointLight`s, which a cmd-only filter would leak. Tiles
        // and inventory items have unambiguous variants and can be safely
        // dropped after the fact.
        let any_hide = self.debug.hide_tiles || self.debug.hide_inventory;
        if any_hide {
            let hide_tiles = self.debug.hide_tiles;
            frame.cmds.retain(|c| {
                use crate::render::draw_cmd::DrawCmd;
                if hide_tiles && matches!(c, DrawCmd::ShowcaseTileBatch(_)) {
                    return false;
                }
                true
            });
        }

        // Convert settle ms to exponential decay speed (inversely proportional).
        // Default: 500ms → speed 8.0, 400ms → speed 10.0.
        let draw_settle_speed = 8.0 * (500.0 / self.cascade_tuning.draw_settle_ms.max(1) as f32);
        let sort_settle_speed = 10.0 * (400.0 / self.cascade_tuning.sort_settle_ms.max(1) as f32);

        // When a run is active, use its tile material (gameplay choice);
        // otherwise fall back to the options-screen cosmetic setting.
        let active_material = frame.tile_material_override.unwrap_or_else(|| {
            if self.run.is_in_progress() {
                self.run.mode.tile_material
            } else {
                self.gfx.tile_material
            }
        });
        // Tell the renderer which scene is active so shared mesh pipelines
        // (Object3dKind::Ofuda, coin/gold piles, etc.) can emit correctly-
        // prefixed canonical pickable names for arrange mode.
        let active_scene_key: Option<&'static str> = match &self.scene {
            Scene::Shop(_) => Some("shop"),
            Scene::Gameplay(_) => Some("gameplay"),
            Scene::Collection(_) => Some("collection"),
            Scene::StartScreen(_) => Some("start_screen"),
            Scene::TutorialCampaign(_) => Some("tutorial"),
            _ => None,
        };
        renderer.set_active_scene(active_scene_key);

        // Push the committed rotation map so every arrange-tagged draw picks
        // up its Placement's rx/ry/rz_deg without each scene site having to
        // wire it into its own rotation matrix.
        renderer.set_committed_arrange_rotations(collect_committed_rotations(&self.scene));

        // Push the volumetric dust-floor density so the fluid sim's inject
        // pass seeds an ambient baseline. Cheap each frame; the renderer
        // no-ops if the fluid sim isn't active.
        renderer.set_dust_strength(self.volumetric_tuning.dust_strength);
        // Push mountain-haze art-direction knobs into the haze shader's
        // uniform — lets the Volumetric debug overlay drive density,
        // colour, horizon band, and wind speed live.
        renderer.set_haze_tuning(
            self.volumetric_tuning.haze_density,
            self.volumetric_tuning.haze_color_r,
            self.volumetric_tuning.haze_color_g,
            self.volumetric_tuning.haze_color_b,
            self.volumetric_tuning.haze_horizon_y,
            self.volumetric_tuning.haze_drift_speed,
        );

        // Push arrange-mode override so the renderer draws the selected object
        // at the edited position/rotation this frame.
        renderer.set_arrange_override(if let Some(Some(ref state)) = self.debug.arrange_mode {
            Some(DebugArrangeOverride {
                name: state.object_name.clone(),
                delta_px: state.delta_px,
                delta_py: state.delta_py,
                delta_lift: state.delta_lift,
                delta_rz_deg: state.delta_rz_deg,
                delta_rx_deg: state.delta_rx_deg,
                delta_ry_deg: state.delta_ry_deg,
            })
        } else {
            None
        });

        if let Err(e) = renderer.render(
            &frame,
            crate::render::wgpu_renderer::RenderSettings {
                smoke_quality: self.gfx.smoke_quality,
                smoke_amount: self.gfx.smoke_amount,
                effects_quality: self.gfx.effects_quality,
                tile_preset: self.gfx.tile_preset,
                tile_material: active_material,
                tileset_name: self.gfx.tileset_name.clone(),
                draw_settle_speed,
                sort_settle_speed,
                gamma: self.gfx.gamma,
                shadows_enabled: self.gfx.shadows_enabled,
                ssr_enabled: self.gfx.ssr_enabled,
            },
        ) {
            log::error!("render: {e:?}");
        }
    }
}
