#[cfg(feature = "game")]
use super::*;

#[cfg(feature = "game")]
use crate::core;
#[cfg(feature = "game")]
use crate::game::engine::GameEngine;
use crate::render;
#[cfg(feature = "game")]
use crate::render::vocabulary_colors::GlossaryMode;
#[cfg(feature = "game")]
use crate::scene_transition::{PendingSceneDestination, SceneTag, overlay_kind_for_transition};
#[cfg(feature = "game")]
use crate::scenes::SceneIntent;
use crate::ui::modal::{Modal, ModalTheme, UnlockPage};
#[cfg(feature = "game")]
use crate::ui::score_format::format_score;

#[cfg(feature = "game")]
#[inline]
fn append_fullscreen_debug_panel(
    frame: &mut UiFrame,
    active_buttons: &mut Vec<ButtonDef>,
    insts: Vec<GpuInstance>,
    labels: Vec<TextLabel>,
) {
    frame.debug_overlay_quads(insts);
    frame.debug_overlay_texts(labels);
    active_buttons.clear();
}

#[cfg(feature = "game")]
#[inline]
fn hide_ui_draw_cmds(frame: &mut UiFrame) {
    frame.cmds.retain(|cmd| {
        !matches!(
            cmd,
            render::draw_cmd::DrawCmd::Quad(_)
                | render::draw_cmd::DrawCmd::OverlayQuad(_)
                | render::draw_cmd::DrawCmd::OverlaySquircleQuad(_)
                | render::draw_cmd::DrawCmd::GradientQuad(_)
                | render::draw_cmd::DrawCmd::ArcRingQuad(_)
                | render::draw_cmd::DrawCmd::SquircleQuad(_)
                | render::draw_cmd::DrawCmd::Text(_)
                | render::draw_cmd::DrawCmd::ImageQuad(_)
                | render::draw_cmd::DrawCmd::TileFaceQuad(_)
        )
    });
}

