//! Main-menu hub: [`main_menu.glb`](../../assets/3d/main_menu.glb) when embedded, else black.

use std::cell::RefCell;
use std::time::Instant;

use crate::sfx_id::SfxId;
use crate::core::progression::PlayerProgress;
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::game::run::RunState;
use crate::persistence::{self, ResumeScene, TileMaterial};
use crate::render::draw_cmd::{CameraParams, ImageQuad, ImageQuadSource, ScenePunctualLight, UiFrame};
use crate::render::main_menu_glb;
use crate::render::rain_field::{RainField, main_menu_rain_spawn_volume};
use crate::render::room_glb::{self, RoomEnvLightingTune};
use crate::render::scene_light_sample::{
    PunctualOccluderAabb, RainVolumetricLit, SceneLightSampleCtx,
};
use crate::render::theme::{color, metrics, typography};
use crate::render::wgpu_renderer::{GpuInstance, PointLight, SpotLight, TextAlign, TextLabel};
use crate::ui::focus_nav::{self, FocusDir};
use crate::ui::input::UiAction;

use super::collection::CollectionScene;
use super::gameplay::GameplayScene;
use super::lamp_moths::{self, BUG_COUNT};
use super::options::OptionsScene;
use super::shop::ShopScene;
use super::start_game_modal::TileSelectScene;
use super::{BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneTransition, UpdateCtx};

const MAIN_MENU_LOGO_ASSET: &str = "textures/main_menu_logo.png";

