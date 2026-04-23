//! WGPU: depth-tested 3D tile meshes for the hand + 2D UI quads on top.

#[path = "wgpu_renderer/resources.rs"]
mod resources;
#[path = "wgpu_renderer/init.rs"]
mod init;
#[path = "wgpu_renderer/runtime.rs"]
mod runtime;
#[path = "wgpu_renderer/showcase.rs"]
mod showcase;

use self::resources::*;
use self::showcase::*;

use std::collections::HashMap;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use chrono::{NaiveDate, Utc};
use glam::Mat4;
use wgpu::util::DeviceExt;
use winit::window::Window;

use crate::core::relic::{RelicId, RelicRenderMaterial, relic_visual};
use crate::core::tile::{Suit, Tile};
use crate::core::tile_pack::TilePackKind;
use crate::render::bone_tablet_mesh::build_bone_tablet_mesh;
use crate::render::cabinet_mesh::{build_cabinet_mesh, build_cabinet_rails_mesh};
use crate::render::candle_mesh::{build_candle_wax_mesh, build_candle_wick_mesh};
use crate::render::coin_mesh::build_coin_mesh;
use crate::render::decal::{
    LabelAlign, load_noto_emoji_font, load_ui_font, rasterize_label_styled_with_fallback,
    rasterize_tile_face_decal,
};
use crate::render::dora_plinth_mesh::build_dora_plinth_mesh;
use crate::render::draw_cmd::{
    CascadeTokenKind, DrawCmd, ShowcaseTilePlacement, ShrinePlacement, TallyFanKind, TileFaceQuad,
    UiFrame, WallStackPlacement, YakuTabletPlacement,
};
use crate::render::gpu_types::{DecodedRelicImage, RelicTextureGpu};
use crate::render::lamp_mesh::{
    build_bug_body_mesh, build_bug_wing_blur_mesh, build_bug_wing_mesh, build_lamp_body_mesh,
    build_lamp_bulb_mesh,
};
use crate::render::lit_mesh::Aabb;
use crate::render::lit_mesh::MeshCpu;
use crate::render::lit_mesh::push_box;
use crate::render::lit_mesh::{
    LitMeshGpu, LitMeshInstance, MaterialKind, MaterialParams, ShadowCasterUniform, ShadowGlobals,
    SsrGlobals, create_lit_mesh_material_layout, create_lit_mesh_ssr_layout,
    create_shadow_caster_layout, create_shadow_sample_layout,
};
use crate::render::mirror_mesh::{MIRROR_LOCAL_CENTER_Y, MIRROR_LOCAL_HALF, build_mirror_mesh};
use crate::render::ofuda_mesh::build_ofuda_mesh;
use crate::render::orb_mesh::build_orb_mesh;
use crate::render::plaque_mesh::build_plaque_mesh;
use crate::render::primitive::MeshId;
use crate::render::relic_dish::{
    build_dish_mesh, build_pack_mesh, build_relic_mesh, build_relic_mesh_from_rgba,
    build_round_dish_mesh, build_shop_action_prop_mesh, build_tent_card_mesh,
};
use crate::render::relic_pipeline::spawn_relic_loader;
use crate::render::ribbon_mesh::build_ribbon_mesh;
use crate::render::river_mesh::{
    RIVER_LOCAL_CENTER_Y as BOWL_LOCAL_CENTER_Y, RIVER_LOCAL_HALF as BOWL_LOCAL_HALF,
    build_river_mesh,
};
use crate::render::shrine_mesh::build_shrine_mesh;
use crate::render::table_mesh::build_table_mesh;
use crate::render::table_transform::{
    mesh_y_thickness_along_local_y_to_z_up, ribbon_submesh, rot_euler_xyz_rad, rot_rz_rx_deg,
    rot_x_rad, rotation_around_point_x_rad, score_popup_glyph_rot_rad, table_mesh_lay_flat,
    tile_mesh_local_to_world, translate_rot_scale,
};
use crate::render::talisman_mesh::{TALISMAN_LOCAL_HALF, build_talisman_mesh, talisman_material};
use crate::render::tally_stick_mesh::{build_tally_stick_base_mesh, build_tally_stick_tip_mesh};
use crate::render::texture_upload::load_pack_textures;
use crate::render::tile_glb::{Vertex3dTex, load_glb_tile_from_bytes, normalize_mesh};
use crate::render::wood_tablet_mesh::build_wood_tablet_mesh;
use crate::render::world_space::pixel_to_world;
use crate::scenes::BackgroundId;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    screen: [f32; 2],
    time: f32,
    gamma: f32,
    cursor_pos: [f32; 2],
    transition_progress: f32,
    quality_level: f32,
    moon_phase: f32,
    _globals_pad: [f32; 3],
}

/// Per-frame art-direction knobs for the procedural mountain-haze shader.
/// `density = 0` turns the haze off; see
/// [`crate::game::volumetric_tuning::VolumetricTuning`] for the slider
/// ranges that drive these values from the debug overlay.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HazeUniform {
    /// RGB haze colour (linear) + density multiplier in the alpha slot.
    color_density: [f32; 4],
    /// `x` = horizon y (0..1), `y` = drift-speed multiplier, `z`/`w` reserved.
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct BloomParams {
    data0: [f32; 4],
    data1: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CameraUniform {
    view_proj: [f32; 16],
    model: [f32; 16],
    base_color_factor: [f32; 4],
    /// World-space camera position, used for fresnel/view-dependent effects in tile_3d.wgsl.
    cam_pos: [f32; 3],
    /// Per-tile instance seed — any finite float. Read by `tile_3d.wgsl` to
    /// offset procedural noise so every tile's tortoise-shell pattern (and
    /// future material variations) is unique. Not all materials sample it.
    tile_seed: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
}

/// Per-frame settings threaded from the app into `WgpuRenderer::render`:
/// quality tiers, tile-look choices, animation settle speeds, gamma, and
/// the shadow/SSR toggles. Grouped so the render entry point takes one
/// value instead of ten individual params.
pub struct RenderSettings {
    pub smoke_quality: crate::persistence::SmokeQuality,
    pub smoke_amount: crate::persistence::SmokeAmount,
    pub effects_quality: crate::persistence::EffectsQuality,
    pub tile_preset: crate::persistence::TilePreset,
    pub tile_material: crate::persistence::TileMaterial,
    /// Subdirectory of `assets/sets/` whose PNGs should be used for tile faces.
    pub tileset_name: String,
    pub draw_settle_speed: f32,
    pub sort_settle_speed: f32,
    pub gamma: f32,
    pub shadows_enabled: bool,
    pub ssr_enabled: bool,
}

/// View uniform consumed by the 3D flame pipeline. Mirrors
/// `FlameView` in `shaders/flame.wgsl`: just the matrices the
/// billboard vertex shader needs. Kept separate from `SsrGlobals`
/// because the SSR layout restricts its uniform to the fragment
/// stage, and the flame vertex shader needs `view_proj`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FlameViewUniform {
    view_proj: [f32; 16],
    view_pos: [f32; 4],
}

/// Instance for the `gradient_quad_pipeline` — same `rect`/`color` payload
/// as `GpuInstance` plus a per-instance `feather` vec4 that drives the
/// shader's alpha falloff. See `shaders/gradient_quad.wgsl` for the exact
/// contract; `feather.x` = edge softness fraction, `feather.y` = axial↔radial
/// blend, `feather.zw` reserved.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GradientQuadInstance {
    pub rect: [f32; 4],
    pub color: [f32; 4],
    pub feather: [f32; 4],
}

/// Maximum number of point lights uploaded each frame. Must match the array
/// length in tile_3d.wgsl.
pub const MAX_POINT_LIGHTS: usize = 16;

/// Maximum number of spotlights uploaded each frame. Must match the array
/// length in tile_3d.wgsl. Spotlights are only sampled by the tile pipeline
/// (not lit_mesh / smoke lightbake) — they're a narrow tool for drawing
/// focused pools onto tile faces (e.g. hint indicators).
pub const MAX_SPOT_LIGHTS: usize = 8;

/// Cheap deterministic hash of `(label, width, height)` used as the cache
/// key for rasterised tablet decals — when this changes, the renderer
/// re-rasterises the engraved label texture and uploads it. FNV-1a 64.
/// Grow `pool` so it holds at least `n` `LitMeshInstance` slots, creating
/// GPU-resource-backed instances on demand. Replaces fixed-size pools
/// (formerly `MAX_PLAQUE_SLOTS` and friends) that silently dropped draws
/// once exceeded — fragile when multiple scene paths share one pool.
///
/// Kept as a free function so call sites inside `draw_frame` can split-borrow
/// the disjoint renderer fields (the pool vs. device/layouts/views) without
fn tablet_label_hash(label: &str, w: u32, h: u32) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in label.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash ^= w as u64;
    hash = hash.wrapping_mul(0x100000001b3);
    hash ^= h as u64;
    hash.wrapping_mul(0x100000001b3)
}