#[cfg(feature = "game")]
impl App {
    /// Canonical scene-key string for the renderer (`active_scene_key`).
    /// Mirrors the match in [`Self::draw`]; pulled out so the per-scene
    /// tonemap resolver and the debug-action handler that opens the
    /// tonemap overlay see the exact same key the renderer does. `None`
    /// for scenes that don't register one — those fall back to the
    /// `_default` slot in [`crate::game::scene_look_tuning::SceneLookTuningSet`].
    pub(super) fn active_scene_key_for_renderer(&self) -> Option<&'static str> {
        let top = self.overlay_stack.last().unwrap_or(&self.scene);
        let parent = crate::scenes::overlay_renderer_parent(&self.scene, &self.overlay_stack);
        crate::scenes::active_scene_key_for_renderer(top, parent)
    }

    /// Tonemap + room GLB look for the active scene. When the scene-look
    /// overlay is editing the active scene, returns its live draft.
    pub(super) fn resolved_scene_look_for(
        &self,
        scene_key: Option<&str>,
    ) -> crate::game::scene_look_tuning::SceneLookTuning {
        let (overlay_key, overlay_look) = self
            .debug
            .scene_look_debug_overlay
            .as_ref()
            .map(|o| (Some(o.scene_key_persist()), Some(o.look)))
            .unwrap_or((None, None));
        crate::game::scene_look_tuning::resolve_scene_look_with_overlay(
            &self.scene_look,
            overlay_key,
            overlay_look,
            scene_key,
        )
    }

    /// Tonemap + room GLB look for the active scene. When the scene-look
    /// overlay is editing that scene (or `_default` with no override), returns its live draft.
    pub(super) fn resolved_scene_look(&self) -> crate::game::scene_look_tuning::SceneLookTuning {
        self.resolved_scene_look_for(self.active_scene_key_for_renderer())
    }

    /// Steam achievements keyed off [`PlayerProgress::runs_completed`].
    /// Called after incrementing the counter on run victory or defeat.
    fn sync_runs_completed_achievements(&mut self) {
        if self.progress.runs_completed >= 10 {
            self.dist
                .unlock_achievement(crate::steam::Achievement::TenRunsPlayed);
        }
    }

    /// Process a `RoundComplete` or `GameOver` event that was held while the
    /// scoring cascade was still playing. Pushes celebration modals, plays the
    /// appropriate sting, and queues the next scene.
    pub(super) fn handle_round_end_event(&mut self, ev: GameEvent) {
        let win_size = self.last_drawable_px;
        let ww = win_size.width as f32;
        let wh = win_size.height as f32;
        match ev {
            GameEvent::RoundComplete {
                payout,
                reached_target,
            } => {
                if self.run.onboarding_active() && reached_target {
                    self.run
                        .apply_yen_reward(payout.total as i32, Some(&mut self.bus));
                    match self.run.onboarding_phase() {
                        Some(crate::game::onboarding::OnboardingPhase::Lessons) => {
                            self.audio.play_sfx(audio::SfxId::RoundWin);
                            let cleared_round_score = self.run.round_score;
                            let cleared_target_score = self.run.target_score;
                            let mut lines = vec![format!(
                                "Score: {} / {}",
                                format_score(cleared_round_score),
                                format_score(cleared_target_score as u64)
                            )];
                            lines.push(
                                "Well done — you have overcome your first chamber".to_string(),
                            );
                            lines.push("Remember: Play melds, build a structure, then cash in to score. Bigger structures score more".to_string());
                            lines.push(
                                "Next: buy supplies from the shop, then face an ordeal".to_string(),
                            );
                            self.modals.push(
                                Modal::new(
                                    "Lesson Complete!",
                                    lines.join("\n"),
                                    ModalTheme::Success,
                                )
                                .with_fireworks(
                                    ww * 0.5,
                                    wh * 0.8,
                                    ww * 0.6,
                                    4,
                                ),
                            );
                            GameEngine::set_onboarding_shop_phase(&mut self.run);
                            self.run.grant_pending_memorial(&mut self.progress);
                            self.mark_profile_dirty();
                            self.begin_scene_replace(
                                SceneIntent::ShopTutorial,
                                SceneTag::from(&self.scene),
                                PendingSceneDestination::default(),
                            );
                            return;
                        }
                        Some(crate::game::onboarding::OnboardingPhase::Finale) => {
                            self.audio.play_music_jingle(audio::MusicId::OrdealWin);
                            self.progress.tutorial_completed = true;
                            let _ = persistence::save_profile(self.active_profile, &self.progress);
                            persistence::delete_saved_run(self.active_profile);
                            self.dist.sync_profile_stats(&self.progress);
                            self.dist
                                .unlock_achievement(crate::steam::Achievement::TutorialComplete);
                            self.begin_scene_replace(
                                SceneIntent::TutorialSummary { won: true },
                                SceneTag::from(&self.scene),
                                PendingSceneDestination::default(),
                            );
                            return;
                        }
                        _ => {}
                    }
                }
                if !reached_target {
                    let cleared_round_score = self.run.round_score;
                    let cleared_target_score = self.run.target_score;
                    self.run.forfeit_current_chamber_second_wind(&mut self.bus);
                    self.audio.play_sfx(audio::SfxId::TilesDestroyed);
                    let modal = Modal::new(
                        "Second Wind",
                        format!(
                            "The relic shatters. No payout for this blind — only your other relics matter now.\n\nScore: {} / {}",
                            format_score(cleared_round_score),
                            format_score(cleared_target_score as u64)
                        ),
                        ModalTheme::Info,
                    );
                    self.modals.push(modal);
                    self.run.grant_pending_memorial(&mut self.progress);
                    self.mark_profile_dirty();
                    self.begin_scene_replace(
                        SceneIntent::ShopFromRun,
                        SceneTag::from(&self.scene),
                        PendingSceneDestination::default(),
                    );
                    return;
                }
                // First non-tutorial chamber cleared. Fires every round, but
                // Steam's set-achievement is idempotent so the toast only
                // shows the first time.
                self.dist
                    .unlock_achievement(crate::steam::Achievement::FirstBlindCleared);
                // Apply the gold payout now that the scoring cascade has
                // finished — kept deferred so the UI doesn't jump early.
                self.run
                    .apply_yen_reward(payout.total as i32, Some(&mut self.bus));
                self.audio.play_sfx(audio::SfxId::RoundWin);
                // Win jingle owns the music sink for the celebration; the
                // pending scene transition will queue Shop/Gameplay BGM
                // behind it via `set_music_track`, and `AudioManager::tick`
                // resumes that loop once the jingle finishes.
                let won_jingle = if self.run.chamber == crate::core::rules::ChamberKind::Ordeal {
                    audio::MusicId::OrdealWin
                } else {
                    audio::MusicId::ChamberWin
                };
                self.audio.play_music_jingle(won_jingle);
                // Capture round_score / target_score before advance_round
                // clobbers target_score with base_target for the next blind.
                let cleared_round_score = self.run.round_score;
                let cleared_target_score = self.run.target_score;
                let cleared_ordeal = self.run.chamber == crate::core::rules::ChamberKind::Ordeal;
                self.run.advance_round(&mut self.bus);

                {
                    let modal = Modal::new("Winner!", "", ModalTheme::Success)
                        .with_payout_breakdown(
                            cleared_round_score,
                            u64::from(cleared_target_score),
                            payout,
                        )
                        .with_fireworks(ww * 0.5, wh * 0.8, ww * 0.6, 5);
                    self.modals.push(modal);
                }

                if self.run.is_run_complete() {
                    self.audio.play_sfx(audio::SfxId::RoundWin);
                }
                let next_intent = if self.run.is_run_complete() {
                    // Victory — save progress (mirrors the GameOver loss path).
                    self.progress.has_won = true;
                    if self.run.mode.tile_material == crate::persistence::TileMaterial::Plastic {
                        self.progress.has_won_with_plastic = true;
                    }
                    self.progress.runs_completed += 1;
                    self.progress.award_level_points_for_outcome(
                        crate::core::progression::RunOutcome::Victory,
                    );
                    self.progress.record_score(self.run.round_score);
                    let level_up = self.progress.check_level_up();
                    self.dist
                        .unlock_achievement(crate::steam::Achievement::FirstRunCompleted);
                    self.sync_runs_completed_achievements();
                    // Season ladder: crediting a full victory on this
                    // (material, season) pair unlocks the next tier for that
                    // material. Idempotent — repeat wins are no-ops. Returns
                    // `Some(season)` exactly when a new tier unlocked.
                    let newly_unlocked_season = self
                        .progress
                        .record_season_victory(self.run.mode.tile_material, self.run.mode.season);
                    if let Some(season) = newly_unlocked_season {
                        if let Some(ach) =
                            crate::steam::Achievement::for_newly_unlocked_season(season)
                        {
                            self.dist.unlock_achievement(ach);
                        }
                    }
                    self.progress.run_history.push(
                        crate::game::progression_run::run_record_from_run(
                            &self.run,
                            crate::core::progression::RunOutcome::Victory,
                        ),
                    );
                    let _ = persistence::save_profile(self.active_profile, &self.progress);
                    persistence::delete_saved_run(self.active_profile);
                    self.dist.sync_profile_stats(&self.progress);

                    if let Some(result) = level_up
                        && let Some(modal) = build_level_up_modal(&result, ww, wh)
                    {
                        log::info!(
                            "Depth increased to {}",
                            core::progression::meta_depth_roman(result.new_level)
                        );
                        self.pending_post_game_over_level_up = Some(modal);
                    }

                    SceneIntent::Victory
                } else if cleared_ordeal && crate::render::staircase_glb::staircase_glb_loaded() {
                    self.run.grant_pending_memorial(&mut self.progress);
                    self.mark_profile_dirty();
                    SceneIntent::Stairway
                } else {
                    self.run.grant_pending_memorial(&mut self.progress);
                    self.mark_profile_dirty();
                    SceneIntent::ShopFromRun
                };
                self.begin_scene_replace(
                    next_intent,
                    SceneTag::from(&self.scene),
                    PendingSceneDestination::default(),
                );
            }
            GameEvent::GameOver { reason } => {
                if self.run.onboarding_active() {
                    let round_score = self.run.round_score;
                    let target_score = self.run.target_score;
                    let discards_left = self.run.discards_remaining;
                    let plays_left = self.run.plays_remaining;
                    let (feedback, retry_blind) = match self.run.onboarding_phase() {
                        Some(crate::game::onboarding::OnboardingPhase::Lessons) => {
                            let feedback = crate::game::onboarding::lessons_failure_feedback(
                                round_score,
                                target_score,
                                plays_left,
                            );
                            self.run.retry_onboarding_lessons();
                            (feedback, self.run.chamber)
                        }
                        _ => {
                            let feedback = crate::game::onboarding::finale_failure_feedback(
                                round_score,
                                target_score,
                                discards_left,
                            );
                            self.run.retry_onboarding_finale();
                            (feedback, self.run.chamber)
                        }
                    };
                    self.audio.play_sfx(audio::SfxId::GameOver);
                    let modal = Modal::new("Try Again!", &feedback, ModalTheme::Info);
                    self.modals.push(modal);
                    self.begin_scene_replace(
                        SceneIntent::GameplayRetryOnboarding(retry_blind),
                        SceneTag::from(&self.scene),
                        PendingSceneDestination::default(),
                    );
                    return;
                }
                self.progress.runs_completed += 1;
                self.sync_runs_completed_achievements();
                self.progress.award_level_points_for_outcome(
                    crate::core::progression::RunOutcome::Defeat { reason },
                );
                self.progress.record_score(self.run.round_score);
                let level_up = self.progress.check_level_up();
                let snap = crate::game::memorial_run::snapshot_from_run(
                    &self.run.defeat_journal,
                    reason,
                    &self.run,
                );
                let memorial = crate::core::memorial_talisman::select_memorial(&snap);
                self.run.defeat_memorial_kind = Some(memorial);
                self.progress.pending_memorial = Some(memorial);
                self.progress.pending_memorial_journal = Some(snap);
                self.progress
                    .run_history
                    .push(crate::game::progression_run::run_record_from_run(
                        &self.run,
                        crate::core::progression::RunOutcome::Defeat { reason },
                    ));
                let _ = persistence::save_profile(self.active_profile, &self.progress);
                // Run is over — drop any saved-on-quit snapshot so the
                // player isn't offered "Continue" into a finished game.
                persistence::delete_saved_run(self.active_profile);
                self.dist.sync_profile_stats(&self.progress);

                if let Some(result) = level_up
                    && let Some(modal) = build_level_up_modal(&result, ww, wh)
                {
                    log::info!(
                        "Depth increased to {}",
                        core::progression::meta_depth_roman(result.new_level)
                    );
                    self.pending_post_game_over_level_up = Some(modal);
                }

                self.audio.play_sfx(audio::SfxId::GameOver);
                // Loss jingle takes over the music sink while the GameOver
                // scene fades in; `sync_music_for_scene` will call
                // `stop_background_music`, which defers until the jingle
                // empties so the stinger isn't truncated mid-fade.
                let loss_jingle = if self.run.chamber == crate::core::rules::ChamberKind::Ordeal {
                    audio::MusicId::OrdealLoss
                } else {
                    audio::MusicId::ChamberLoss
                };
                self.audio.play_music_jingle(loss_jingle);
                self.begin_scene_replace(
                    SceneIntent::Defeat(reason),
                    SceneTag::from(&self.scene),
                    PendingSceneDestination::default(),
                );
            }
            _ => {}
        }
    }

    pub(super) fn draw(&mut self, shell: &mut crate::sdl_shell::SdlShell) {
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
        //
        // Pushdown overlays on `overlay_stack` (Rumble Lab, material viewer,
        // …) are the *only* source of `frame.buttons` while they are on top.
        // `app_overlay_wipe` would erase those hit targets even though the
        // player still sees the overlay — clicks fall through as misses.
        let app_overlay_wipe = self.modals.is_active()
            || self.debug.tuning_overlay.is_some()
            || self.debug.sfx_test_overlay.is_some()
            || self.debug.camera_debug_overlay.is_some()
            || self.debug.scene_look_debug_overlay.is_some()
            || self.debug.rain_debug_overlay.is_some()
            || self.debug.flame_debug_overlay.is_some()
            || self.debug.victory_moon_debug_overlay.is_some()
            || self.debug.hallway_distortion_debug_overlay.is_some();
        let preserve_overlay_stack_buttons = matches!(
            self.overlay_stack.last(),
            Some(
                Scene::RumbleLab(_)
                    | Scene::MaterialViewer(_)
                    | Scene::TransitionPlayground(_)
                    | Scene::AnimationLab(_)
                    | Scene::TileAnchorLab(_)
                    | Scene::TileStressLab(_)
                    | Scene::ButtonAabbLab(_)
                    | Scene::RollerLab(_)
                    | Scene::CascadeLab(_)
                    | Scene::ShadowAoLab(_)
                    | Scene::TextShadowLab(_)
            )
        );
        let scene_look = self.resolved_scene_look();
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        self.cpu_profiler
            .begin(crate::render::cpu_profiler::CpuStage::DrawPrep);
        if let Scene::Splash(splash) = &mut self.scene {
            splash.mark_visible();
        }
        let size = self.last_drawable_px;
        let layout = self
            .layout_engine
            .solve(size.width as f32, size.height as f32);

        let showcase_orbit_top = self.overlay_stack.last().is_some_and(|top| {
            matches!(
                top,
                crate::scenes::Scene::Showcase(s) if s.wants_orbit_input()
            )
        });
        let suspended_shop = match (&self.scene, showcase_orbit_top) {
            (crate::scenes::Scene::Shop(s), true) => Some(s),
            _ => None,
        };
        let suspended_collection = match (&self.scene, showcase_orbit_top) {
            (crate::scenes::Scene::Archive(c), true) => Some(c),
            _ => None,
        };

        let p = self.active_profile.min(2);
        let archive_chronicle_last_seen_run_len = self.archive_last_seen_run_len[p];
        let archive_has_new = crate::core::archive_seen::archive_has_any_new(
            &self.progress,
            archive_chronicle_last_seen_run_len,
        );
        let settings = crate::persistence::load_settings();
        let detected = self
            .input
            .as_ref()
            .map(|i| i.gamepad_style)
            .unwrap_or_default();
        let prompt_style = settings.glyph_prompt.resolve(detected);
        let swap_ab = self.input.as_ref().map(|i| i.swap_ab).unwrap_or(false);
        let swap_xy = self.input.as_ref().map(|i| i.swap_xy).unwrap_or(false);
        let glyphs = crate::ui::glyph_source::GlyphResolver::new(prompt_style, swap_ab, swap_xy);
        let shop_env_for_frame = self.room_gltf_brownout.apply(scene_look.room);
        let (overlay_key, overlay_look) = self
            .debug
            .scene_look_debug_overlay
            .as_ref()
            .map(|o| (Some(o.scene_key_persist()), Some(o.look)))
            .unwrap_or((None, None));
        let mut env_per_scene = rustc_hash::FxHashMap::default();
        let mut env_frame_tunes = Vec::new();
        for &key in crate::game::scene_look_tuning::GLTF_ENV_SCENE_KEYS {
            let look = crate::game::scene_look_tuning::resolve_scene_look_with_overlay(
                &self.scene_look,
                overlay_key,
                overlay_look,
                Some(key),
            );
            let room = self.room_gltf_brownout.apply(look.room);
            env_per_scene.insert(key, (room, look.room_gltf_height_scale));
            env_frame_tunes.push((
                key,
                mahjuro_render::tuning::scene_look::room_env_frame_from_scene_look(&look, room),
            ));
        }
        let loading_hub_progress = if matches!(self.scene, Scene::Splash(_)) {
            let _g = crate::render::cpu_profiler::scope("draw_prep.loading_hub_progress");
            renderer.splash_hub_boot_progress(&self.gfx.tileset_name)
        } else {
            1.0
        };
        if self.debug.focus_nav_overlay {
            self.debug.focus_nav_snapshot = None;
        }
        let ctx = DrawCtx::new(
            &layout,
            &self.anim,
            &self.run,
            &self.progress,
            self.active_profile,
            self.run.is_in_progress(),
            renderer.projections(),
            // Reuse the frame_tick pick cache; both are computed against
            // the same cursor and frame matrices, and walking the AABB
            // tests twice per frame for free was the prior behavior.
            self.frame_picks.gameplay,
            self.frame_picks.shop,
            self.debug.visibility,
            modal_active,
            scene_look.room_gltf_height_scale,
            shop_env_for_frame,
            &env_per_scene,
            self.effect_layers,
            self.input
                .as_ref()
                .map(|i| i.last_cursor)
                .unwrap_or((0.0, 0.0)),
            self.input
                .as_ref()
                .map(|i| i.mode)
                .unwrap_or(crate::ui::input::InputMode::Cursor),
            glyphs,
            suspended_shop,
            suspended_collection,
            self.gfx.tile_preset,
            archive_has_new,
            archive_chronicle_last_seen_run_len,
            self.debug
                .hallway_distortion_debug_overlay
                .as_ref()
                .map(|o| o.to_snapshot())
                .or_else(|| {
                    self.debug
                        .trailer_mode
                        .as_ref()
                        .and_then(|tm| tm.hallway_snapshot_at(self.last_frame))
                }),
            self.debug
                .trailer_mode
                .as_ref()
                .and_then(|tm| tm.main_menu_camera_at(self.last_frame, size.height as f32)),
            loading_hub_progress,
            self.hub_loading,
            renderer.main_menu_effects,
            renderer.flame_tuning,
            self.debug.victory_moon_debug,
            self.gfx.graphics_mode,
            renderer.victory_moon_gpu_draw_ready(),
            self.debug.focus_nav_overlay,
            &mut self.debug.focus_nav_snapshot,
        );
        self.cpu_profiler
            .end(crate::render::cpu_profiler::CpuStage::DrawPrep);
        // Build the scene's frame in canonical push-order. For migrated
        // scenes (gameplay) this calls their direct `draw_frame` impl;
        // for legacy scenes the default impl forwards through `draw()` +
        // `into_frame()`. Either way we get back a single ordered
        // `UiFrame.cmds` list whose push order is z-order.
        self.cpu_profiler
            .begin(crate::render::cpu_profiler::CpuStage::DrawFrame);
        let mut frame: UiFrame = if let Some(top) = self.overlay_stack.last() {
            top.draw_frame(ctx)
        } else {
            self.scene.draw_frame(ctx)
        };
        self.cpu_profiler
            .end(crate::render::cpu_profiler::CpuStage::DrawFrame);

        let h = size.height as f32;
        self.debug.last_effective_camera = frame
            .camera_override
            .unwrap_or_else(|| CameraParams::default_table_camera(h));

        // SDL3 → Wayland/X11 charges a syscall on every set_title; only
        // call when the title actually changes. The title rarely moves
        // outside of debug toggles or scene transitions.
        if frame.window_title != self.last_window_title {
            let _ = shell.window.set_title(&frame.window_title);
            self.last_window_title = frame.window_title.clone();
        }
        self.active_buttons = frame.buttons.clone();

        // Click-safety wipe: if any modal-like overlay is up, scene buttons
        // must not be clickable through it. Overlays that want their own
        // clickable surface (e.g. `ModalQueue`'s full-screen dismiss button)
        // write to `active_buttons` *after* this point in their draw step.
        // See `App::modal_overlay_active` for the contract.
        if app_overlay_wipe && (!preserve_overlay_stack_buttons || self.modals.is_active()) {
            self.active_buttons.clear();
        }

        // Forward the cursor for scenes that need screen-space hit tests or layout.
        frame.cursor_pos = self.input.as_ref().map(|i| i.last_cursor);

        // Apply transition alpha to everything that's part of the scene
        // (after into_frame so all scene cmds exist; before overlays are
        // appended so they fade in cleanly).
        let alpha = self.transition_alpha;
        frame.apply_alpha(alpha);

        // Overlay fullscreen transition shaders (not zodiac’s in-scene cascade).
        if self.transition_timer > 0.0
            && self.effect_layers.transition_fullscreen_fx
            && let Some(kind) = overlay_kind_for_transition(self.transition_kind)
        {
            crate::render::transition_fx::push_overlay_transition(
                &mut frame,
                kind,
                self.transition_timer,
                (size.width as f32, size.height as f32),
            );
        }
        let mut hide_all_ui = false;

        self.modals.update();
        if let Some((
            modal_insts,
            modal_labels,
            modal_buttons,
            modal_relic_objects,
            modal_gradient_quads,
        )) = self.modals.draw(size.width as f32, size.height as f32)
        {
            // Modal chrome must use OverlayQuad so it draws in the text-overlay
            // pass after scene labels (regular Quad is tonemapped in Pass A).
            frame.overlay_quads(modal_insts);
            frame.texts(modal_labels);
            if !modal_gradient_quads.is_empty() {
                frame.gradient_quads(modal_gradient_quads);
            }
            apply_modal_relic_staging(
                &mut frame,
                size.width as f32,
                size.height as f32,
                modal_relic_objects,
            );
            // Replace scene buttons with modal buttons so only dismiss works.
            self.active_buttons = modal_buttons;
        }

        // Tuning overlay — on top of modals.
        if let Some(ref overlay) = self.debug.tuning_overlay {
            let (tuning_insts, tuning_labels) = overlay.draw(size.width as f32, size.height as f32);
            append_fullscreen_debug_panel(
                &mut frame,
                &mut self.active_buttons,
                tuning_insts,
                tuning_labels,
            );
        }

        // SFX test overlay — on top of modals.
        if let Some(ref mut overlay) = self.debug.sfx_test_overlay {
            let (insts, lbls) = overlay.draw(size.width as f32, size.height as f32);
            append_fullscreen_debug_panel(&mut frame, &mut self.active_buttons, insts, lbls);
        }

        // Camera debug overlay — on top of modals.
        if let Some(ref overlay) = self.debug.camera_debug_overlay {
            frame.camera_override = Some(overlay.to_camera_params());
            let (insts, lbls) = overlay.draw(size.width as f32, size.height as f32);
            append_fullscreen_debug_panel(&mut frame, &mut self.active_buttons, insts, lbls);
        }

        // Scene look debug overlay — on top of modals (right panel).
        if let Some(ref overlay) = self.debug.scene_look_debug_overlay {
            let (insts, lbls) = overlay.draw(size.width as f32, size.height as f32);
            append_fullscreen_debug_panel(&mut frame, &mut self.active_buttons, insts, lbls);
        }

        if let Some(ref overlay) = self.debug.rain_debug_overlay {
            frame.debug_rain_hit_colliders = overlay.show_rain_hit_colliders;
            frame.debug_rain_depth = overlay.show_rain_depth;
            hide_all_ui |= overlay.hide_all_ui;
            let cam = frame
                .camera_override
                .unwrap_or(self.debug.last_effective_camera);
            let env_scale = crate::render::main_menu_glb::main_menu_env_height_scale(
                scene_look.room_gltf_height_scale,
            );
            let (insts, lbls) = overlay.draw(size.width as f32, size.height as f32, cam, env_scale);
            append_fullscreen_debug_panel(&mut frame, &mut self.active_buttons, insts, lbls);
        }

        // Pick-blind hallway warp debug (left panel).
        if let Some(ref overlay) = self.debug.hallway_distortion_debug_overlay {
            let (insts, lbls) = overlay.draw(
                size.width as f32,
                size.height as f32,
                scene_look.room_gltf_height_scale,
            );
            append_fullscreen_debug_panel(&mut frame, &mut self.active_buttons, insts, lbls);
        }

        // Candle flame tuning (left panel).
        if let Some(ref overlay) = self.debug.flame_debug_overlay {
            let (insts, lbls) = overlay.draw(size.width as f32, size.height as f32);
            append_fullscreen_debug_panel(&mut frame, &mut self.active_buttons, insts, lbls);
        }

        // Victory 3D moon rotation + phase (left panel).
        if let Some(ref overlay) = self.debug.victory_moon_debug_overlay {
            let (insts, lbls) = overlay.draw(size.width as f32, size.height as f32);
            append_fullscreen_debug_panel(&mut frame, &mut self.active_buttons, insts, lbls);
        }

        // Debug visibility overlay — on top of modals.
        if let Some(ref overlay) = self.debug.visibility_overlay {
            let (insts, lbls) = overlay.draw(size.width as f32, size.height as f32);
            append_fullscreen_debug_panel(&mut frame, &mut self.active_buttons, insts, lbls);
        }

        if self.debug.focus_nav_overlay {
            if let Some(ref snap) = self.debug.focus_nav_snapshot {
                let w = size.width as f32;
                let h = size.height as f32;
                let scale = crate::render::theme::metrics::scene_scale(w, h);
                let mut insts = Vec::new();
                let mut lbls = Vec::new();
                crate::ui::focus_nav::push_focus_nav_debug_overlay(
                    snap, w, h, scale, &mut insts, &mut lbls,
                );
                frame.debug_overlay_quads(insts);
                frame.debug_overlay_texts(lbls);
            }
        }

        hide_all_ui |= self.debug.hide_2d_ui;
        if hide_all_ui {
            self.active_buttons.clear();
            frame.buttons.clear();
            hide_ui_draw_cmds(&mut frame);
        }

        // Cursor hover labels for `ButtonDef::hover_label`. Scan in vec order (same as
        // click hit-test): first matching rect with a label wins — so smaller rects
        // pushed before a fullscreen catch-all still show tooltips.
        if let Some(ref input) = self.input
            && input.mode == crate::ui::input::InputMode::Cursor
        {
            let cursor = input.last_cursor;
            let w = size.width as f32;
            let h = size.height as f32;
            let scale = crate::render::theme::metrics::scene_scale(w, h);
            if let Some(btn) = self.active_buttons.iter().find(|b| {
                let (bx, by, bw, bh) = b.rect;
                let inside =
                    cursor.0 >= bx && cursor.0 <= bx + bw && cursor.1 >= by && cursor.1 <= by + bh;
                inside && b.hover_label.is_some()
            }) && let Some(ref label) = btn.hover_label
            {
                let pad = (h * 0.012 * scale).max(6.0);
                let min_outer_h = ((h * 0.035 * scale).max(22.0)).min(h * 0.12);
                let line_h = (min_outer_h * 0.52).max(8.0);
                let max_tooltip_w =
                    crate::render::theme::metrics::tooltip_max_panel_px(w, h) * 0.72;
                let max_inner_w = (max_tooltip_w - 2.0 * pad).max(40.0);
                let preferred_inner_w = crate::ui::styled_text::colored_paragraph_preferred_width(
                    label.as_ref(),
                    line_h,
                    max_inner_w,
                    GlossaryMode::Prose,
                )
                .max(40.0);
                let tooltip_w = (preferred_inner_w + 2.0 * pad).clamp(72.0, max_tooltip_w);
                let (bx, by, bw, bh) = btn.rect;
                let cx = bx + bw * 0.5;
                let mut tip_x = cx - tooltip_w * 0.5;
                tip_x = tip_x.max(pad).min(w - tooltip_w - pad);

                let parchment = crate::render::theme::color::PARCHMENT;
                let inner_w = (tooltip_w - 2.0 * pad).max(40.0);
                let color_lines = crate::ui::styled_text::wrap_colored_text_multiline(
                    label.as_ref(),
                    inner_w,
                    line_h,
                    parchment,
                    false,
                    GlossaryMode::Prose,
                );
                let content_h = crate::ui::styled_text::colored_multiline_block_height(
                    color_lines.len(),
                    line_h,
                );
                let inner_h = (content_h).max(min_outer_h - 2.0 * pad);
                let tooltip_h = (inner_h + 2.0 * pad).max(min_outer_h).min(h * 0.35);

                let mut tip_y = by - tooltip_h - pad;
                if tip_y < pad {
                    tip_y = by + bh + pad;
                }
                if tip_y + tooltip_h > h - pad {
                    tip_y = (h - tooltip_h - pad).max(pad);
                }
                // Same brass + midnight frame as [`crate::ui::tooltip`] / focus inspect panels.
                let mut tip_quads: Vec<GpuInstance> = Vec::with_capacity(2);
                let border = crate::render::theme::metrics::tooltip_border_px(w, h);
                crate::ui::tooltip::push_tooltip_frame_quads(
                    &mut tip_quads,
                    tip_x,
                    tip_y,
                    tooltip_w,
                    tooltip_h,
                    border,
                );
                frame.overlay_quads(tip_quads);
                let text_top = tip_y + pad + ((tooltip_h - 2.0 * pad - content_h) * 0.5).max(0.0);
                let mut tip_texts: Vec<crate::render::wgpu_renderer::TextLabel> = Vec::new();
                crate::ui::styled_text::push_colored_rows_in_width(
                    &mut tip_texts,
                    crate::ui::styled_text::ColoredRowsLayout {
                        text_left: tip_x + pad,
                        top_y: text_top,
                        inner_w,
                        line_h,
                        fallback_plain: label.as_ref(),
                        fallback_color: parchment,
                        italic: false,
                        glossary: GlossaryMode::Prose,
                    },
                    &color_lines,
                    crate::render::wgpu_renderer::TextAlign::Center,
                );
                frame.texts(tip_texts);
            }
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
            frame.debug_overlay_quads([GpuInstance {
                rect: [w - label_w - margin, margin, label_w, label_h],
                color: [0.0, 0.0, 0.0, 0.55],
                user: 0,
            }]);
            frame.debug_overlay_texts([TextLabel {
                rect: [w - label_w - margin, margin, label_w, label_h],
                text: format!("{:.0} FPS", self.debug.fps_smoothed),
                color: [0.9, 0.9, 0.3, 1.0],
                font_px: Some(crate::render::theme::typography::size(
                    crate::render::theme::typography::H42,
                    h,
                )),
                ..Default::default()
            }]);
        }

        // Final scene-transition blackout pass. `apply_alpha` fades 2D draw
        // commands, but environment/room draw cmds are opaque and do not
        // participate in that alpha scaling.
        let blackout_alpha = (1.0 - self.transition_alpha).clamp(0.0, 1.0);
        if blackout_alpha > 0.0 {
            self.active_buttons.clear();
            frame.overlay_quads([GpuInstance {
                rect: [0.0, 0.0, size.width as f32, size.height as f32],
                color: [0.0, 0.0, 0.0, blackout_alpha],
                user: 0,
            }]);
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
        // prefixed canonical pickable names.
        let top = self.overlay_stack.last().unwrap_or(&self.scene);
        let parent = crate::scenes::overlay_renderer_parent(&self.scene, &self.overlay_stack);
        let active_scene_key = crate::scenes::active_scene_key_for_renderer(top, parent);
        renderer.set_active_scene(active_scene_key);

        if matches!(active_scene_key, Some("gameplay") | Some("tutorial"))
            && self.debug.visibility.any_hide()
        {
            crate::scenes::debug_visibility::filter_gameplay_frame_cmds(
                &mut frame,
                &self.debug.visibility,
            );
        }

        renderer.set_tonemap_tuning(&scene_look.tonemap);

        renderer.set_frame_scene_env_tunes(active_scene_key, &env_frame_tunes);

        let active_tileset_name = self.gfx.tileset_name.clone();
        let mut render_settings = self.effect_layers.wgpu_render_settings(
            &crate::effect_layers::WgpuRenderSettingsParams {
                gfx: &self.gfx,
                tile_preset: self.gfx.tile_preset,
                tile_material: active_material,
                tileset_name: active_tileset_name,
                draw_settle_speed,
                sort_settle_speed,
            },
        );
        if let Some(q) = frame.shadow_quality_override {
            render_settings.shadow_quality = q;
        }
        renderer.set_graphics_budget(self.gfx.graphics_mode);
        renderer.set_hdr_enabled(self.effect_layers.hdr_enabled(&self.gfx));
        renderer.main_menu_pride_rainbow_debug = self.debug.main_menu_pride_rainbow_debug;
        renderer.main_menu_moon_phase_debug = self.debug.main_menu_moon_phase_debug;

        // Journal pre-pass: when the shop set `journal_prepass_frame`,
        // render that frame to the offscreen `journal_scene_texture`
        // before the main pass. The shop's book mesh samples that texture
        // in screen space, so the page region reads as a live render of
        // the embedded yaku-journal scene rather than a flat decal.
        // That path renders into an offscreen target; only `renderer.render`
        // below presents the visible frame.
        if let Some(prepass) = frame.journal_prepass_frame.take()
            && let Err(e) = renderer.render_journal_prepass(&prepass, render_settings.clone())
        {
            log::error!("journal prepass: {e:?}");
        }

        self.cpu_profiler
            .begin(crate::render::cpu_profiler::CpuStage::Render);
        if let Err(e) = renderer.render(&frame, render_settings) {
            log::error!("render: {e:?}");
        }
        if let Scene::Gameplay(gp) = &self.scene {
            gp.stash_hand_tile_pick_rects(renderer.projections().hand_rects.clone());
        }
        let rollers_loop = frame
            .gameplay_score_roller_values
            .and_then(|(score, target)| {
                renderer
                    .gameplay_score_rollers_spinning(score, target)
                    .then_some((
                        crate::audio::SfxId::RollersSpin,
                        renderer.gameplay_score_roller_loop_speed(),
                    ))
            });
        match rollers_loop {
            Some((id, speed)) => self.audio.set_sfx_loop(Some(id), speed),
            None => self.audio.set_sfx_loop(None, 1.0),
        }
        self.cpu_profiler
            .end(crate::render::cpu_profiler::CpuStage::Render);
        self.cpu_profiler.end_frame();
    }
}

