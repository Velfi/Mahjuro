//! Main-menu hub: [`main_menu.glb`](../../assets/3d/main_menu.glb) when embedded, else black.

use std::time::Instant;

use crate::core::moon_quips;
use crate::core::progression::PlayerProgress;
use crate::game::engine::GameEngine;
use crate::game::event_bus::GameEvent;
use crate::game::run::RunState;
use crate::persistence::ResumeScene;
use crate::render::draw_cmd::{
    CameraParams, ImageQuad, ImageQuadSource, ScenePunctualLight, UiFrame,
};
use crate::render::main_menu_glb;
use crate::render::rain_field::{RainField, main_menu_rain_spawn_volume};
use crate::render::room_glb::{self, RoomEnvLightingTune};
use crate::render::scene_keys;
use crate::render::scene_light_sample::{
    PunctualOccluderAabb, RainVolumetricLit, SceneLightSampleCtx,
};
use crate::render::theme::{color, metrics, typography};
use crate::render::vocabulary_colors::GlossaryMode;
use crate::render::wgpu_renderer::{
    GpuInstance, MAIN_MENU_PICK_MOON, PointLight, TextAlign, TextLabel,
};
use crate::sfx_id::SfxId;
use crate::trailer_mode::MainMenuTrailer;
use crate::ui::controller_hints::{HintStyle, menu_footer_row, push_screen_footer_hint};
use crate::ui::focus_nav;
use crate::ui::input::UiAction;
use crate::ui::styled_text;
use crate::ui::tooltip;
use crate::ui::widget_tree::{FlatItem, FocusId, TreeInput, TreeState};

use super::lamp_moths::{self, BUG_COUNT};
use super::shop::ShopScene;
use super::{
    BackgroundId, ButtonDef, DrawCtx, Scene, SceneBehavior, SceneIntent, SceneTransition, UpdateCtx,
};

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
    let tune = ctx.room_env_for(scene_keys::MAIN_MENU).0;
    let bundle = build_main_menu_rain_lighting(w, h, env_scale, &tune);
    let lighting = main_menu_rain_light_sample_ctx(w, h, env_scale, &cam, &tune, &bundle);
    let volume = main_menu_rain_spawn_volume(env_scale, h, &ctx.main_menu_effects.rain);
    let (d_min, d_max) = volume.frustum_depth_range(&cam);
    let base_rgb = [
        ctx.main_menu_effects.rain.field.drop_color[0],
        ctx.main_menu_effects.rain.field.drop_color[1],
        ctx.main_menu_effects.rain.field.drop_color[2],
    ];
    let lit = Some(RainVolumetricLit::build(
        &cam, base_rgb, d_min, d_max, &lighting,
    ));
    rain_field.push_quads(
        frame,
        &cam,
        w,
        h,
        ctx.main_menu_effects.rain.field.streak_len_px,
        ctx.main_menu_effects.rain.field.drop_color,
        lit,
    );
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HubMenuLoading {
    pub continue_loading: bool,
    pub new_game_loading: bool,
    pub archive_loading: bool,
}

impl HubMenuLoading {
    fn for_item(self, item: HubFocus) -> bool {
        match item {
            HubFocus::Continue => self.continue_loading,
            HubFocus::NewGame => self.new_game_loading,
            HubFocus::Archive => self.archive_loading,
            _ => false,
        }
    }
}