/// CPU world rain streaks + splashes (see [`RainField`]).
fn push_main_menu_rain(
    frame: &mut UiFrame,
    rain_field: &RainField,
    ctx: &DrawCtx<'_>,
    w: f32,
    h: f32,
    env_scale: f32,
) {
    if !ctx.effect_layers.rain {
        return;
    }
    let cam = frame
        .camera_override
        .unwrap_or_else(|| main_menu_glb::main_menu_camera_base(w, h, env_scale));
    let tune = ctx.room_env_for("main_menu_exterior").0;
    let bundle = build_main_menu_rain_lighting(w, h, env_scale, &tune);
    let lighting = main_menu_rain_light_sample_ctx(w, h, env_scale, &cam, &tune, &bundle);
    let volume = main_menu_rain_spawn_volume(env_scale, h, &ctx.rain_tuning);
    let (d_min, d_max) = volume.frustum_depth_range(&cam);
    let base_rgb = [
        ctx.rain_tuning.field.drop_color[0],
        ctx.rain_tuning.field.drop_color[1],
        ctx.rain_tuning.field.drop_color[2],
    ];
    let lit = Some(RainVolumetricLit::build(
        &cam, base_rgb, d_min, d_max, &lighting,
    ));
    rain_field.push_quads(
        frame,
        &cam,
        w,
        h,
        ctx.rain_tuning.field.streak_len_px,
        ctx.rain_tuning.field.drop_color,
        lit,
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HubFocus {
    Continue,
    NewGame,
    Archive,
    Options,
    Quit,
}

fn menu_items(in_progress: bool) -> Vec<HubFocus> {
    let mut items = Vec::with_capacity(8);
    if in_progress {
        items.push(HubFocus::Continue);
    }
    items.push(HubFocus::NewGame);
    items.push(HubFocus::Archive);
    items.push(HubFocus::Options);
    items.push(HubFocus::Quit);
    items
}

fn label_for(item: HubFocus, in_progress: bool, archive_has_new: bool) -> String {
    match item {
        HubFocus::Continue => "Continue".into(),
        HubFocus::NewGame => {
            if in_progress {
                "New Game".into()
            } else {
                "Play".into()
            }
        }
        HubFocus::Archive => {
            if archive_has_new {
                "Archive (new)".into()
            } else {
                "Archive".into()
            }
        }
        HubFocus::Options => "Options".into(),
        HubFocus::Quit => "Quit".into(),
    }
}

fn default_focus(in_progress: bool) -> HubFocus {
    if in_progress {
        HubFocus::Continue
    } else {
        HubFocus::NewGame
    }
}

fn main_menu_scene_punctual(
    w: f32,
    h: f32,
    env_scale: f32,
    tune: &RoomEnvLightingTune,
) -> (Vec<ScenePunctualLight>, Vec<SpotLight>) {
    let room_glb = main_menu_glb::main_menu_glb_has_embedded_lights();
    let spots = if room_glb {
        main_menu_glb::main_menu_embedded_spot_lights_runtime(w, h, env_scale, tune)
    } else {
        Vec::new()
    };
    let mut punctual: Vec<ScenePunctualLight> = if room_glb {
        main_menu_glb::main_menu_embedded_point_lights_runtime(w, h, env_scale, tune)
            .into_iter()
            .map(ScenePunctualLight::InverseSquare)
            .collect()
    } else {
        Vec::new()
    };
    let fill: Vec<PointLight> = if room_glb {
        Vec::new()
    } else {
        vec![
            PointLight {
                pos: [w * 0.42, h * 0.38, h * 0.55],
                radius: h * 1.4,
                color: [1.0, 0.92, 0.78],
                intensity: 1.35,
            },
            PointLight {
                pos: [w * 0.62, h * 0.72, h * 0.28],
                radius: h * 1.8,
                color: [0.45, 0.58, 0.92],
                intensity: 0.42,
            },
        ]
    };
    punctual.extend(fill.into_iter().map(ScenePunctualLight::Smooth));
    (punctual, spots)
}

struct MainMenuRainLighting {
    punctual: Vec<ScenePunctualLight>,
    spots: Vec<SpotLight>,
    occluders: Vec<PunctualOccluderAabb>,
}

fn build_main_menu_rain_lighting(
    w: f32,
    h: f32,
    env_scale: f32,
    tune: &RoomEnvLightingTune,
) -> MainMenuRainLighting {
    let (punctual, spots) = main_menu_scene_punctual(w, h, env_scale, tune);
    let occluders = main_menu_glb::main_menu_rain_env_model_matrix(h, env_scale)
        .map(|model| {
            PunctualOccluderAabb::from_room_collision_meshes(
                model,
                &main_menu_glb::main_menu_collision_meshes(),
            )
        })
        .unwrap_or_default();
    MainMenuRainLighting {
        punctual,
        spots,
        occluders,
    }
}

fn main_menu_rain_light_sample_ctx<'a>(
    w: f32,
    h: f32,
    env_scale: f32,
    cam: &'a CameraParams,
    tune: &RoomEnvLightingTune,
    bundle: &'a MainMenuRainLighting,
) -> SceneLightSampleCtx<'a> {
    let room_glb = main_menu_glb::main_menu_glb_has_embedded_lights();
    let world_scale = room_glb::room_env_world_scale(h, env_scale);
    let inv_doc_scale = if room_glb {
        1.0 / world_scale.max(1e-6)
    } else {
        0.0
    };
    let ambient = tune
        .ambient_scale
        .max(main_menu_glb::MAIN_MENU_ENV_AMBIENT_SCALE_MIN);
    let linear_exposure = if room_glb {
        tune.linear_exposure
            * room_glb::ROOM_GLB_LINEAR_EXPOSURE_BASE
            * main_menu_glb::MAIN_MENU_ENV_LINEAR_EXPOSURE_MUL
    } else {
        1.0
    };
    SceneLightSampleCtx {
        screen_w: w,
        screen_h: h,
        cam: Some(cam),
        ambient_scale: ambient,
        inv_doc_scale,
        linear_exposure,
        punctual: &bundle.punctual,
        spots: &bundle.spots,
        occluders: &bundle.occluders,
    }
}

fn push_main_menu_room_frame(
    frame: &mut UiFrame,
    w: f32,
    h: f32,
    env_scale: f32,
    tune: &RoomEnvLightingTune,
) {
    if !main_menu_glb::main_menu_room_draw_ready() {
        return;
    }
    frame.background(BackgroundId::Black);
    frame.main_menu_environment();
    frame.camera_override = Some(main_menu_glb::main_menu_camera_base(w, h, env_scale));
    let room_glb = main_menu_glb::main_menu_glb_has_embedded_lights();
    frame.scene_lighting.embedded_gltf_punctual = room_glb;
    frame.scene_lighting.room_glb_brdf = room_glb;
    let (punctual, spots) = main_menu_scene_punctual(w, h, env_scale, tune);
    frame.scene_lighting.spot_lights = spots;
    frame.scene_lighting.punctual = punctual;
}

