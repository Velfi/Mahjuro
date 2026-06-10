use super::locals::FrameLocals;
use crate::scene_transition::{
    apply_post_scene_transition_effects, sync_music_for_scene, DEFAULT_QUICK_SPEC,
    PendingSceneDestination, PostSceneTransitionCtx, SceneTag,
};
use crate::scenes::{Scene, SceneBehavior};
use crate::scenes;
use crate::steam::DistributionBackend;
use crate::App;
use crate::persistence;
use crate::sdl_shell::SdlShell;

pub fn run(app: &mut App, shell: &mut SdlShell, locals: &mut FrameLocals) {
        if app.shop_storeroom_dwell_active() {
            let milestones = app
                .progress
                .accumulate_shop_storeroom_seconds(app.last_frame_dt);
            if milestones > 0 {
                app.mark_profile_dirty();
                if let Scene::Shop(shop) = &mut app.scene {
                    for _ in 0..milestones {
                        shop.play_eyeball_travel_milestone();
                    }
                }
            }
        }
        if locals.seed_archive_seen {
            crate::core::archive_seen::archive_seen_migration_seed(&mut app.progress);
            app.mark_profile_dirty();
        }
        if let Some(n) = locals.bump_archive_chronicle_seen {
            let p = app.active_profile.min(2);
            let mut s = persistence::load_settings();
            if s.archive_last_seen_run_len[p] != n {
                s.archive_last_seen_run_len[p] = n;
                app.archive_last_seen_run_len[p] = n;
                let _ = persistence::save_settings(&s);
            }
        }
        // Apply overlay push/pop before a SceneTransition (Replace).
        // Push/Pop operate on the overlay stack; they never fade.
        match locals.overlay_request.take() {
            Some(scenes::OverlayRequest::Push(s)) => {
                app.overlay_stack.push(*s);
                if matches!(app.overlay_stack.last(), Some(Scene::Credits(_))) {
                    app.audio.set_music_track(crate::audio::MusicId::Credits);
                }
            }
            Some(scenes::OverlayRequest::Pop) => {
                let was_credits = app
                    .overlay_stack
                    .last()
                    .is_some_and(|s| matches!(s, Scene::Credits(_)));
                let _ = app.overlay_stack.pop();
                if was_credits {
                    let tag = SceneTag::from(&app.scene);
                    let gameplay_ordeal_chamber = tag == SceneTag::Gameplay
                        && matches!(
                            &app.scene,
                            Scene::Gameplay(g) if g.music_chamber_kind(app.run.chamber)
                                == crate::core::rules::ChamberKind::Ordeal
                        );
                    sync_music_for_scene(&mut app.audio, tag, gameplay_ordeal_chamber, None);
                }
            }
            None => {}
        }
        app.dispatch_rumble_lab_ops(shell, locals.now, std::mem::take(&mut locals.rumble_lab_ops));
        let shop_ready = matches!(&app.scene, Scene::Shop(_))
            && app.overlay_stack.is_empty()
            && !app.scene.has_blocking_overlay();
        // Only drive shop-hold rumble on the unobstructed shop face. When `hold`
        // is false, sync stops motors — if we ran that every frame globally it
        // would cancel rumble lab / scoring pulses the same tick they fire.
        if shop_ready && let Some(input) = app.input.as_ref() {
            let hold = matches!(
                &app.scene,
                Scene::Shop(s) if s.sell_hold_in_progress() || s.buy_hold_in_progress()
            );
            let progress = match &app.scene {
                Scene::Shop(s) if hold => {
                    let shop = crate::game::engine::GameEngine::read_shop(&app.run);
                    s.sell_hold_progress(locals.now, &shop)
                        .or_else(|| s.buy_hold_progress(locals.now, &app.run, &shop))
                        .unwrap_or(0.0)
                }
                _ => 0.0,
            };
            let controller = input.mode == crate::ui::input::InputMode::Controller;
            let enabled = input.hold_to_sell_rumble_enabled;
            app.sync_shop_sell_hold_rumble(shell, hold, controller, enabled, progress);
        }
        // Gameplay hold-to-cash-in rumble. Only driven while actively charging:
        // we never call sync with `hold = false` here, so the motors expire on
        // their own and we don't clobber scoring-cascade pulses the frame the
        // cash-in completes.
        let gameplay_cash_in_hold = matches!(&app.scene, Scene::Gameplay(g) if g.cash_in_hold_in_progress())
            && app.overlay_stack.is_empty()
            && !app.scene.has_blocking_overlay();
        if gameplay_cash_in_hold && let Some(input) = app.input.as_ref() {
            let progress = match &app.scene {
                Scene::Gameplay(g) => {
                    let trigger_enabled =
                        crate::game::engine::GameEngine::read(&app.run).trigger_enabled;
                    g.cash_in_hold_progress(locals.now, trigger_enabled).unwrap_or(0.0)
                }
                _ => 0.0,
            };
            let controller = input.mode == crate::ui::input::InputMode::Controller;
            let enabled = input.hold_to_sell_rumble_enabled;
            app.sync_shop_sell_hold_rumble(shell, true, controller, enabled, progress);
        }
        if let Some(intent) = locals.update_result.take() {
            app.begin_scene_replace(
                intent,
                SceneTag::from(&app.scene),
                if locals.updated_overlay {
                    PendingSceneDestination::OverlayTop
                } else {
                    PendingSceneDestination::Base
                },
            );
        }

        if locals.complete_onboarding {
            app.progress.tutorial_completed = true;
            app.mark_profile_dirty();
            app.dist
                .unlock_achievement(crate::steam::Achievement::TutorialComplete);
        }

        // Sync live audio/graphics settings whenever the player has
        // an options menu open — either the standalone Options scene
        // (from the start screen) or the embedded options overlay
        // inside the in-game pause menu.
        let active_options_overlay = match &app.scene {
            // Standalone Options scene IS the options screen, so its
            // own state is what we sync. Every other scene defers to
            // its `SceneBehavior::pause_options_overlay()` (default
            // `None` for scenes without an embedded pause menu).
            Scene::Options(opts) => Some(opts),
            other => other.pause_options_overlay(),
        };
        if let Some(opts) = active_options_overlay {
            app.audio.set_master_volume(opts.master_volume);
            app.audio.set_sfx_volume(opts.sfx_volume);
            app.audio.set_music_volume(opts.music_volume);
            app.audio.set_enabled(opts.sfx_enabled);
            app.gfx.effects_quality = opts.effects_quality;
            app.gfx.tile_preset = opts.tile_preset;
            app.gfx.tileset_name = opts.tileset_name.clone();
            app.gfx.gamma = opts.gamma;
            app.gfx.graphics_mode = opts.graphics_mode;
            app.gfx.hdr_enabled = opts.hdr_enabled;
            if opts.take_borderless_fullscreen_apply_armed()
                && opts.borderless_fullscreen != shell.desktop_fullscreen_on()
            {
                let _ = shell.set_desktop_fullscreen(opts.borderless_fullscreen);
            }
            app.run
                .set_auto_cash_in_on_full_structure(opts.auto_cash_in_on_full_structure);
            if let Some(ref mut input) = app.input {
                input.swap_ab = opts.swap_ab;
                input.swap_xy = opts.swap_xy;
                input.xy_quick_action = opts.xy_quick_action;
                input.hold_to_sell_rumble_enabled = opts.hold_to_sell_rumble;
            }
        }

        // Handle profile switch request.
        if let Some(idx) = locals.switch_profile {
            let new_idx = if idx == usize::MAX {
                // Previous profile (wrapping), from start screen arrows.
                (app.active_profile + 3 - 1) % 3
            } else if idx == usize::MAX - 1 {
                // Next profile (wrapping), from start screen arrows.
                (app.active_profile + 1) % 3
            } else {
                // Absolute index, from profile select scene.
                idx.min(2)
            };
            if new_idx != app.active_profile {
                app.switch_profile(new_idx);
            }
        }

        // Handle profile delete request.
        if let Some(idx) = locals.delete_profile {
            let idx = idx.min(2);
            persistence::delete_profile(idx);
            // If we just deleted the active profile, reload it (now
            // returns a fresh default since the file is gone).
            if idx == app.active_profile {
                app.progress = persistence::load_profile(idx);
                let loaded_run = persistence::load_run(idx);
                app.resume_scene = loaded_run
                    .as_ref()
                    .map(|saved| saved.scene)
                    .unwrap_or(persistence::ResumeScene::Gameplay);
                app.run = loaded_run
                    .map(|saved| saved.run)
                    .unwrap_or_else(crate::game::run::RunState::new_demo);
                app.run.apply_progression(&app.progress);
                app.dist.sync_profile_stats(&app.progress);
            }
        }

        // If we deferred a round-end event so the player could watch the
        // scoring cascade, reel, and popups finish, fire it once that
        // presentation is done (not every unrelated gameplay tween).
        if app.deferred_round_end.is_some() {
            let cascade_lab = app
                .overlay_stack
                .last()
                .is_some_and(|s| matches!(s, Scene::CascadeLab(_)));
            if app.run.suppress_chamber_resolution || cascade_lab {
                app.deferred_round_end = None;
            } else {
                let scoring_done = match &app.scene {
                    Scene::Gameplay(g) => g.ready_for_round_end(locals.now),
                    _ => true,
                };
                if scoring_done && let Some(ev) = app.deferred_round_end.take() {
                    app.handle_round_end_event(ev);
                }
            }
        }

        // Advance transition animation using the animation controller.
        // Pause the transition while a modal is active so the player
        // must dismiss milestone / celebration modals before the scene
        // change proceeds (e.g. "First Pair!" before the recap screen).
        // Stairway → shop after decimation is exempt — burn already finished.
        if app.scene_replace_in_flight() && app.scene_transition_unblocked() {
            app.transition_alpha -= app.transition_speed;
            // Map alpha 1→0 onto timer 0→0.5 (first half of transition).
            app.transition_timer = (1.0 - app.transition_alpha.max(0.0)).clamp(0.0, 1.0) * 0.5;
            if app.transition_alpha <= 0.0 {
                app.transition_alpha = 0.0;
                if app.pending_scene.is_none() {
                    app.resolve_pending_scene_intent_at_black();
                }
                if !app.pending_scene_room_gpu_ready() {
                    // Hold at full black until pending scene room uploads complete.
                    app.transition_timer = 0.5;
                } else if let Some(next) = app.pending_scene.take() {
                    if matches!(&next, Scene::Shop(_)) {
                        app.run.grant_pending_memorial(&mut app.progress);
                        app.mark_profile_dirty();
                    }
                    let from_tag = SceneTag::from(&app.scene);
                    let to_tag = SceneTag::from(&next);
                    let gameplay_ordeal_chamber = to_tag == SceneTag::Gameplay
                        && matches!(
                            &next,
                            Scene::Gameplay(g) if g.music_chamber_kind(app.run.chamber)
                                == crate::core::rules::ChamberKind::Ordeal
                        );
                    // Route the new scene to the target recorded
                    // when the transition started, not whatever is
                    // on top now — overlays may have been pushed
                    // mid-fade (e.g. a zodiac celebration after a
                    // skip) and must not clobber them.
                    match app.pending_scene_destination {
                        PendingSceneDestination::OverlayTop => {
                            if let Some(top) = app.overlay_stack.last_mut() {
                                *top = next;
                            } else {
                                app.scene = next;
                            }
                        }
                        PendingSceneDestination::Base => {
                            app.scene = next;
                        }
                    }
                    app.pending_scene_destination = PendingSceneDestination::default();
                    if let Some(scene) = App::saved_resume_scene_for(&app.scene) {
                        app.resume_scene = scene;
                    }
                    apply_post_scene_transition_effects(PostSceneTransitionCtx {
                        from: from_tag,
                        to: to_tag,
                        gameplay_ordeal_chamber,
                        anim: &mut app.anim,
                        renderer: app.renderer.as_mut(),
                        input: app.input.as_mut(),
                        audio: &mut app.audio,
                    });
                }
            }
        } else if app.transition_alpha < 1.0 {
            app.transition_alpha = (app.transition_alpha + app.transition_speed).min(1.0);
            // Map alpha 0→1 onto timer 0.5→1.0 (second half).
            app.transition_timer = 0.5 + (app.transition_alpha.clamp(0.0, 1.0)) * 0.5;
            // Reset transition kind once fully faded in.
            if app.transition_alpha >= 1.0 {
                app.transition_timer = 0.0;
                app.transition_kind = DEFAULT_QUICK_SPEC.kind;
                app.transition_speed = DEFAULT_QUICK_SPEC.speed;
            }
        }

        app.try_surface_pending_post_game_over_level_up();

}
