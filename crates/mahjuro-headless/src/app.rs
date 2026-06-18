use std::time::Instant;

#[cfg(feature = "screenshot")]
use std::path::PathBuf;

use mahjuro::game::cascade::CascadeTuning;
use mahjuro::game::event_bus::EventBus;
use mahjuro::game::run::RunState;
use mahjuro::main_render_settings::RenderSettings;
use mahjuro::persistence;
use mahjuro::persistence::{GraphicsMode, ShadowQuality};
use mahjuro::render::animation::AnimationController;
use mahjuro::render::draw_cmd::{UiFrame, apply_modal_relic_staging};
use mahjuro::render::wgpu_renderer::WgpuRenderer;
use mahjuro::scenes::{DrawCtx, Scene, SceneBehavior, UpdateCtx};
use mahjuro::ui::input::{InputMode, UiAction};
use mahjuro::ui::layout::UiLayout;

const LOAD_WAIT_MAX_EXTRA: u32 = 600;
const LOAD_WAIT_SLEEP_MS: u64 = 16;

/// Scratch state reused each headless tick for [`UpdateCtx`] out-parameters.
struct HeadlessTickScratch {
    quit_requested: bool,
    switch_profile: Option<usize>,
    delete_profile: Option<usize>,
    complete_onboarding: bool,
    overlay_request: Option<mahjuro::scenes::OverlayRequest>,
    bump_archive_chronicle_seen: Option<u32>,
    seed_archive_seen: bool,
    rumble_lab_ops: Vec<mahjuro::ui::input::RumbleLabOp>,
}

impl Default for HeadlessTickScratch {
    fn default() -> Self {
        Self {
            quit_requested: false,
            switch_profile: None,
            delete_profile: None,
            complete_onboarding: false,
            overlay_request: None,
            bump_archive_chronicle_seen: None,
            seed_archive_seen: false,
            rumble_lab_ops: Vec::new(),
        }
    }
}

/// Minimal non-winit runner for screenshot / room-bake CLIs.
pub(crate) struct HeadlessApp {
    pub(crate) renderer: WgpuRenderer,
    pub(crate) layout_engine: UiLayout,
    pub(crate) scene: Scene,
    pub(crate) overlay_stack: Vec<Scene>,
    pub(crate) run: RunState,
    anim: AnimationController,
    pub(crate) progress: mahjuro::core::progression::PlayerProgress,
    active_profile: usize,
    gfx: RenderSettings,
    effect_layers: mahjuro::effect_layers::EffectLayers,
    scene_look: mahjuro::game::scene_look_tuning::SceneLookTuningSet,
    width: u32,
    height: u32,
    game_in_progress: bool,
    tick_count: u32,
    queued_actions: Vec<(u32, UiAction)>,
    modal_overlay: Option<mahjuro::ui::modal::ModalQueue>,
    pub(crate) shop_env_lighting: mahjuro::render::room_glb::RoomEnvLightingTune,
    pub(crate) input_mode_override: Option<InputMode>,
    pub(crate) hide_ui: bool,
}

impl HeadlessApp {
    pub(crate) fn with_run(
        scene: Scene,
        run: RunState,
        width: u32,
        height: u32,
        game_in_progress: bool,
        active_profile: usize,
        progress: mahjuro::core::progression::PlayerProgress,
    ) -> anyhow::Result<Self> {
        let settings = persistence::load_settings();
        let graphics_mode = GraphicsMode::from_env_override().unwrap_or(settings.graphics_mode);
        let renderer = WgpuRenderer::new(mahjuro::render::wgpu_renderer::TargetInit::Headless {
            width,
            height,
            hdr_enabled: false,
        })?;
        Ok(Self {
            renderer,
            layout_engine: UiLayout::new(),
            scene,
            overlay_stack: Vec::new(),
            run,
            anim: AnimationController::new(),
            progress,
            active_profile,
            gfx: RenderSettings {
                effects_quality: settings.effects_quality,
                tile_preset: settings.tile_preset,
                tile_material: settings.tile_material,
                tileset_name: settings.tileset_name.clone(),
                gamma: settings.gamma,
                graphics_mode,
                hdr_enabled: settings.hdr_enabled,
                vhs_enabled: false,
            },
            effect_layers: mahjuro::effect_layers::EffectLayers::BASELINE,
            scene_look: mahjuro::game::scene_look_tuning::SceneLookTuningSet::load(),
            width,
            height,
            game_in_progress,
            tick_count: 0,
            queued_actions: Vec::new(),
            modal_overlay: None,
            input_mode_override: None,
            shop_env_lighting: mahjuro::render::room_glb::RoomEnvLightingTune::SOURCE_DEFAULTS,
            hide_ui: false,
        })
    }