pub(crate) fn scene_from_resume(
    resume_scene: ResumeScene,
    run: &mut RunState,
    progress: &PlayerProgress,
) -> Scene {
    match resume_scene {
        ResumeScene::Gameplay => Scene::Gameplay(Box::new(GameplayScene::new())),
        ResumeScene::Shop => {
            if GameEngine::resumes_to_tutorial_shop(run) {
                Scene::Shop(ShopScene::new_tutorial(run))
            } else {
                Scene::Shop(ShopScene::new(run, progress))
            }
        }
        ResumeScene::PickChamber => {
            Scene::PickChamber(super::pick_chamber::PickChamberScene::new())
        }
    }
}

pub struct MainMenuExteriorScene {
    focus: Option<HubFocus>,
    last_focus_rects: RefCell<Vec<(HubFocus, [f32; 4])>>,
    cursor_pos: (f32, f32),
    last_frame: Instant,
    age_secs: f32,
    bug_phases: [f32; BUG_COUNT],
    rain_field: RainField,
}

impl MainMenuExteriorScene {
    pub fn new() -> Self {
        Self {
            focus: None,
            last_focus_rects: RefCell::new(Vec::new()),
            cursor_pos: (0.0, 0.0),
            last_frame: Instant::now(),
            age_secs: 0.0,
            bug_phases: lamp_moths::initial_bug_phases(),
            rain_field: RainField::new(),
        }
    }

    /// Logo (A) on the left; menu (B) below A, left-aligned with A.
    /// Menu rows get horizontal inset inside the logo column; the logo image does not.
    /// The A+B block is vertically centered on screen and scales with window size.
    fn hub_layout(w: f32, h: f32, items: &[HubFocus]) -> HubLayout {
        let scale = metrics::scene_scale(w, h);
        let margin_x = (w * 0.04).max(16.0 * scale);
        let margin_y = (h * 0.04).max(12.0 * scale);
        let row_h = (h * 0.046).max(28.0 * scale);
        let row_gap = (h * 0.014).max(6.0 * scale);
        let n = items.len() as f32;
        let menu_stack_h = n * row_h + (n - 1.0).max(0.0) * row_gap;
        let max_block_h = (h - 2.0 * margin_y).max(1.0);
        let logo_by_h = h * 0.5;
        let logo_by_w = (w - 2.0 * margin_x) * 0.42;
        let logo_room = (max_block_h - menu_stack_h).max(row_h * 2.0);
        let logo_size = logo_by_h.min(logo_by_w).min(logo_room).max(96.0 * scale);
        let block_h = logo_size + menu_stack_h;
        let logo_y = ((h - block_h) * 0.5).max(margin_y);
        let logo_x = margin_x;
        let menu_margin_h = (logo_size * 0.04).max(8.0 * scale);
        let menu_x = logo_x + menu_margin_h;
        let menu_w = (logo_size - menu_margin_h * 2.0).max(row_h);
        let mut menu_y = logo_y + logo_size;
        let menu_rects = items
            .iter()
            .copied()
            .map(|item| {
                let r = [menu_x, menu_y, menu_w, row_h];
                menu_y += row_h + row_gap;
                (item, r)
            })
            .collect();
        HubLayout {
            logo_rect: [logo_x, logo_y, logo_size, logo_size],
            menu_rects,
        }
    }
}

struct HubLayout {
    logo_rect: [f32; 4],
    menu_rects: Vec<(HubFocus, [f32; 4])>,
}

