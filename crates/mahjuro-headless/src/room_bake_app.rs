use std::time::Instant;

use mahjuro::game::cascade::CascadeTuning;
use mahjuro::game::event_bus::EventBus;
use mahjuro::game::run::RunState;
use mahjuro::main_render_settings::RenderSettings;
use mahjuro::persistence;
use mahjuro::render::animation::AnimationController;
use mahjuro::render::draw_cmd::UiFrame;
use mahjuro::render::wgpu_renderer::WgpuRenderer;
use mahjuro::scenes::{DrawCtx, Scene, SceneBehavior, UpdateCtx};
use mahjuro::ui::input::InputMode;
use mahjuro::ui::layout::UiLayout;

const LOAD_WAIT_MAX_EXTRA: u32 = 600;
const LOAD_WAIT_SLEEP_MS: u64 = 16;

/// Scratch out-parameters for a room-bake tick (no overlays / profile / modals).
struct BakeTickScratch {
    quit_requested: bool,
    switch_profile: Option<usize>,
    delete_profile: Option<usize>,
    complete_onboarding: bool,
    overlay_request: Option<mahjuro::scenes::OverlayRequest>,
    bump_archive_chronicle_seen: Option<u32>,
    seed_archive_seen: bool,
    rumble_lab_ops: Vec<mahjuro::ui::input::RumbleLabOp>,
}

impl Default for BakeTickScratch {
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

/// Headless runner used only by `mahjuro-bake` — no screenshot queue, modals, or overlay stack.
pub(crate) struct RoomBakeApp {
    renderer: WgpuRenderer,
    layout_engine: UiLayout,
    scene: Scene,
    run: RunState,
    anim: AnimationController,
    progress: mahjuro::core::progression::PlayerProgress,
    active_profile: usize,
    gfx: RenderSettings,
    effect_layers: mahjuro::effect_layers::EffectLayers,
    scene_look: mahjuro::game::scene_look_tuning::SceneLookTuningSet,
    width: u32,
    height: u32,
    game_in_progress: bool,
    shop_env_lighting: mahjuro::render::room_glb::RoomEnvLightingTune,
}

impl RoomBakeApp {
    pub(crate) fn new(
        scene: Scene,
        run: RunState,
        width: u32,
        height: u32,
        game_in_progress: bool,
        active_profile: usize,
        progress: mahjuro::core::progression::PlayerProgress,
    ) -> anyhow::Result<Self> {
        let settings = persistence::load_settings();
        let renderer = WgpuRenderer::new(mahjuro::render::wgpu_renderer::TargetInit::Headless {
            width,
            height,
            hdr_enabled: false,
        })?;
        Ok(Self {
            renderer,
            layout_engine: UiLayout::new(),
            scene,
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
                graphics_mode: settings.graphics_mode,
                hdr_enabled: settings.hdr_enabled,
                vhs_enabled: false,
            },
            effect_layers: mahjuro::effect_layers::EffectLayers::BASELINE,
            scene_look: mahjuro::game::scene_look_tuning::SceneLookTuningSet::load(),
            width,
            height,
            game_in_progress,
            shop_env_lighting: mahjuro::render::room_glb::RoomEnvLightingTune::SOURCE_DEFAULTS,
        })
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
            log::debug!("room bake: waited {extra} extra ticks for asset loading");
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        self.anim.update(now);
        let layout = self
            .layout_engine
            .solve(self.width as f32, self.height as f32);
        let mut scratch = BakeTickScratch::default();
        let headless_cascade = CascadeTuning::default();
        let loading_done = !self.renderer.is_loading();
        let mut bus = EventBus::default();

        let _ = self.scene.update(UpdateCtx {
            actions: &[],
            button_clicks: &[],
            progress: &self.progress,
            active_profile: self.active_profile,
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
            cascade_tuning: &headless_cascade,
            picked_shop_object: None,
            picked_gameplay_object: None,
            input_mode: InputMode::Cursor,
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
            rumble_lab_ops: &mut scratch.rumble_lab_ops,
            suspended_shop: None,
            suspended_collection: None,
            room_gltf_height_scale: mahjuro::render::room_glb::SHOP_ENV_HEIGHT_SCALE,
            bump_archive_chronicle_seen: &mut scratch.bump_archive_chronicle_seen,
            seed_archive_seen: &mut scratch.seed_archive_seen,
            archive_chronicle_last_seen: 0,
            main_menu_effects: self.renderer.main_menu_effects,
            flame_tuning: self.renderer.flame_tuning,
        });

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
                mahjuro_render::tuning::scene_look::room_env_frame_from_scene_look(
                    &look, room,
                ),
            ));
        }

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
            InputMode::Cursor,
            glyphs,
            None,
            None,
            self.gfx.tile_preset,
            false,
            0,
            None,
            None,
            1.0,
            self.renderer.main_menu_effects,
            self.renderer.flame_tuning,
        );
        let frame: UiFrame = self.scene.draw_frame(ctx);

        let active_scene_key = mahjuro::scenes::active_scene_key(&self.scene);
        self.renderer.set_active_scene(active_scene_key);
        let look = self.scene_look.resolve(active_scene_key);
        self.renderer.set_tonemap_tuning(&look.tonemap);
        self.renderer.set_frame_scene_env_tunes(active_scene_key, &env_frame_tunes);

        let active_material = frame
            .tile_material_override
            .unwrap_or(self.gfx.tile_material);
        let render_settings = self.effect_layers.wgpu_render_settings(
            &mahjuro::effect_layers::WgpuRenderSettingsParams {
                gfx: &self.gfx,
                tile_preset: self.gfx.tile_preset,
                tile_material: active_material,
                tileset_name: self.gfx.tileset_name.clone(),
                draw_settle_speed: 8.0,
                sort_settle_speed: 10.0,
            },
        );
        if let Err(e) = self.renderer.render(&frame, render_settings) {
            log::error!("room bake render: {e:?}");
        }
    }

    pub(crate) fn bake_room_gi(
        &mut self,
        room: mahjuro::render::room_gi_bake::RoomGiRoom,
        warmup_frames: u32,
    ) -> anyhow::Result<mahjuro::render::room_gi_bake::RoomGiBake> {
        self.effect_layers.procedural_surface_quality = true;
        self.gfx.effects_quality = mahjuro::persistence::EffectsQuality::High;
        self.renderer.request_room_gi_capture(room);
        self.run_warmup(warmup_frames);
        self.tick();
        self.renderer.take_room_gi_capture().ok_or_else(|| {
            anyhow::anyhow!(
                "room GI bake: GPU readback missing (was probe compute dispatched?) — rebake with \
                 `cargo run -p mahjuro-headless --bin mahjuro-bake --features bake -- --kinds gi`"
            )
        })
    }

    pub(crate) fn bake_room_shadow(
        &mut self,
        room: mahjuro::render::room_gi_bake::RoomGiRoom,
        warmup_frames: u32,
    ) -> anyhow::Result<mahjuro::render::room_shadow_bake::RoomShadowBake> {
        self.renderer.request_room_shadow_capture(room);
        self.run_warmup(warmup_frames);
        self.tick();
        self.renderer.take_room_shadow_capture().ok_or_else(|| {
            anyhow::anyhow!("room shadow bake: GPU readback missing")
        }).and_then(|bake| {
            mahjuro::render::room_shadow_bake::validate_room_shadow_bake_effective(&bake, room)?;
            Ok(bake)
        })
    }
}