    #[cfg(feature = "screenshot")]
    pub(crate) fn unlock_all_yaku_for_screenshot(&mut self) {
        use mahjuro::core::yaku::YakuKind;
        for yk in YakuKind::all() {
            *self.progress.yaku_times_scored.entry(*yk).or_insert(0) += 1;
        }
        for yk in [
            YakuKind::Yakuhai,
            YakuKind::Toitoi,
            YakuKind::Honitsu,
            YakuKind::Shousangen,
        ] {
            self.run.yaku_levels.levels.insert(yk, 3);
        }
    }

    #[cfg(feature = "screenshot")]
    pub(crate) fn unlock_all_for_collection_screenshot(&mut self) {
        let signature_hand = mahjuro::scenes::archive_career::sample_signature_hand_tiles();
        self.progress
            .apply_screenshot_collection_demo(signature_hand);
    }

    #[cfg(feature = "screenshot")]
    pub(crate) fn force_relic_unlock_modal(&mut self) {
        use mahjuro::core::relic::all_relic_defs;
        use mahjuro::ui::modal::{Modal, ModalQueue, ModalTheme, UnlockPage};

        let defs = all_relic_defs();
        let chosen = defs
            .iter()
            .find(|d| d.name == "Kong Collector")
            .or_else(|| defs.first())
            .expect("at least one relic must be defined");
        let accent = mahjuro::render::theme::color::rarity(chosen.rarity.tier());
        let hero = UnlockPage {
            category: "New Relic".into(),
            name: chosen.name.into(),
            description: chosen.id.description(),
            relic_id: Some(chosen.id),
            accent_color: accent,
        };
        let mut pages = Vec::with_capacity(14);
        for i in 0..14 {
            pages.push(if i == 5 {
                hero.clone()
            } else {
                UnlockPage {
                    category: "Placeholder".into(),
                    name: "—".into(),
                    description: String::new(),
                    relic_id: Some(chosen.id),
                    accent_color: accent,
                }
            });
        }
        let mut modal = Modal::new("Relic Unlocked", "", ModalTheme::Success).with_pages(pages);
        modal.current_page = 5;
        let w = self.width as f32;
        let h = self.height as f32;
        let modal = modal.with_fireworks(w * 0.5, h * 0.92, w * 0.85, 24);
        let mut queue = ModalQueue::default();
        queue.push(modal);
        self.modal_overlay = Some(queue);
    }

    #[cfg(feature = "screenshot")]
    pub(crate) fn force_round_win_modal(&mut self) {
        use mahjuro::game::event_bus::RoundPayout;
        use mahjuro::ui::modal::{Modal, ModalQueue, ModalResultStyle, ModalTheme};

        let w = self.width as f32;
        let h = self.height as f32;
        let modal = Modal::new("Winner!", "", ModalTheme::Success)
            .with_title_scale(3.0)
            .with_result_style(ModalResultStyle::Winner)
            .with_payout_breakdown(
                self.run.round_score,
                u64::from(self.run.target_score),
                RoundPayout {
                    base_reward: 4,
                    unused_play_bonus: 5,
                    interest: 1,
                    green_luck_bonus: 0,
                    total: 10,
                },
            )
            .with_fireworks(w * 0.5, h * 0.8, w * 0.6, 5);
        let mut queue = ModalQueue::default();
        queue.push(modal);
        self.modal_overlay = Some(queue);
    }

