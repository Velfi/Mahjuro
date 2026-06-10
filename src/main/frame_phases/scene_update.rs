use super::locals::FrameLocals;
use crate::render::scene_keys;
use crate::scenes::{Scene, SceneBehavior, UpdateCtx};
use crate::scenes;
use crate::App;
use crate::FramePicks;
use crate::persistence;

pub fn run(app: &mut App, locals: &mut FrameLocals) {
        let focus = app.input.as_ref().map(|i| i.focused_index()).unwrap_or(0);
        let win_size = app.last_drawable_px;
        let update_layout = app
            .layout_engine
            .solve(win_size.width as f32, win_size.height as f32);
        let cursor_pos = app
            .input
            .as_ref()
            .map(|i| i.last_cursor)
            .unwrap_or((0.0, 0.0));
        let continue_warmup = app.continue_room_warmup();
        if matches!(&app.scene, Scene::Splash(_)) {
            if let Some(r) = app.renderer.as_mut() {
                r.tick_splash_hub_boot();
                r.poll_pending_texture_uploads();
                let tileset = app.gfx.tileset_name.clone();
                r.ensure_active_showcase_decal_atlas(&tileset);
                r.poll_pending_texture_uploads();
            }
        }
        let active_tileset = app.gfx.tileset_name.clone();
        let loading_done = match &app.scene {
            // Splash stays up until showcase atlases and main_menu.glb are on the GPU.
            Scene::Splash(_) => app
                .renderer
                .as_ref()
                .is_some_and(|r| r.splash_hub_boot_ready(&active_tileset)),
            _ => app.renderer.as_ref().is_none_or(|r| !r.is_loading()),
        };
        let tutorial_eligible =
            app.progress.runs_completed == 0 && !app.progress.tutorial_completed;
        let hub_loading = app.hub_menu_loading(
            loading_done,
            tutorial_eligible,
            app.progress.plastic_unlocked(),
        );
        app.hub_loading = hub_loading;
        // Compute every scene pick once per frame. The same four results
        // are consumed below for `update` and again later by `draw` (via
        // `App::frame_picks`). Without this caching, each gameplay frame
        // pays for two full walks of the per-class matrix lists for
        // shop/gameplay objects in particular.
        let scene_key = match &app.scene {
            Scene::Splash(_) => Some(scene_keys::MAIN_MENU),
            _ => crate::scenes::active_scene_key(&app.scene),
        };
        let pending_scene_key = app.pending_destination_scene_key();
        let pending_transition_at_black = app.scene_replace_in_flight()
            && app.transition_alpha <= 0.0
            && app.scene_transition_unblocked();
        if pending_scene_key.is_some_and(|k| {
            matches!(
                k,
                scene_keys::GAMEPLAY | scene_keys::VICTORY | scene_keys::DEFEAT | "tutorial"
            )
        }) {
            app.audio.prefetch_gameplay_music();
        }
        let tile_stress_lab_open = app
            .overlay_stack
            .last()
            .is_some_and(|s| matches!(s, Scene::TileStressLab(_)));
        let stairway_tile_pick =
            matches!(&app.scene, Scene::Stairway(s) if s.wants_hand_tile_pick());
        app.frame_picks = if let Some(r) = app.renderer.as_mut() {
            r.poll_room_prefetch_gpu_uploads(
                scene_key,
                app.last_frame_dt * 1000.0,
                continue_warmup,
                pending_scene_key,
                pending_transition_at_black,
            );
            r.ensure_rooms_for_scene_key(scene_key);
            FramePicks {
                hand: if matches!(scene_key, Some("gameplay") | Some("tutorial"))
                    || tile_stress_lab_open
                    || stairway_tile_pick
                {
                    r.pick_hand_tile(cursor_pos.0, cursor_pos.1)
                } else {
                    None
                },
                shop: if matches!(
                    scene_key,
                    Some(scene_keys::SHOP) | Some("showcase") | Some(scene_keys::HALLWAY)
                ) {
                    r.pick_shop_object(cursor_pos.0, cursor_pos.1)
                } else {
                    None
                },
                gameplay: if matches!(scene_key, Some("gameplay") | Some("tutorial")) {
                    r.pick_gameplay_object(cursor_pos.0, cursor_pos.1)
                } else {
                    None
                },
            }
        } else {
            FramePicks::default()
        };
        let picked_shop_object = app.frame_picks.shop;
        let cascade_lab_open = app
            .overlay_stack
            .last()
            .is_some_and(|s| matches!(s, Scene::CascadeLab(_)));
        let picked_gameplay_object = if cascade_lab_open {
            match app.frame_picks.gameplay {
                Some(crate::render::wgpu_renderer::GameplayPick::CashInButton) => {
                    Some(crate::render::wgpu_renderer::GameplayPick::CashInButton)
                }
                _ => None,
            }
        } else {
            app.frame_picks.gameplay
        };
        let picked_hand_tile_for_update = if cascade_lab_open {
            None
        } else {
            app.frame_picks.hand
        };
        let mut scroll_lines = std::mem::take(&mut app.scroll_delta);
        let active_scene = app.overlay_stack.last().unwrap_or(&app.scene);
        // Stick vertical scroll is opt-in by scene. Yaku Journal, Chronicle,
        // and Credits use the right stick; defeat/victory run summaries accept
        // both sticks. Guide Tanuki tips maps right-stick X into scroll_lines
        // for horizontal panning. Other scenes keep sticks free for gameplay / orbit.
        let stick_scroll_axis = {
            let input = app.input.as_ref();
            let right = input.map(|i| i.right_stick_scroll_axis).unwrap_or(0.0);
            match active_scene {
                Scene::YakuJournal(_) | Scene::Credits(_) => right,
                Scene::Archive(scene) if scene.is_chronicle_tab() => right,
                Scene::Defeat(_) | Scene::Victory(_) => {
                    let left = input.map(|i| i.left_stick_scroll_axis).unwrap_or(0.0);
                    right + left
                }
                _ => 0.0,
            }
        };
        if stick_scroll_axis.abs() > 0.0 {
            const STICK_SCROLL_LINES_PER_SEC: f32 = 24.0;
            scroll_lines += stick_scroll_axis * app.last_frame_dt * STICK_SCROLL_LINES_PER_SEC;
        }
        if matches!(active_scene, Scene::Guide(g) if g.is_tanuki_tips_page()) {
            let right_x = app
                .input
                .as_ref()
                .map(|i| i.right_stick_scroll_axis_x)
                .unwrap_or(0.0);
            if right_x.abs() > 0.0 {
                const STICK_SCROLL_LINES_PER_SEC: f32 = 24.0;
                scroll_lines += right_x * app.last_frame_dt * STICK_SCROLL_LINES_PER_SEC;
            }
        }
        let p = app.active_profile.min(2);
        let settings_for_archive = persistence::load_settings();
        let archive_chronicle_last_seen = settings_for_archive.archive_last_seen_run_len[p];
        let room_gltf_height_for_update = app.resolved_scene_look().room_gltf_height_scale;
        locals.updated_overlay = !app.overlay_stack.is_empty();
        let shop_storeroom_orbit_drag_px = app
            .input
            .as_mut()
            .map(|i| i.take_shop_storeroom_mouse_orbit_px())
            .unwrap_or((0.0, 0.0));
        app.cpu_profiler
            .begin(crate::render::cpu_profiler::CpuStage::Update);
        if let Some(scenes::Scene::CascadeLab(lab)) = app.overlay_stack.last_mut() {
            app.cascade_tuning = lab.tuning.clone();
        }
        let scene_transitioning = app.scene_replace_in_flight();
        locals.update_result = if app.overlay_stack.is_empty() {
            app.scene.update(UpdateCtx {
                actions: &locals.actions,
                button_clicks: &locals.button_clicks,
                progress: &app.progress,
                active_profile: app.active_profile,
                run: &mut app.run,
                bus: &mut app.bus,
                anim: &mut app.anim,
                layout: &update_layout,
                focus_tile_index: focus,
                quit_requested: &mut locals.quit_requested,
                switch_profile: &mut locals.switch_profile,
                delete_profile: &mut locals.delete_profile,
                complete_onboarding: &mut locals.complete_onboarding,
                cursor_pos,
                mouse_left_down: app.mouse_left_down,
                loading_done,
                hub_loading,
                cascade_tuning: &app.cascade_tuning,
                picked_shop_object,
                picked_gameplay_object,
                input_mode: app
                    .input
                    .as_ref()
                    .map(|i| i.mode)
                    .unwrap_or(crate::ui::input::InputMode::Cursor),
                picked_hand_tile: picked_hand_tile_for_update,
                scroll_lines,
                tutorial_eligible: app.progress.runs_completed == 0
                    && !app.progress.tutorial_completed,
                multiple_materials: app.progress.plastic_unlocked(),
                resume_scene: app.resume_scene,
                transitioning: scene_transitioning,
                overlay_request: &mut locals.overlay_request,
                headless: false,
                effect_layers: app.effect_layers,
                item_inspect_orbit_stick: app
                    .input
                    .as_ref()
                    .map(|i| i.item_inspect_orbit_stick)
                    .unwrap_or((0.0, 0.0)),
                item_inspect_zoom_triggers: app
                    .input
                    .as_ref()
                    .map(|i| i.item_inspect_zoom_triggers)
                    .unwrap_or(0.0),
                shop_storeroom_orbit_drag_px,
                shop_storeroom_orbit_stick: app
                    .input
                    .as_ref()
                    .map(|i| i.shop_storeroom_orbit_stick)
                    .unwrap_or((0.0, 0.0)),
                rumble_lab_ops: &mut locals.rumble_lab_ops,
                suspended_shop: None,
                suspended_collection: None,
                room_gltf_height_scale: room_gltf_height_for_update,
                bump_archive_chronicle_seen: &mut locals.bump_archive_chronicle_seen,
                seed_archive_seen: &mut locals.seed_archive_seen,
                archive_chronicle_last_seen,
                main_menu_effects: app
                    .renderer
                    .as_ref()
                    .map(|r| r.main_menu_effects)
                    .unwrap_or_else(
                        crate::render::main_menu_effects_tuning::MainMenuEffectsTuning::load,
                    ),
                flame_tuning: app
                    .renderer
                    .as_ref()
                    .map(|r| r.flame_tuning)
                    .unwrap_or_else(crate::render::flame_tuning::FlameTuning::load),
                audio: Some(&mut app.audio),
            })
        } else {
            let showcase_shop_inspect = app.overlay_stack.last().is_some_and(|top| {
                matches!(
                    top,
                    Scene::Showcase(s)
                        if matches!(s.presenter, scenes::ShowcasePresenter::ShopInspect(_))
                )
            });
            let showcase_archive_inspect = app.overlay_stack.last().is_some_and(|top| {
                matches!(
                    top,
                    Scene::Showcase(s)
                        if matches!(s.presenter, scenes::ShowcasePresenter::ArchiveInspect(_))
                )
            });
            let (suspended_shop, suspended_collection) = match &mut app.scene {
                Scene::Shop(shop) if showcase_shop_inspect => {
                    shop.tick_suspended_animation_clock();
                    (Some(shop), None)
                }
                Scene::Archive(collection) if showcase_archive_inspect => (None, Some(collection)),
                _ => (None, None),
            };
            app.overlay_stack
                .last_mut()
                .expect("overlay stack non-empty")
                .update(UpdateCtx {
                    actions: &locals.actions,
                    button_clicks: &locals.button_clicks,
                    progress: &app.progress,
                    active_profile: app.active_profile,
                    run: &mut app.run,
                    bus: &mut app.bus,
                    anim: &mut app.anim,
                    layout: &update_layout,
                    focus_tile_index: focus,
                    quit_requested: &mut locals.quit_requested,
                    switch_profile: &mut locals.switch_profile,
                    delete_profile: &mut locals.delete_profile,
                    complete_onboarding: &mut locals.complete_onboarding,
                    cursor_pos,
                    mouse_left_down: app.mouse_left_down,
                    loading_done,
                    hub_loading,
                    cascade_tuning: &app.cascade_tuning,
                    picked_shop_object,
                    picked_gameplay_object,
                    input_mode: app
                        .input
                        .as_ref()
                        .map(|i| i.mode)
                        .unwrap_or(crate::ui::input::InputMode::Cursor),
                    picked_hand_tile: picked_hand_tile_for_update,
                    scroll_lines,
                    tutorial_eligible: app.progress.runs_completed == 0
                        && !app.progress.tutorial_completed,
                    multiple_materials: app.progress.plastic_unlocked(),
                    resume_scene: app.resume_scene,
                    transitioning: scene_transitioning,
                    overlay_request: &mut locals.overlay_request,
                    headless: false,
                    effect_layers: app.effect_layers,
                    item_inspect_orbit_stick: app
                        .input
                        .as_ref()
                        .map(|i| i.item_inspect_orbit_stick)
                        .unwrap_or((0.0, 0.0)),
                    item_inspect_zoom_triggers: app
                        .input
                        .as_ref()
                        .map(|i| i.item_inspect_zoom_triggers)
                        .unwrap_or(0.0),
                    shop_storeroom_orbit_drag_px,
                    shop_storeroom_orbit_stick: (0.0, 0.0),
                    rumble_lab_ops: &mut locals.rumble_lab_ops,
                    suspended_shop,
                    suspended_collection,
                    room_gltf_height_scale: room_gltf_height_for_update,
                    bump_archive_chronicle_seen: &mut locals.bump_archive_chronicle_seen,
                    seed_archive_seen: &mut locals.seed_archive_seen,
                    archive_chronicle_last_seen,
                    main_menu_effects: app
                        .renderer
                        .as_ref()
                        .map(|r| r.main_menu_effects)
                        .unwrap_or_else(
                            crate::render::main_menu_effects_tuning::MainMenuEffectsTuning::load,
                        ),
                    flame_tuning: app
                        .renderer
                        .as_ref()
                        .map(|r| r.flame_tuning)
                        .unwrap_or_else(crate::render::flame_tuning::FlameTuning::load),
                    audio: Some(&mut app.audio),
                })
        };
        if matches!(&app.scene, crate::scenes::Scene::MainMenu(_)) {
            app.effect_layers.rain = true;
            app.effect_layers.starfield = true;
        }
        app.cpu_profiler
            .end(crate::render::cpu_profiler::CpuStage::Update);
}