/// Three pulsing dots at the right edge of a hub menu row.
fn push_hub_loading_dots(
    quads: &mut Vec<GpuInstance>,
    row_rect: [f32; 4],
    t_secs: f32,
    scale: f32,
) {
    let dot_r = (row_rect[3] * 0.07).max(2.5 * scale).min(6.0 * scale);
    let gap = dot_r * 1.6;
    let trio_w = dot_r * 6.0 + gap * 2.0;
    let cx0 = row_rect[0] + row_rect[2] - trio_w;
    let cy = row_rect[1] + row_rect[3] * 0.5;
    let rgb = color::PARCHMENT;
    for i in 0..3 {
        let phase = ((t_secs * 3.0 - i as f32 * 0.45).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
        let alpha = 0.2 + 0.8 * phase;
        let cx = cx0 + i as f32 * (dot_r * 2.0 + gap) + dot_r;
        quads.push(GpuInstance {
            rect: [cx - dot_r, cy - dot_r, dot_r * 2.0, dot_r * 2.0],
            color: [rgb[0], rgb[1], rgb[2], alpha],
            user: 0,
        });
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HubFocus {
    Continue,
    NewGame,
    Archive,
    Options,
    Quit,
}

impl HubFocus {
    fn id(self) -> FocusId {
        FocusId(match self {
            HubFocus::Continue => 0x1200,
            HubFocus::NewGame => 0x1201,
            HubFocus::Archive => 0x1202,
            HubFocus::Options => 0x1203,
            HubFocus::Quit => 0x1204,
        })
    }

    fn from_id(id: FocusId) -> Option<Self> {
        Some(match id.0 {
            0x1200 => HubFocus::Continue,
            0x1201 => HubFocus::NewGame,
            0x1202 => HubFocus::Archive,
            0x1203 => HubFocus::Options,
            0x1204 => HubFocus::Quit,
            _ => return None,
        })
    }
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
) -> (Vec<ScenePunctualLight>, Vec<Option<String>>) {
    let room_glb = main_menu_glb::main_menu_glb_has_embedded_lights();
    let (mut punctual, mut nodes): (Vec<ScenePunctualLight>, Vec<Option<String>>) = if room_glb {
        let tagged =
            main_menu_glb::main_menu_embedded_point_lights_runtime_tagged(w, h, env_scale, tune);
        (
            tagged
                .iter()
                .map(|t| ScenePunctualLight::InverseSquare(t.light))
                .collect(),
            tagged.into_iter().map(|t| Some(t.gltf_node_name)).collect(),
        )
    } else {
        (Vec::new(), Vec::new())
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
    let fill_len = fill.len();
    punctual.extend(fill.into_iter().map(ScenePunctualLight::Smooth));
    nodes.extend(std::iter::repeat_n(None, fill_len));
    (punctual, nodes)
}

struct MainMenuRainLighting {
    punctual: Vec<ScenePunctualLight>,
    occluders: Vec<PunctualOccluderAabb>,
}

fn build_main_menu_rain_lighting(
    w: f32,
    h: f32,
    env_scale: f32,
    tune: &RoomEnvLightingTune,
) -> MainMenuRainLighting {
    let (punctual, _nodes) = main_menu_scene_punctual(w, h, env_scale, tune);
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
    let ambient = tune.ambient_scale;
    let linear_exposure = if room_glb {
        tune.room_glb_linear_hdr_gain()
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
        spots: &[],
        occluders: &bundle.occluders,
    }
}

fn push_main_menu_room_frame(
    frame: &mut UiFrame,
    w: f32,
    h: f32,
    env_scale: f32,
    tune: &RoomEnvLightingTune,
    camera: Option<CameraParams>,
) {
    if !main_menu_glb::main_menu_room_draw_ready() {
        return;
    }
    frame.background(BackgroundId::Black);
    frame.main_menu_environment();
    frame.camera_override =
        Some(camera.unwrap_or_else(|| main_menu_glb::main_menu_camera_base(w, h, env_scale)));
    let room_glb = main_menu_glb::main_menu_glb_has_embedded_lights();
    frame.scene_lighting.embedded_gltf_punctual = room_glb;
    frame.scene_lighting.room_glb_brdf = room_glb;
    let (punctual, nodes) = main_menu_scene_punctual(w, h, env_scale, tune);
    frame.scene_lighting.punctual = punctual;
    frame.scene_lighting.punctual_gltf_nodes = nodes;
}

pub(crate) fn scene_from_resume(
    resume_scene: ResumeScene,
    run: &mut RunState,
    progress: &PlayerProgress,
) -> Scene {
    match resume_scene {
        ResumeScene::Gameplay => Scene::Gameplay(Box::default()),
        ResumeScene::Shop => {
            if GameEngine::resumes_to_tutorial_shop(run) {
                Scene::Shop(ShopScene::new_tutorial(run))
            } else {
                Scene::Shop(ShopScene::new(run, progress))
            }
        }
        ResumeScene::Hallway => Scene::Hallway(super::hallway::HallwayScene::new()),
    }
}

pub struct MainMenuScene {
    tree: TreeState,
    cursor_pos: (f32, f32),
    last_frame: Instant,
    age_secs: f32,
    bug_phases: [f32; BUG_COUNT],
    rain_field: RainField,
    /// Speech bubble beside the hub moon after clicking it.
    moon_quip_visible: bool,
    /// Text for the current bubble (rolled when the bubble opens).
    moon_quip_message: String,
    /// Line indices not yet shown this hub visit (weighted pick, no repeat until exhausted).
    moon_quip_remaining: Vec<usize>,
    intro_trailer: Option<MainMenuTrailer>,
    intro_trailer_started: bool,
}

impl Default for MainMenuScene {
    fn default() -> Self {
        Self::new()
    }
}

impl MainMenuScene {
    pub fn new() -> Self {
        Self {
            tree: TreeState::new(),
            cursor_pos: (0.0, 0.0),
            last_frame: Instant::now(),
            age_secs: 0.0,
            bug_phases: lamp_moths::initial_bug_phases(),
            rain_field: RainField::new(),
            moon_quip_visible: false,
            moon_quip_message: String::new(),
            moon_quip_remaining: {
                let mut bag = Vec::new();
                moon_quips::refill_moon_quip_bag(&mut bag);
                bag
            },
            intro_trailer: None,
            intro_trailer_started: false,
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

fn hub_flat_items(w: f32, h: f32, in_progress: bool) -> Vec<FlatItem<HubFocus>> {
    let items = menu_items(in_progress);
    let layout = MainMenuScene::hub_layout(w, h, &items);
    layout
        .menu_rects
        .into_iter()
        .map(|(action, rect)| FlatItem::new(action.id(), rect, action))
        .collect()
}

fn hub_focus(tree: &TreeState) -> Option<HubFocus> {
    tree.focused().and_then(HubFocus::from_id)
}

/// Walnut speech bubble beside the projected moon hit rect.
fn push_moon_quip_bubble(
    frame: &mut UiFrame,
    moon_rect: [f32; 4],
    message: &str,
    w: f32,
    h: f32,
    scale: f32,
) {
    let [mx, my, mw, mh] = moon_rect;
    let pad = (h * 0.012 * scale).max(8.0);
    let border = metrics::tooltip_border_px(w, h);
    let body_font = typography::size(typography::H28, h);
    let line_h = body_font * 1.15;
    let max_inner_w = metrics::tooltip_max_panel_px(w, h) * 0.72;
    let inner_w = styled_text::colored_paragraph_preferred_width(
        message,
        line_h,
        max_inner_w,
        GlossaryMode::Prose,
    )
    .clamp(72.0, max_inner_w);
    let lines = styled_text::wrap_colored_text_multiline(
        message,
        inner_w,
        line_h,
        color::PARCHMENT,
        false,
        GlossaryMode::Prose,
    );
    let inner_h = styled_text::colored_multiline_block_height(lines.len(), line_h);
    let panel_w = inner_w + pad * 2.0;
    let panel_h = inner_h + pad * 2.0;
    let gap = (14.0 * scale).max(10.0);
    let tail_base_half_w = (panel_h * 0.10).clamp(6.0, 14.0);

    let moon_cx = mx + mw * 0.5;
    let moon_cy = my + mh * 0.5;
    let mut panel_x = mx + mw + gap;
    let mut panel_y = moon_cy - panel_h * 0.5;
    if panel_x + panel_w > w - pad {
        panel_x = (mx - gap - panel_w).max(pad);
    }
    panel_y = panel_y.clamp(pad, (h - panel_h - pad).max(pad));

    // Curved tail sweeps from the bubble lower-left toward the moon's lower edge.
    let tail_tip = [moon_cx, my + mh * 0.78];

    let mut quads = Vec::with_capacity(48);
    let mut squircles = Vec::with_capacity(2);
    tooltip::push_speech_bubble_overlay(
        &mut quads,
        &mut squircles,
        panel_x,
        panel_y,
        panel_w,
        panel_h,
        border,
        tail_tip,
        tail_base_half_w,
    );
    frame.overlay_quads(quads);
    frame.overlay_squircle_quads(squircles);
    let mut texts = Vec::new();
    styled_text::push_colored_rows_in_width(
        &mut texts,
        styled_text::ColoredRowsLayout {
            text_left: panel_x + pad,
            top_y: panel_y + pad,
            inner_w,
            line_h,
            fallback_plain: message,
            fallback_color: color::PARCHMENT,
            italic: false,
            glossary: GlossaryMode::Prose,
        },
        &lines,
        TextAlign::Center,
    );
    frame.texts(texts);
}

impl SceneBehavior for MainMenuScene {
    fn update(&mut self, ctx: UpdateCtx<'_>) -> SceneTransition {
        self.cursor_pos = ctx.cursor_pos;
        let now = Instant::now();
        let dt = now.saturating_duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.age_secs += dt;
        lamp_moths::advance_bug_phases(&mut self.bug_phases, dt, &ctx.main_menu_effects.moths);
        let w = ctx.layout.window_w;
        let h = ctx.layout.window_h;
        let env_scale = main_menu_glb::main_menu_env_height_scale(ctx.room_gltf_height_scale);
        if !self.intro_trailer_started && main_menu_glb::main_menu_room_draw_ready() {
            self.intro_trailer_started = true;
            if !ctx.headless {
                self.intro_trailer = MainMenuTrailer::start(w, h, env_scale);
            }
        }
        if self
            .intro_trailer
            .as_ref()
            .is_some_and(|trailer| trailer.finished_at(now))
        {
            self.intro_trailer = None;
        }
        if ctx.effect_layers.rain {
            let cam = self
                .intro_trailer
                .as_ref()
                .and_then(|trailer| trailer.camera_at(now, h))
                .unwrap_or_else(|| main_menu_glb::main_menu_camera_base(w, h, env_scale));
            let tune = RoomEnvLightingTune::default();
            let bundle = build_main_menu_rain_lighting(w, h, env_scale, &tune);
            let lighting = main_menu_rain_light_sample_ctx(w, h, env_scale, &cam, &tune, &bundle);
            let rain_mesh = main_menu_glb::main_menu_rain_collision_mesh();
            self.rain_field.update(
                dt,
                &ctx.main_menu_effects.rain,
                &cam,
                w,
                h,
                env_scale,
                rain_mesh.as_deref(),
                Some(&lighting),
            );
        }
        if self.intro_trailer.is_some() {
            return None;
        }
        let in_progress = GameEngine::run_in_progress(ctx.run);
        let flat = hub_flat_items(w, h, in_progress);
        if hub_focus(&self.tree).is_none()
            || hub_focus(&self.tree).is_some_and(|f| !menu_items(in_progress).contains(&f))
        {
            self.tree.set_focus(default_focus(in_progress).id());
        }

        let action = self.tree.update_flat(
            &flat,
            TreeInput {
                actions: ctx.actions,
                button_clicks: ctx.button_clicks,
                cursor_pos: ctx.cursor_pos,
                window: (ctx.layout.window_w, ctx.layout.window_h),
                input_mode: ctx.input_mode,
                scroll_lines: 0.0,
            },
        );
        if self.tree.take_focus_changed() {
            ctx.bus.push(GameEvent::UiSound(SfxId::TilePlace));
        }

        for a in ctx.actions {
            if matches!(a, UiAction::Cancel | UiAction::Pause) {
                ctx.bus.push(GameEvent::UiSound(SfxId::UiCancel));
                self.tree.set_focus(HubFocus::Quit.id());
            }
        }

        let moon_clicked = ctx.button_clicks.contains(&MAIN_MENU_PICK_MOON);
        if moon_clicked {
            if self.moon_quip_visible {
                self.moon_quip_visible = false;
            } else {
                self.moon_quip_message =
                    moon_quips::roll_moon_quip(&mut self.moon_quip_remaining).to_string();
                self.moon_quip_visible = true;
            }
            ctx.bus.push(GameEvent::UiSound(SfxId::TileClick));
        }

        if let Some(item) = action {
            let confirm_sfx = match item {
                HubFocus::NewGame => SfxId::NewGameStinger,
                _ => SfxId::UiConfirm,
            };
            ctx.bus.push(GameEvent::UiSound(confirm_sfx));
            match item {
                HubFocus::Continue => {
                    return Some(SceneIntent::Continue(ctx.resume_scene));
                }
                HubFocus::NewGame => {
                    if ctx.tutorial_eligible {
                        return Some(SceneIntent::TileSelect { tutorial: true });
                    }
                    if ctx.multiple_materials {
                        return Some(SceneIntent::TileSelect { tutorial: false });
                    }
                    return Some(SceneIntent::StartRunDefaultMaterialAndShop);
                }
                HubFocus::Archive => {
                    return Some(SceneIntent::Archive);
                }
                HubFocus::Options => {
                    return Some(SceneIntent::Options);
                }
                HubFocus::Quit => {
                    *ctx.quit_requested = true;
                }
            }
        }

        None
    }

    fn draw_frame(&self, mut ctx: DrawCtx<'_>) -> UiFrame {
        let layout = ctx.layout;
        let w = layout.window_w;
        let h = layout.window_h;
        let scale = metrics::scene_scale(w, h);
        let now = Instant::now();
        let intro_trailer_camera = self
            .intro_trailer
            .as_ref()
            .and_then(|trailer| trailer.camera_at(now, h));
        let main_menu_trailer_camera = ctx.main_menu_trailer_camera.or(intro_trailer_camera);

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
                push_main_menu_room_frame(
                    &mut frame,
                    w,
                    h,
                    env_scale,
                    &ctx.room_env_for(scene_keys::MAIN_MENU).0,
                    main_menu_trailer_camera,
                );
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
                        &ctx.main_menu_effects.moths,
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

        let trailer_shot = main_menu_trailer_camera.is_some();
        let env_scale = main_menu_glb::main_menu_env_height_scale(ctx.room_gltf_height_scale);

        let mut quads: Vec<GpuInstance> = Vec::new();
        let mut text_labels = Vec::new();
        let mut buttons = Vec::new();
        let hub_layout = if trailer_shot {
            None
        } else {
            let in_progress = ctx.game_in_progress;
            let flat = hub_flat_items(w, h, in_progress);
            let layout = Self::hub_layout(w, h, &menu_items(in_progress));

            if let Some(focus) = hub_focus(&self.tree)
                && let Some(&(_, rect)) = layout.menu_rects.iter().find(|(t, _)| *t == focus)
            {
                focus_nav::push_focus_ring(rect, scale, w, h, &mut quads);
            }

            let menu_font = typography::size(typography::H36, h);
            let label_color = color::PARCHMENT;
            for &(item, rect) in &layout.menu_rects {
                let loading = ctx.hub_loading.for_item(item);
                let mut label_alpha = 1.0;
                if loading {
                    label_alpha = 0.55;
                    push_hub_loading_dots(&mut quads, rect, self.age_secs, scale);
                }
                let mut color = label_color;
                color[3] *= label_alpha;
                text_labels.push(TextLabel {
                    rect,
                    text: label_for(item, in_progress, ctx.archive_has_new),
                    font_px: Some(menu_font),
                    color,
                    align: TextAlign::Left,
                    ..Default::default()
                });
            }

            if let Some(moon_rect) = main_menu_glb::main_menu_moon_screen_hit_rect(w, h, env_scale)
            {
                buttons.push(ButtonDef::scene(
                    (moon_rect[0], moon_rect[1], moon_rect[2], moon_rect[3]),
                    MAIN_MENU_PICK_MOON,
                ));
            }
            self.tree.register_flat_buttons(&flat, &mut buttons);
            Some(layout)
        };

        let mut frame = UiFrame::new();
        frame.background(BackgroundId::Black);
        if main_menu_glb::main_menu_room_draw_ready() {
            push_main_menu_room_frame(
                &mut frame,
                w,
                h,
                env_scale,
                &ctx.room_env_for(scene_keys::MAIN_MENU).0,
                main_menu_trailer_camera,
            );
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
                    &ctx.main_menu_effects.moths,
                );
            }
        }
        push_main_menu_rain(&mut frame, &self.rain_field, &ctx, w, h, env_scale);
        if ctx.effect_layers.starfield {
            frame.starfield();
        }
        if !trailer_shot {
            if let Some(layout) = hub_layout {
                frame.quads(quads);
                frame.image_quads([ImageQuad {
                    inst: GpuInstance {
                        rect: layout.logo_rect,
                        color: [1.0, 1.0, 1.0, 1.0],
                        user: 0,
                    },
                    source: ImageQuadSource::RawAsset {
                        path: MAIN_MENU_LOGO_ASSET,
                    },
                    clip_rect: None,
                }]);
            }
            frame.texts(text_labels);
            if self.moon_quip_visible
                && !self.moon_quip_message.is_empty()
                && let Some(moon_rect) =
                    main_menu_glb::main_menu_moon_screen_hit_rect(w, h, env_scale)
            {
                push_moon_quip_bubble(&mut frame, moon_rect, &self.moon_quip_message, w, h, scale);
            }
            frame.buttons = buttons;
            frame.cursor_pos = Some(self.cursor_pos);
            push_screen_footer_hint(
                &mut frame,
                &ctx,
                menu_footer_row(ctx.input_mode),
                HintStyle::standard(w, h),
            );
        }
        frame.window_title = format!(
            "Mahjuro — {}",
            if cfg!(debug_assertions) {
                "vNEXT"
            } else {
                env!("CARGO_PKG_VERSION")
            }
        );

        if !trailer_shot {
            let flat = hub_flat_items(w, h, ctx.game_in_progress);
            ctx.stash_focus_nav_tree_flat(&self.tree, &flat, |f| format!("{f:?}"));
        }

        frame
    }
}