    #[cfg(feature = "screenshot")]
    pub(crate) fn force_game_over_modal(&mut self) {
        use mahjuro::game::event_bus::GameOverReason;
        use mahjuro::ui::modal::{Modal, ModalQueue, ModalResultStyle, ModalTheme};
        use mahjuro::ui::score_format::format_score;

        let reason = GameOverReason::OutOfPlays;
        let points_short = (self.run.target_score as u64).saturating_sub(self.run.round_score);
        let points_short_line = if points_short == 1 {
            "1 point short".to_string()
        } else {
            format!("{} points short", format_score(points_short))
        };
        let modal = Modal::new(
            "Loser!",
            format!(
                "{}\nCause: {}\nScore: {} / {}",
                points_short_line,
                reason.death_cause(),
                format_score(self.run.round_score),
                format_score(self.run.target_score as u64)
            ),
            ModalTheme::Info,
        )
        .with_title_scale(3.0)
        .with_result_style(ModalResultStyle::Loser);
        let mut queue = ModalQueue::default();
        queue.push(modal);
        self.modal_overlay = Some(queue);
    }

    fn tick_warmup(&mut self, frames: u32) {
        for _ in 0..frames {
            self.tick();
        }
    }

    fn wait_for_assets(&mut self) -> u32 {
        let mut extra = 0u32;
        while self.renderer.is_loading() && extra < LOAD_WAIT_MAX_EXTRA {
            self.tick();
            std::thread::sleep(std::time::Duration::from_millis(LOAD_WAIT_SLEEP_MS));
            extra += 1;
        }
        extra
    }