impl SceneBehavior for MainMenuExteriorScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.cursor_pos = ctx.cursor_pos;
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.age_secs += dt;
        lamp_moths::advance_bug_phases(&mut self.bug_phases, dt);
        if ctx.effect_layers.rain {
            let w = ctx.layout.window_w;
            let h = ctx.layout.window_h;
            let env_scale = main_menu_glb::main_menu_env_height_scale(ctx.room_gltf_height_scale);
            let cam = main_menu_glb::main_menu_camera_base(w, h, env_scale);
            let tune = RoomEnvLightingTune::default();
            let bundle = build_main_menu_rain_lighting(w, h, env_scale, &tune);
            let lighting = main_menu_rain_light_sample_ctx(w, h, env_scale, &cam, &tune, &bundle);
            let rain_mesh = main_menu_glb::main_menu_rain_collision_mesh();
            self.rain_field.update(
                dt,
                &ctx.rain_tuning,
                &cam,
                w,
                h,
                env_scale,
                rain_mesh.as_ref(),
                Some(&lighting),
            );
        }
        let in_progress = GameEngine::run_in_progress(ctx.run);
        let items = menu_items(in_progress);

        if self.focus.is_none() || !items.contains(&self.focus.unwrap()) {
            self.focus = Some(default_focus(in_progress));
        }
        let prev_focus = self.focus;

        let focus_rects = self.last_focus_rects.borrow().clone();

        let pointer_pick =
            if ctx.input_mode == crate::ui::input::InputMode::Cursor && !focus_rects.is_empty() {
                focus_nav::focus_target_at_cursor(&focus_rects, ctx.cursor_pos.0, ctx.cursor_pos.1)
            } else {
                None
            };

        if ctx.input_mode == crate::ui::input::InputMode::Cursor
            && let Some(m) = pointer_pick
        {
            self.focus = Some(m);
        }

        let mut activated = false;
        for action in ctx.actions {
            match action {
                UiAction::FocusUp | UiAction::FocusPrev => {
                    if let Some(cur) = self.focus
                        && let Some(&(_, rect)) = focus_rects.iter().find(|(t, _)| *t == cur)
                        && let Some(next) =
                            focus_nav::pick_neighbor(rect, FocusDir::Up, &focus_rects)
                    {
                        self.focus = Some(next);
                    }
                }
                UiAction::FocusDown | UiAction::FocusNext => {
                    if let Some(cur) = self.focus
                        && let Some(&(_, rect)) = focus_rects.iter().find(|(t, _)| *t == cur)
                        && let Some(next) =
                            focus_nav::pick_neighbor(rect, FocusDir::Down, &focus_rects)
                    {
                        self.focus = Some(next);
                    }
                }
                UiAction::Confirm => activated = true,
                UiAction::Cancel | UiAction::Pause => {
                    ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                    self.focus = Some(HubFocus::Quit);
                }
                _ => {}
            }
        }

        if !ctx.button_clicks.is_empty()
            && let Some(m) = pointer_pick
        {
            self.focus = Some(m);
            activated = true;
        }

        if self.focus != prev_focus {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        if activated {
            // Avoid entering 3D-heavy scenes until async relic/backdrop uploads
            // finish (`WgpuRenderer::is_loading`). Tile pick flows first — those
            // gates live in `TileSelectScene::update`.
            let needs_gpu_ready = match self.focus {
                Some(HubFocus::Continue) => true,
                Some(HubFocus::NewGame) => !ctx.tutorial_eligible && !ctx.multiple_materials,
                _ => false,
            };
            if needs_gpu_ready && !ctx.loading_done {
                ctx.bus.push(GameEvent::UiSound(SfxId::InvalidAction));
                return None;
            }
            ctx.bus.push(GameEvent::UiSound(SfxId::UiConfirm));
            match self.focus {
                Some(HubFocus::Continue) => {
                    return Some(scene_from_resume(ctx.resume_scene, ctx.run, ctx.progress));
                }
                Some(HubFocus::NewGame) => {
                    if ctx.tutorial_eligible {
                        return Some(Scene::TileSelect(TileSelectScene::new_tutorial()));
                    }
                    if ctx.multiple_materials {
                        return Some(Scene::TileSelect(TileSelectScene::new()));
                    }
                    let settings = persistence::load_settings();
                    GameEngine::start_run_with_material(
                        ctx.run,
                        TileMaterial::default(),
                        ctx.progress,
                        &settings,
                    );
                    return Some(Scene::Shop(ShopScene::new(ctx.run, ctx.progress)));
                }
                Some(HubFocus::Archive) => {
                    return Some(Scene::Collection(CollectionScene::new()));
                }
                Some(HubFocus::Options) => {
                    return Some(Scene::Options(OptionsScene::new()));
                }
                Some(HubFocus::Quit) => {
                    *ctx.quit_requested = true;
                }
                None => {}
            }
        }

        None
    }

    fn draw_frame(&self, ctx: DrawCtx<'_>) -> UiFrame {
        let layout = ctx.layout;
        let w = layout.window_w;
        let h = layout.window_h;
        let scale = metrics::scene_scale(w, h);

        // App modals (level-up / relic unlock) append 3D hero staging after the
        // scene, but all `Text` cmds share one post-tonemap overlay pass. Hub
        // labels would composite with modal copy and read on top of the relic.
        // Treat this like shop/collection inspect: backdrop only until the modal
        // dismisses (see `SplashScene` and `DrawCtx::modal_active`).
        if ctx.modal_active {
            let mut frame = UiFrame::new();
            frame.background(BackgroundId::Black);
            let env_scale = main_menu_glb::main_menu_env_height_scale(ctx.room_gltf_height_scale);
            if main_menu_glb::main_menu_room_draw_ready() {
                push_main_menu_room_frame(&mut frame, w, h, env_scale, &ctx.room_env_for("main_menu_exterior").0);
                if let Some(light_door) =
                    main_menu_glb::main_menu_light_door_object3d_anchor(w, h, env_scale)
                {
                    lamp_moths::push_moths_around_lamp(
                        &mut frame,
                        w,
                        h,
                        light_door,
                        h * 0.16,
                        h * 0.20,
                        self.age_secs,
                        &self.bug_phases,
                    );
                }
            }
            push_main_menu_rain(&mut frame, &self.rain_field, &ctx, w, h, env_scale);
            if ctx.effect_layers.starfield {
                frame.starfield();
            }
            frame.cursor_pos = Some(self.cursor_pos);
            frame.window_title = format!(
                "Mahjuro — {}",
                if cfg!(debug_assertions) {
                    "vNEXT"
                } else {
                    env!("CARGO_PKG_VERSION")
                }
            );
            return frame;
        }

        let in_progress = ctx.game_in_progress;
        let items = menu_items(in_progress);
        let layout = Self::hub_layout(w, h, &items);
        let focus_rects = layout.menu_rects.clone();
        *self.last_focus_rects.borrow_mut() = focus_rects.clone();

        let mut quads: Vec<GpuInstance> = Vec::new();
        if let Some(focus) = self.focus
            && let Some(&(_, rect)) = focus_rects.iter().find(|(t, _)| *t == focus)
        {
            focus_nav::push_focus_ring(rect, scale, w, h, &mut quads);
        }

        let menu_font = typography::size(typography::H36, h);
        let label_color = color::PARCHMENT;
        let mut text_labels = Vec::with_capacity(focus_rects.len());

        for &(item, rect) in &focus_rects {
            text_labels.push(TextLabel {
                rect,
                text: label_for(item, in_progress, ctx.archive_has_new),
                font_px: Some(menu_font),
                color: label_color,
                align: TextAlign::Left,
                ..Default::default()
            });
        }

        let buttons = vec![ButtonDef::scene((0.0, 0.0, w, h), 0)];

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        let env_scale = main_menu_glb::main_menu_env_height_scale(ctx.room_gltf_height_scale);
        if main_menu_glb::main_menu_room_draw_ready() {
            push_main_menu_room_frame(&mut frame, w, h, env_scale, &ctx.room_env_for("main_menu_exterior").0);
            if let Some(light_door) =
                main_menu_glb::main_menu_light_door_object3d_anchor(w, h, env_scale)
            {
                lamp_moths::push_moths_around_lamp(
                    &mut frame,
                    w,
                    h,
                    light_door,
                    h * 0.16,
                    h * 0.20,
                    self.age_secs,
                    &self.bug_phases,
                );
            }
        }
        push_main_menu_rain(&mut frame, &self.rain_field, &ctx, w, h, env_scale);
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        frame.quads(quads);
        frame.image_quads([ImageQuad {
            inst: GpuInstance {
                rect: layout.logo_rect,
                color: [1.0, 1.0, 1.0, 1.0],
                user: 0,
            },
            source: ImageQuadSource::Asset {
                path: MAIN_MENU_LOGO_ASSET,
            },
        }]);
        frame.texts(text_labels);
        frame.buttons = buttons;
        frame.cursor_pos = Some(self.cursor_pos);
        frame.window_title = format!(
            "Mahjuro — {}",
            if cfg!(debug_assertions) {
                "vNEXT"
            } else {
                env!("CARGO_PKG_VERSION")
            }
        );

        frame
    }
}