/// Maximum number of analytic tile occluders uploaded for the candle-pool
/// shadow tests in `lit_mesh.wgsl`. One per visible hand tile, conservatively
/// sized so the full hand fits.
pub const MAX_TILE_OCCLUDERS: usize = 16;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileOccluderGpu {
    /// xyz = world-space AABB center, w = unused.
    center: [f32; 4],
    /// xyz = world-space AABB half-extents, w = unused.
    half_extents: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct TileOccludersBuf {
    /// `count.x` = number of active occluders; rest is std140 padding.
    count: [u32; 4],
    boxes: [TileOccluderGpu; MAX_TILE_OCCLUDERS],
}

/// World-space AABB covering every candle flame currently in the frame.
/// Returns `None` if there are no candles. Used by the fluid scissor so
/// the flame sub-march in the volume shader isn't scissored out when
/// the smoke field is empty. Padding is `flame_height_world` on every
/// axis — generous enough to cover either axis convention for "which
/// way the flame points" (shader is Y-up, world is Z-up) and the
/// bounding sphere the shader itself uses for ray-flame intersection.
fn compute_flame_world_aabb(
    candles: &[PointLight],
    flame_height_world: f32,
    screen_w: f32,
    screen_h: f32,
) -> Option<(glam::Vec3, glam::Vec3)> {
    if candles.is_empty() || flame_height_world <= 0.0 {
        return None;
    }
    let pad = flame_height_world.max(4.0);
    let pad_v = glam::Vec3::splat(pad);
    let mut mn = glam::Vec3::splat(f32::INFINITY);
    let mut mx = glam::Vec3::splat(f32::NEG_INFINITY);
    for l in candles {
        let p = pixel_to_world(screen_w, screen_h, l.pos[0], l.pos[1], l.pos[2]);
        mn = mn.min(p - pad_v);
        mx = mx.max(p + pad_v);
    }
    Some((mn, mx))
}

fn current_moon_phase() -> f32 {
    let known_new_moon = NaiveDate::from_ymd_opt(2000, 1, 6)
        .expect("valid new moon reference date")
        .and_hms_opt(18, 14, 0)
        .expect("valid new moon reference time");
    let days_since_reference =
        (Utc::now().naive_utc() - known_new_moon).num_seconds() as f64 / 86_400.0;
    let synodic_month_days = 29.530_588_853_f64;
    (days_since_reference.rem_euclid(synodic_month_days) / synodic_month_days) as f32
}

impl TileOccludersBuf {
    fn empty() -> Self {
        Self {
            count: [0; 4],
            boxes: [TileOccluderGpu {
                center: [0.0; 4],
                half_extents: [0.0; 4],
            }; MAX_TILE_OCCLUDERS],
        }
    }
}

/// CPU-side description of a point light. Scenes push these into
/// [`crate::render::draw_cmd::UiFrame::point_lights`]; the renderer translates
/// them into [`PointLightGpu`] each frame.
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    /// World-space position of the light. The first two components match the
    /// pixel-space coordinate system used for tile model matrices (with the
    /// usual `y → -y` flip the renderer applies); `z` lets candle wicks sit in
    /// front of the table plane so 3D meshes catch the light correctly.
    pub pos: [f32; 3],
    /// Falloff radius in pixels. Outside this distance the light contributes
    /// nothing.
    pub radius: f32,
    /// Linear-space RGB tint.
    pub color: [f32; 3],
    /// Brightness multiplier. >1.0 is fine — the tile shader is unclamped.
    pub intensity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PointLightGpu {
    /// xyz = world-space position, w = radius.
    pos: [f32; 4],
    /// rgb = colour, a = intensity.
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PointLightsBuf {
    /// `count.x` = number of active lights; rest is std140 padding.
    count: [u32; 4],
    /// Frame-wide extras shared with shaders that bind this buffer:
    /// `extras.x` = display gamma (used to gamma-correct 3D fragments
    /// that don't have access to the screen-space `Globals` uniform).
    /// `extras.y` = wall-clock time in seconds (used by `MaterialKind::Water`
    /// to scroll the river surface and animate foam crests).
    /// `extras.z` = candle flame height in world units (for the volumetric
    /// lightbake flame emission envelope).
    /// `extras.w` reserved.
    extras: [f32; 4],
    lights: [PointLightGpu; MAX_POINT_LIGHTS],
}

/// CPU-side description of a spotlight. A spotlight has a direction + cone
/// half-angle, so it pools light onto a specific surface region rather than
/// radiating omnidirectionally. Used to draw focused visual-highlight pools
/// on specific tiles (hint indicators). Scenes push these into
/// [`crate::render::draw_cmd::UiFrame::spot_lights`]; the renderer translates
/// them into [`SpotLightGpu`] each frame. Only the tile pipeline samples the
/// spotlight buffer — table / candles / smoke do not receive spotlight
/// contribution.
#[derive(Clone, Copy, Debug)]
pub struct SpotLight {
    /// Pixel-space position (same convention as `PointLight`). `z` is the
    /// vertical lift above the felt (Z-up world).
    pub pos: [f32; 3],
    /// World-space direction the light points, FROM the light TOWARD the
    /// illuminated surface. Does not need to be normalized; the GPU side
    /// normalises. Typical use: `[0.0, 0.0, -1.0]` for straight-down.
    pub dir: [f32; 3],
    /// Falloff radius in pixels. Outside this distance the light contributes
    /// nothing.
    pub radius: f32,
    /// Cosine of the outer cone half-angle. Outside this angle, contribution
    /// drops to zero. `cos(30°) ≈ 0.866` for a 60°-wide cone.
    pub cos_outer: f32,
    /// Cosine of the inner cone half-angle. Inside this angle, contribution
    /// is full. Between inner and outer the factor smoothsteps. Must be
    /// greater than or equal to `cos_outer` (inner angle ≤ outer angle).
    pub cos_inner: f32,
    /// Linear-space RGB tint.
    pub color: [f32; 3],
    /// Brightness multiplier.
    pub intensity: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpotLightGpu {
    /// xyz = world-space position, w = radius.
    pos: [f32; 4],
    /// xyz = world-space direction (normalized), w = cos_outer.
    dir: [f32; 4],
    /// rgb = colour, a = intensity.
    color: [f32; 4],
    /// x = cos_inner, y/z/w reserved.
    params: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SpotLightsBuf {
    /// `count.x` = number of active spotlights; rest is std140 padding.
    count: [u32; 4],
    lights: [SpotLightGpu; MAX_SPOT_LIGHTS],
}

impl SpotLightsBuf {
    fn empty() -> Self {
        Self {
            count: [0; 4],
            lights: [SpotLightGpu {
                pos: [0.0; 4],
                dir: [0.0, 0.0, -1.0, 1.0],
                color: [0.0; 4],
                params: [1.0; 4],
            }; MAX_SPOT_LIGHTS],
        }
    }

    /// Build the std140 spotlight buffer. Positions are mapped from pixel
    /// space to world (Z-up) via `pixel_to_world`. Direction is taken as-is
    /// in world space (already Z-up) and normalised on the GPU side — we
    /// normalise here too to keep the uniform sane to inspect.
    fn from_lights(src: &[SpotLight], screen_w: f32, screen_h: f32) -> Self {
        let mut lights = [SpotLightGpu {
            pos: [0.0; 4],
            dir: [0.0, 0.0, -1.0, 1.0],
            color: [0.0; 4],
            params: [1.0; 4],
        }; MAX_SPOT_LIGHTS];
        let n = src.len().min(MAX_SPOT_LIGHTS);
        for (i, l) in src.iter().take(n).enumerate() {
            let p = pixel_to_world(screen_w, screen_h, l.pos[0], l.pos[1], l.pos[2]);
            let d = glam::Vec3::from(l.dir).normalize_or_zero();
            let d = if d.length_squared() < 0.5 {
                glam::Vec3::new(0.0, 0.0, -1.0)
            } else {
                d
            };
            lights[i] = SpotLightGpu {
                pos: [p.x, p.y, p.z, l.radius],
                dir: [d.x, d.y, d.z, l.cos_outer],
                color: [l.color[0], l.color[1], l.color[2], l.intensity],
                params: [l.cos_inner, 0.0, 0.0, 0.0],
            };
        }
        Self {
            count: [n as u32, 0, 0, 0],
            lights,
        }
    }
}

impl PointLightsBuf {
    /// Build the std140 light buffer, mapping each light's pixel-space
    /// `(x, y)` onto the table-plane world (`world_x = x - w/2`,
    /// `world_y = y - h/2`). The third position component is **+Z** lift above the felt.
    fn from_lights(
        src: &[PointLight],
        candle_count: u32,
        flame_height_world: f32,
        screen_w: f32,
        screen_h: f32,
        gamma: f32,
        time: f32,
    ) -> Self {
        let mut lights = [PointLightGpu {
            pos: [0.0; 4],
            color: [0.0; 4],
        }; MAX_POINT_LIGHTS];
        let n = src.len().min(MAX_POINT_LIGHTS);
        for (i, l) in src.iter().take(n).enumerate() {
            let p = pixel_to_world(screen_w, screen_h, l.pos[0], l.pos[1], l.pos[2]);
            lights[i] = PointLightGpu {
                pos: [p.x, p.y, p.z, l.radius],
                color: [l.color[0], l.color[1], l.color[2], l.intensity],
            };
        }
        Self {
            count: [n as u32, candle_count.min(n as u32), 0, 0],
            extras: [gamma.max(0.01), time, flame_height_world, 0.0],
            lights,
        }
    }
}

/// One material slot of the tile mesh — vertex/index buffers + the primitive's
/// own albedo texture.  A tile may consist of several of these (e.g. an ivory
/// face primitive and a bamboo back primitive).
struct TilePrimitiveGpu {
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    albedo_view: wgpu::TextureView,
}

/// A relic icon to draw as a textured quad at a screen-space rect.
pub struct RelicIcon {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Which relic image to display.
    pub relic_id: crate::core::relic::RelicId,
}

/// Horizontal alignment of text inside its rect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlign {
    Left,
    #[default]
    Center,

    Right,
}

/// A rasterized text label to draw over a screen-space rect.
///
/// `font_px = None` falls back to the legacy auto-shrink behaviour where
/// `rasterize_label` picks `min(rect.h * 0.55, rect.w * 1.5 / chars)`. Set
/// `font_px = Some(px)` to pin the font size — this is what `push_text_block`
/// uses to keep every wrapped line of a paragraph at a consistent size.
///
/// `text` may contain `\n` to indicate explicit line breaks; the rasteriser
/// stacks lines vertically and applies the chosen alignment to each line.
pub struct TextLabel {
    /// Position in screen pixels: [x, y, w, h].
    pub rect: [f32; 4],
    /// Text to render. May contain `\n` for hard line breaks.
    pub text: String,
    /// Colour for the text glyphs (default: white).
    pub color: [f32; 4],
    /// Pinned font size in pixels. If `None`, the renderer auto-sizes from
    /// the rect's dimensions (legacy behaviour).
    pub font_px: Option<f32>,
    /// Horizontal alignment within the rect.
    pub align: TextAlign,
    /// Suppress glossary-term tooltip detection on this label.  Set on labels
    /// that are part of an element with its own dedicated hover tooltip
    /// (e.g. yaku progress cards), so terms inside them don't get underlined
    /// or trigger nested tooltips.
    pub no_glossary: bool,
    /// Horizontal scroll offset in pixels (for marquee-style text).
    /// Shifts the rasterised text leftward by this many pixels so the
    /// caller can animate it for overflow text.  Default 0.0.
    pub scroll_offset: f32,
}

impl Default for TextLabel {
    fn default() -> Self {
        Self {
            rect: [0.0; 4],
            text: String::new(),
            color: [1.0; 4],
            font_px: None,
            align: TextAlign::Center,
            no_glossary: false,
            scroll_offset: 0.0,
        }
    }
}

/// GPU resources for a single hand tile.
///
/// Each tile has its own uniform buffer (updated every frame with the per-tile
/// model matrix) and bind group (holds the tile's rasterised decal texture).
/// Storing them per-tile means all 14 `write_buffer` calls target distinct
/// GPU allocations, so every tile's matrix is visible when the command buffer
/// executes — no dynamic-offset trickery required.
struct HandTileGpu {
    /// Written every frame with view_proj + model + base_color_factor.
    uniform_buffer: wgpu::Buffer,
    /// Companion uniform buffer for the gold-metal outline shell. Written
    /// every frame the tile is *selected* with an inflated model matrix
    /// (uniform 1.06× scale around the tile center). Always allocated so
    /// the bind group can stay constant for the lifetime of the tile.
    outline_uniform_buffer: wgpu::Buffer,
    /// Per-tile shadow caster uniform (light_view_proj * model). Written
    /// every frame in lockstep with `uniform_buffer` and consumed by the
    /// shadow pre-pass via `shadow_bind_group`.
    shadow_uniform_buffer: wgpu::Buffer,
    shadow_bind_group: wgpu::BindGroup,
    /// Cached to skip re-rasterisation when the tile hasn't changed.
    /// Includes the talisman enhancement so stamping a tile triggers a fresh
    /// decal upload (the enhancement is baked into the texture as a coloured
    /// border + corner gem in `rasterize_tile_face_decal`).
    tile_id: (Suit, u8, Option<crate::core::tile::TileEnhancement>, bool),
}

/// GPU resources for a showcase tile (pack celebration, hand strip, choose-tiles grid, etc.).
struct ShowcaseTileGpu {
    uniform_buffer: wgpu::Buffer,
    bind_groups: Vec<wgpu::BindGroup>,
    /// Outline shell uniform + bind groups — always allocated so the bind
    /// group can stay constant; only written when `p.outline` is true.
    outline_uniform_buffer: wgpu::Buffer,
    outline_bind_groups: Vec<wgpu::BindGroup>,
    shadow_uniform_buffer: wgpu::Buffer,
    shadow_bind_group: wgpu::BindGroup,
    /// Cache key to skip re-rasterisation when the tile hasn't changed.
    tile_id: (Suit, u8, Option<crate::core::tile::TileEnhancement>, bool),
}

struct TileFaceOverlayGpu {
    _texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

const MAX_SHOWCASE_TILE_SLOTS: usize = 160;

// Tile-mesh local extents (after `normalize_mesh` in tile_glb.rs):
//   local X — long face axis  (extent ~1.000) → table-Z (front-back)
//   local Y — thickness        (extent ~0.424) → world Y (up off table)
//   local Z — short face axis  (extent ~0.734) → table-X (left-right)
pub(super) const LOCAL_X_EXTENT: f32 = 1.000;
pub(super) const LOCAL_Y_EXTENT: f32 = 0.424;
pub(super) const LOCAL_Z_EXTENT: f32 = 0.734;

/// Camera state captured at the end of a frame, for unprojecting cursor
/// positions into world-space rays in `pick_hand_tile`.
#[derive(Clone, Copy)]
pub(super) struct PickCamera {
    pub(super) inv_view_proj: Mat4,
    pub(super) viewport_w: f32,
    pub(super) viewport_h: f32,
}

/// A tile animating away from the hand. Two-phase trajectory: an
/// **Arcing** phase that throws the tile from its hand slot down into the
/// discard river, followed by a **Drifting** phase where the tile rides
/// the current along the channel and fades out. The split is what reads
/// as "throw the tile away" vs the previous fly-off-the-table arc.
struct DepartingTile {
    /// Visual identity for rendering.
    symbol: String,
    suit_emoji: String,
    suit_color: [f32; 4],
    /// Screen-space rect at the moment of departure (top-left + size).
    start_rect: [f32; 4],
    /// Splash point — center of the river rect at spawn time, with a
    /// small per-tile jitter so multiple tiles land at slightly different
    /// spots instead of stacking pixel-perfect.
    river_target: (f32, f32),
    /// Pixel direction the tile drifts after splashing. Currently
    /// hard-coded to +X (the river flows left-to-right in screen space).
    /// Stored as a unit vector so the render path can change the river
    /// orientation without touching the simulation.
    drift_dir: (f32, f32),
    /// Per-tile drift speed in pixels/sec.
    drift_speed: f32,
    /// Phase 1 duration (seconds) — how long the arc-into-river takes.
    arc_dur: f32,
    /// Phase 2 duration (seconds) — how long the tile drifts before
    /// fading out. The total visible lifetime is `arc_dur + drift_dur`.
    drift_dur: f32,
    /// Seconds elapsed since departure started.
    elapsed: f32,
    /// Total lifetime convenience field — equals `arc_dur + drift_dur`.
    /// Kept so the existing `retain(|t| t.elapsed < t.lifetime)` cull and
    /// the gameplay-scene refill timer (which reads
    /// `cascade_tuning.depart_lifetime_ms`) keep working.
    lifetime: f32,
}

/// Per-frame projected screen-space rects for every 3D element category.
/// Written during rendering, read one-frame-stale by scenes via
/// `DrawCtx` to anchor 2D overlays to visible 3D positions.
#[derive(Default)]
pub struct ProjectionCache {
    pub hand_rects: Vec<(usize, [f32; 4])>,
    pub relic_rects: Vec<[f32; 4]>,
    pub pack_rects: Vec<([f32; 4], Option<u32>)>,
    pub shrine_rects: Vec<[f32; 4]>,
    pub ribbon_rects: Vec<[f32; 4]>,
    pub talisman_rects: Vec<[f32; 4]>,
    pub plaque_rects: Vec<[f32; 4]>,
    pub peg_rects: [Option<[f32; 4]>; 2],
    pub yaku_tablet_rects: Vec<[f32; 4]>,
    pub wood_tablet_rects: Vec<[f32; 4]>,
    pub bowl_rect: Option<[f32; 4]>,
    pub mirror_rect: Option<[f32; 4]>,
    pub aux_dish_rects: Vec<(Option<u32>, [f32; 4])>,
    pub dora_plinth_rect: Option<[f32; 4]>,
}

/// Tag identifying which cached triangle list backs a trimesh pickable.
/// The slab-test is sometimes a bad silhouette proxy (hanging lamp has a
/// narrow cord above a wide shade), so we ray-cast against the real mesh
/// for those objects. Add a variant here when adding a new trimesh pick.
#[derive(Clone, Copy, Debug)]
pub(super) enum TrimeshRef {
    LampBody,
}

/// Active arrange-mode override for the renderer. When set, the matching
/// object's model matrix is rebuilt each frame using these values instead of
/// the placement data from the scene's draw commands.
#[derive(Clone, Debug)]
pub struct DebugArrangeOverride {
    /// Name as registered in `last_debug_pickables` (e.g. `"BlindPlaque"`).
    pub name: String,
    /// Pixel nudge along X. Because world_x = pixel_x − w/2, this maps 1:1
    /// to world X regardless of window size.
    pub delta_px: f32,
    /// Pixel nudge along Y (positive = toward player). world_y = h/2 − pixel_y,
    /// so delta_py maps to −world_y, also 1:1 regardless of window size.
    pub delta_py: f32,
    /// World-Z nudge (lift above the felt). Window-size-independent.
    pub delta_lift: f32,
    /// Rotation delta around Z, degrees (additive on top of original).
    pub delta_rz_deg: f32,
    /// Rotation delta around X, degrees (additive on top of original).
    pub delta_rx_deg: f32,
    /// Rotation delta around Y, degrees (additive on top of original).
    pub delta_ry_deg: f32,
}

/// Where the final composited frame lands. `Surface` is the normal
/// swapchain path used by the interactive game; `Offscreen` is a plain
/// render-attachment texture used by headless screenshot mode (no window,
/// no window-server occlusion games, no swapchain `Outdated` retries).
///
/// `config` on `WgpuRenderer` still holds the format/size that downstream
/// scene-color/SSR textures track against — the offscreen path writes the
/// same values there so `resize()` and the various post textures don't
/// need to branch.
enum RenderTarget {
    Surface(wgpu::Surface<'static>),
    Offscreen { texture: wgpu::Texture },
}

/// Where the renderer should send frames. Chosen once at construction:
/// the interactive game builds a `Windowed`; the screenshot CLI builds a
/// `Headless`.
pub enum TargetInit {
    Windowed {
        window: Arc<Window>,
        hdr_enabled: bool,
    },
    Headless {
        width: u32,
        height: u32,
        hdr_enabled: bool,
    },
}

pub struct WgpuRenderer {
    target: RenderTarget,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    depth_texture: wgpu::Texture,
    depth_view: wgpu::TextureView,
    /// Snapshot of the scene depth, copied between the pre-smoke and
    /// post-smoke render passes so the volumetric smoke can sample depth
    /// without aliasing the live depth attachment.
    depth_copy_texture: wgpu::Texture,
    depth_copy_view: wgpu::TextureView,
    /// Snapshot of the scene depth taken at the *end of pass A1* (before
    /// the hanging plaques are drawn). The lacquered-table SSR group
    /// samples this view so the table never reflects the plaques' engraved
    /// text — keeping the plaque + decal out of the SSR depth and the
    /// matching `scene_prev_texture` colour snapshot prevents the ghost
    /// text artefact that would otherwise appear in the table reflection
    /// directly below the plaque.
    ssr_prev_depth_texture: wgpu::Texture,
    ssr_prev_depth_view: wgpu::TextureView,
    quad_pipeline: wgpu::RenderPipeline,
    gradient_quad_pipeline: wgpu::RenderPipeline,
    /// 3D billboarded flame particle pipeline (replaces the legacy 2D
    /// additive-quad flame when smoke is Off). See
    /// [`crate::render::flame_particles`] and `shaders/flame.wgsl`.
    flame_pipeline: wgpu::RenderPipeline,
    /// Per-frame camera matrices uploaded for the flame vertex shader.
    /// Structurally identical to the view_proj + view_pos portion of
    /// `SsrGlobals`, but with a vertex-visible binding.
    flame_view_buffer: wgpu::Buffer,
    flame_view_bind_group: wgpu::BindGroup,
    /// CPU pool backing the 3D flame pipeline. Owned by the renderer so it
    /// persists across frames (the scene just supplies per-candle
    /// emitters each draw).
    flame_particles: crate::render::flame_particles::FlameParticleSystem,
    /// Reusable staging vec for uploading live particles to the GPU each
    /// frame. Kept here to avoid per-frame allocations.
    flame_particle_staging: Vec<crate::render::flame_particles::GpuFlameParticle>,
    starfield_pipeline: wgpu::RenderPipeline,
    ember_drift_pipeline: wgpu::RenderPipeline,
    golden_dust_pipeline: wgpu::RenderPipeline,
    moonlit_water_pipeline: wgpu::RenderPipeline,
    // Owns the GPU resource that `moon_albedo_bind_group` samples from.
    moon_albedo_bind_group: wgpu::BindGroup,
    sunlit_water_pipeline: wgpu::RenderPipeline,
    mountain_haze_pipeline: wgpu::RenderPipeline,
    haze_uniform_buffer: wgpu::Buffer,
    haze_uniform_bind_group: wgpu::BindGroup,
    /// Expensive shooting-star cascade transition renders into a half-res
    /// offscreen target to keep costs bounded at large resolutions; these
    /// fields own that target plus the two pipelines involved.
    shooting_star_cascade_pipeline: wgpu::RenderPipeline,
    cascade_composite_pipeline: wgpu::RenderPipeline,
    cascade_composite_layout: wgpu::BindGroupLayout,
    cascade_composite_sampler: wgpu::Sampler,
    cascade_offscreen_texture: wgpu::Texture,
    cascade_offscreen_view: wgpu::TextureView,
    cascade_composite_bind_group: wgpu::BindGroup,
    tile_pipeline: wgpu::RenderPipeline,
    /// Gold-metal "shell" pipeline used to draw a 3D outline behind each
    /// selected hand tile. Same vertex layout / bind group layout as
    /// `tile_pipeline`, but with front-face culling so only the back of
    /// the inflated shell shows around the tile silhouette, and a fragment
    /// shader that outputs polished gold lit by the candle point lights.
    tile_outline_pipeline: wgpu::RenderPipeline,
    /// Additive radial glow drawn behind selected tiles. A soft elliptical
    /// halo in warm gold that spills out past the tile silhouette and
    /// pulses gently with the candlelight rhythm.
    tile_glow_pipeline: wgpu::RenderPipeline,
    globals_buffer: wgpu::Buffer,
    globals_bind_group: wgpu::BindGroup,
    tile_material_layout: wgpu::BindGroupLayout,
    /// Per-frame point-light array uploaded to the tile pipeline (group 1).
    point_lights_buffer: wgpu::Buffer,
    tile_occluders_buffer: wgpu::Buffer,
    point_lights_bind_group: wgpu::BindGroup,
    /// Per-frame spotlight array uploaded to the tile pipeline (group 3).
    /// Only the tile pipeline binds this — lit_mesh / smoke do not.
    spot_lights_buffer: wgpu::Buffer,
    spot_lights_bind_group: wgpu::BindGroup,
    tile_sampler: wgpu::Sampler,
    /// Per-primitive GPU resources for the tile mesh (one entry per glTF
    /// primitive, e.g. ivory face + bamboo body).
    tile_primitives: Vec<TilePrimitiveGpu>,
    /// Identity factor used by every primitive (kept for the cam uniform).
    tile_base_color_factor: [f32; 4],
    /// Active tileset directory name (e.g. `"original"`). When `Some`, tile
    /// decals are loaded from `assets/sets/<name>/` instead of rasterized.
    tile_set: Option<String>,
    /// Per-hand-tile GPU resources; kept in sync with the hand via `update_hand_tiles`.
    hand_tiles: Vec<HandTileGpu>,
    /// Per-showcase-tile GPU resources (pack celebration, etc.). Grown on
    /// demand up to `MAX_SHOWCASE_TILE_SLOTS`; decals re-rasterised only
    /// when the tile identity changes.
    showcase_tiles: Vec<ShowcaseTileGpu>,
    /// Cached 2D tile-face overlays keyed by tile identity.
    tile_face_overlays:
        HashMap<(Suit, u8, Option<crate::core::tile::TileEnhancement>, bool), TileFaceOverlayGpu>,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    // --- Text overlay pipeline ---
    text_pipeline: wgpu::RenderPipeline,
    text_bind_group_layout: wgpu::BindGroupLayout,
    // --- Image quad pipeline (full-colour textures for relic icons) ---
    image_pipeline: wgpu::RenderPipeline,
    ui_font: Option<fontdue::Font>,
    emoji_font: Option<fontdue::Font>,
    pub size: winit::dpi::PhysicalSize<u32>,
    /// Last focused tile index — used to detect focus changes.
    last_focus: usize,
    /// When the focused tile changed: (slot_index, start_time). Drives the 360° spin.
    focus_spin: Option<(usize, Instant)>,
    /// Per-tile focus blend factor (0.0 = unfocused, 1.0 = focused). Lerped each frame.
    focus_t: Vec<f32>,
    /// Per-tile Y animation offset (positive = below rest position). Lerped toward 0 each frame.
    tile_anim_y: Vec<f32>,
    /// Per-tile X animation offset (in slot-width units). Used for sort shuffle animations.
    tile_anim_x: Vec<f32>,
    /// Per-tile unique id — used to detect when a tile slot changes identity.
    tile_uids: Vec<u32>,
    /// Tiles currently animating away (discard/score). Each entry: (HandTileGpu data, slot rect, velocity, elapsed time).
    departing_tiles: Vec<DepartingTile>,
    /// Hand slots from the previous frame, used for departure animation positioning.
    prev_hand_slots: Vec<(f32, f32, f32, f32)>,
    /// All per-frame projected screen-space rects for 3D elements.
    pub proj: ProjectionCache,
    /// Per-hand-tile world-space model matrices captured at the end of the
    /// previous frame. Combined with `last_pick_camera`, these let
    /// `pick_hand_tile` cast a world ray through the cursor and intersect it
    /// with each tile's OBB (the normalized mesh's local AABB transformed by
    /// its model matrix). Indexed by hand position; one frame stale.
    pub(super) last_pick_models: Vec<(usize, Mat4)>,
    /// Camera state captured at the end of the previous frame, used by
    /// `pick_hand_tile` to unproject the cursor into a world-space ray.
    pub(super) last_pick_camera: Option<PickCamera>,
    /// Timestamp of the previous frame — used to compute delta time for lerping.
    last_frame: Instant,
    /// Delta time of the *current* frame in seconds (set early in
    /// `draw_frame` so per-instance prep loops further down can ease
    /// animation envelopes without recomputing the timestamp).
    frame_dt: f32,
    /// Per-object smoothed hover envelopes, keyed by [`Object3d::anim_id`].
    ///
    /// Each entry eases toward the per-frame `hover_target` at rate ≈ 14
    /// (≈ 70 ms time constant) so lift/tilt animations run in both directions
    /// instead of snapping. Entries are created on first use and never removed
    /// (the map stays tiny — one entry per interactive 3D object in the game).
    obj3d_hover_state: HashMap<u64, f32>,
    /// Creation time — used as a stable reference for cyclic animations.
    creation_time: Instant,
    /// Cached relic icon textures, populated asynchronously from the loader thread.
    relic_textures: HashMap<RelicId, RelicTextureGpu>,
    /// Receives decoded relic RGBA data from the background loader thread.
    relic_rx: Option<mpsc::Receiver<DecodedRelicImage>>,
    /// Wall-clock start of the relic load pipeline (spawn → last GPU upload).
    relic_load_start: Option<Instant>,
    /// Cached tile-pack box art textures, keyed by `TilePackKind`.
    pack_textures: HashMap<TilePackKind, RelicTextureGpu>,
    /// Cached background textures, populated asynchronously.
    background_textures: HashMap<BackgroundId, BackgroundTextureGpu>,
    /// Receives decoded background image data from the background loader thread.
    background_rx: Option<mpsc::Receiver<DecodedBackgroundImage>>,
    /// Wall-clock start of the background load pipeline (spawn → last GPU upload).
    background_load_start: Option<Instant>,
    /// GPU fluid simulation for atmospheric smoke effects (None if compute unsupported).
    pub fluid: Option<super::fluid::FluidSim>,
    /// Whether the fluid sim's render bind group must be (re)built before the
    /// next draw — set true at startup and on every depth-texture recreation.
    fluid_render_bg_dirty: bool,
    /// Per-hand-tile last-known world position keyed by tile uid. Used to
    /// compute per-frame velocity → smoke impulse so moving tiles disturb
    /// the volumetric smoke.
    prev_tile_world: HashMap<u32, glam::Vec3>,
    /// Last cursor world position (table-plane intersection) for cursor-driven
    /// smoke wind impulses.
    prev_cursor_world: Option<glam::Vec3>,
    /// Last cursor screen position. Used to gate smoke emission on actual
    /// pointer motion — otherwise a static cursor over an orbiting/swaying
    /// camera would emit continuous puffs as the unprojected world hit drifts.
    prev_cursor_screen: Option<(f32, f32)>,

    // ── Procedural lit meshes (candles + wood table) ────────────────────
    /// Bind-group layout shared by every lit-mesh instance.
    lit_mesh_material_layout: wgpu::BindGroupLayout,
    /// Bind-group layout for the lit-mesh SSR group (group 3): scene
    /// colour history + depth + SSR globals uniform.
    lit_mesh_ssr_layout: wgpu::BindGroupLayout,
    /// Frame-shared SSR uniform (camera matrices + toggle + tuning).
    lit_mesh_ssr_buffer: wgpu::Buffer,
    /// Frame-shared SSR bind group bound as group 3 on every lit_mesh
    /// draw. Recreated on resize whenever the scene-history texture or
    /// depth-copy texture is reallocated.
    lit_mesh_ssr_bind_group: wgpu::BindGroup,
    /// Sampler used by the SSR pass for both the scene-history colour
    /// texture and the depth snapshot.
    lit_mesh_ssr_sampler: wgpu::Sampler,
    /// Snapshot of the previous frame's swapchain colour. Read by the
    /// lacquered floor as the SSR source.
    scene_prev_texture: wgpu::Texture,
    scene_prev_view: wgpu::TextureView,
    /// Current frame's full scene color, rendered offscreen before bloom
    /// and final composite into the swapchain.
    scene_color_texture: wgpu::Texture,
    scene_color_view: wgpu::TextureView,
    bloom_extract_pipeline: wgpu::RenderPipeline,
    bloom_blur_pipeline: wgpu::RenderPipeline,
    bloom_composite_pipeline: wgpu::RenderPipeline,
    bloom_bind_group_layout: wgpu::BindGroupLayout,
    bloom_composite_bind_group_layout: wgpu::BindGroupLayout,
    bloom_params_buffer: wgpu::Buffer,
    bloom_sampler: wgpu::Sampler,
    bloom_scene_bind_group: wgpu::BindGroup,
    bloom_ping_bind_group: wgpu::BindGroup,
    bloom_pong_bind_group: wgpu::BindGroup,
    bloom_composite_bind_group: wgpu::BindGroup,
    bloom_ping_texture: wgpu::Texture,
    bloom_ping_view: wgpu::TextureView,
    bloom_pong_texture: wgpu::Texture,
    bloom_pong_view: wgpu::TextureView,
    /// Pipeline for procedural scene props (candles, table). Shares the
    /// `point_lights_layout` (group 1) with the tile pipeline.
    lit_mesh_pipeline: wgpu::RenderPipeline,
    /// Sibling pipeline for translucent lit_mesh draws (e.g. bug motion-blur
    /// ghost trails). Alpha-blended, no depth write — same shader and bind
    /// group layouts as `lit_mesh_pipeline`.
    lit_mesh_blended_pipeline: wgpu::RenderPipeline,
    /// 1×1 white texture used as a placeholder albedo for procedural meshes
    /// that don't sample from a texture.
    lit_mesh_white_view: wgpu::TextureView,
    /// Default `relief_tex` for lit meshes without a per-asset height map.
    lit_mesh_relief_default_view: wgpu::TextureView,
    /// Per-kind procedural heightmap textures for talisman tablets. Indexed
    /// by `TalismanKind::all()` order (see `talisman_height_paths` in `new`).
    /// The talisman shader branch samples these as a
    /// grayscale heightfield to perturb the surface normal.
    talisman_height_views: Vec<wgpu::TextureView>,
    /// Which heightmap is currently bound per talisman slot. Used to skip
    /// rebinding when the kind hasn't changed since last frame. Indexed
    /// parallel with `talisman_instances`; `None` means the white fallback
    /// is still bound.
    talisman_slot_kind: Vec<Option<u8>>,
    /// Shared procedural meshes.
    candle_wax_mesh: LitMeshGpu,
    candle_wick_mesh: LitMeshGpu,
    table_mesh: LitMeshGpu,
    /// Round dish mesh — retained while the sell tray still lives on a
    /// bespoke `LitMeshGpu` rather than the primitive registry. Drop
    /// this field once `Object3dKind::SellTray` migrates to
    /// `Primitive { shape: DiscRound, … }`.
    round_dish_mesh: LitMeshGpu,
    /// Folded "tent card" mesh sat on the sell-tray floor when focused; carries
    /// a "SELL" decal on each side via `sell_card_instance`.
    sell_card_mesh: LitMeshGpu,
    relic_box_mesh: LitMeshGpu,
    /// Unit box for tile booster packs (correct UVs per face; avoids the relic
    /// cylinder's repeated side strips).
    pack_mesh: LitMeshGpu,
    /// Per-relic silhouette-derived meshes generated from the loaded relic
    /// texture alpha. Falls back to `relic_box_mesh` when no usable silhouette
    /// can be derived.
    relic_meshes: HashMap<RelicId, LitMeshGpu>,
    /// CPU-side triangle lists for the fallback relic box, used by the
    /// trimesh ray-picker when a relic's per-ID mesh isn't loaded yet.
    /// Built once at renderer init from `build_relic_mesh()`.
    pub(super) relic_box_tris: Vec<[glam::Vec3; 3]>,
    /// CPU-side triangle lists per relic, extracted from the same CPU mesh
    /// used to build `relic_meshes`. Drives per-triangle trimesh picking so
    /// the click silhouette matches the visible relic outline instead of a
    /// loose AABB slab.
    pub(super) relic_tri_lists: HashMap<RelicId, Vec<[glam::Vec3; 3]>>,
    /// Pre-allocated per-candle uniform buffers + bind groups (one per
    /// primitive). Indexed by candle slot, then 0=wax/1=wick.
    candle_instances: Vec<[LitMeshInstance; 2]>,
    /// Single uniform buffer + bind group for the gameplay-scene table.
    table_instance: LitMeshInstance,
    /// Pre-allocated per-relic-placeholder instances. Sized at startup to
    /// match `MAX_RELIC_SLOTS`; indexed by placement order each frame.
    relic_instances: Vec<LitMeshInstance>,
    /// Per-relic-placeholder (world-space model matrix, relic id) captured
    /// each frame for `pick_shop_object` raycasting and the bulk screen-rect
    /// reprojection. The relic id drives per-triangle trimesh picking so the
    /// click silhouette matches the visible relic outline.
    pub(super) last_relic_models: Vec<(Mat4, RelicId)>,
    /// Per-relic (pick_id, model, relic_id) snapshot for scenes that want
    /// to route clicks to a specific artifact index instead of the
    /// positional order used by `last_relic_models`. Populated from any
    /// `Object3dKind::Relic` draw whose `pick_id` is `Some`. Consumed by
    /// `pick_collection_object`.
    pub(super) last_pickable_relic_models: Vec<(u32, Mat4, RelicId)>,
    /// Currently bound relic texture per slot. `Some(id)` means that slot's
    /// bind group already points at the texture for `id`; `None` means the
    /// flat-white fallback. Avoids rebuilding bind groups every frame.
    relic_slot_texture: Vec<Option<RelicId>>,
    /// Pre-allocated per-pack instances (lit-mesh foil; uses `pack_mesh` geometry).
    pack_instances: Vec<LitMeshInstance>,
    pack_slot_texture: Vec<Option<TilePackKind>>,
    // ── Shop scene meshes (curio cabinet + ribbons + talismans) ─
    ribbon_mesh: LitMeshGpu,
    talisman_mesh: LitMeshGpu,
    /// Procedural shrine mesh used by the pick-blind scene.
    shrine_mesh: LitMeshGpu,
    /// Procedural ornate brass plinth used by the gameplay scene to hold
    /// the dora indicator tile(s).
    dora_plinth_mesh: LitMeshGpu,
    /// Per-ribbon draw-slot instances (shop scene). Each textured ribbon uses
    /// up to 3 slots (top/mid/bot); untextured ribbons use 1. Truncated at
    /// `MAX_RIBBON_SLOTS`.
    ribbon_instances: Vec<LitMeshInstance>,
    /// Currently bound zodiac texture per ribbon slot. `Some((zodiac_idx, part))`
    /// where part is 0=top, 1=mid, 2=bot. `None` means the flat-white
    /// fallback is bound. Used to skip redundant bind-group rebuilds.
    ribbon_slot_zodiac: Vec<Option<(u8, u8)>>,
    /// Three-part zodiac silk textures (top/mid/bot per zodiac).
    ribbon_zodiac_tex: crate::render::texture_upload::ZodiacRibbonTextures,
    /// Per-talisman instances (shop scene). Indexed sequentially by
    /// `TalismanBatch` placement order; truncated at `MAX_TALISMAN_SLOTS`.
    talisman_instances: Vec<LitMeshInstance>,
    /// Single instance for SellTray.
    sell_tray_instance: LitMeshInstance,
    /// Single instance for the folded "SELL" tent card; only drawn when the
    /// sell tray is focused.
    sell_card_instance: LitMeshInstance,
    /// Whether the SELL decal texture has been rasterized + uploaded.
    sell_card_decal_ready: bool,
    /// Last-frame model matrix for the SELL card; `Some` triggers the draw.
    /// Cleared each frame and re-set by the `SellTray` Object3d branch when
    /// the tray is focused.
    last_sell_card_model: Option<Mat4>,
    /// Brass pole + conical shade mesh for the shop lamp (Metal material).
    lamp_body_mesh: LitMeshGpu,
    /// CPU-side triangle list for the lamp body, reused for trimesh-accurate
    /// arrange-mode picking so the hit region traces the cord + shade cone
    /// instead of the full [lamp_w, lamp_h, lamp_w] bounding box.
    pub(super) lamp_body_tris: Vec<[glam::Vec3; 3]>,
    /// Tight local-space AABB half-extents + center-Y offset for the lamp
    /// body trimesh. Used by name-lookup consumers (arrange wireframe,
    /// `debug_object_origin`) that still want a boxy approximation for
    /// visualization or anchor-point queries.
    lamp_body_local_half: glam::Vec3,
    lamp_body_local_center_y: f32,
    /// Glass bulb mesh for the shop lamp (Glass material).
    lamp_bulb_mesh: LitMeshGpu,
    /// Single instance for the lamp body.
    lamp_body_instance: LitMeshInstance,
    /// Single instance for the lamp bulb.
    lamp_bulb_instance: LitMeshInstance,
    /// Chitin ellipsoid body mesh for hovering insects near the lamp.
    bug_body_mesh: LitMeshGpu,
    /// Flat wing-pair mesh for hovering insects.
    bug_wing_mesh: LitMeshGpu,
    /// Per-bug body instance slots.
    bug_body_instances: Vec<LitMeshInstance>,
    /// Per-bug left-wing instance slots. Drawn with identity Y so the
    /// wing mesh (which lives in +Y half-space) ends up on the bug's
    /// +Y side. A per-frame flap rotation about mesh +X is baked into
    /// the model matrix uploaded to this instance.
    bug_wing_instances: Vec<LitMeshInstance>,
    /// Per-bug right-wing instance slots. Shares `bug_wing_mesh` with
    /// the left wing but the model matrix flips Y (mirror) and applies
    /// the opposite flap rotation, so the two wings counter-sweep the
    /// way a real moth's do.
    bug_wing_r_instances: Vec<LitMeshInstance>,
    /// Swept-fan motion-blur surrogate mesh for the flapping wing. The
    /// mesh itself is the volume the wing sweeps through during a full
    /// stroke, so no per-frame flap rotation is applied at draw time —
    /// only the alpha is modulated by the wing's angular speed.
    bug_wing_blur_mesh: LitMeshGpu,
    /// Per-bug left-wing blur-fan instance slots. Drawn with the bug's
    /// body model matrix (no Y-mirror, no flap rotation) through the
    /// alpha-blended pipeline.
    bug_wing_blur_instances: Vec<LitMeshInstance>,
    /// Per-bug right-wing blur-fan instance slots. Shares
    /// `bug_wing_blur_mesh` with the left wing; the model matrix applies
    /// the Y-mirror transform so the same mesh serves both sides.
    bug_wing_blur_r_instances: Vec<LitMeshInstance>,
    /// Unit sphere mesh shared by every material-preview orb. The scene
    /// supplies the per-instance `MaterialParams` so a single mesh previews
    /// every `MaterialKind`.
    orb_mesh: LitMeshGpu,
    /// Per-orb instances for the material viewer scene. Bound with the 1×1
    /// default albedo and relief textures — materials that sample heightmaps
    /// render flat, previewing the shading model rather than any per-asset
    /// heightmap.
    orb_instances: Vec<LitMeshInstance>,
    /// Sell tray model + pick_id, one-frame-stale.
    pub(super) last_sell_tray_model: Option<(Mat4, u32)>,
    /// Per-shrine instances (pick-blind scene). Indexed sequentially by
    /// `ShrineBatch` placement order; truncated at `MAX_SHRINE_SLOTS`.
    shrine_instances: Vec<LitMeshInstance>,
    /// Per-dora-plinth instances (gameplay scene). Truncated at
    /// `MAX_DORA_PLINTH_SLOTS`. The gameplay scene only ever pushes one
    /// plinth per frame, but the slot pool tolerates more without allocation.
    dora_plinth_instances: Vec<LitMeshInstance>,
    /// Per-ribbon world-space model matrices for `pick_shop_object`.
    pub(super) last_ribbon_models: Vec<Mat4>,
    /// Total number of ribbon draw-slots populated this frame (across all
    /// `ZodiacBatch` cmds). Used by the shadow pass.
    last_ribbon_slot_count: usize,
    /// Per-batch ribbon slot counts: `last_ribbon_batch_slot_counts[batch_idx]`
    /// is how many draw-slots that batch consumed (2-3 per textured ribbon,
    /// 1 per untextured).
    last_ribbon_batch_slot_counts: Vec<usize>,
    /// Per-talisman world-space model matrices for `pick_shop_object`.
    pub(super) last_talisman_models: Vec<Mat4>,
    /// World-space `(center, half_extents)` parallel with
    /// `last_aux_dish_rects`, used by `pick_shop_object` for AABB raycasts.
    pub(super) last_aux_dish_aabbs: Vec<(glam::Vec3, glam::Vec3)>,

    // ── Skeuomorphic gameplay HUD meshes (phase 1 infrastructure) ──────
    bone_tablet_mesh: LitMeshGpu,
    wood_tablet_mesh: LitMeshGpu,
    bowl_mesh: LitMeshGpu,
    mirror_mesh: LitMeshGpu,
    tally_stick_base_mesh: LitMeshGpu,
    tally_stick_tip_mesh: LitMeshGpu,
    yaku_tablet_instances: Vec<LitMeshInstance>,
    wood_tablet_instances: Vec<LitMeshInstance>,
    bowl_instances: Vec<LitMeshInstance>,
    mirror_instances: Vec<LitMeshInstance>,
    /// Per-stick instances for the tally-counter fans. Each visible stick
    /// consumes two slots: one for the bone-colored base and one for the
    /// colored tip cap that rides the top fraction of the stick.
    tally_stick_instances: Vec<LitMeshInstance>,
    /// Per-wall-tile instances for the back-of-table facedown stack. Reuses
    /// `bone_tablet_mesh` for phase 1 (a plain box) — phase 7 may swap to the
    /// real tile mesh.
    wall_tile_instances: Vec<LitMeshInstance>,
    /// Per-cascade-token instances. Reuses `bone_tablet_mesh` (geometry)
    /// but the instances are kept in a dedicated pool so the cascade pulse
    /// scaling doesn't compete with the yaku tablet pool.
    cascade_token_instances: Vec<LitMeshInstance>,
    /// Per-extruded-glyph-popup instances. Each slot owns its own uniform
    /// buffer + bind group; the *mesh* it renders against is fetched from
    /// `extruded_glyph_meshes` per draw because each label has a different
    /// vertex/index buffer.
    extruded_glyph_instances: Vec<LitMeshInstance>,
    /// Lazy cache of GPU-uploaded glyph meshes keyed by label string. The
    /// CPU-side `GlyphMeshCache` (font + tessellation) lives next to it so
    /// the renderer can build a mesh on first sight of a new label string
    /// and reuse it on every subsequent frame the same string appears.
    glyph_cpu_cache: crate::render::glyph_mesh::GlyphMeshCache,
    extruded_glyph_meshes: HashMap<String, LitMeshGpu>,
    /// Shape registry for `Object3dKind::Primitive`. During the Phase-1
    /// migration, entries share GPU allocations with the legacy named
    /// fields (`plaque_mesh`, `cabinet_mesh`, …) via `Arc`. Once a
    /// legacy kind is deleted, the registry entry becomes the sole
    /// owner.
    primitive_meshes: HashMap<crate::render::primitive::MeshId, std::sync::Arc<LitMeshGpu>>,
    /// Per-shape instance pools for `Object3dKind::Primitive`. Keyed by
    /// `MeshId`; each `Vec` grows on-demand via `ensure_lit_mesh_pool`.
    primitive_instances: HashMap<crate::render::primitive::MeshId, Vec<LitMeshInstance>>,
    /// Per-shape texture overrides for primitive instances. When a
    /// shape has an entry here, `dispatch_primitive` binds the
    /// specified albedo + relief textures at instance creation instead
    /// of the default white + flat relief. Used by meshes whose
    /// material samples a heightmap (e.g. engraved coin faces).
    primitive_textures:
        HashMap<crate::render::primitive::MeshId, (wgpu::TextureView, wgpu::TextureView)>,
    /// Per-pick-id model matrix snapshot for primitive hit-testing.
    pub(super) last_primitive_pick_models: HashMap<u32, Mat4>,
    /// Three reusable lit-mesh instances for the debug world-axes overlay
    /// (one per axis: 0 = X red, 1 = Y green, 2 = Z blue). Drawn through
    /// the shared `relic_box_mesh` unit cube; per-frame uniforms position
    /// and stretch each instance into a thin colored bar.
    debug_axes_instances: Vec<LitMeshInstance>,
    /// Per-yaku-tablet world-space model matrices for `pick_gameplay_object`.
    /// Parallel with `last_projected_yaku_tablet_rects`.
    pub(super) last_yaku_tablet_models: Vec<Mat4>,
    /// Per-wood-tablet world-space model matrices for `pick_gameplay_object`.
    /// Index 0 = sort suit, 1 = sort rank, 2 = play hand.
    pub(super) last_wood_tablet_models: Vec<Mat4>,
    /// Discard bowl world-space model matrix for `pick_gameplay_object`.
    pub(super) last_bowl_model: Option<Mat4>,
    /// Bronze mirror world-space model matrix for `pick_gameplay_object`.
    pub(super) last_mirror_model: Option<Mat4>,
    /// Per-frame catch-all of "what's this thing under the cursor" entries
    /// used by the debug "Object Hit Test" menu action. Each entry is a
    /// `(name, model, half_extents, center_offset_y)` tuple — the same
    /// local-space slab-test format the existing `pick_*_object` methods
    /// use, just with a human-readable name attached. Populated as the
    /// renderer walks the frame's draw cmds; consumed by
    /// `pick_debug_object`.
    pub(super) last_debug_pickables: Vec<(String, Mat4, glam::Vec3, f32)>,
    /// Trimesh pickables for objects whose tight silhouette differs enough
    /// from an AABB that the box feels wrong (lamp cord + cone shade).
    /// `(name, model, triangle_list_ref)` — triangles are stored per mesh
    /// in dedicated renderer fields (e.g. `lamp_body_tris`) and referenced
    /// here by index to avoid per-frame allocation.
    pub(super) last_debug_trimesh_pickables: Vec<(String, Mat4, TrimeshRef)>,
    /// Canonical scene-path prefix for the currently active scene — e.g.
    /// `"shop"` or `"gameplay"`. Set per-frame by `App` so the renderer can
    /// disambiguate shared mesh pipelines (e.g. `Object3dKind::Ofuda` is used
    /// by both shop and gameplay). Used by [`Self::scene_path`] helpers.
    active_scene_key: Option<&'static str>,
    /// When arrange mode is active, this override is applied to the matching
    /// object's model matrix each frame before GPU upload. Set from `App`
    /// via [`Self::set_arrange_override`].
    debug_arrange_override: Option<DebugArrangeOverride>,
    /// Committed rotations keyed by arrange_name. Populated each frame by
    /// `App` from the active scene's Placements so that every arrange-tagged
    /// draw picks up `rx_deg/ry_deg/rz_deg` without each construction site
    /// having to wire them through. Degrees, applied in Z→Y→X order, world
    /// space (left-multiplied onto the model's rotation+scale block).
    committed_arrange_rotations: std::collections::HashMap<String, [f32; 3]>,

    // ── Shadow mapping ─────────────────────────────────────────────────
    /// Fixed-size depth texture written by the shadow pre-pass and sampled
    /// by every 3D shader through `shadow_sample_bind_group`.
    shadow_map_view: wgpu::TextureView,
    /// Bind-group layout for per-caster uniforms (group 0 of the shadow
    /// pipeline). Each `LitMeshInstance` and `HandTileGpu` owns one bind
    /// group built against this layout.
    shadow_caster_layout: wgpu::BindGroupLayout,
    /// Frame-shared uniform: light_view_proj + (enabled, bias, texel size).
    shadow_globals_buffer: wgpu::Buffer,
    /// Frame-shared bind group bound as group 2 on every 3D draw in the
    /// main pass. Wraps the depth texture, comparison sampler, and
    /// `shadow_globals_buffer`.
    shadow_sample_bind_group: wgpu::BindGroup,
    /// Depth-only pipeline used for the shadow pre-pass. Both lit-mesh
    /// casters and hand tiles share this pipeline because both vertex
    /// layouts start with `position : vec3<f32>` at offset 0.
    shadow_pipeline: wgpu::RenderPipeline,
    /// Optional GPU timestamp profiler. Built once at startup; activated
    /// on demand from the Debug menu via `start_gpu_profile`.
    gpu_profiler: crate::render::gpu_profiler::GpuProfiler,
    /// When `Some`, the next `draw()` call copies the surface texture into
    /// a staging buffer right before `present()` and writes it as a PNG to
    /// this path. Cleared after the capture. Set via
    /// [`WgpuRenderer::queue_screenshot`].
    pending_screenshot: std::cell::Cell<Option<std::path::PathBuf>>,
}

/// One hit returned by `WgpuRenderer::pick_shop_object`. The renderer's pick
/// path tests against three categories: relic cuboids (RelicBatch), ribbons
/// (ZodiacBatch), and explicit dishes (DishExplicit). The shop scene further
/// partitions the relic/ribbon indices into for-sale vs owned by tracking
/// how many of each it pushed in the same frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShopHit {
    /// Index into the most recent flat list of `RelicPlacement`s pushed this
    /// frame (across all `RelicBatch` cmds).
    Relic(usize),
    /// Index into the most recent flat list of `ZodiacRibbonPlacement`s
    /// pushed this frame (across all `ZodiacBatch` cmds).
    Ribbon(usize),
    /// Index into the most recent flat list of `TalismanPlacement`s pushed
    /// this frame (across all `TalismanBatch` cmds).
    Talisman(usize),
    /// The auxiliary dish whose `pick_id` matched. The scene assigns ids
    /// when it pushes the dish (e.g. `1` for the relic dish, `2` for the
    /// coin dish).
    Dish(u32),
    /// Index into the most recent flat list of `TilePackPlacement`s pushed
    /// this frame (across all `TilePackBatch` cmds).
    TilePack(u32),
}

/// What 3D gameplay-scene object the cursor is over this frame.
///
/// Resolved by [`WgpuRenderer::pick_gameplay_object`] via per-class local
/// AABB raycasting against the previous frame's model matrices — the same
/// pattern as `pick_hand_tile` / `pick_shop_object`. The gameplay scene
/// uses this for hover state and the click-injection path uses it to
/// route mouse clicks to the right action without screen-space rect
/// projection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameplayPick {
    /// Index into the most recent `YakuTabletBatch` (hover only — yaku
    /// tablets aren't clickable, just informational).
    YakuTablet(usize),
    /// Index into the most recent `WoodTabletBatch` — 0 = sort suit,
    /// 1 = sort rank, 2 = yaku journal book. The play-hand action is no
    /// longer a wood tablet — it's now the bronze mirror, picked
    /// separately as `BronzeMirror`.
    WoodTablet(usize),
    /// The discard bowl. Click target = commit the selected discard.
    DiscardBowl,
    /// The bronze mirror. Click target = play the selected hand.
    BronzeMirror,
}

/// Maximum number of physical relic placeholders rendered in one batch. Must
/// match the size of the `relic_instances` slot pool below; the renderer
/// silently truncates batches longer than this.
pub const MAX_RELIC_SLOTS: usize = 128;
/// Maximum number of zodiac/talisman ribbon *draw slots* per frame (across all
/// `ZodiacBatch` cmds). Each textured ribbon uses up to 3 slots (top/mid/bot
/// caps), so 16 logical ribbons × 3 = 48. Truncated silently.
pub const MAX_RIBBON_SLOTS: usize = 48;
/// Maximum number of talisman tablets rendered per frame.
pub const MAX_TALISMAN_SLOTS: usize = 8;
/// Maximum number of 3D bugs (insects near the lamp) rendered per frame.
/// Each live bug consumes one slot for body + two for live wings + two for
/// blur-fan surrogates (L/R). The blur-fan slot pools share this same size.
pub const MAX_BUG_SLOTS: usize = 8;
/// Maximum number of material-preview orbs rendered per frame. Only the
/// material viewer debug scene uses these; 32 covers every `MaterialKind`
/// with room to grow.
pub const MAX_ORB_SLOTS: usize = 32;
/// Maximum number of explicit auxiliary dishes per frame (the shop uses 2:
/// the relic dish and the coin dish).
/// Maximum number of shrine instances per frame (pick-blind uses 3: Small,
/// Big, Boss). Truncated silently.
pub const MAX_SHRINE_SLOTS: usize = 4;
/// Maximum number of dora-plinth instances per frame (gameplay uses 1).
pub const MAX_DORA_PLINTH_SLOTS: usize = 2;
/// Maximum number of yaku tablets per frame (5 visible + headroom).
pub const MAX_YAKU_TABLET_SLOTS: usize = 12;
/// Maximum number of wood action tablets per frame (sort suit, sort rank, play).
pub const MAX_WOOD_TABLET_SLOTS: usize = 8;
/// Maximum number of bowls per frame (gameplay uses 1: discard).
pub const MAX_BOWL_SLOTS: usize = 2;
/// Maximum number of bronze mirrors per frame (gameplay uses 1: play hand).
pub const MAX_MIRROR_SLOTS: usize = 2;
/// Maximum number of tally fans per frame (gameplay uses 2: draws + discards).
pub const MAX_TALLY_FAN_SLOTS: usize = 2;
/// Maximum total number of tally sticks rendered per frame across all fans.
/// Each fan emits `count` base sticks plus `count` tip-cap overlays, so this
/// bound is on the sum of both.
pub const MAX_TALLY_STICK_SLOTS: usize = 32;
/// Maximum number of facedown wall tiles drawn at the back of the table.
pub const MAX_WALL_TILE_SLOTS: usize = 80;
/// Cascade scoring bone pool (modifier strip + structure tier preview batches).
pub use crate::render::gpu_types::MAX_CASCADE_TOKEN_SLOTS;
/// Maximum number of in-flight 3D extruded-glyph score popups. A single
/// cascade rarely fires more than 8-10 steps, so 32 is plenty for the
/// per-step popups plus the running-total readout that holds across the
/// final beat.
/// Score reel uses up to 2 × N_COLS slots (prev + current per spinning column)
/// plus popup labels. 48 gives headroom for reel overflow columns.
pub const MAX_EXTRUDED_GLYPH_SLOTS: usize = 80;

/// GPU + asset handles needed to build a single showcase-tile's per-tile
/// resources. Grouped so callers can pass one `&ShowcaseTileCtx` instead
/// of threading 9 separate handles through the call site.
impl WgpuRenderer {


    /// Queue a screenshot of the next presented frame. The renderer copies
    /// the surface texture into a staging buffer between `submit` and
    /// `present`, then maps + PNG-encodes it synchronously. Intended for
    /// the `screenshot` CLI subcommand; not for hot paths (the synchronous
    /// readback stalls the GPU pipeline).
    pub fn queue_screenshot(&self, path: std::path::PathBuf) {
        self.pending_screenshot.set(Some(path));
    }

    /// `true` while a screenshot is queued and waiting for a draw call
    /// to fulfill it. The capture-frame path in `App` polls this so it
    /// keeps requesting redraws (instead of exiting early) when the
    /// swapchain returns Outdated/Lost on the warmup frames and the
    /// draw early-returns before the screenshot block.


    /// Begin a GPU pass timing capture for the next `frames` frames. The
    /// debug menu binds this to the "Profile GPU…" entry. Results are
    /// emitted via `log::info!` once the capture finishes; if the adapter
    /// lacks `TIMESTAMP_QUERY` support a warning is logged instead.
    pub fn start_gpu_profile(&mut self, frames: u32) {
        self.gpu_profiler
            .start(frames, self.size.width, self.size.height);
    }

    pub fn set_active_scene(&mut self, key: Option<&'static str>) {
        self.active_scene_key = key;
    }

    /// Returns `true` while background asset loading (relic/background textures)
    /// is still in progress.
    pub fn is_loading(&self) -> bool {
        self.relic_rx.is_some() || self.background_rx.is_some()
    }

    fn relic_mesh_for(&self, relic_id: RelicId) -> &LitMeshGpu {
        self.relic_meshes
            .get(&relic_id)
            .unwrap_or(&self.relic_box_mesh)
    }

    /// Drain any decoded relic images from the background loader and upload them
    /// to the GPU.  Called once per frame; a no-op once all images are loaded.
    fn poll_relic_textures(&mut self) {
        let Some(ref rx) = self.relic_rx else { return };
        let mut finished = false;
        // Non-blocking drain: upload every image that's ready this frame.
        loop {
            match rx.try_recv() {
                Ok(img) => {
                    let mesh_source = img
                        .mesh_rgba
                        .as_deref()
                        .map(|rgba| (rgba, img.mesh_width, img.mesh_height))
                        .unwrap_or((&img.rgba, img.width, img.height));
                    if let Some(cpu) =
                        build_relic_mesh_from_rgba(mesh_source.0, mesh_source.1, mesh_source.2)
                    {
                        // Cache the CPU triangle list alongside the GPU mesh so
                        // `pick_collection_object` / `pick_shop_object` can do
                        // per-triangle ray casts against the real silhouette
                        // instead of a loose AABB slab.
                        let tris: Vec<[glam::Vec3; 3]> = cpu
                            .indices
                            .chunks_exact(3)
                            .map(|c| {
                                let a = cpu.vertices[c[0] as usize].position;
                                let b = cpu.vertices[c[1] as usize].position;
                                let d = cpu.vertices[c[2] as usize].position;
                                [
                                    glam::Vec3::from(a),
                                    glam::Vec3::from(b),
                                    glam::Vec3::from(d),
                                ]
                            })
                            .collect();
                        self.relic_tri_lists.insert(img.id, tris);
                        self.relic_meshes.insert(
                            img.id,
                            LitMeshGpu::new(
                                &self.device,
                                &cpu,
                                &format!("relic-mesh-{:?}", img.id),
                            ),
                        );
                    }
                    let (tex, view) = upload_rgba_texture(
                        &self.device,
                        &self.queue,
                        img.name,
                        &img.rgba,
                        img.width,
                        img.height,
                    );
                    let (relief_tex, relief_view) = upload_rgba_texture_linear(
                        &self.device,
                        &self.queue,
                        &format!("{}-relief", img.name),
                        &img.relief_rgba,
                        img.relief_width,
                        img.relief_height,
                    );
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(img.name),
                        layout: &self.text_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                            },
                        ],
                    });
                    self.relic_textures.insert(
                        img.id,
                        RelicTextureGpu {
                            view,
                            texture: tex,
                            bind_group,
                            relief_texture: Some(relief_tex),
                            relief_view,
                        },
                    );
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            let elapsed = self
                .relic_load_start
                .take()
                .map(|t| t.elapsed())
                .unwrap_or_default();
            log::info!(
                "all {} relic textures uploaded to GPU in {:?} (spawn → last upload)",
                self.relic_textures.len(),
                elapsed,
            );
            self.relic_rx = None; // drop the channel
        }
    }

    /// Drain any decoded background images from the loader and upload to GPU.
    fn poll_background_textures(&mut self) {
        let Some(ref rx) = self.background_rx else {
            return;
        };
        let mut finished = false;
        loop {
            match rx.try_recv() {
                Ok(img) => {
                    let label = format!("bg-{:?}", img.id);
                    let (_tex, view) = upload_rgba_texture(
                        &self.device,
                        &self.queue,
                        &label,
                        &img.rgba,
                        img.width,
                        img.height,
                    );
                    let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some(&label),
                        layout: &self.text_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureView(&view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&self.tile_sampler),
                            },
                        ],
                    });
                    self.background_textures
                        .insert(img.id, BackgroundTextureGpu { bind_group });
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    finished = true;
                    break;
                }
            }
        }
        if finished {
            let elapsed = self
                .background_load_start
                .take()
                .map(|t| t.elapsed())
                .unwrap_or_default();
            log::info!(
                "all {} background textures uploaded to GPU in {:?} (spawn → last upload)",
                self.background_textures.len(),
                elapsed,
            );
            self.background_rx = None;
        }
    }

    /// Returns true while any tile animation (spin or lift lerp) is still running.
    pub fn is_spinning(&self) -> bool {
        const SPIN_SECS: f32 = 0.4;
        let spin_active = if let Some((_slot, start)) = self.focus_spin {
            start.elapsed().as_secs_f32() < SPIN_SECS
        } else {
            false
        };
        // Also keep animating while any tile's focus_t hasn't settled.
        let lerp_active = self.focus_t.iter().enumerate().any(|(i, &ft)| {
            let target = if i == self.last_focus { 1.0 } else { 0.0 };
            (ft - target).abs() > 0.001
        });
        // Keep animating while any tile is sliding into position.
        let slide_active = self.tile_anim_y.iter().any(|&y| y.abs() > 0.5)
            || self.tile_anim_x.iter().any(|&x| x.abs() > 0.01);
        let departing_active = !self.departing_tiles.is_empty();
        spin_active
            || lerp_active
            || slide_active
            || departing_active
            || !self.hand_tiles.is_empty()
    }

    /// Per-hand-tile screen-space rects after the perspective projection,
    /// captured at the end of the previous frame. Indexed by hand position.
    /// Borrow the entire projection cache for bulk access (e.g. building
    /// `DrawCtx`).
    pub fn projections(&self) -> &ProjectionCache {
        &self.proj
    }

    /// Set (or clear) the arrange-mode model-matrix override. Called each frame
    /// from `App` when arrange mode has a selected object. Pass `None` to clear.
    pub fn set_arrange_override(&mut self, ov: Option<DebugArrangeOverride>) {
        self.debug_arrange_override = ov;
    }

    /// Set the ambient-dust floor density on the fluid sim, if present.
    /// 0.0 disables. See `FluidSim::set_dust_strength`.
    pub fn set_dust_strength(&mut self, v: f32) {
        if let Some(fluid) = self.fluid.as_mut() {
            fluid.set_dust_strength(v);
        }
    }

    /// Push art-direction knobs for the procedural mountain-haze shader
    /// into its uniform buffer. Called once per frame from `main.rs` so
    /// debug-overlay edits take effect immediately. The tuple is
    /// `(density, r, g, b, horizon_y, drift_speed)`.
    pub fn set_haze_tuning(
        &self,
        density: f32,
        r: f32,
        g: f32,
        b: f32,
        horizon_y: f32,
        drift_speed: f32,
    ) {
        let uniform = HazeUniform {
            color_density: [r, g, b, density.max(0.0)],
            params: [horizon_y.clamp(0.0, 1.0), drift_speed.max(0.0), 0.0, 0.0],
        };
        self.queue
            .write_buffer(&self.haze_uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }

    /// Replace the committed-rotation map (used to apply each Placement's
    /// `rx/ry/rz_deg` to its matching arrange-tagged draw). Called each frame
    /// from `App` with the active scene's entries.
    pub fn set_committed_arrange_rotations(
        &mut self,
        rotations: std::collections::HashMap<String, [f32; 3]>,
    ) {
        self.committed_arrange_rotations = rotations;
    }

    /// Build a fully-qualified arrange-mode path for the active scene.
    /// Returns `{scene}.{suffix}` when a scene is set, or just `suffix`
    /// otherwise. Used when the draw site doesn't statically know which
    /// scene is rendering.
    fn scene_path(&self, suffix: &str) -> String {
        match self.active_scene_key {
            Some(scene) => format!("{scene}.{suffix}"),
            None => suffix.to_string(),
        }
    }

    /// If an arrange override is active and `name` matches, apply the
    /// accumulated position and rotation deltas to `model` and return the
    /// modified matrix. The override is expressed as layout-pixel deltas so
    /// it remains layout-relative across window resizes:
    ///
    /// - Translation: `(delta_px, -delta_py, delta_lift)` in world space
    ///   (pixel_x maps 1:1 to world_x; pixel_y maps 1:1 to −world_y).
    /// - Rotation: a `Rz(delta_rz) * Rx(delta_rx)` matrix is left-multiplied
    ///   onto the original 3×3 rotation+scale block, so the delta rotates the
    ///   object in world space on top of whatever convention the placement uses.
    ///
    /// Returns the matrix unchanged if no override is set or the name doesn't
    /// match.
    fn apply_arrange_override(&self, name: &str, model: Mat4) -> Mat4 {
        // Fuse the committed rotation (from the Placement) with any staged
        // arrange-mode rotation delta into a single Euler-angle sum before
        // left-multiplying onto the model. This matters because rotations
        // don't commute: applying `R_delta * R_committed` separately would
        // visually jump at Enter-time (when the delta folds into committed
        // via Euler addition). Summing first keeps preview == commit.
        let committed = self.committed_arrange_rotations.get(name).copied();
        let staged = self
            .debug_arrange_override
            .as_ref()
            .filter(|ov| ov.name == name);
        let (rx, ry, rz) = {
            let [crx, cry, crz] = committed.unwrap_or([0.0, 0.0, 0.0]);
            let (drx, dry, drz) = staged
                .map(|ov| (ov.delta_rx_deg, ov.delta_ry_deg, ov.delta_rz_deg))
                .unwrap_or((0.0, 0.0, 0.0));
            (crx + drx, cry + dry, crz + drz)
        };
        let mut model = if rx != 0.0 || ry != 0.0 || rz != 0.0 {
            let r = Mat4::from_rotation_z(rz.to_radians())
                * Mat4::from_rotation_y(ry.to_radians())
                * Mat4::from_rotation_x(rx.to_radians());
            let t = model.w_axis.truncate();
            let nx = r.transform_vector3(model.x_axis.truncate());
            let ny = r.transform_vector3(model.y_axis.truncate());
            let nz = r.transform_vector3(model.z_axis.truncate());
            Mat4::from_cols(
                nx.extend(0.0),
                ny.extend(0.0),
                nz.extend(0.0),
                t.extend(1.0),
            )
        } else {
            model
        };
        // Translation delta (only applies while a delta is staged for this name).
        if let Some(ov) = staged {
            let dt = glam::Vec3::new(ov.delta_px, -ov.delta_py, ov.delta_lift);
            let t = model.w_axis.truncate() + dt;
            model = Mat4::from_cols(model.x_axis, model.y_axis, model.z_axis, t.extend(1.0));
        }
        model
    }

    /// Like [`Self::pick_debug_object`] but also returns the world-space model
    /// matrix of the closest hit. Used by arrange mode to seed the initial
    /// World-space translation of the pickable registered under `name` in
    /// the most recent frame. `None` if the name isn't currently pickable.
    pub fn debug_object_origin(&self, name: &str) -> Option<glam::Vec3> {
        if let Some((_, m, _, _)) = self
            .last_debug_pickables
            .iter()
            .find(|(n, _, _, _)| n == name)
        {
            return Some(m.transform_point3(glam::Vec3::ZERO));
        }
        self.last_debug_trimesh_pickables
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, m, _)| m.transform_point3(glam::Vec3::ZERO))
    }



    /// Clear the volumetric smoke field and reset per-tile velocity tracking
    /// so the next scene starts with a clean atmosphere.
    pub fn clear_smoke(&mut self) {
        if let Some(ref mut fluid) = self.fluid {
            fluid.clear();
        }
        self.prev_tile_world.clear();
        self.prev_cursor_world = None;
        self.prev_cursor_screen = None;
    }

    /// Render one frame.
    ///
    /// `frame.cmds` is walked in order — earlier cmds render under later ones.
    /// Contiguous runs of `DrawCmd::Quad` are batched into a single instanced
    /// draw, which is invisible to scenes and preserves ordering.


    /// Encode a copy of the swapchain texture into a freshly-allocated
    /// staging buffer using the active encoder. Returns the buffer + the
    /// dimensions/padded-bytes-per-row needed to decode it.
    fn encode_screenshot_copy(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        tex: &wgpu::Texture,
        _path: &std::path::Path,
    ) -> ScreenshotStaging {
        let width = tex.width();
        let height = tex.height();
        // wgpu requires bytes_per_row to be a multiple of 256.
        let bytes_per_pixel: u32 = 4; // BGRA8 / RGBA8 — 4 bytes
        let unpadded_bytes_per_row = width * bytes_per_pixel;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(256) * 256;
        let buffer_size = (padded_bytes_per_row as u64) * (height as u64);

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("screenshot-staging"),
            size: buffer_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        ScreenshotStaging {
            buffer,
            width,
            height,
            padded_bytes_per_row,
            unpadded_bytes_per_row,
            format: tex.format(),
        }
    }

    /// Map the staging buffer, decode pixels (handling BGRA→RGBA + row
    /// stride), and write the PNG. Synchronous: blocks on `device.poll`.
    fn finalize_screenshot(
        &self,
        staging: ScreenshotStaging,
        path: &std::path::Path,
    ) -> anyhow::Result<()> {
        let slice = staging.buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = sender.send(result);
        });
        // Block until the GPU finishes the copy.
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        receiver.recv()??;

        let data = slice.get_mapped_range();

        // Strip row padding and (if needed) swap BGRA → RGBA.
        let w = staging.width as usize;
        let h = staging.height as usize;
        let unpadded = staging.unpadded_bytes_per_row as usize;
        let padded = staging.padded_bytes_per_row as usize;
        let mut pixels: Vec<u8> = Vec::with_capacity(w * h * 4);
        let swap_bgra = matches!(
            staging.format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        for row in 0..h {
            let row_start = row * padded;
            let row_end = row_start + unpadded;
            let row_pixels = &data[row_start..row_end];
            if swap_bgra {
                for chunk in row_pixels.chunks_exact(4) {
                    pixels.push(chunk[2]);
                    pixels.push(chunk[1]);
                    pixels.push(chunk[0]);
                    pixels.push(chunk[3]);
                }
            } else {
                pixels.extend_from_slice(row_pixels);
            }
        }
        drop(data);
        staging.buffer.unmap();

        let img = image::RgbaImage::from_raw(staging.width, staging.height, pixels)
            .ok_or_else(|| anyhow::anyhow!("RgbaImage::from_raw failed"))?;
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        img.save(path)?;
        Ok(())
    }
}