    fn run_warmup(&mut self, warmup_frames: u32) {
        self.tick_warmup(warmup_frames);
        let extra = self.wait_for_assets();
        if extra > 0 {
            log::debug!("headless: waited {extra} extra ticks for asset loading");
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        self.anim.update(now);
        let layout = self
            .layout_engine
            .solve(self.width as f32, self.height as f32);
        let mut scratch = HeadlessTickScratch::default();
        let actions_this_tick: Vec<UiAction> = self
            .queued_actions
            .iter()
            .filter(|(t, _)| *t == self.tick_count)
            .map(|(_, a)| *a)
            .collect();
        self.tick_count += 1;
        let headless_cascade = CascadeTuning::default();

        let loading_done = if self.overlay_stack.is_empty() {
            match &self.scene {
                Scene::Splash(_) => true,
                _ => !self.renderer.is_loading(),
            }
        } else {
            !self.renderer.is_loading()
        };

        let mut bus = EventBus::default();
        let update_result = if self.overlay_stack.is_empty() {
            self.scene.update(UpdateCtx {
                actions: &actions_this_tick,
                button_clicks: &[],
                progress: &self.progress,
                active_profile: 0,
                run: &mut self.run,
                bus: &mut bus,
                anim: &mut self.anim,
                layout: &layout,
                focus_tile_index: 0,
                quit_requested: &mut scratch.quit_requested,
                switch_profile: &mut scratch.switch_profile,
                delete_profile: &mut scratch.delete_profile,
                complete_onboarding: &mut scratch.complete_onboarding,
                cursor_pos: (0.0, 0.0),
                mouse_left_down: false,
                loading_done,
                hub_loading: mahjuro::scenes::main_menu::HubMenuLoading::default(),
                cascade_tuning: &headless_cascade,
                picked_shop_object: None,
                picked_gameplay_object: None,
                input_mode: self.input_mode_override.unwrap_or(InputMode::Cursor),
                picked_hand_tile: None,
                scroll_lines: 0.0,
                tutorial_eligible: false,
                multiple_materials: self.progress.plastic_unlocked(),
                resume_scene: persistence::ResumeScene::default(),
                transitioning: false,
                overlay_request: &mut scratch.overlay_request,
                headless: true,
                effect_layers: self.effect_layers,
                item_inspect_orbit_stick: (0.0, 0.0),
                item_inspect_zoom_triggers: 0.0,
                shop_storeroom_orbit_drag_px: (0.0, 0.0),
                shop_storeroom_orbit_stick: (0.0, 0.0),
                rumble_lab_ops: &mut scratch.rumble_lab_ops,
                suspended_shop: None,
                suspended_collection: None,
                room_gltf_height_scale: mahjuro::render::room_glb::SHOP_ENV_HEIGHT_SCALE,
                bump_archive_chronicle_seen: &mut scratch.bump_archive_chronicle_seen,
                seed_archive_seen: &mut scratch.seed_archive_seen,
                archive_chronicle_last_seen: 0,
                main_menu_effects: self.renderer.main_menu_effects,
                flame_tuning: self.renderer.flame_tuning,
                audio: None,
            })
        } else {
            let showcase_shop_inspect = self.overlay_stack.last().is_some_and(|top| {
                matches!(
                    top,
                    Scene::Showcase(s)
                        if matches!(s.presenter, mahjuro::scenes::ShowcasePresenter::ShopInspect(_))
                )
            });
            let showcase_archive_inspect = self.overlay_stack.last().is_some_and(|top| {
                matches!(
                    top,
                    Scene::Showcase(s)
                        if matches!(s.presenter, mahjuro::scenes::ShowcasePresenter::ArchiveInspect(_))
                )
            });
            let (suspended_shop, suspended_collection) = match &mut self.scene {
                mahjuro::scenes::Scene::Shop(shop) if showcase_shop_inspect => {
                    shop.tick_suspended_animation_clock();
                    (Some(shop), None)
                }
                mahjuro::scenes::Scene::Archive(collection) if showcase_archive_inspect => {
                    (None, Some(collection))
                }
                _ => (None, None),
            };
            self.overlay_stack
                .last_mut()
                .expect("overlay stack non-empty")
                .update(UpdateCtx {
                    actions: &actions_this_tick,
                    button_clicks: &[],
                    progress: &self.progress,
                    active_profile: 0,
                    run: &mut self.run,
                    bus: &mut bus,
                    anim: &mut self.anim,
                    layout: &layout,
                    focus_tile_index: 0,
                    quit_requested: &mut scratch.quit_requested,
                    switch_profile: &mut scratch.switch_profile,
                    delete_profile: &mut scratch.delete_profile,
                    complete_onboarding: &mut scratch.complete_onboarding,
                    cursor_pos: (0.0, 0.0),
                    mouse_left_down: false,
                    loading_done,
                    hub_loading: mahjuro::scenes::main_menu::HubMenuLoading::default(),
                    cascade_tuning: &headless_cascade,
                    picked_shop_object: None,
                    picked_gameplay_object: None,
                    input_mode: self.input_mode_override.unwrap_or(InputMode::Cursor),
                    picked_hand_tile: None,
                    scroll_lines: 0.0,
                    tutorial_eligible: false,
                    multiple_materials: self.progress.plastic_unlocked(),
                    resume_scene: persistence::ResumeScene::default(),
                    transitioning: false,
                    overlay_request: &mut scratch.overlay_request,
                    headless: true,
                    effect_layers: self.effect_layers,
                    item_inspect_orbit_stick: (0.0, 0.0),
                    item_inspect_zoom_triggers: 0.0,
                    shop_storeroom_orbit_drag_px: (0.0, 0.0),
                    shop_storeroom_orbit_stick: (0.0, 0.0),
                    rumble_lab_ops: &mut scratch.rumble_lab_ops,
                    suspended_shop,
                    suspended_collection,
                    room_gltf_height_scale: mahjuro::render::room_glb::SHOP_ENV_HEIGHT_SCALE,
                    bump_archive_chronicle_seen: &mut scratch.bump_archive_chronicle_seen,
                    seed_archive_seen: &mut scratch.seed_archive_seen,
                    archive_chronicle_last_seen: 0,
                    main_menu_effects: self.renderer.main_menu_effects,
                    flame_tuning: self.renderer.flame_tuning,
                    audio: None,
                })
        };
        match scratch.overlay_request {
            Some(mahjuro::scenes::OverlayRequest::Push(s)) => self.overlay_stack.push(*s),
            Some(mahjuro::scenes::OverlayRequest::Pop) => {
                self.overlay_stack.pop();
            }
            None => {}
        }
        let _ = update_result;

        let showcase_orbit_top = self
            .overlay_stack
            .last()
            .is_some_and(|top| matches!(top, Scene::Showcase(s) if s.wants_orbit_input()));
        let suspended_shop = match (&self.scene, showcase_orbit_top) {
            (Scene::Shop(s), true) => Some(s),
            _ => None,
        };
        let suspended_collection = match (&self.scene, showcase_orbit_top) {
            (Scene::Archive(c), true) => Some(c),
            _ => None,
        };

        let settings = persistence::load_settings();
        let detected = mahjuro::ui::button_prompts::GamepadStyle::default();
        let prompt_style = settings.glyph_prompt.resolve(detected);
        let glyphs = mahjuro::ui::glyph_source::GlyphResolver::new(prompt_style, false, false);
        let mut env_per_scene = rustc_hash::FxHashMap::default();
        let mut env_frame_tunes = Vec::new();
        for &key in mahjuro::game::scene_look_tuning::GLTF_ENV_SCENE_KEYS {
            let look = self.scene_look.resolve(Some(key));
            let room = look.room;
            env_per_scene.insert(key, (room, look.room_gltf_height_scale));
            env_frame_tunes.push((
                key,
                mahjuro_render::tuning::scene_look::room_env_frame_from_scene_look(&look, room),
            ));
        }
        mahjuro_render::tuning::scene_look::push_doc_tile_env_frame_tunes(
            &mut env_frame_tunes,
            &self.scene_look,
            None,
            None,
            |room| room,
        );
        let mut focus_nav_snapshot = None;
        let ctx = DrawCtx::new(
            &layout,
            &self.anim,
            &self.run,
            &self.progress,
            self.active_profile,
            self.game_in_progress,
            self.renderer.projections(),
            None,
            None,
            mahjuro::scenes::DebugVisibility::default(),
            false,
            mahjuro::render::room_glb::SHOP_ENV_HEIGHT_SCALE,
            self.shop_env_lighting,
            &env_per_scene,
            self.effect_layers,
            (0.0, 0.0),
            self.input_mode_override.unwrap_or(InputMode::Cursor),
            glyphs,
            suspended_shop,
            suspended_collection,
            self.gfx.tile_preset,
            false,
            0,
            None,
            None,
            1.0,
            mahjuro::scenes::main_menu::HubMenuLoading::default(),
            self.renderer.main_menu_effects,
            self.renderer.flame_tuning,
            mahjuro::render::victory_moon_tuning::VictoryMoonDebug::default(),
            self.gfx.graphics_mode,
            false,
            false,
            &mut focus_nav_snapshot,
        );
        let mut frame: UiFrame = if let Some(top) = self.overlay_stack.last() {
            top.draw_frame(ctx)
        } else {
            self.scene.draw_frame(ctx)
        };

        if let Some(ref mut queue) = self.modal_overlay {
            queue.update();
            if let Some((
                modal_insts,
                modal_labels,
                modal_buttons,
                modal_relic_objects,
                modal_gradient_quads,
            )) = queue.draw(self.width as f32, self.height as f32)
            {
                let _ = modal_buttons;
                frame.overlay_quads(modal_insts);
                frame.texts(modal_labels);
                if !modal_gradient_quads.is_empty() {
                    frame.gradient_quads(modal_gradient_quads);
                }
                apply_modal_relic_staging(
                    &mut frame,
                    self.width as f32,
                    self.height as f32,
                    modal_relic_objects,
                );
            }
        }
        if self.hide_ui {
            frame.buttons.clear();
            hide_ui_draw_cmds(&mut frame);
        }

        let top = self.overlay_stack.last().unwrap_or(&self.scene);
        let parent = mahjuro::scenes::overlay_renderer_parent(&self.scene, &self.overlay_stack);
        let active_scene_key = mahjuro::scenes::active_scene_key_for_renderer(top, parent);
        self.renderer.set_active_scene(active_scene_key);
        let look = self.scene_look.resolve(active_scene_key);
        self.renderer.set_tonemap_tuning(&look.tonemap);
        self.renderer
            .set_frame_scene_env_tunes(active_scene_key, &env_frame_tunes);

        let active_material = frame
            .tile_material_override
            .unwrap_or(self.gfx.tile_material);
        let active_tileset_name = self.gfx.tileset_name.clone();
        let mut render_settings = self.effect_layers.wgpu_render_settings(
            &mahjuro::effect_layers::WgpuRenderSettingsParams {
                gfx: &self.gfx,
                tile_preset: self.gfx.tile_preset,
                tile_material: active_material,
                tileset_name: active_tileset_name,
                draw_settle_speed: 8.0,
                sort_settle_speed: 10.0,
            },
        );
        if let Some(shadow_quality) = std::env::var("MAHJURO_HEADLESS_SHADOW_QUALITY")
            .ok()
            .as_deref()
            .and_then(|raw| match raw {
                "off" => Some(ShadowQuality::Off),
                "low" => Some(ShadowQuality::Low),
                "high" => Some(ShadowQuality::High),
                _ => None,
            })
        {
            render_settings.shadow_quality = shadow_quality;
        }
        if let Err(e) = self.renderer.render(&frame, render_settings) {
            log::error!("headless render: {e:?}");
        }
    }

    #[cfg(feature = "screenshot")]
    pub(crate) fn queue_action_on_tick(&mut self, tick: u32, action: UiAction) {
        self.queued_actions.push((tick, action));
    }

    #[cfg(feature = "screenshot")]
    pub(crate) fn run_screenshot(
        mut self,
        output: PathBuf,
        warmup_frames: u32,
    ) -> anyhow::Result<()> {
        self.run_warmup(warmup_frames);
        if let Some(profile_frames) = std::env::var("MAHJURO_HEADLESS_GPU_PROFILE_FRAMES")
            .ok()
            .and_then(|raw| raw.parse::<u32>().ok())
            .filter(|frames| *frames > 0)
        {
            self.renderer.start_gpu_profile(profile_frames);
            let mut done = false;
            for _ in 0..(profile_frames + 32) {
                self.tick();
                if self.renderer.take_gpu_profile_just_completed() {
                    done = true;
                    break;
                }
            }
            if !done {
                log::warn!(
                    "headless screenshot: GPU profile did not complete within {} ticks",
                    profile_frames + 32
                );
            }
        }
        self.renderer.queue_screenshot(output.clone());
        self.tick();
        if self.renderer.screenshot_pending() {
            anyhow::bail!(
                "headless screenshot: final tick did not consume the queued path ({})",
                output.display()
            );
        }
        Ok(())
    }
}

fn hide_ui_draw_cmds(frame: &mut UiFrame) {
    frame.cmds.retain(|cmd| {
        !matches!(
            cmd,
            mahjuro::render::draw_cmd::DrawCmd::Quad(_)
                | mahjuro::render::draw_cmd::DrawCmd::OverlayQuad(_)
                | mahjuro::render::draw_cmd::DrawCmd::OverlaySquircleQuad(_)
                | mahjuro::render::draw_cmd::DrawCmd::GradientQuad(_)
                | mahjuro::render::draw_cmd::DrawCmd::ArcRingQuad(_)
                | mahjuro::render::draw_cmd::DrawCmd::SquircleQuad(_)
                | mahjuro::render::draw_cmd::DrawCmd::Text(_)
                | mahjuro::render::draw_cmd::DrawCmd::ImageQuad(_)
                | mahjuro::render::draw_cmd::DrawCmd::TileFaceQuad(_)
        )
    });
}