#[cfg(any(feature = "game", feature = "headless-screenshot"))]
/// Build the paginated celebration modal for a level-up. Returns `None`
/// when the level grants no new relics.
pub fn build_level_up_modal(
    result: &crate::core::progression::LevelUpResult,
    window_w: f32,
    window_h: f32,
) -> Option<Modal> {
    let mut pages = Vec::new();
    let relic_defs = crate::core::relic::all_relic_defs();
    for rid in &result.relics {
        if let Some(def) = relic_defs.iter().find(|d| d.id == *rid) {
            let accent = render::theme::color::rarity(def.rarity.tier());
            pages.push(UnlockPage {
                category: "New Relic".into(),
                name: def.name.into(),
                description: rid.description(),
                relic_id: Some(*rid),
                accent_color: accent,
            });
        }
    }
    if pages.is_empty() {
        return None;
    }
    Some(
        Modal::new(
            format!(
                "Deeper — Depth {}",
                crate::core::progression::meta_depth_roman(result.new_level)
            ),
            "",
            ModalTheme::Success,
        )
        .with_pages(pages)
        // Lantern-mote celebration: rises from the bottom band of the
        // screen behind the hero relic. Spread is wide so motes drift
        // across the full backdrop rather than columning behind the
        // relic. Internally this gets multiplied (see
        // `Fireworks::launch`) so the modest count produces a dense
        // swarm without spammy callsites.
        .with_fireworks(window_w * 0.5, window_h * 0.92, window_w * 0.85, 24),
    )
}