/// Per-screenshot staging buffer + the metadata needed to decode it.
struct ScreenshotStaging {
    buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
    unpadded_bytes_per_row: u32,
    format: wgpu::TextureFormat,
}

// ---------------------------------------------------------------------------
// Instance builders
// ---------------------------------------------------------------------------

/// Build GPU instances for the score panel and modifier strip.
///
/// Hand tiles are now 3D meshes, so this function no longer generates hand
/// slot quads.
pub fn build_instances_from_layout(
    score: (f32, f32, f32, f32),
    _modifier: (f32, f32, f32, f32),
    _anim_scale_score: f32,
    plays: u32,
    plays_max: u32,
    discards: u32,
    discards_max: u32,
) -> Vec<GpuInstance> {
    use crate::render::theme::color as themec;

    // The score cartouche backplane is replaced by the hanging wooden plaque
    // (`DrawCmd::Plaque`) pushed by the gameplay scene. This function now
    // only emits the plays/discards pip indicators that float at the right
    // edge of the score panel — phase 4 of the skeuomorphic UI redesign
    // replaces these with a physical peg block.
    let (sx, sy, sw, sh) = (score.0, score.1, score.2, score.3);
    let mut v: Vec<GpuInstance> = Vec::new();

    // Pip indicators — two stacked rows of jade/amber pills floating at the
    // right edge of the logical score-panel region (NOT over the cartouche).
    let pip = (sh * 0.22).clamp(8.0, 28.0);
    let gap = pip * 0.25;
    let margin = pip * 0.9;
    let row_gap = pip * 0.3;

    let total_h = pip + row_gap + pip;
    let row1_y = sy + (sh - total_h) * 0.5;
    let row2_y = row1_y + pip + row_gap;

    let plays_row_w = plays_max as f32 * pip + (plays_max.saturating_sub(1)) as f32 * gap;
    let plays_x0 = sx + sw - margin - plays_row_w;
    for i in 0..plays_max {
        let x = plays_x0 + i as f32 * (pip + gap);
        let filled = i < plays;
        v.push(GpuInstance {
            rect: [x, row1_y, pip, pip],
            color: if filled {
                themec::JADE
            } else {
                themec::alpha(themec::JADE, 0.25)
            },
        });
    }

    let disc_row_w = discards_max as f32 * pip + (discards_max.saturating_sub(1)) as f32 * gap;
    let disc_x0 = sx + sw - margin - disc_row_w;
    for i in 0..discards_max {
        let x = disc_x0 + i as f32 * (pip + gap);
        let filled = i < discards;
        v.push(GpuInstance {
            rect: [x, row2_y, pip, pip],
            color: if filled {
                themec::AMBER
            } else {
                themec::alpha(themec::AMBER, 0.25)
            },
        });
    }

    v
}
