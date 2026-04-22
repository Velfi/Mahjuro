//! WGPU: depth-tested 3D tile meshes for the hand + 2D UI quads on top.

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

/// Pre-loaded background texture + bind group for the image pipeline.
struct BackgroundTextureGpu {
    bind_group: wgpu::BindGroup,
}

/// Decoded background image data sent from the background loader thread.
struct DecodedBackgroundImage {
    id: BackgroundId,
    rgba: Vec<u8>,
    width: u32,
    height: u32,
}

fn relic_material_params(relic_id: RelicId, base_color: [f32; 4], glow: f32) -> MaterialParams {
    let visual = relic_visual(relic_id);
    let g = glow.clamp(0.0, 1.0);
    match visual.material {
        RelicRenderMaterial::Iron => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.42 + base_color[0] * 0.14,
                0.44 + base_color[1] * 0.14,
                0.48 + base_color[2] * 0.14,
                base_color[3],
            ],
            specular_strength: 0.38 + 0.18 * g,
            specular_power: 26.0,
        },
        RelicRenderMaterial::Copper => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.78 + base_color[0] * 0.16,
                0.46 + base_color[1] * 0.14,
                0.26 + base_color[2] * 0.10,
                base_color[3],
            ],
            specular_strength: 0.52 + 0.22 * g,
            specular_power: 34.0,
        },
        RelicRenderMaterial::Silver => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.82 + base_color[0] * 0.14,
                0.84 + base_color[1] * 0.14,
                0.88 + base_color[2] * 0.12,
                base_color[3],
            ],
            specular_strength: 0.78 + 0.22 * g,
            specular_power: 64.0,
        },
        RelicRenderMaterial::Gold => MaterialParams {
            kind: MaterialKind::Enamel,
            base_color: [
                0.94 + base_color[0] * 0.14,
                0.78 + base_color[1] * 0.14,
                0.28 + base_color[2] * 0.10,
                base_color[3],
            ],
            specular_strength: 0.88 + 0.24 * g,
            specular_power: 80.0,
        },
    }
}

// ---------------------------------------------------------------------------
// Texture helpers
// ---------------------------------------------------------------------------

fn upload_rgba_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn white_albedo(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture(
        device,
        queue,
        "tile-albedo-white",
        &[255, 255, 255, 255],
        1,
        1,
    )
}

/// 1×1 mid-gray linear texture — default `relief_tex` for lit meshes that
/// don't use a separate height map (enamel shader reads ~0.5 → flat relief).
fn flat_relief_height(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    upload_rgba_texture_linear(
        device,
        queue,
        "lit-relief-flat",
        &[128, 128, 128, 255],
        1,
        1,
    )
}

/// Same as `upload_rgba_texture` but allocates the texture in **linear**
/// (non-sRGB) format. Used for data textures like heightmaps where the
/// stored byte value is a raw scalar, not a perceptual color.
fn upload_rgba_texture_linear(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    label: &str,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * 4),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Decode the embedded coin face heightmap PNG and upload it as a linear
/// data texture. Falls back to a flat mid-gray 1×1 if the asset is missing
/// or fails to decode (so the coin still renders, just without engraving).
fn load_coin_heightmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    load_metal_heightmap(
        device,
        queue,
        "textures/coin_heightmap.png",
        "coin-heightmap",
    )
}

/// Decode the embedded bronze mirror heightmap PNG and upload it as a
/// linear data texture. Bound at slot 1 of every gameplay mirror instance;
/// the metal branch in lit_mesh.wgsl samples it as a heightfield to perturb
/// the polished face's surface normal so the cast four-spirit relief catches
/// the candle highlights. Same fallback behavior as the coin loader.
fn load_mirror_heightmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> (wgpu::Texture, wgpu::TextureView) {
    load_metal_heightmap(
        device,
        queue,
        "textures/mirror_heightmap.png",
        "mirror-heightmap",
    )
}

/// Shared body for the per-asset heightmap loaders. Reads `path` from the
/// embedded assets, decodes it, and uploads as a linear (non-sRGB) RGBA8
/// texture. Falls back to a flat mid-gray 1×1 on any failure so the
/// metal-perturbation branch degrades gracefully to a smooth surface.
fn load_metal_heightmap(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &str,
    label: &str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let flat_label: &'static str = match label {
        "coin-heightmap" => "coin-heightmap-flat",
        "mirror-heightmap" => "mirror-heightmap-flat",
        _ => "metal-heightmap-flat",
    };
    let bytes = match crate::asset_path::get(path) {
        Some(file) => file.data.to_vec(),
        None => {
            log::warn!("{label} asset missing at {path} — using flat fallback");
            return upload_rgba_texture_linear(
                device,
                queue,
                flat_label,
                &[128, 128, 128, 255],
                1,
                1,
            );
        }
    };
    match image::load_from_memory(&bytes) {
        Ok(img) => {
            let rgba = img.into_rgba8();
            let (w, h) = rgba.dimensions();
            upload_rgba_texture_linear(device, queue, label, &rgba.into_raw(), w, h)
        }
        Err(e) => {
            log::warn!("failed to decode {label}: {e} — using flat fallback");
            upload_rgba_texture_linear(device, queue, flat_label, &[128, 128, 128, 255], 1, 1)
        }
    }
}

/// Load the three-part zodiac silk ribbon textures (top/mid/bot per zodiac).
fn load_zodiac_ribbon_textures(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> crate::render::texture_upload::ZodiacRibbonTextures {
    crate::render::texture_upload::load_zodiac_ribbon_textures(device, queue)
}

/// Spawn a background thread that decodes all background PNGs and sends the RGBA
/// data back over a channel.
fn spawn_background_loader() -> mpsc::Receiver<DecodedBackgroundImage> {
    let (tx, rx) = mpsc::channel();

    let backgrounds: Vec<(BackgroundId, &'static str)> = [BackgroundId::Menu, BackgroundId::Score]
        .iter()
        .filter_map(|id| id.asset_path().map(|p| (*id, p)))
        .collect();

    std::thread::Builder::new()
        .name("bg-loader".into())
        .spawn(move || {
            let t_thread = Instant::now();
            let mut decoded = 0usize;
            let mut decode_time = std::time::Duration::ZERO;
            for (id, asset_path) in backgrounds {
                let bytes = match crate::asset_path::get(asset_path) {
                    Some(file) => file.data.to_vec(),
                    None => {
                        log::warn!("background image not found: {asset_path}");
                        continue;
                    }
                };
                let t_decode = Instant::now();
                let img = match image::load_from_memory(&bytes) {
                    Ok(img) => img.into_rgba8(),
                    Err(e) => {
                        log::warn!("failed to decode background {asset_path}: {e}");
                        continue;
                    }
                };
                decode_time += t_decode.elapsed();
                decoded += 1;
                let (w, h) = img.dimensions();
                let msg = DecodedBackgroundImage {
                    id,
                    rgba: img.into_raw(),
                    width: w,
                    height: h,
                };
                if tx.send(msg).is_err() {
                    break;
                }
            }
            log::info!(
                "bg-loader thread finished: decoded {decoded} images in {decode_time:?} (thread total {:?})",
                t_thread.elapsed(),
            );
        })
        .expect("failed to spawn bg-loader thread");

    rx
}

fn create_depth(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Snapshot of the previous frame's swapchain colour. Bound by the lacquered
/// floor as the source for screen-space reflections — the table is drawn
/// before the candles each frame, so it has to reflect *last* frame's
/// composited candles + flames + tiles. The camera is fixed, so the
/// one-frame stale image is essentially correct.
fn create_scene_prev(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene-prev"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn create_scene_color(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("scene-color"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Half-resolution offscreen target for the shooting-star cascade shader.
/// The shader is heavy per-pixel, so it runs at 1/2 × 1/2 = 1/4 pixel count
/// and is additively composited up to the main scene target.
fn create_cascade_offscreen(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    full_width: u32,
    full_height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let w = (full_width / 2).max(1);
    let h = (full_height / 2).max(1);
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cascade-offscreen"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn create_post_texture(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    width: u32,
    height: u32,
    label: &'static str,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// Sibling depth texture used as a sampleable snapshot of the scene depth
/// between the pre-smoke and post-smoke render passes.
fn create_depth_copy(
    device: &wgpu::Device,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth-copy"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

/// GPU + asset handles needed to build a single showcase-tile's per-tile
/// resources. Grouped so callers can pass one `&ShowcaseTileCtx` instead
/// of threading 9 separate handles through the call site.
#[derive(Copy, Clone)]
struct ShowcaseTileCtx<'a> {
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    layout: &'a wgpu::BindGroupLayout,
    shadow_caster_layout: &'a wgpu::BindGroupLayout,
    primitives: &'a [TilePrimitiveGpu],
    sampler: &'a wgpu::Sampler,
    ui_font: Option<&'a fontdue::Font>,
    emoji_font: Option<&'a fontdue::Font>,
}

fn make_showcase_tile_gpu(
    ctx: &ShowcaseTileCtx<'_>,
    base_color_factor: [f32; 4],
    tile: &Tile,
    tile_set: Option<&str>,
) -> ShowcaseTileGpu {
    let ShowcaseTileCtx {
        device,
        queue,
        layout,
        shadow_caster_layout,
        primitives,
        sampler,
        ui_font,
        emoji_font,
    } = *ctx;
    let identity = Mat4::IDENTITY;
    let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
            cam_pos: [0.0; 3],
            tile_seed: 0.0,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });

    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    // Use `true` (hand-tile quality) so hand-strip tiles get the same
    // full-resolution decal as the old HandTileGpu path did.
    let rgba =
        rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set, true);
    let (_decal_texture, decal_view) = upload_rgba_texture(
        device,
        queue,
        "showcase-tile-decal",
        &rgba,
        DECAL_W,
        DECAL_H,
    );

    let bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("showcase-tile-bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&prim.albedo_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&decal_view),
                    },
                ],
            })
        })
        .collect();

    // Outline shell — always allocated so the bind group is stable.
    let outline_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-outline-cam"),
        contents: bytemuck::bytes_of(&CameraUniform {
            view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
            base_color_factor,
            cam_pos: [0.0; 3],
            tile_seed: 0.0,
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let outline_bind_groups: Vec<wgpu::BindGroup> = primitives
        .iter()
        .map(|prim| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("showcase-tile-outline-bg"),
                layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: outline_uniform_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&prim.albedo_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::TextureView(&decal_view),
                    },
                ],
            })
        })
        .collect();

    let shadow_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("showcase-tile-shadow-uniform"),
        contents: bytemuck::bytes_of(&ShadowCasterUniform {
            light_view_proj: identity.to_cols_array(),
            model: identity.to_cols_array(),
        }),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("showcase-tile-shadow-bg"),
        layout: shadow_caster_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: shadow_uniform_buffer.as_entire_binding(),
        }],
    });

    ShowcaseTileGpu {
        uniform_buffer,
        bind_groups,
        outline_uniform_buffer,
        outline_bind_groups,
        shadow_uniform_buffer,
        shadow_bind_group,
        tile_id: (tile.suit, tile.rank, tile.enhancement, tile.debuffed_visual),
    }
}

fn make_tile_face_overlay_gpu(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    ui_font: Option<&fontdue::Font>,
    emoji_font: Option<&fontdue::Font>,
    tile: &Tile,
    tile_set: Option<&str>,
) -> TileFaceOverlayGpu {
    const DECAL_W: u32 = 192;
    const DECAL_H: u32 = 256;
    let rgba =
        rasterize_tile_face_decal(tile, ui_font, emoji_font, DECAL_W, DECAL_H, tile_set, false);
    let (texture, view) = upload_rgba_texture(
        device,
        queue,
        "tile-face-overlay",
        &rgba,
        DECAL_W,
        DECAL_H,
    );
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("tile-face-overlay-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    TileFaceOverlayGpu {
        _texture: texture,
        bind_group,
    }
}

// ---------------------------------------------------------------------------
// WgpuRenderer impl
// ---------------------------------------------------------------------------

impl WgpuRenderer {
    pub fn new(target_init: TargetInit) -> anyhow::Result<Self> {
        let t_total = Instant::now();
        let instance = wgpu::Instance::default();

        // Branch on target: the windowed path creates a Surface *before*
        // adapter selection (compatible_surface), then picks a format from
        // the surface caps. The headless path requests an adapter without
        // any surface and picks the format itself.
        let (surface_opt, size, hdr_enabled): (
            Option<wgpu::Surface<'static>>,
            winit::dpi::PhysicalSize<u32>,
            bool,
        ) = match &target_init {
            TargetInit::Windowed {
                window,
                hdr_enabled,
            } => {
                let size = window.inner_size();
                let surface = instance.create_surface(window.clone())?;
                (Some(surface), size, *hdr_enabled)
            }
            TargetInit::Headless {
                width,
                height,
                hdr_enabled,
            } => {
                let size = winit::dpi::PhysicalSize::new((*width).max(1), (*height).max(1));
                (None, size, *hdr_enabled)
            }
        };

        let t0 = Instant::now();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::default(),
            compatible_surface: surface_opt.as_ref(),
            force_fallback_adapter: false,
        }))
        .map_err(|e| anyhow::anyhow!("adapter: {e:?}"))?;
        log::info!("wgpu adapter acquired in {:?}", t0.elapsed());

        // Flag CPU-fallback adapters in headless mode so local runs don't
        // silently get wrong anti-aliasing. Still valid on CI without a GPU.
        if surface_opt.is_none() {
            let info = adapter.get_info();
            if info.device_type == wgpu::DeviceType::Cpu {
                log::warn!(
                    "headless renderer using CPU fallback adapter '{}' ({:?}); anti-aliasing may differ from GPU runs",
                    info.name,
                    info.backend
                );
            }
        }

        // Pick the output format. Windowed mode queries the surface caps;
        // headless mode hard-picks Rgba8UnormSrgb — every backend supports
        // it as RENDER_ATTACHMENT | COPY_SRC and the existing PNG readback
        // already handles sRGB8 correctly (no BGRA swap needed).
        let format = match surface_opt.as_ref() {
            Some(surface) => {
                let caps = surface.get_capabilities(&adapter);
                if hdr_enabled {
                    if caps.formats.contains(&wgpu::TextureFormat::Rgba16Float) {
                        log::info!("HDR enabled — using Rgba16Float surface format");
                        wgpu::TextureFormat::Rgba16Float
                    } else {
                        log::warn!(
                            "HDR requested but Rgba16Float not supported; falling back to sRGB"
                        );
                        caps.formats
                            .iter()
                            .find(|f| f.is_srgb())
                            .copied()
                            .unwrap_or(caps.formats[0])
                    }
                } else {
                    caps.formats
                        .iter()
                        .find(|f| f.is_srgb())
                        .copied()
                        .unwrap_or(caps.formats[0])
                }
            }
            None => {
                if hdr_enabled {
                    log::info!("headless renderer ignoring hdr_enabled; screenshots are sRGB8");
                }
                wgpu::TextureFormat::Rgba8UnormSrgb
            }
        };

        let mut limits =
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits());

        // Upgrade compute/storage limits from the adapter so the fluid simulation
        // can use compute shaders on native targets.  The base webgl2 defaults set
        // these to 0 and using_resolution() doesn't touch them.
        let al = adapter.limits();
        limits.max_compute_workgroups_per_dimension = al.max_compute_workgroups_per_dimension;
        limits.max_compute_workgroup_size_x = al.max_compute_workgroup_size_x;
        limits.max_compute_workgroup_size_y = al.max_compute_workgroup_size_y;
        limits.max_compute_workgroup_size_z = al.max_compute_workgroup_size_z;
        limits.max_compute_invocations_per_workgroup = al.max_compute_invocations_per_workgroup;
        limits.max_storage_buffers_per_shader_stage = al.max_storage_buffers_per_shader_stage;
        limits.max_storage_textures_per_shader_stage = al.max_storage_textures_per_shader_stage;
        limits.max_storage_buffer_binding_size = al.max_storage_buffer_binding_size;

        // Opt into TIMESTAMP_QUERY when the adapter supports it so the GPU
        // pass profiler (Debug menu → "Profile GPU…") can record start/end
        // ticks per render pass. The feature is optional — on backends that
        // lack it the profiler stays a no-op.
        let mut required_features = wgpu::Features::CLEAR_TEXTURE;
        let timestamp_supported = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        if timestamp_supported {
            required_features |= wgpu::Features::TIMESTAMP_QUERY;
            // INSIDE_ENCODERS allows `encoder.write_timestamp()` outside of
            // render passes — only needed for debug profiling tools.
            #[cfg(debug_assertions)]
            if adapter
                .features()
                .contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS)
            {
                required_features |= wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS;
            }
        }

        let t0 = Instant::now();
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("mahjuro-device"),
            required_features,
            required_limits: limits,
            experimental_features: wgpu::ExperimentalFeatures::default(),
            memory_hints: wgpu::MemoryHints::default(),
            trace: wgpu::Trace::default(),
        }))
        .map_err(|e| anyhow::anyhow!("device: {e:?}"))?;
        log::info!("wgpu device created in {:?}", t0.elapsed());

        // Build the shared `SurfaceConfiguration` that downstream textures
        // track against. Windowed mode seeds it from `get_default_config`
        // and calls `surface.configure`; headless mode fills in the same
        // fields by hand (alpha_mode / view_formats don't matter for the
        // texture path) and creates the offscreen render-attachment.
        let (target, config) = match surface_opt {
            Some(surface) => {
                let mut config = surface
                    .get_default_config(&adapter, size.width.max(1), size.height.max(1))
                    .ok_or_else(|| anyhow::anyhow!("no default surface config"))?;
                config.format = format;
                config.present_mode = wgpu::PresentMode::Fifo;
                config.desired_maximum_frame_latency = 2;
                // Need COPY_SRC so we can snapshot the swapchain into
                // `scene_prev_texture` at end-of-frame for the lacquer SSR pass.
                config.usage |= wgpu::TextureUsages::COPY_SRC;
                surface.configure(&device, &config);
                (RenderTarget::Surface(surface), config)
            }
            None => {
                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    format,
                    width: size.width.max(1),
                    height: size.height.max(1),
                    present_mode: wgpu::PresentMode::Fifo,
                    desired_maximum_frame_latency: 2,
                    alpha_mode: wgpu::CompositeAlphaMode::Auto,
                    view_formats: vec![],
                };
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("headless-frame-target"),
                    size: wgpu::Extent3d {
                        width: config.width,
                        height: config.height,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: config.usage,
                    view_formats: &[],
                });
                (RenderTarget::Offscreen { texture }, config)
            }
        };

        let (depth_texture, depth_view) =
            create_depth(&device, size.width.max(1), size.height.max(1));
        let (depth_copy_texture, depth_copy_view) =
            create_depth_copy(&device, size.width.max(1), size.height.max(1));
        // Separate depth snapshot for the lacquered-table SSR sample —
        // populated at the end of pass A1 (before plaques are drawn) so
        // the table never reflects the plaque face. See `ssr_prev_depth_*`
        // doc on the field for the full rationale.
        let (ssr_prev_depth_texture, ssr_prev_depth_view) =
            create_depth_copy(&device, size.width.max(1), size.height.max(1));

        let t0 = Instant::now();
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("quad-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/quad.wgsl").into()),
        });

        let tile_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-3d-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/tile_3d.wgsl").into()),
        });

        let text_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("text-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/text_quad.wgsl").into()),
        });

        let globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&Globals {
                screen: [size.width as f32, size.height as f32],
                time: 0.0,
                gamma: 1.0,
                cursor_pos: [size.width as f32 * 0.5, size.height as f32 * 0.5],
                transition_progress: 0.0,
                quality_level: 2.0,
                moon_phase: current_moon_phase(),
                _globals_pad: [0.0; 3],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let globals_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals-bg"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: globals_buffer.as_entire_binding(),
            }],
        });

        // Point-light uniform buffer + bind group (group 1 of the tile pipeline).
        // Initialised empty; populated each frame from `frame.point_lights`.
        let point_lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("point-lights"),
            contents: bytemuck::bytes_of(&PointLightsBuf::from_lights(
                &[],
                0,
                0.0,
                1.0,
                1.0,
                1.0,
                0.0,
            )),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        // Companion uniform: per-frame analytic tile occluders for the
        // candle-pool ray-AABB shadow test in lit_mesh.wgsl. Lives on the
        // same bind group so the lit-mesh pipeline only needs one extra
        // binding to read it. Other shaders sharing this layout simply
        // ignore the binding.
        let tile_occluders_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tile-occluders"),
            contents: bytemuck::bytes_of(&TileOccludersBuf::empty()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let point_lights_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("point-lights-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let point_lights_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("point-lights-bg"),
            layout: &point_lights_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: point_lights_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: tile_occluders_buffer.as_entire_binding(),
                },
            ],
        });

        // Spotlight buffer + bind group (group 3 of the tile pipeline).
        // Initialised empty; populated each frame from `frame.spot_lights`.
        let spot_lights_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("spot-lights"),
            contents: bytemuck::bytes_of(&SpotLightsBuf::empty()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let spot_lights_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("spot-lights-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let spot_lights_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spot-lights-bg"),
            layout: &spot_lights_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: spot_lights_buffer.as_entire_binding(),
            }],
        });

        let tile_material_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("tile-material-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                ],
            });

        let loaded_glb = match crate::asset_path::get("Tile.glb") {
            Some(file) => load_glb_tile_from_bytes(&file.data),
            None => Err(anyhow::anyhow!("Tile.glb not found in embedded assets")),
        };

        let tile_base_color_factor = [1.0_f32, 1.0, 1.0, 1.0];

        let tile_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("tile-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let quad_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("quad-pl"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });

        // ---- Moon albedo texture (LRO WAC real heightmap) ----
        let moon_albedo_tex_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("moon-albedo-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let moonlit_water_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("moonlit-water-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&moon_albedo_tex_layout)],
            immediate_size: 0,
        });
        let (_moon_albedo_texture, moon_albedo_view) =
            crate::render::texture_upload::load_metal_heightmap(
                &device,
                &queue,
                "textures/moon_albedo.png",
                "moon-albedo",
            );
        let moon_albedo_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("moon-albedo-bg"),
            layout: &moon_albedo_tex_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&moon_albedo_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&tile_sampler),
                },
            ],
        });

        // ── Mountain-haze uniform (density/colour/horizon/drift) ───────────
        // Live-driven from the Volumetric debug overlay; see
        // `set_haze_tuning` below for the per-frame write path.
        let haze_uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("haze-uniform"),
            contents: bytemuck::bytes_of(&HazeUniform {
                color_density: [0.080, 0.105, 0.145, 1.0],
                params: [0.55, 1.0, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let haze_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("haze-uniform-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let haze_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("haze-uniform-bg"),
            layout: &haze_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: haze_uniform_buffer.as_entire_binding(),
            }],
        });
        let mountain_haze_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mountain-haze-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&haze_uniform_layout)],
            immediate_size: 0,
        });

        // ---- Shadow map resources (depth texture + sampler + layouts) ----
        // Built up here so the shared sampling layout can be plumbed into
        // both `tile_layout` and `lit_mesh_pl` below as group 2.
        const SHADOW_MAP_SIZE: u32 = 2048;
        let _shadow_map_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow-map"),
            size: wgpu::Extent3d {
                width: SHADOW_MAP_SIZE,
                height: SHADOW_MAP_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_map_view =
            _shadow_map_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });
        let shadow_caster_layout = create_shadow_caster_layout(&device);
        let shadow_sample_layout = create_shadow_sample_layout(&device);
        let shadow_globals_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("shadow-globals"),
            contents: bytemuck::bytes_of(&ShadowGlobals {
                light_view_proj: glam::Mat4::IDENTITY.to_cols_array(),
                params: [0.0, 0.0015, 1.0 / SHADOW_MAP_SIZE as f32, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let shadow_sample_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-sample-bg"),
            layout: &shadow_sample_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: shadow_globals_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&shadow_map_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&shadow_sampler),
                },
            ],
        });

        let tile_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile-pl"),
            bind_group_layouts: &[
                Some(&tile_material_layout),
                Some(&point_lights_layout),
                Some(&shadow_sample_layout),
                Some(&spot_lights_layout),
            ],
            immediate_size: 0,
        });

        let vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<[f32; 2]>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[wgpu::VertexAttribute {
                offset: 0,
                shader_location: 0,
                format: wgpu::VertexFormat::Float32x2,
            }],
        };

        let instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        };

        let depth_3d = wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };

        let depth_ui = wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };

        let quad_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("quad-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(depth_ui.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // Gradient-quad pipeline — alpha-feathered panels behind HUD
        // content. Same `rect`/`color` payload as the base quad_pipeline
        // plus a per-instance `feather` vec4 that drives the shader's
        // falloff (edge softness + axial↔radial blend). Standard alpha
        // blend so multiple gradient quads compose correctly.
        let gradient_instance_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GradientQuadInstance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        };

        let gradient_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gradient_quad.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/shaders/gradient_quad.wgsl"
                ))
                .into(),
            ),
        });

        let gradient_quad_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("gradient-quad-pipeline"),
                layout: Some(&quad_layout),
                vertex: wgpu::VertexState {
                    module: &gradient_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[vertex_layout.clone(), gradient_instance_layout],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &gradient_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(depth_ui.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        // Flame pipeline — 3D billboarded particle fire. The shader
        // reads per-particle world position + age from the instance
        // buffer, constructs a camera-facing quad in the vertex stage,
        // and dissolves the billboard against an age threshold in the
        // fragment stage. Depth-test Less + write off so the particles
        // are correctly occluded by meshes (wax body, coin pile, etc)
        // without self-occluding in z-fight when they overlap.
        //
        // Bind groups: group(0) = 2D Globals (time + screen + gamma),
        //              group(1) = SSR globals (view_proj, view_pos).
        //
        // The SSR layout is created here (slightly earlier than the
        // lit-mesh block below that would normally own it) so this
        // flame pipeline and the lit-mesh pipeline can share a single
        // layout object — wgpu matches bind groups to pipelines by
        // layout identity, not structural equality.
        let lit_mesh_ssr_layout = create_lit_mesh_ssr_layout(&device);
        // Flame-only view layout: just the view_proj/view_pos buffer at
        // binding(0), visible to BOTH stages (the vertex stage needs
        // view_proj to project billboards; the fragment stage may use
        // view_pos for view-relative tricks later). The lit-mesh SSR
        // layout restricts binding(0) to FRAGMENT only, so we can't
        // reuse it here.
        let flame_view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("flame-view-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let flame_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("flame.wgsl"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/shaders/flame.wgsl")).into(),
            ),
        });
        let flame_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("flame-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&flame_view_layout)],
            immediate_size: 0,
        });
        let flame_particle_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<crate::render::flame_particles::GpuFlameParticle>()
                as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // pos (xyz) + age (w) packed into a vec4 for tidy layout.
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                // scale + phase + brightness + pad → vec4.
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        };
        let flame_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("flame-pipeline"),
            layout: Some(&flame_pl),
            vertex: wgpu::VertexState {
                module: &flame_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), flame_particle_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &flame_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Additive blend so flames brighten whatever's behind them.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Billboards are symmetric so cull is a footgun (we'd
                // have to care about wind-space normal orientation).
                cull_mode: None,
                ..Default::default()
            },
            // Depth-test Less + write off. Matches the `lit_mesh_blended`
            // pattern: particles are occluded by opaque geometry in front
            // of them (coin pile, wax body) but stack freely with each
            // other in additive blend.
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let flame_view_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("flame-view-uniform"),
            contents: bytemuck::bytes_of(&FlameViewUniform {
                view_proj: Mat4::IDENTITY.to_cols_array(),
                view_pos: [0.0; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let flame_view_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("flame-view-bg"),
            layout: &flame_view_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: flame_view_buffer.as_entire_binding(),
            }],
        });

        // ── Fullscreen additive vignette pipelines ─────────────────────
        // Starfield, ember-drift, and golden-dust all share the same
        // layout: no vertex buffers, globals-only bind group, additive
        // blend onto the UI colour target.
        let vignette_pipeline = |label: &str, wgsl: &str| -> wgpu::RenderPipeline {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(label),
                source: wgpu::ShaderSource::Wgsl(wgsl.into()),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&quad_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(depth_ui.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let starfield_pipeline = vignette_pipeline(
            "starfield-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/starfield.wgsl"
            )),
        );
        let ember_drift_pipeline = vignette_pipeline(
            "ember-drift-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/ember_drift.wgsl"
            )),
        );
        let golden_dust_pipeline = vignette_pipeline(
            "golden-dust-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/golden_dust.wgsl"
            )),
        );
        // moonlit_water gets its own pipeline so it can bind the moon albedo
        // texture at group 1 in addition to the globals at group 0.
        let moonlit_water_pipeline = {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("moonlit-water-pipeline"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shaders/moonlit_water.wgsl"
                    ))
                    .into(),
                ),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("moonlit-water-pipeline"),
                layout: Some(&moonlit_water_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(depth_ui.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let sunlit_water_pipeline = vignette_pipeline(
            "sunlit-water-pipeline",
            include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/shaders/sunlit_water.wgsl"
            )),
        );
        // Mountain-haze uses a custom pipeline layout so the fragment shader
        // can bind the haze uniform (group 1) alongside globals (group 0).
        let mountain_haze_pipeline = {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("mountain-haze-pipeline"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shaders/mountain_haze.wgsl"
                    ))
                    .into(),
                ),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mountain-haze-pipeline"),
                layout: Some(&mountain_haze_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(depth_ui.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        // The cascade shader is heavy per-pixel; it renders into a half-res
        // offscreen target and is additively composited back into the main
        // pass. The offscreen pipeline writes with REPLACE blend since the
        // target is cleared per-frame before the pre-pass.
        let shooting_star_cascade_pipeline = {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("shooting-star-cascade-pipeline"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shaders/shooting_star_cascade.wgsl"
                    ))
                    .into(),
                ),
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("shooting-star-cascade-pipeline"),
                layout: Some(&quad_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let cascade_composite_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("cascade-composite-bgl"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let cascade_composite_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("cascade-composite-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let cascade_composite_pipeline = {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("cascade-composite-pipeline"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/shaders/shooting_star_cascade_composite.wgsl"
                    ))
                    .into(),
                ),
            });
            let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("cascade-composite-pl"),
                bind_group_layouts: &[Some(&cascade_composite_layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("cascade-composite-pipeline"),
                layout: Some(&pl),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState {
                            color: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                            alpha: wgpu::BlendComponent {
                                src_factor: wgpu::BlendFactor::One,
                                dst_factor: wgpu::BlendFactor::One,
                                operation: wgpu::BlendOperation::Add,
                            },
                        }),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    ..Default::default()
                },
                depth_stencil: Some(depth_ui.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };

        let tile_vertex_layout = wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
            ],
        };

        let tile_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile-pipeline"),
            layout: Some(&tile_layout),
            vertex: wgpu::VertexState {
                module: &tile_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: std::slice::from_ref(&tile_vertex_layout),
            },
            fragment: Some(wgpu::FragmentState {
                module: &tile_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_3d.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ---- Gold outline shell pipeline (selected tiles) ----
        let tile_outline_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-outline-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../shaders/tile_outline.wgsl").into(),
            ),
        });
        let tile_outline_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("tile-outline-pipeline"),
                layout: Some(&tile_layout),
                vertex: wgpu::VertexState {
                    module: &tile_outline_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[tile_vertex_layout],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &tile_outline_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    // We want to draw only the *back* side of the inflated
                    // shell (the side facing away from the camera) so the
                    // regular tile mesh can overwrite the interior and
                    // leave a thin rim. The tile model matrix has
                    // determinant +1 (tile_basis is an even permutation),
                    // so winding is preserved — culling Front leaves only
                    // the back-facing shell fragments (the ones that peek
                    // out past the tile silhouette), which is what we want.
                    cull_mode: Some(wgpu::Face::Front),
                    ..Default::default()
                },
                depth_stencil: Some(depth_3d.clone()),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        // ---- Tile glow pipeline (selected tile additive halo) ----
        let tile_glow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tile-glow-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/tile_glow.wgsl").into()),
        });
        let tile_glow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("tile-glow-pipeline"),
            layout: Some(&quad_layout),
            vertex: wgpu::VertexState {
                module: &tile_glow_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &tile_glow_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    // Additive blend so the glow brightens the table /
                    // tile sides without darkening anything.
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::SrcAlpha,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(depth_ui.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ---- Lit-mesh pipeline (procedural candles + wood table) ----
        let lit_mesh_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lit-mesh-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/lit_mesh.wgsl").into()),
        });
        let lit_mesh_material_layout = create_lit_mesh_material_layout(&device);
        // `lit_mesh_ssr_layout` was created earlier (alongside the flame
        // pipeline) so both pipelines share one layout object.
        let lit_mesh_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lit-mesh-pl"),
            bind_group_layouts: &[
                Some(&lit_mesh_material_layout),
                Some(&point_lights_layout),
                Some(&shadow_sample_layout),
                Some(&lit_mesh_ssr_layout),
            ],
            immediate_size: 0,
        });
        let lit_mesh_ssr_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("lit-mesh-ssr-uniform"),
            contents: bytemuck::bytes_of(&SsrGlobals {
                inv_view_proj: Mat4::IDENTITY.to_cols_array(),
                view_proj: Mat4::IDENTITY.to_cols_array(),
                view_pos: [0.0; 4],
                params: [0.0; 4],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let lit_mesh_ssr_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("lit-mesh-ssr-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let (scene_prev_texture, scene_prev_view) =
            create_scene_prev(&device, format, size.width.max(1), size.height.max(1));
        let (scene_color_texture, scene_color_view) =
            create_scene_color(&device, format, size.width.max(1), size.height.max(1));
        let (cascade_offscreen_texture, cascade_offscreen_view) =
            create_cascade_offscreen(&device, format, size.width.max(1), size.height.max(1));
        let cascade_composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("cascade-composite-bg"),
            layout: &cascade_composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&cascade_offscreen_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&cascade_composite_sampler),
                },
            ],
        });
        let bloom_w = (size.width.max(1) / 2).max(1);
        let bloom_h = (size.height.max(1) / 2).max(1);
        let (bloom_ping_texture, bloom_ping_view) =
            create_post_texture(&device, format, bloom_w, bloom_h, "bloom-ping");
        let (bloom_pong_texture, bloom_pong_view) =
            create_post_texture(&device, format, bloom_w, bloom_h, "bloom-pong");
        let bloom_params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("bloom-params"),
            contents: bytemuck::bytes_of(&BloomParams {
                data0: [1.1, 0.0, 1.0 / bloom_w as f32, 1.0 / bloom_h as f32],
                data1: [1.0, 0.0, 0.0, 0.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bloom_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let lit_mesh_ssr_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-mesh-ssr-bg"),
            layout: &lit_mesh_ssr_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: lit_mesh_ssr_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&scene_prev_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&ssr_prev_depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&lit_mesh_ssr_sampler),
                },
            ],
        });
        let lit_mesh_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("lit-mesh-pipeline"),
            layout: Some(&lit_mesh_pl),
            vertex: wgpu::VertexState {
                module: &lit_mesh_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            offset: 0,
                            shader_location: 0,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 12,
                            shader_location: 1,
                            format: wgpu::VertexFormat::Float32x3,
                        },
                        wgpu::VertexAttribute {
                            offset: 24,
                            shader_location: 2,
                            format: wgpu::VertexFormat::Float32x2,
                        },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &lit_mesh_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_3d.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let lit_mesh_blended_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("lit-mesh-blended-pipeline"),
                layout: Some(&lit_mesh_pl),
                vertex: wgpu::VertexState {
                    module: &lit_mesh_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttribute {
                                offset: 0,
                                shader_location: 0,
                                format: wgpu::VertexFormat::Float32x3,
                            },
                            wgpu::VertexAttribute {
                                offset: 12,
                                shader_location: 1,
                                format: wgpu::VertexFormat::Float32x3,
                            },
                            wgpu::VertexAttribute {
                                offset: 24,
                                shader_location: 2,
                                format: wgpu::VertexFormat::Float32x2,
                            },
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &lit_mesh_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth32Float,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        // ---- Shadow pipeline (depth-only pre-pass) ----
        // Shared by lit-mesh casters (table-excluded) and hand tiles —
        // both vertex layouts begin with `position : vec3<f32>` at
        // offset 0, and the shader only reads location 0.
        let shadow_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shadow-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/shadow.wgsl").into()),
        });
        let shadow_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow-pl"),
            bind_group_layouts: &[Some(&shadow_caster_layout)],
            immediate_size: 0,
        });
        let shadow_depth_state = wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: Some(true),
            depth_compare: Some(wgpu::CompareFunction::Less),
            stencil: wgpu::StencilState::default(),
            // Slope-scaled bias to fight acne. The constant component is
            // small because the lit shaders also subtract a depth bias.
            bias: wgpu::DepthBiasState {
                constant: 2,
                slope_scale: 2.5,
                clamp: 0.0,
            },
        };
        let shadow_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow-pipeline"),
            layout: Some(&shadow_pl),
            vertex: wgpu::VertexState {
                module: &shadow_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                // Match the lit_mesh / tile_glb vertex stride so a single
                // pipeline can render either caster type. Only attribute 0
                // (position) is read by the shader.
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex3dTex>() as wgpu::BufferAddress,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x3,
                    }],
                }],
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(shadow_depth_state),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ---- Text pipeline ----
        let text_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("text-bg-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

        let text_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text-pl"),
            bind_group_layouts: &[Some(&globals_layout), Some(&text_bind_group_layout)],
            immediate_size: 0,
        });
        let text_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("text-pipeline"),
            layout: Some(&text_layout),
            vertex: wgpu::VertexState {
                module: &text_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout.clone(), instance_layout.clone()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &text_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(depth_ui.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // ---- Image pipeline (full-colour textured quads for relic icons, etc.) ----
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("image-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/image_quad.wgsl").into()),
        });
        let image_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("image-pipeline"),
            layout: Some(&text_layout),
            vertex: wgpu::VertexState {
                module: &image_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[vertex_layout, instance_layout],
            },
            fragment: Some(wgpu::FragmentState {
                module: &image_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(depth_ui.clone()),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let bloom_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bloom-bg-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let bloom_composite_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("bloom-composite-bg-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            multisampled: false,
                            view_dimension: wgpu::TextureViewDimension::D2,
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        let bloom_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom-pl"),
            bind_group_layouts: &[Some(&bloom_bind_group_layout)],
            immediate_size: 0,
        });
        let bloom_extract_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-extract-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../shaders/bloom_extract.wgsl").into(),
            ),
        });
        let bloom_blur_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-blur-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shaders/bloom_blur.wgsl").into()),
        });
        let bloom_extract_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bloom-extract-pipeline"),
                layout: Some(&bloom_layout),
                vertex: wgpu::VertexState {
                    module: &bloom_extract_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &bloom_extract_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let bloom_blur_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("bloom-blur-pipeline"),
            layout: Some(&bloom_layout),
            vertex: wgpu::VertexState {
                module: &bloom_blur_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &bloom_blur_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let bloom_composite_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("bloom-composite-pl"),
                bind_group_layouts: &[Some(&bloom_composite_bind_group_layout)],
                immediate_size: 0,
            });
        let bloom_composite_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom-composite-shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../../shaders/bloom_composite.wgsl").into(),
            ),
        });
        let bloom_composite_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bloom-composite-pipeline"),
                layout: Some(&bloom_composite_layout),
                vertex: wgpu::VertexState {
                    module: &bloom_composite_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &bloom_composite_shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
        let bloom_scene_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-scene-bg"),
            layout: &bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&scene_color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&bloom_sampler),
                },
            ],
        });
        let bloom_ping_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-ping-bg"),
            layout: &bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_ping_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&bloom_sampler),
                },
            ],
        });
        let bloom_pong_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-pong-bg"),
            layout: &bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&bloom_pong_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&bloom_sampler),
                },
            ],
        });
        let bloom_composite_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-composite-bg"),
            layout: &bloom_composite_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&scene_color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&bloom_ping_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&bloom_sampler),
                },
            ],
        });

        log::info!("shaders + pipelines compiled in {:?}", t0.elapsed());

        let t0 = Instant::now();
        let ui_font = load_ui_font();
        if ui_font.is_some() {
            log::info!("UI font loaded.");
        } else {
            log::warn!("No UI font found; panel text will be blank.");
        }
        let emoji_font = load_noto_emoji_font();
        if emoji_font.is_some() {
            log::info!("Noto Emoji font loaded.");
        } else {
            log::warn!("No Noto Emoji font found; tile symbols may be blank.");
        }

        log::info!("fonts loaded in {:?}", t0.elapsed());

        let quad_v: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad-verts"),
            contents: bytemuck::cast_slice(&quad_v),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let idx: [u16; 6] = [0, 1, 2, 2, 1, 3];
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("quad-idx"),
            contents: bytemuck::cast_slice(&idx),
            usage: wgpu::BufferUsages::INDEX,
        });

        let t0 = Instant::now();
        let tile_primitives: Vec<TilePrimitiveGpu> = match loaded_glb {
            Ok(mut mesh) => {
                normalize_mesh(&mut mesh);
                log::info!("Loaded 3D tile: {} primitive(s)", mesh.primitives.len());
                let mut out = Vec::with_capacity(mesh.primitives.len());
                for (i, prim) in mesh.primitives.iter().enumerate() {
                    let vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile-verts"),
                        contents: bytemuck::cast_slice(&prim.vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    let ib = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile-idx"),
                        contents: bytemuck::cast_slice(&prim.indices),
                        usage: wgpu::BufferUsages::INDEX,
                    });
                    let (_albedo_texture, albedo_view) = match &prim.albedo_rgba {
                        Some((rgba, w, h)) => {
                            upload_rgba_texture(&device, &queue, "tile-prim-albedo", rgba, *w, *h)
                        }
                        None => white_albedo(&device, &queue),
                    };
                    log::info!(
                        "  prim {}: {} verts, {} idx, has_tex={}",
                        i,
                        prim.vertices.len(),
                        prim.indices.len(),
                        prim.albedo_rgba.is_some(),
                    );
                    out.push(TilePrimitiveGpu {
                        vertex_buffer: vb,
                        index_buffer: ib,
                        index_count: prim.indices.len() as u32,
                        albedo_view,
                    });
                }
                out
            }
            Err(e) => {
                log::warn!("Could not load Tile.glb (3D hand tiles disabled): {e:#}");
                Vec::new()
            }
        };

        log::info!("tile mesh loaded in {:?}", t0.elapsed());

        // Kick off background relic image loading (non-blocking).
        let relic_load_start = Some(Instant::now());
        let relic_rx = Some(spawn_relic_loader());
        let (_lit_mesh_relief_default_tex, lit_mesh_relief_default_view) =
            flat_relief_height(&device, &queue);
        let pack_textures_map = load_pack_textures(
            &device,
            &queue,
            &text_bind_group_layout,
            &tile_sampler,
            &lit_mesh_relief_default_view,
        );
        // Kick off background image loading (non-blocking).
        let background_load_start = Some(Instant::now());
        let background_rx = Some(spawn_background_loader());

        // Create fluid simulation (requires compute shader support).
        let fluid = {
            let limits = device.limits();
            if limits.max_compute_workgroups_per_dimension > 0 {
                log::info!("Compute shaders supported — creating fluid simulation.");
                Some(super::fluid::FluidSim::new(
                    &device,
                    &queue,
                    &globals_layout,
                    format,
                    size.width as f32,
                    size.height as f32,
                ))
            } else {
                log::warn!("Compute shaders not supported — smoke effects disabled.");
                None
            }
        };

        // ---- Lit-mesh procedural geometry (candles + table) ----
        let candle_wax_mesh = LitMeshGpu::new(&device, &build_candle_wax_mesh(), "candle-wax");
        let candle_wick_mesh = LitMeshGpu::new(&device, &build_candle_wick_mesh(), "candle-wick");
        let table_mesh = LitMeshGpu::new(&device, &build_table_mesh(), "table");
        let round_dish_mesh = LitMeshGpu::new(&device, &build_round_dish_mesh(), "round-dish");
        let sell_card_mesh = LitMeshGpu::new(&device, &build_tent_card_mesh(), "sell-card");
        let relic_box_cpu = build_relic_mesh();
        let relic_box_tris: Vec<[glam::Vec3; 3]> = relic_box_cpu
            .indices
            .chunks_exact(3)
            .map(|c| {
                let a = relic_box_cpu.vertices[c[0] as usize].position;
                let b = relic_box_cpu.vertices[c[1] as usize].position;
                let d = relic_box_cpu.vertices[c[2] as usize].position;
                [
                    glam::Vec3::from(a),
                    glam::Vec3::from(b),
                    glam::Vec3::from(d),
                ]
            })
            .collect();
        let relic_box_mesh = LitMeshGpu::new(&device, &relic_box_cpu, "relic-mesh");
        let pack_mesh = LitMeshGpu::new(&device, &build_pack_mesh(), "pack-mesh");
        let ribbon_mesh = LitMeshGpu::new(&device, &build_ribbon_mesh(), "ribbon");
        let talisman_mesh = LitMeshGpu::new(&device, &build_talisman_mesh(), "talisman");
        let shrine_mesh = LitMeshGpu::new(&device, &build_shrine_mesh(), "shrine");
        let dora_plinth_mesh = LitMeshGpu::new(&device, &build_dora_plinth_mesh(), "dora-plinth");
        let lamp_body_cpu = build_lamp_body_mesh();
        let lamp_body_tris: Vec<[glam::Vec3; 3]> = lamp_body_cpu
            .indices
            .chunks_exact(3)
            .map(|c| {
                let a = lamp_body_cpu.vertices[c[0] as usize].position;
                let b = lamp_body_cpu.vertices[c[1] as usize].position;
                let d = lamp_body_cpu.vertices[c[2] as usize].position;
                [
                    glam::Vec3::from(a),
                    glam::Vec3::from(b),
                    glam::Vec3::from(d),
                ]
            })
            .collect();
        let (lamp_body_local_half, lamp_body_local_center_y) = {
            let mut lo = glam::Vec3::splat(f32::INFINITY);
            let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
            for tri in &lamp_body_tris {
                for p in tri {
                    lo = lo.min(*p);
                    hi = hi.max(*p);
                }
            }
            let half = (hi - lo) * 0.5;
            let cy = (hi.y + lo.y) * 0.5;
            (half, cy)
        };
        let lamp_body_mesh = LitMeshGpu::new(&device, &lamp_body_cpu, "lamp-body");
        let lamp_bulb_mesh = LitMeshGpu::new(&device, &build_lamp_bulb_mesh(), "lamp-bulb");
        // Phase-1 primitive registry: parallel GPU copies of meshes
        // the generic `Object3dKind::Primitive` dispatch can reach by
        // `MeshId`. Legacy named fields above still own their own
        // allocations during the migration window.
        let mut primitive_meshes: HashMap<MeshId, std::sync::Arc<LitMeshGpu>> = HashMap::new();
        {
            let unit_cube_cpu = {
                let mut verts: Vec<crate::render::tile_glb::Vertex3dTex> = Vec::new();
                let mut idx: Vec<u32> = Vec::new();
                push_box(
                    &mut verts,
                    &mut idx,
                    Aabb::new(-0.5, 0.5, -0.5, 0.5, -0.5, 0.5),
                );
                MeshCpu {
                    vertices: verts,
                    indices: idx,
                    default_material: MaterialParams {
                        kind: MaterialKind::Plain,
                        base_color: [1.0, 1.0, 1.0, 1.0],
                        specular_strength: 0.25,
                        specular_power: 32.0,
                    },
                }
            };
            primitive_meshes.insert(
                MeshId::Cube,
                std::sync::Arc::new(LitMeshGpu::new(&device, &unit_cube_cpu, "primitive-cube")),
            );
            primitive_meshes.insert(
                MeshId::BeveledSlab,
                std::sync::Arc::new(LitMeshGpu::new(
                    &device,
                    &build_plaque_mesh(),
                    "primitive-slab",
                )),
            );
            primitive_meshes.insert(
                MeshId::CabinetColumn,
                std::sync::Arc::new(LitMeshGpu::new(
                    &device,
                    &build_cabinet_mesh(),
                    "primitive-cabinet-column",
                )),
            );
            primitive_meshes.insert(
                MeshId::CabinetRails,
                std::sync::Arc::new(LitMeshGpu::new(
                    &device,
                    &build_cabinet_rails_mesh(),
                    "primitive-cabinet-rails",
                )),
            );
            primitive_meshes.insert(
                MeshId::ShopActionProp,
                std::sync::Arc::new(LitMeshGpu::new(
                    &device,
                    &build_shop_action_prop_mesh(),
                    "primitive-shop-action-prop",
                )),
            );
            primitive_meshes.insert(
                MeshId::DiscSquare,
                std::sync::Arc::new(LitMeshGpu::new(
                    &device,
                    &build_dish_mesh(),
                    "primitive-dish-square",
                )),
            );
            primitive_meshes.insert(
                MeshId::DiscRound,
                std::sync::Arc::new(LitMeshGpu::new(
                    &device,
                    &build_round_dish_mesh(),
                    "primitive-dish-round",
                )),
            );
            // Cylinder is sized by `Object3d::extents` — reuse the coin
            // mesh (Y-up unit cylinder) so callers pay nothing extra.
            primitive_meshes.insert(
                MeshId::Cylinder,
                std::sync::Arc::new(LitMeshGpu::new(
                    &device,
                    &build_coin_mesh(),
                    "primitive-cylinder",
                )),
            );
            primitive_meshes.insert(
                MeshId::Ofuda,
                std::sync::Arc::new(LitMeshGpu::new(
                    &device,
                    &build_ofuda_mesh(),
                    "primitive-ofuda",
                )),
            );
        }
        // Per-shape texture override: the coin cylinder needs its
        // engraved heightmap bound at both albedo and relief slots so
        // the Metal branch in lit_mesh.wgsl can sample the cash-coin
        // relief. Populated now so `primitive_textures` is ready by
        // the time `dispatch_primitive` first creates an instance.
        let mut primitive_textures: HashMap<
            crate::render::primitive::MeshId,
            (wgpu::TextureView, wgpu::TextureView),
        > = HashMap::new();
        let bone_tablet_mesh = LitMeshGpu::new(&device, &build_bone_tablet_mesh(), "bone-tablet");
        let wood_tablet_mesh = LitMeshGpu::new(&device, &build_wood_tablet_mesh(), "wood-tablet");
        // The legacy "bowl" slot now hosts the discard river mesh — a stone
        // trough with an animated water surface. Field/variant names stayed
        // (`bowl_mesh`, `BowlPlacement`, `GameplayPick::DiscardBowl`) to keep
        // this swap to a single mesh substitution; renaming is a follow-up.
        let bowl_mesh = LitMeshGpu::new(&device, &build_river_mesh(), "river");
        let mirror_mesh = LitMeshGpu::new(&device, &build_mirror_mesh(), "mirror");
        let tally_stick_base_mesh =
            LitMeshGpu::new(&device, &build_tally_stick_base_mesh(), "tally-stick-base");
        let tally_stick_tip_mesh =
            LitMeshGpu::new(&device, &build_tally_stick_tip_mesh(), "tally-stick-tip");
        // Shared 1×1 white texture for procedural meshes that don't sample.
        let (_lit_mesh_white_tex, lit_mesh_white_view) = white_albedo(&device, &queue);

        // Pre-allocate candle slots (gameplay: score pair + two hand-strip
        // pairs + footlight). Each slot owns two instances: wax + wick.
        const NUM_CANDLE_SLOTS: usize = 7;
        let mut candle_instances: Vec<[LitMeshInstance; 2]> = Vec::with_capacity(NUM_CANDLE_SLOTS);
        for _ in 0..NUM_CANDLE_SLOTS {
            candle_instances.push([
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_white_view,
                    &lit_mesh_relief_default_view,
                    &tile_sampler,
                ),
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_white_view,
                    &lit_mesh_relief_default_view,
                    &tile_sampler,
                ),
            ]);
        }
        let table_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
        let mut relic_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_RELIC_SLOTS);
        for _ in 0..MAX_RELIC_SLOTS {
            relic_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }
        let mut pack_instances: Vec<LitMeshInstance> = Vec::with_capacity(4);
        for _ in 0..4 {
            pack_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }
        let ribbon_zodiac_tex = load_zodiac_ribbon_textures(&device, &queue);
        let mut ribbon_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_RIBBON_SLOTS);
        for _ in 0..MAX_RIBBON_SLOTS {
            ribbon_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }
        let ribbon_slot_zodiac: Vec<Option<(u8, u8)>> = vec![None; MAX_RIBBON_SLOTS];
        // Coin face heightmap (Chinese cash-coin engraving). Bound at slot 1
        // of every shop-pile coin instance; the metal branch in lit_mesh.wgsl
        // samples it as a heightfield to perturb the coin's surface normal so
        // the engraved characters and rim catch the candle highlights. Pegs
        // reuse coin geometry but keep the white texture and `Plain` material
        // so they aren't affected.
        let (_lit_mesh_coin_height_tex, lit_mesh_coin_height_view) =
            load_coin_heightmap(&device, &queue);
        // Register the coin heightmap as the per-shape texture override
        // for Cylinder primitives so engraved-coin callers sample it.
        primitive_textures.insert(
            crate::render::primitive::MeshId::Cylinder,
            (
                lit_mesh_coin_height_view.clone(),
                lit_mesh_coin_height_view.clone(),
            ),
        );
        // Single sell tray instance.
        let sell_tray_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
        // Folded "SELL" tent card sat in the sell tray when focused. The decal
        // texture is rasterized lazily on first show and reused thereafter.
        let sell_card_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
        // Shop lamp — one instance for the brass body, one for the glass bulb.
        let lamp_body_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
        let lamp_bulb_instance = LitMeshInstance::new(
            &device,
            &lit_mesh_material_layout,
            &shadow_caster_layout,
            &lit_mesh_white_view,
            &lit_mesh_relief_default_view,
            &tile_sampler,
        );
        let bug_body_mesh = LitMeshGpu::new(&device, &build_bug_body_mesh(), "bug-body");
        let bug_wing_mesh = LitMeshGpu::new(&device, &build_bug_wing_mesh(), "bug-wing");
        let bug_wing_blur_mesh =
            LitMeshGpu::new(&device, &build_bug_wing_blur_mesh(), "bug-wing-blur");
        let mut bug_body_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
        let mut bug_wing_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
        let mut bug_wing_r_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
        let mut bug_wing_blur_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
        let mut bug_wing_blur_r_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_BUG_SLOTS);
        for _ in 0..MAX_BUG_SLOTS {
            bug_body_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
            bug_wing_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
            bug_wing_r_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
            bug_wing_blur_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
            bug_wing_blur_r_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }
        let orb_mesh = LitMeshGpu::new(&device, &build_orb_mesh(), "material-orb");
        let mut orb_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_ORB_SLOTS);
        for _ in 0..MAX_ORB_SLOTS {
            orb_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }
        // Per-kind heightmap textures for talisman tablets. Each is a PNG
        // asset loaded from assets/textures/ and uploaded as a linear RGBA8
        // texture. Falls back to a flat mid-gray 1×1 if the asset is missing.
        // Order matches `TalismanKind::all()` — reuse art where dedicated assets
        // are not yet present.
        let talisman_height_paths = [
            ("textures/talismans/talisman_jade.png", "talisman-jade-hm"),
            ("textures/talismans/talisman_pearl.png", "talisman-pearl-hm"),
            (
                "textures/talismans/talisman_gilded.png",
                "talisman-gilded-hm",
            ),
            (
                "textures/talismans/talisman_polychrome.png",
                "talisman-polychrome-hm",
            ),
            ("textures/talismans/talisman_kiln.png", "talisman-kiln-hm"),
            (
                "textures/talismans/talisman_bamboo.png",
                "talisman-bamboo-hm",
            ),
            ("textures/talismans/talisman_dots.png", "talisman-dots-hm"),
            (
                "textures/talismans/talisman_characters.png",
                "talisman-characters-hm",
            ),
            (
                "textures/talismans/talisman_honors.png",
                "talisman-honors-hm",
            ),
            (
                "textures/talismans/talisman_wildflower.png",
                "talisman-wildflower-hm",
            ),
            (
                "textures/talismans/talisman_conformity.png",
                "talisman-conformity-hm",
            ),
        ];
        let mut talisman_height_views: Vec<wgpu::TextureView> = Vec::new();
        for &(path, label) in &talisman_height_paths {
            let (_tex, view) = load_metal_heightmap(&device, &queue, path, label);
            talisman_height_views.push(view);
        }
        let talisman_slot_kind: Vec<Option<u8>> = vec![None; MAX_TALISMAN_SLOTS];
        let mut talisman_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_TALISMAN_SLOTS);
        for _ in 0..MAX_TALISMAN_SLOTS {
            talisman_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }
        let mut shrine_instances: Vec<LitMeshInstance> = Vec::with_capacity(MAX_SHRINE_SLOTS);
        for _ in 0..MAX_SHRINE_SLOTS {
            shrine_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }
        let mut dora_plinth_instances: Vec<LitMeshInstance> =
            Vec::with_capacity(MAX_DORA_PLINTH_SLOTS);
        for _ in 0..MAX_DORA_PLINTH_SLOTS {
            dora_plinth_instances.push(LitMeshInstance::new(
                &device,
                &lit_mesh_material_layout,
                &shadow_caster_layout,
                &lit_mesh_white_view,
                &lit_mesh_relief_default_view,
                &tile_sampler,
            ));
        }

        // ── Skeuomorphic gameplay HUD slot pools (phase 1) ─────────────
        let make_pool = |n: usize| -> Vec<LitMeshInstance> {
            (0..n)
                .map(|_| {
                    LitMeshInstance::new(
                        &device,
                        &lit_mesh_material_layout,
                        &shadow_caster_layout,
                        &lit_mesh_white_view,
                        &lit_mesh_relief_default_view,
                        &tile_sampler,
                    )
                })
                .collect()
        };
        // Plaque instances grow on demand via `ensure_plaque_slots` rather
        // than reserving a fixed cap — see that helper for context.
        // Cabinet instances grow on demand via `ensure_lit_mesh_pool` —
        // collection is the only consumer for now and only ever needs a
        // single instance, so reserving a fixed cap would be silly.
        let yaku_tablet_instances = make_pool(MAX_YAKU_TABLET_SLOTS);
        let wood_tablet_instances = make_pool(MAX_WOOD_TABLET_SLOTS);
        let bowl_instances = make_pool(MAX_BOWL_SLOTS);
        // Bronze mirror face heightmap (Han/Tang four-spirit relief). Bound
        // at slot 1 of every mirror instance; the metal branch in
        // lit_mesh.wgsl samples it as a heightfield to perturb the polished
        // face's surface normal so the cast guardians and TLV marks catch
        // the candle highlights. Same setup as the coin pile above.
        let (_lit_mesh_mirror_height_tex, lit_mesh_mirror_height_view) =
            load_mirror_heightmap(&device, &queue);
        let mirror_instances: Vec<LitMeshInstance> = (0..MAX_MIRROR_SLOTS)
            .map(|_| {
                LitMeshInstance::new(
                    &device,
                    &lit_mesh_material_layout,
                    &shadow_caster_layout,
                    &lit_mesh_mirror_height_view,
                    &lit_mesh_mirror_height_view,
                    &tile_sampler,
                )
            })
            .collect();
        // Each visible stick consumes two slots (bone + tip) so the pool is
        // sized at `2 × MAX_TALLY_STICK_SLOTS` to cover the worst case of
        // every slot populated.
        let tally_stick_instances = make_pool(MAX_TALLY_STICK_SLOTS * 2);
        let wall_tile_instances = make_pool(MAX_WALL_TILE_SLOTS);
        let cascade_token_instances = make_pool(MAX_CASCADE_TOKEN_SLOTS);
        let extruded_glyph_instances = make_pool(MAX_EXTRUDED_GLYPH_SLOTS);
        let debug_axes_instances = make_pool(3);

        // Build the GPU profiler up-front while we still have a borrow of
        // device/queue (the struct literal below moves them).
        let gpu_profiler =
            crate::render::gpu_profiler::GpuProfiler::new(&device, &queue, timestamp_supported);

        log::info!("WgpuRenderer::new() total: {:?}", t_total.elapsed());

        Ok(Self {
            target,
            device,
            queue,
            config,
            depth_texture,
            depth_view,
            depth_copy_texture,
            depth_copy_view,
            ssr_prev_depth_texture,
            ssr_prev_depth_view,
            quad_pipeline,
            gradient_quad_pipeline,
            flame_pipeline,
            flame_view_buffer,
            flame_view_bind_group,
            flame_particles: crate::render::flame_particles::FlameParticleSystem::new(),
            flame_particle_staging: Vec::with_capacity(256),
            starfield_pipeline,
            ember_drift_pipeline,
            golden_dust_pipeline,
            moonlit_water_pipeline,
            moon_albedo_bind_group,
            sunlit_water_pipeline,
            mountain_haze_pipeline,
            haze_uniform_buffer,
            haze_uniform_bind_group,
            shooting_star_cascade_pipeline,
            cascade_composite_pipeline,
            cascade_composite_layout,
            cascade_composite_sampler,
            cascade_offscreen_texture,
            cascade_offscreen_view,
            cascade_composite_bind_group,
            tile_pipeline,
            tile_outline_pipeline,
            tile_glow_pipeline,
            globals_buffer,
            globals_bind_group,
            tile_material_layout,
            point_lights_buffer,
            tile_occluders_buffer,
            point_lights_bind_group,
            spot_lights_buffer,
            spot_lights_bind_group,
            tile_sampler,
            tile_primitives,
            tile_base_color_factor,
            tile_set: Some("original".to_string()),
            hand_tiles: Vec::new(),
            showcase_tiles: Vec::new(),
            tile_face_overlays: HashMap::new(),
            vertex_buffer,
            index_buffer,
            text_pipeline,
            text_bind_group_layout,
            image_pipeline,
            ui_font,
            emoji_font,
            size,
            last_focus: usize::MAX,
            focus_spin: None,
            focus_t: Vec::new(),
            tile_anim_y: Vec::new(),
            tile_anim_x: Vec::new(),
            tile_uids: Vec::new(),
            departing_tiles: Vec::new(),
            prev_hand_slots: Vec::new(),
            proj: ProjectionCache::default(),
            last_pick_models: Vec::new(),
            last_pick_camera: None,
            last_relic_models: Vec::new(),
            last_pickable_relic_models: Vec::new(),
            relic_slot_texture: vec![None; MAX_RELIC_SLOTS],
            pack_instances,
            pack_slot_texture: vec![None; 4],
            ribbon_mesh,
            talisman_mesh,
            shrine_mesh,
            dora_plinth_mesh,
            ribbon_instances,
            ribbon_slot_zodiac,
            ribbon_zodiac_tex,
            talisman_instances,
            sell_tray_instance,
            sell_card_instance,
            sell_card_decal_ready: false,
            last_sell_card_model: None,
            lamp_body_mesh,
            lamp_body_tris,
            lamp_body_local_half,
            lamp_body_local_center_y,
            lamp_bulb_mesh,
            lamp_body_instance,
            lamp_bulb_instance,
            bug_body_mesh,
            bug_wing_mesh,
            bug_body_instances,
            bug_wing_instances,
            bug_wing_r_instances,
            bug_wing_blur_mesh,
            bug_wing_blur_instances,
            bug_wing_blur_r_instances,
            orb_mesh,
            orb_instances,
            last_sell_tray_model: None,
            shrine_instances,
            dora_plinth_instances,
            last_ribbon_models: Vec::new(),
            last_ribbon_slot_count: 0,
            last_ribbon_batch_slot_counts: Vec::new(),
            last_talisman_models: Vec::new(),
            last_aux_dish_aabbs: Vec::new(),
            bone_tablet_mesh,
            wood_tablet_mesh,
            bowl_mesh,
            mirror_mesh,
            tally_stick_base_mesh,
            tally_stick_tip_mesh,
            yaku_tablet_instances,
            wood_tablet_instances,
            bowl_instances,
            mirror_instances,
            tally_stick_instances,
            wall_tile_instances,
            cascade_token_instances,
            extruded_glyph_instances,
            glyph_cpu_cache: crate::render::glyph_mesh::GlyphMeshCache::new(),
            extruded_glyph_meshes: HashMap::new(),
            primitive_meshes,
            primitive_instances: HashMap::new(),
            primitive_textures,
            last_primitive_pick_models: HashMap::new(),
            debug_axes_instances,
            last_yaku_tablet_models: Vec::new(),
            last_wood_tablet_models: Vec::new(),
            last_bowl_model: None,
            last_mirror_model: None,
            last_debug_pickables: Vec::new(),
            last_debug_trimesh_pickables: Vec::new(),
            active_scene_key: None,
            debug_arrange_override: None,
            committed_arrange_rotations: std::collections::HashMap::new(),
            last_frame: Instant::now(),
            frame_dt: 0.0,
            obj3d_hover_state: HashMap::new(),
            creation_time: Instant::now(),
            relic_textures: HashMap::new(),
            relic_rx,
            relic_load_start,
            pack_textures: pack_textures_map,
            background_textures: HashMap::new(),
            background_rx,
            background_load_start,
            fluid,
            fluid_render_bg_dirty: true,
            prev_tile_world: HashMap::new(),
            prev_cursor_world: None,
            prev_cursor_screen: None,
            lit_mesh_material_layout,
            lit_mesh_ssr_layout,
            lit_mesh_ssr_buffer,
            lit_mesh_ssr_bind_group,
            lit_mesh_ssr_sampler,
            scene_prev_texture,
            scene_prev_view,
            scene_color_texture,
            scene_color_view,
            bloom_extract_pipeline,
            bloom_blur_pipeline,
            bloom_composite_pipeline,
            bloom_bind_group_layout,
            bloom_composite_bind_group_layout,
            bloom_params_buffer,
            bloom_sampler,
            bloom_scene_bind_group,
            bloom_ping_bind_group,
            bloom_pong_bind_group,
            bloom_composite_bind_group,
            bloom_ping_texture,
            bloom_ping_view,
            bloom_pong_texture,
            bloom_pong_view,
            lit_mesh_pipeline,
            lit_mesh_blended_pipeline,
            lit_mesh_white_view,
            lit_mesh_relief_default_view,
            talisman_height_views,
            talisman_slot_kind,
            candle_wax_mesh,
            candle_wick_mesh,
            table_mesh,
            round_dish_mesh,
            sell_card_mesh,
            relic_box_mesh,
            relic_box_tris,
            relic_tri_lists: HashMap::new(),
            pack_mesh,
            relic_meshes: HashMap::new(),
            candle_instances,
            table_instance,
            relic_instances,
            shadow_map_view,
            shadow_caster_layout,
            shadow_globals_buffer,
            shadow_sample_bind_group,
            shadow_pipeline,
            gpu_profiler,
            pending_screenshot: std::cell::Cell::new(None),
        })
    }

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
    pub fn screenshot_pending(&self) -> bool {
        let p = self.pending_screenshot.take();
        let pending = p.is_some();
        // Restore (Cell::take() removes the value; put it back so the
        // next draw can fulfil it).
        self.pending_screenshot.set(p);
        pending
    }

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

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        match &self.target {
            RenderTarget::Surface(surface) => surface.configure(&self.device, &self.config),
            RenderTarget::Offscreen { .. } => {
                // Headless screenshot mode renders at a fixed size chosen
                // at startup; window resize events never reach this path.
                // Leaving the offscreen texture untouched here keeps the
                // per-frame render target stable across ticks.
            }
        }

        self.depth_texture.destroy();
        let (dt, dv) = create_depth(&self.device, new_size.width, new_size.height);
        self.depth_texture = dt;
        self.depth_view = dv;
        self.depth_copy_texture.destroy();
        let (dct, dcv) = create_depth_copy(&self.device, new_size.width, new_size.height);
        self.depth_copy_texture = dct;
        self.depth_copy_view = dcv;
        self.ssr_prev_depth_texture.destroy();
        let (sdt, sdv) = create_depth_copy(&self.device, new_size.width, new_size.height);
        self.ssr_prev_depth_texture = sdt;
        self.ssr_prev_depth_view = sdv;

        // SSR scene history texture follows the swapchain size; rebuild
        // the bind group so it points at the freshly allocated views.
        self.scene_prev_texture.destroy();
        let (spt, spv) = create_scene_prev(
            &self.device,
            self.config.format,
            new_size.width,
            new_size.height,
        );
        self.scene_prev_texture = spt;
        self.scene_prev_view = spv;
        self.scene_color_texture.destroy();
        let (sct, scv) = create_scene_color(
            &self.device,
            self.config.format,
            new_size.width,
            new_size.height,
        );
        self.scene_color_texture = sct;
        self.scene_color_view = scv;
        self.cascade_offscreen_texture.destroy();
        let (cot, cov) = create_cascade_offscreen(
            &self.device,
            self.config.format,
            new_size.width,
            new_size.height,
        );
        self.cascade_offscreen_texture = cot;
        self.cascade_offscreen_view = cov;
        self.cascade_composite_bind_group =
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("cascade-composite-bg"),
                layout: &self.cascade_composite_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.cascade_offscreen_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.cascade_composite_sampler),
                    },
                ],
            });
        self.bloom_ping_texture.destroy();
        self.bloom_pong_texture.destroy();
        let bloom_w = (new_size.width.max(1) / 2).max(1);
        let bloom_h = (new_size.height.max(1) / 2).max(1);
        let (bpt, bpv) = create_post_texture(
            &self.device,
            self.config.format,
            bloom_w,
            bloom_h,
            "bloom-ping",
        );
        self.bloom_ping_texture = bpt;
        self.bloom_ping_view = bpv;
        let (bot, bov) = create_post_texture(
            &self.device,
            self.config.format,
            bloom_w,
            bloom_h,
            "bloom-pong",
        );
        self.bloom_pong_texture = bot;
        self.bloom_pong_view = bov;
        self.lit_mesh_ssr_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("lit-mesh-ssr-bg"),
            layout: &self.lit_mesh_ssr_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.lit_mesh_ssr_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.scene_prev_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.ssr_prev_depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.lit_mesh_ssr_sampler),
                },
            ],
        });
        self.bloom_scene_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-scene-bg"),
            layout: &self.bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.scene_color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });
        self.bloom_ping_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-ping-bg"),
            layout: &self.bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_ping_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });
        self.bloom_pong_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-pong-bg"),
            layout: &self.bloom_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.bloom_params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_pong_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });
        self.bloom_composite_bind_group =
            self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom-composite-bg"),
                layout: &self.bloom_composite_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.bloom_params_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&self.scene_color_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(&self.bloom_ping_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                    },
                ],
            });

        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [new_size.width as f32, new_size.height as f32],
                time: self.creation_time.elapsed().as_secs_f32(),
                // Gamma will be re-uploaded on the next render() call.
                gamma: 1.0,
                cursor_pos: [new_size.width as f32 * 0.5, new_size.height as f32 * 0.5],
                transition_progress: 0.0,
                quality_level: 2.0,
                moon_phase: current_moon_phase(),
                _globals_pad: [0.0; 3],
            }),
        );
        self.queue.write_buffer(
            &self.bloom_params_buffer,
            0,
            bytemuck::bytes_of(&BloomParams {
                data0: [1.1, 0.0, 1.0 / bloom_w as f32, 1.0 / bloom_h as f32],
                data1: [1.0, 0.0, 0.0, 0.0],
            }),
        );

        if let Some(ref mut fluid) = self.fluid {
            fluid.update_screen_size(new_size.width as f32, new_size.height as f32);
        }
        // Depth view was just recreated — the volumetric smoke pass needs a
        // fresh bind group that points at the new view.
        self.fluid_render_bg_dirty = true;
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
    pub fn render(
        &mut self,
        frame: &UiFrame,
        settings: RenderSettings,
    ) -> anyhow::Result<()> {
        let RenderSettings {
            smoke_quality,
            smoke_amount,
            effects_quality,
            tile_preset,
            tile_material,
            draw_settle_speed,
            sort_settle_speed,
            gamma,
            shadows_enabled,
            ssr_enabled,
        } = settings;
        // Encode the tile material choice into base_color_factor.w so the
        // tile_3d shader can branch on it (0 = bamboo, 1 = plastic, …).
        self.tile_base_color_factor[3] = tile_material.shader_id();

        // Hand tile fields removed — hand tiles now rendered via ShowcaseTileBatch.
        let hand_slots: &[(f32, f32, f32, f32)] = &[];
        let focus: usize = usize::MAX;
        let selected: &[bool] = &[];
        let hint_indices: &[usize] = &[];
        // Upload any relic/background textures that finished decoding.
        self.poll_relic_textures();
        self.poll_background_textures();

        // Acquire the per-frame texture to draw into. In the interactive
        // path this is a swapchain image; in headless screenshot mode it's
        // a plain render-attachment texture owned by `self.target`. Either
        // way we end up with a `&wgpu::Texture` (for the screenshot copy)
        // and a `TextureView` (for the render passes).
        let surface_frame: Option<wgpu::SurfaceTexture> = match &self.target {
            RenderTarget::Surface(surface) => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(t) => Some(t),
                wgpu::CurrentSurfaceTexture::Suboptimal(t) => Some(t),
                wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Outdated => {
                    surface.configure(&self.device, &self.config);
                    return Ok(());
                }
                wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Validation => {
                    return Ok(());
                }
            },
            RenderTarget::Offscreen { .. } => None,
        };
        let frame_texture: &wgpu::Texture = match (&surface_frame, &self.target) {
            (Some(sf), _) => &sf.texture,
            (None, RenderTarget::Offscreen { texture, .. }) => texture,
            (None, RenderTarget::Surface(_)) => {
                unreachable!("Surface target always produces a surface_frame or early-returns")
            }
        };
        let view = frame_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let scene_view = &self.scene_color_view;
        let bloom_active = {
            use crate::render::draw_cmd::Object3dKind;
            frame.cmds.iter().any(|cmd| match cmd {
                DrawCmd::MoonlitWater => true,
                DrawCmd::Object3d(obj) => matches!(obj.kind, Object3dKind::ShopLamp { .. }),
                DrawCmd::Object3dBatch(objs) => objs
                    .iter()
                    .any(|o| matches!(o.kind, Object3dKind::ShopLamp { .. })),
                _ => false,
            })
        };

        let _w = self.size.width.max(1) as f32;
        let _h = self.size.height.max(1) as f32;

        // Detect focus changes and start a 360° CW spin for the newly focused tile.
        if focus != self.last_focus {
            self.focus_spin = Some((focus, Instant::now()));
            self.last_focus = focus;
        }

        // Lerp per-tile slide animations toward 0 (ease-out).
        let now = Instant::now();
        let dt = now
            .saturating_duration_since(self.last_frame)
            .as_secs_f32()
            .min(0.05);
        self.last_frame = now;
        // Cache for downstream prep loops (bowl/mirror hover envelopes,
        // etc.) so they don't have to recompute or re-clamp the timestamp.
        self.frame_dt = dt;
        let slide_speed = draw_settle_speed; // higher = faster settle
        for y in self.tile_anim_y.iter_mut() {
            *y *= (-slide_speed * dt).exp(); // exponential ease-out
            if y.abs() < 0.5 {
                *y = 0.0;
            }
        }
        let slide_speed_x = sort_settle_speed; // horizontal settle for sort/drag
        for x in self.tile_anim_x.iter_mut() {
            *x *= (-slide_speed_x * dt).exp();
            if x.abs() < 0.01 {
                *x = 0.0;
            }
        }

        // Update globals with current time for animated shaders.
        let w_f = self.size.width as f32;
        let h_f = self.size.height as f32;
        let (cx, cy) = frame.cursor_pos.unwrap_or((w_f * 0.5, h_f * 0.5));
        self.queue.write_buffer(
            &self.globals_buffer,
            0,
            bytemuck::bytes_of(&Globals {
                screen: [w_f, h_f],
                time: self.creation_time.elapsed().as_secs_f32(),
                gamma: gamma.max(0.01),
                cursor_pos: [cx, cy],
                transition_progress: frame.transition_progress,
                quality_level: effects_quality.quality_level_f32(),
                moon_phase: current_moon_phase(),
                _globals_pad: [0.0; 3],
            }),
        );

        // Upload point lights for the tile shader (group 1). Scenes push
        // candle/spot lights into `frame.point_lights` in pixel-layout
        // coordinates; we map them onto the table-plane world for upload.
        let pl_w = self.size.width.max(1) as f32;
        let pl_h = self.size.height.max(1) as f32;
        self.queue.write_buffer(
            &self.point_lights_buffer,
            0,
            bytemuck::bytes_of(&PointLightsBuf::from_lights(
                &frame.point_lights,
                frame.candle_light_count,
                frame.flame_height_world,
                pl_w,
                pl_h,
                gamma,
                self.creation_time.elapsed().as_secs_f32(),
            )),
        );

        // Upload spotlights for the tile shader (group 3). Scenes push
        // directional cone lights into `frame.spot_lights`; only the tile
        // pipeline samples them (lit_mesh and the smoke lightbake don't).
        self.queue.write_buffer(
            &self.spot_lights_buffer,
            0,
            bytemuck::bytes_of(&SpotLightsBuf::from_lights(&frame.spot_lights, pl_w, pl_h)),
        );

        // Advance departing-tile timers. The two-phase trajectory is
        // recomputed analytically from `elapsed` in the render block
        // below, so the only state to update here is the clock + the
        // cull on tiles past their combined lifetime.
        for tile in self.departing_tiles.iter_mut() {
            tile.elapsed += dt;
        }
        self.departing_tiles.retain(|t| t.elapsed < t.lifetime);

        // Save hand slots for next frame's departure animations.
        self.prev_hand_slots = hand_slots.to_vec();

        // Build 2D backdrop quads (selection borders, hint pulses) and text
        // labels (just the focused arrow — the symbol+emoji live in the 3D
        // tile decal now).  Per-tile model matrices for the 3D mesh draw are
        // also written here.
        let mut tile_quads: Vec<GpuInstance> = Vec::new();
        let mut tile_labels: Vec<TextLabel> = Vec::new();
        let mut emoji_labels: Vec<TextLabel> = Vec::new();
        let mut tile_3d_rects: Vec<(usize, [f32; 4])> = Vec::new();
        // Per-tile world-space model matrices, snapshotted for next frame's
        // cursor pick (`pick_hand_tile`).
        let mut tile_pick_models: Vec<(usize, Mat4)> = Vec::new();
        // Additive glow halos for selected tiles, drawn behind the 3D
        // tile mesh as part of the Tiles3d render op.
        let mut tile_glows: Vec<GpuInstance> = Vec::new();
        // Additive glow halos for relics activated by the scoring cascade.
        // Populated below from the relic projection loop (we need each
        // relic's projected screen rect to size the halo). Drawn through
        // `tile_glow_pipeline` immediately after the 3D relic boxes so the
        // warm light blooms out around the box silhouette.
        let mut relic_glows: Vec<GpuInstance> = Vec::new();

        // ── Person-at-the-table camera ──────────────────────────────────
        // Z-up world, standard right-hand conventions: +X right, +Y into the
        // table (away from player), +Z up from the felt. Table is z = 0 (XY).
        // Camera sits at large -Y (behind the player), elevated in +Z, looking
        // toward +Y. See [`crate::render::world_space::pixel_to_world`] for the
        // pixel → world mapping:
        //
        //   world_x =  pixel_x - w * 0.5       (screen-right → +X)
        //   world_y =  h * 0.5 - pixel_y       (screen-bottom → -Y, toward player)
        //   world_z =  lift above the felt
        //
        // The 2D UI overlays (score panel, buttons, text) keep using the
        // pixel-orthographic quad pipeline and float over the 3D scene as
        // a HUD.
        let w = self.size.width.max(1) as f32;
        let h = self.size.height.max(1) as f32;
        let aspect = w / h;
        let (cam_pos, look_target, fov_y) = if let Some(ref c) = frame.camera_override {
            (
                glam::Vec3::from_array(c.eye),
                glam::Vec3::from_array(c.target),
                c.fovy_deg.to_radians(),
            )
        } else {
            let c = crate::render::draw_cmd::CameraParams::default_table_camera(h);
            (
                glam::Vec3::from_array(c.eye),
                glam::Vec3::from_array(c.target),
                c.fovy_deg.to_radians(),
            )
        };
        let up_v = frame
            .camera_override
            .as_ref()
            .map(|c| glam::Vec3::from_array(c.up))
            .unwrap_or(glam::Vec3::Z);
        let view_mat = Mat4::look_at_rh(cam_pos, look_target, up_v);
        let proj = Mat4::perspective_rh(fov_y, aspect, 1.0, h * 12.0);
        let view_proj = proj * view_mat;
        let view_proj_arr = view_proj.to_cols_array();

        // Upload the SSR globals so the lacquered-floor branch in
        // lit_mesh.wgsl can unproject screen-space depth taps and march
        // reflection rays in world space. Tunables match the plan:
        // ~24 linear steps with binary refinement, max distance scaled
        // to the screen height. Disabled when the user toggles SSR off.
        let ssr_max_distance = h * 2.0;
        let ssr_stride = h * 0.04;
        let ssr_max_steps = 24.0;
        self.queue.write_buffer(
            &self.lit_mesh_ssr_buffer,
            0,
            bytemuck::bytes_of(&SsrGlobals {
                inv_view_proj: view_proj.inverse().to_cols_array(),
                view_proj: view_proj_arr,
                view_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
                params: [
                    if ssr_enabled { 1.0 } else { 0.0 },
                    ssr_max_distance,
                    ssr_stride,
                    ssr_max_steps,
                ],
            }),
        );

        // Matching upload for the flame pipeline's view uniform. Same
        // camera, smaller struct — see `FlameViewUniform`.
        self.queue.write_buffer(
            &self.flame_view_buffer,
            0,
            bytemuck::bytes_of(&FlameViewUniform {
                view_proj: view_proj_arr,
                view_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
            }),
        );

        // Table plane mapping: see [`crate::render::world_space::pixel_to_world`].
        // Helper: project a world position to integer screen pixels for use
        // in 2D overlay quads (selection halos, hint pulses, hover arrows).
        let project_to_screen = |world: glam::Vec3| -> (f32, f32) {
            let clip = view_proj * glam::Vec4::new(world.x, world.y, world.z, 1.0);
            let inv_w = 1.0 / clip.w.max(1e-6);
            let nx = clip.x * inv_w;
            let ny = clip.y * inv_w;
            let sx = (nx * 0.5 + 0.5) * w;
            let sy = (1.0 - (ny * 0.5 + 0.5)) * h;
            (sx, sy)
        };

        // ── Debug axes overlay ──────────────────────────────────────────
        // When `frame.debug_axes` is set, write three thin colored boxes
        // (red = +X, green = +Y, blue = +Z) anchored at the current camera
        // look target. Each axis box extends from the origin in the
        // *positive* direction so the user can read sign as well as axis.
        if frame.debug_axes {
            // Length: a chunky fraction of screen height so the bars are
            // visible against the table from the default camera.
            let length = h * 0.35;
            let thickness = (h * 0.012).max(4.0);
            let origin = look_target;
            let axes: [(glam::Vec3, glam::Vec3, [f32; 4]); 3] = [
                // +X — red
                (
                    glam::Vec3::X,
                    glam::Vec3::new(length, thickness, thickness),
                    [1.6, 0.10, 0.10, 1.0],
                ),
                // +Y — green
                (
                    glam::Vec3::Y,
                    glam::Vec3::new(thickness, length, thickness),
                    [0.10, 1.6, 0.10, 1.0],
                ),
                // +Z — blue
                (
                    glam::Vec3::Z,
                    glam::Vec3::new(thickness, thickness, length),
                    [0.20, 0.40, 1.8, 1.0],
                ),
            ];
            for (i, (axis_dir, scale, color)) in axes.iter().enumerate() {
                // Center the box halfway down the positive axis so its
                // -end sits at `origin` and its +end sticks out by `length`.
                let center = origin + *axis_dir * (length * 0.5);
                let model = translate_rot_scale(center, Mat4::IDENTITY, *scale);
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: *color,
                    specular_strength: 0.0,
                    specular_power: 8.0,
                };
                if let Some(inst) = self.debug_axes_instances.get(i) {
                    inst.write_uniform(&self.queue, view_proj_arr, model, material);
                }
            }
        }

        // ── Flame emitters (world-space) ─────────────────────────────
        // Each candle in the scene becomes one emitter for the 3D flame
        // particle system. We walk the cmd list, find every Candle, and
        // project the wick tip into world space using the same
        // `pixel_to_world` mapping the rest of the scene uses. The
        // `DrawCmd::Flame` batch loop below consumes the candle-ordered
        // list of emitters in submission order.
        //
        // Brightness + phase are pulled from the scene's per-candle
        // `GpuInstance`: `color.b` = brightness, `color.a` = phase. The
        // anchor loop also computes a per-candle wind vector by sampling
        // `frame.wind_gusts` at the wick's world position so the particle
        // system can lean the plume in real wind.
        let flame_emitters: Vec<crate::render::flame_particles::FlameEmitter> = {
            let mut out: Vec<crate::render::flame_particles::FlameEmitter> = Vec::new();
            // Walk all Object3ds in the frame, picking out Candles in
            // submission order (matches scene `frame.flames(...)` order).
            let candles: Vec<(&crate::render::draw_cmd::Object3d, f32, f32)> = frame
                .cmds
                .iter()
                .flat_map(|cmd| {
                    let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> =
                        match cmd {
                            DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                            DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                            _ => Box::new(std::iter::empty()),
                        };
                    objs.filter_map(|o| {
                        if let crate::render::draw_cmd::Object3dKind::Candle {
                            scale,
                            height_scale,
                        } = o.kind
                        {
                            Some((o, scale, height_scale))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                })
                .collect();
            // Scene-supplied per-flame data (brightness + phase), pulled
            // out of the cmd stream in the same order candles appear.
            let mut flame_cmd_iter = frame.cmds.iter().filter_map(|cmd| match cmd {
                DrawCmd::Flame(inst) => Some(*inst),
                _ => None,
            });
            for (o, p_scale, p_height) in candles.into_iter() {
                let p_pos = o.pos;
                let tip_world = pixel_to_world(
                    w,
                    h,
                    p_pos[0],
                    p_pos[1],
                    crate::render::candle_mesh::WICK_TIP_Y * p_scale * p_height,
                );
                let scene_inst = flame_cmd_iter.next();
                let (brightness, phase) = scene_inst
                    .map(|inst| (inst.color[2], inst.color[3]))
                    .unwrap_or((1.0, 0.0));

                // Sample scene wind gusts at the wick, weighted by a
                // soft falloff around each gust's world-space radius.
                // Convert the resulting world-space velocity into the
                // flame-relative units the particle system uses.
                let mut wind_world = glam::Vec3::ZERO;
                for g in frame.wind_gusts.iter() {
                    let g_world = pixel_to_world(w, h, g.center_px.0, g.center_px.1, g.lift);
                    let dist = (g_world - tip_world).length();
                    let r = (g.radius * 3.0).max(1.0);
                    let falloff = (1.0 - (dist / r).clamp(0.0, 1.0)).powf(1.5);
                    if falloff <= 0.0 {
                        continue;
                    }
                    wind_world +=
                        glam::Vec3::new(g.velocity[0], g.velocity[1], g.velocity[2]) * falloff;
                }
                // Flame-relative wind: normalise against a reference
                // per-candle velocity so neighbouring candles react to
                // the same gust by the same visible amount. 300 units/s
                // → 1.0 in flame-relative space is the heuristic that
                // matched the previous 2D behaviour.
                let wind_scale = 1.0 / 300.0;
                let wind = glam::Vec2::new(
                    (wind_world.x * wind_scale).clamp(-1.5, 1.5),
                    (wind_world.z * wind_scale).clamp(-1.5, 1.5),
                );

                out.push(crate::render::flame_particles::FlameEmitter {
                    wick_world: tip_world,
                    // Scale the particle size to the candle's physical
                    // scale. A candle drawn at scale `p_scale * p_height`
                    // (world units) should produce a plume whose width
                    // is a fraction of that scale; 0.22 lines up with
                    // the previous 2D flame's visual width.
                    scale: p_scale * p_height * 0.22,
                    wind,
                    brightness,
                    phase,
                });
            }
            out
        };

        let tile_basis = tile_mesh_local_to_world();
        // After tile_basis, the tile lies flat with face normal pointing +Z.
        // Rx(+π/2) rotates +Z → -Y so the face points toward the camera (at large -Y).
        let hand_tile_face_to_camera = rot_x_rad(std::f32::consts::PI / 2.0);

        {
            for (i, _htg_ref) in self.hand_tiles.iter().enumerate() {
                let Some(&(sx, sy, sw, sh)) = hand_slots.get(i) else {
                    continue;
                };
                let is_focused = i == focus;
                let is_selected = selected.get(i).copied().unwrap_or(false);
                let slide_y = self.tile_anim_y.get(i).copied().unwrap_or(0.0);
                let slide_x_slots = self.tile_anim_x.get(i).copied().unwrap_or(0.0);

                // Tile face dimensions in pixel units (pre-projection). The
                // long axis runs front-back on the table. The face aspect
                // and thickness come from the user-selected regional preset
                // (Wikipedia gives Chinese 30×20×15, Japanese 26×19×16,
                // American 32×25×19) so swapping presets actually changes
                // the tile shape, not just a uniform scale.
                let tile_short_px = sw * 0.85; // left-right footprint on the table
                let tile_long_px = tile_short_px * tile_preset.face_long_ratio();
                let tile_thickness_px = tile_short_px * tile_preset.thickness_ratio();

                // Tile center in pixel-layout coords (toward bottom of slot = toward player).
                let cx_px = sx + sw * 0.5 + slide_x_slots * sw;
                // The slide_y residual still pushes the tile briefly; larger
                // `py` → more −world Y (nearer player; see [`pixel_to_world`]).
                let cy_px = sy + sh * crate::ui::layout::HAND_TILE_MESH_Y_FRAC + slide_y;

                // World position: laid flat just above the table.
                let world_y_lift = tile_thickness_px * 0.5 + 4.0;
                let world = pixel_to_world(w, h, cx_px, cy_px, world_y_lift);

                // Smoke impulse from per-tile motion: compare to last
                // frame's world position for this uid and inject the
                // delta as velocity. Skip the first frame (no history).
                if let Some(uid) = self.tile_uids.get(i).copied() {
                    if let Some(prev) = self.prev_tile_world.get(&uid).copied() {
                        let delta = world - prev;
                        let speed = delta.length();
                        if speed > 0.5
                            && let Some(ref mut fluid) = self.fluid
                        {
                            let inv_dt = 1.0 / dt.max(1.0 / 120.0);
                            fluid.inject_impulse(
                                world,
                                delta * inv_dt * 0.45,
                                tile_short_px * 0.55,
                                speed * 0.04,
                                0.0,
                                0.0,
                            );
                        }
                    }
                    self.prev_tile_world.insert(uid, world);
                }

                let r_static = hand_tile_face_to_camera * tile_basis;

                // Tilt rotation, computed once and reused for both the
                // model matrix below and the overlay-anchor projection.
                // Pivot: bottom‑front corner in **world** Z-up axes (after
                // `hand_tile_face_to_camera` * `tile_mesh_local_to_world` * scale):
                // +Y = along table toward larger `py`, +Z = up from felt.
                let tilt_angle = 22.0_f32.to_radians();
                let tilt_pivot = hand_tile_face_to_camera.transform_point3(glam::Vec3::new(
                    0.0,
                    tile_long_px * 0.5,
                    -tile_thickness_px * 0.5,
                ));
                let tilt = rotation_around_point_x_rad(tilt_pivot, tilt_angle);

                // Helper: offset from tile center in **world** axes after
                // `r_static` (mesh → world, no tilt), then tilt and project.
                let tilted_to_screen = |pre_tilt: glam::Vec3| -> (f32, f32) {
                    let tilted = tilt.transform_point3(pre_tilt);
                    project_to_screen(world + tilted)
                };

                // Project the tile center to screen space so 2D overlay
                // anchors (selection halo, hint pulse, hover arrow) follow
                // the tile's actual on-screen position under the tilted
                // camera.
                let (proj_cx, proj_cy) = tilted_to_screen(glam::Vec3::ZERO);
                // Project all 8 corners of the tilted slab and take the
                // actual screen-space AABB. Earlier this used a single
                // back-top corner mirrored around the projected center,
                // which underestimates the rect for tiles off the optical
                // axis: under perspective the silhouette is asymmetric
                // around the projected center, so click/hover hit-testing
                // felt off near the camera edges.
                let lx = tile_long_px * 0.5; // mesh local ±X (long)
                let ly = tile_thickness_px * 0.5; // mesh local ±Y (thick)
                let lz = tile_short_px * 0.5; // mesh local ±Z (short)
                let corners = [
                    glam::Vec3::new(-lx, -ly, -lz),
                    glam::Vec3::new(lx, -ly, -lz),
                    glam::Vec3::new(-lx, ly, -lz),
                    glam::Vec3::new(lx, ly, -lz),
                    glam::Vec3::new(-lx, -ly, lz),
                    glam::Vec3::new(lx, -ly, lz),
                    glam::Vec3::new(-lx, ly, lz),
                    glam::Vec3::new(lx, ly, lz),
                ];
                let mut min_x = f32::INFINITY;
                let mut min_y = f32::INFINITY;
                let mut max_x = f32::NEG_INFINITY;
                let mut max_y = f32::NEG_INFINITY;
                for c in corners {
                    let pre_tilt = r_static.transform_point3(c);
                    let (px, py) = tilted_to_screen(pre_tilt);
                    if px < min_x {
                        min_x = px;
                    }
                    if py < min_y {
                        min_y = py;
                    }
                    if px > max_x {
                        max_x = px;
                    }
                    if py > max_y {
                        max_y = py;
                    }
                }
                let overlay_w = (max_x - min_x).max(16.0);
                let overlay_h = (max_y - min_y).max(16.0);
                let overlay_x = min_x;
                let overlay_y = min_y;
                // Keep the projected center available for downstream
                // anchors that want a tile-centered point rather than the
                // AABB.
                let _ = (proj_cx, proj_cy);

                // Selected tiles get a 3D gold-metal outline shell drawn
                // by the outline pipeline below, plus an additive radial
                // glow halo (built here, drawn at the start of the
                // Tiles3d render op so it sits behind the tile mesh).
                if is_selected {
                    // Glow rect ~2× the tile in both axes so the falloff
                    // has room to spill out around the silhouette.
                    let gw = overlay_w * 2.10;
                    let gh = overlay_h * 2.20;
                    let gx = overlay_x + (overlay_w - gw) * 0.5;
                    let gy = overlay_y + (overlay_h - gh) * 0.5;
                    tile_glows.push(GpuInstance {
                        rect: [gx, gy, gw, gh],
                        // Warm champagne gold. The alpha channel scales
                        // overall intensity inside the glow shader.
                        color: [1.00, 0.78, 0.32, 1.10],
                    });
                }

                // Hint tiles get a vertical light beam (built below) but no
                // border-style halo — the rectangular halo reads as a
                // selection indicator and confused which tiles are actually
                // selected.

                tile_3d_rects.push((i, [overlay_x, overlay_y, overlay_w, overlay_h]));

                // Hover arrow above the focused tile (in screen space).
                if is_focused {
                    let bob_period = 1.5_f32;
                    let bob_amp = overlay_h * 0.08;
                    let bob_y = (self.creation_time.elapsed().as_secs_f32() / bob_period
                        * std::f32::consts::TAU)
                        .sin()
                        * bob_amp;
                    let arrow_h = overlay_h * 0.32;
                    let arrow_w = overlay_w * 0.65;
                    let arrow_x = overlay_x + (overlay_w - arrow_w) * 0.5;
                    let arrow_y = overlay_y - arrow_h - overlay_h * 0.05 + bob_y;
                    tile_labels.push(TextLabel {
                        rect: [arrow_x, arrow_y, arrow_w, arrow_h],
                        text: "▼".to_string(),
                        color: [0.85, 0.1, 0.1, 1.0],
                        ..Default::default()
                    });
                }

                // Build the per-tile model matrix and write its uniform.
                if let Some(htg) = self.hand_tiles.get(i) {
                    let scale = glam::Vec3::new(
                        tile_long_px / LOCAL_X_EXTENT,
                        tile_thickness_px / LOCAL_Y_EXTENT,
                        tile_short_px / LOCAL_Z_EXTENT,
                    ); // local X,Y,Z — oriented by [`tile_mesh_local_to_world`]
                    let oriented = tilt * hand_tile_face_to_camera * tile_basis;
                    // Pack enhancement kind into .z so the tile shader can
                    // apply fresnel-masked sheen effects per-enhancement.
                    let mut bcf = self.tile_base_color_factor;
                    // Channels .x and .y carry showcase-tile flags
                    // (brightness, selection). Hand tiles use the outline
                    // shell + glow halo instead, so force neutral values.
                    bcf[0] = 1.0;
                    bcf[1] = 0.0;
                    bcf[2] = htg.tile_id.2.map_or(0.0, |e| e.shader_id());
                    // When this tile is selected, also write an inflated
                    // model matrix into the outline shell uniform so the
                    // outline pipeline draws a slightly larger version of
                    // the same mesh around the tile silhouette.
                    // Slot index drives per-tile procedural variation (e.g.
                    // tortoise shell mottling) in tile_3d.wgsl.
                    let tile_seed = i as f32;
                    if is_selected {
                        // ~5–6% larger; tuned so the rim is visible without
                        // overlapping neighbouring tiles.
                        const OUTLINE_GROW: f32 = 1.055;
                        let outline_scale = scale * OUTLINE_GROW;
                        let outline_model = translate_rot_scale(world, oriented, outline_scale);
                        self.queue.write_buffer(
                            &htg.outline_uniform_buffer,
                            0,
                            bytemuck::bytes_of(&CameraUniform {
                                view_proj: view_proj_arr,
                                model: outline_model.to_cols_array(),
                                base_color_factor: bcf,
                                cam_pos: cam_pos.to_array(),
                                tile_seed,
                            }),
                        );
                    }
                    // `tilt` was computed above the projection block so
                    // both the model matrix and the overlay anchors share
                    // the same rotation.
                    let model = translate_rot_scale(world, oriented, scale);
                    // Snapshot for next frame's cursor pick.
                    tile_pick_models.push((i, model));
                    self.queue.write_buffer(
                        &htg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: bcf,
                            cam_pos: cam_pos.to_array(),
                            tile_seed,
                        }),
                    );
                }
            }
        }

        // Snapshot deferred to after the showcase pre-pass so that
        // ShowcaseTileBatch tiles with pick_id also land in hand_rects and
        // last_pick_models (the showcase pre-pass runs further below).

        // Tile hints are now real green PointLights pushed by the gameplay
        // scene into `frame.point_lights` (see the hint-lights block in
        // `scenes/gameplay.rs`). The 2D fake "light beam" overlay that
        // used to live here was removed in favour of letting the real
        // lighting model do the work — the hinted tile picks up a green
        // top-down pool through the same shader path as the candles.
        let _ = hint_indices;

        // Render departing tiles (two-phase: arc into river, then drift).
        for dep in &self.departing_tiles {
            let t = dep.elapsed.max(0.0);
            let w = dep.start_rect[2];
            let h = dep.start_rect[3];
            let start_cx = dep.start_rect[0] + w * 0.5;
            let start_cy = dep.start_rect[1] + h * 0.5;

            // Phase split: Arcing → Drifting at t = arc_dur.
            let (cx, cy, alpha, scale) = if t < dep.arc_dur {
                // Phase 1 — quadratic Bezier from the hand slot, over an
                // apex above the midpoint, into the river center. The
                // apex sits 110px above the higher of the two endpoints
                // so the tile reads as being *thrown* upward before
                // arcing down into the water rather than sliding in a
                // straight line.
                let u = (t / dep.arc_dur).clamp(0.0, 1.0);
                let mid_x = (start_cx + dep.river_target.0) * 0.5;
                let mid_y = start_cy.min(dep.river_target.1) - 110.0;
                let one_u = 1.0 - u;
                let bx =
                    one_u * one_u * start_cx + 2.0 * one_u * u * mid_x + u * u * dep.river_target.0;
                let by =
                    one_u * one_u * start_cy + 2.0 * one_u * u * mid_y + u * u * dep.river_target.1;
                // Slight shrink during the arc — the tile reads as
                // moving away from the camera as it falls into the
                // recessed water surface.
                let s = 1.0 - 0.18 * u;
                (bx, by, 1.0, s)
            } else {
                // Phase 2 — drift downstream and fade. Position
                // continues from the splash point along `drift_dir` at
                // `drift_speed`. Alpha eases from 1 → 0 over the drift
                // duration; scale shrinks further so the tile reads as
                // sinking into the water.
                let dt2 = t - dep.arc_dur;
                let u2 = (dt2 / dep.drift_dur).clamp(0.0, 1.0);
                let dx = dep.drift_dir.0 * dep.drift_speed * dt2;
                let dy = dep.drift_dir.1 * dep.drift_speed * dt2;
                let bx = dep.river_target.0 + dx;
                let by = dep.river_target.1 + dy;
                let a = 1.0 - u2;
                let s = 0.82 - 0.40 * u2;
                (bx, by, a, s)
            };

            let sw = w * scale;
            let sh = h * scale;
            let sx = cx - sw * 0.5;
            let sy = cy - sh * 0.5;

            // Tile background.
            tile_quads.push(GpuInstance {
                rect: [sx, sy, sw, sh],
                color: [0.0, 0.0, 0.0, alpha],
            });

            // Main label.
            let inset_x = sw * 0.10;
            let top_h = sh * 0.50;
            tile_labels.push(TextLabel {
                rect: [sx + inset_x, sy + sh * 0.05, sw - inset_x * 2.0, top_h],
                text: dep.symbol.clone(),
                color: [
                    dep.suit_color[0],
                    dep.suit_color[1],
                    dep.suit_color[2],
                    alpha,
                ],
                ..Default::default()
            });

            // Suit emoji.
            let bot_h = sh * 0.40;
            emoji_labels.push(TextLabel {
                rect: [sx + inset_x, sy + sh * 0.55, sw - inset_x * 2.0, bot_h],
                text: dep.suit_emoji.clone(),
                color: [
                    dep.suit_color[0],
                    dep.suit_color[1],
                    dep.suit_color[2],
                    alpha,
                ],
                ..Default::default()
            });
        }

        // Tile glow instance buffer (additive halo behind selected tiles).
        let tile_glow_buffer = if tile_glows.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("tile-glow-instances"),
                        contents: bytemuck::cast_slice(&tile_glows),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        // ── Pre-rasterize text labels → GPU textures + bind groups ──────
        struct TextDraw {
            inst_buf: wgpu::Buffer,
            bind_group: wgpu::BindGroup,

            _tex: wgpu::Texture,
        }
        let make_text_draw = |device: &wgpu::Device,
                              queue: &wgpu::Queue,
                              text_bgl: &wgpu::BindGroupLayout,
                              sampler: &wgpu::Sampler,
                              lbl: &TextLabel,
                              font: &fontdue::Font,
                              emoji_fallback: Option<&fontdue::Font>|
         -> TextDraw {
            // Clamp before casting: `f32 as u32` saturates negatives/NaN to u32::MAX,
            // which blows past wgpu's 16384 texture limit and panics. Seen in arrange mode
            // when layout math produces a negative rect width.
            let tw = (lbl.rect[2].clamp(1.0, 16384.0) as u32).max(1);
            let th = (lbl.rect[3].clamp(1.0, 16384.0) as u32).max(1);
            let align = match lbl.align {
                TextAlign::Left => LabelAlign::Left,
                TextAlign::Center => LabelAlign::Center,
                TextAlign::Right => LabelAlign::Right,
            };
            let rgba = rasterize_label_styled_with_fallback(
                font,
                emoji_fallback,
                &lbl.text,
                tw,
                th,
                crate::render::decal::LabelStyle {
                    font_px: lbl.font_px,
                    align,
                    scroll_offset: lbl.scroll_offset,
                },
            );
            let (tex, view) = upload_rgba_texture(device, queue, "text-lbl", &rgba, tw, th);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("text-lbl-bg"),
                layout: text_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            });
            let inst = GpuInstance {
                rect: lbl.rect,
                color: lbl.color,
            };
            let inst_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("text-inst"),
                contents: bytemuck::cast_slice(&[inst]),
                usage: wgpu::BufferUsages::VERTEX,
            });
            TextDraw {
                inst_buf,
                bind_group,
                _tex: tex,
            }
        };

        // ── Hand tile face/emoji label GPU draws (consumed by HandTileFaces) ──
        let mut hand_face_draws: Vec<TextDraw> = Vec::new();
        if let Some(ref font) = self.ui_font {
            for lbl in &tile_labels {
                hand_face_draws.push(make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
                    lbl,
                    font,
                    None,
                ));
            }
        }
        if let Some(ref font) = self.emoji_font {
            for lbl in &emoji_labels {
                hand_face_draws.push(make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
                    lbl,
                    font,
                    None,
                ));
            }
        }

        // ── Walk frame.cmds; build per-cmd GPU resources + a parallel ─────
        // ── ordered op list, batching contiguous Quad runs into a single ──
        // ── instanced draw. ────────────────────────────────────────────────
        enum RenderOp {
            Background(BackgroundId),
            Starfield,
            EmberDrift,
            GoldenDust,
            MoonlitWater,
            SunlitWater,
            MountainHaze,
            ShootingStarCascade,
            Table,
            QuadBatch { buf_idx: usize, count: u32 },
            GradientQuadBatch { buf_idx: usize, count: u32 },
            FlameBatch { buf_idx: usize, count: u32 },
            TextDraw(usize),
            TileFaceQuad(usize),
            FluidSmoke,
            // Skeuomorphic gameplay HUD (phase 1).
            ShowcaseTileBatch(usize), // index into `showcase_tile_batches`
            Object3dBatch { start: usize, end: usize }, // range into `object3d_draw_list`
        }

        // Each Object3dKind that gets drawn through the lit-mesh pipeline
        // gets one variant here. The pre-pass pushes `(DrawKind, slot_i)`
        // into `object3d_draw_list`; the dispatch loop matches on the
        // variant to pick the right mesh + instance pool. Keeping this as
        // an enum (rather than raw u8 ids) means the compiler catches any
        // collision or missing dispatch arm.
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum DrawKind {
            YakuTablet,
            WoodTablet,
            Relic,
            Pack,
            Ribbon,
            Talisman,
            Shrine,
            SellTray,
            LampBody,
            LampBulb,
            BugBody,
            BugWingL,
            BugWingBlurL,
            Orb,
            DoraPlinth,
            Bowl,
            Mirror,
            TallyStickBase,
            TallyStickTip,
            CandleWax,
            CandleWick,
            CascadeToken,
            ExtrudedGlyph,
            BugWingR,
            BugWingBlurR,
            Primitive(crate::render::primitive::MeshId),
        }

        let mut quad_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut gradient_quad_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut flame_buffers: Vec<wgpu::Buffer> = Vec::new();
        let mut text_draws: Vec<TextDraw> = Vec::new();
        let mut tile_face_quads: Vec<TileFaceQuad> = Vec::new();
        let mut tile_face_inst_buffers: Vec<wgpu::Buffer> = Vec::new();
        // Skeuomorphic gameplay HUD cmd buffers (phase 1).
        // Dead empty vecs — kept so existing shadow/draw loops that still iterate
        // these compile; scenes no longer push to these variants.
        let shrine_batches: Vec<&[ShrinePlacement]> = Vec::new();
        let yaku_tablet_batches: Vec<&[YakuTabletPlacement]> = Vec::new();
        let wall_stack_cmds: Vec<&WallStackPlacement> = Vec::new();
        let mut showcase_tile_batches: Vec<&[ShowcaseTilePlacement]> = Vec::new();
        let mut object3d_cmds: Vec<&[crate::render::draw_cmd::Object3d]> = Vec::new();
        // Flat draw list built during the Object3d pre-pass: (DrawKind, slot_i).
        let mut object3d_draw_list: Vec<(DrawKind, usize)> = Vec::new();
        let mut ops: Vec<RenderOp> = Vec::new();

        let mut i = 0;
        while i < frame.cmds.len() {
            match &frame.cmds[i] {
                DrawCmd::Background(id) => {
                    ops.push(RenderOp::Background(*id));
                    i += 1;
                }
                DrawCmd::Starfield => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::Starfield);
                    }
                    i += 1;
                }
                DrawCmd::EmberDrift => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::EmberDrift);
                    }
                    i += 1;
                }
                DrawCmd::GoldenDust => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::GoldenDust);
                    }
                    i += 1;
                }
                DrawCmd::MoonlitWater => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::MoonlitWater);
                    }
                    i += 1;
                }
                DrawCmd::SunlitWater => {
                    if effects_quality >= crate::persistence::EffectsQuality::Medium {
                        ops.push(RenderOp::SunlitWater);
                    }
                    i += 1;
                }
                DrawCmd::MountainHaze => {
                    ops.push(RenderOp::MountainHaze);
                    i += 1;
                }
                DrawCmd::ShootingStarCascade => {
                    if effects_quality >= crate::persistence::EffectsQuality::Low {
                        ops.push(RenderOp::ShootingStarCascade);
                    }
                    i += 1;
                }
                DrawCmd::Table => {
                    ops.push(RenderOp::Table);
                    i += 1;
                }
                DrawCmd::FluidSmoke => {
                    ops.push(RenderOp::FluidSmoke);
                    i += 1;
                }
                DrawCmd::Quad(_) => {
                    // Collect contiguous run of Quad cmds into a single batch.
                    let mut batch: Vec<GpuInstance> = Vec::new();
                    while let Some(DrawCmd::Quad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("quad-batch"),
                            contents: bytemuck::cast_slice(&batch),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let buf_idx = quad_buffers.len();
                    quad_buffers.push(buf);
                    ops.push(RenderOp::QuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::GradientQuad(_) => {
                    let mut batch: Vec<GradientQuadInstance> = Vec::new();
                    while let Some(DrawCmd::GradientQuad(inst)) = frame.cmds.get(i) {
                        batch.push(*inst);
                        i += 1;
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("gradient-quad-batch"),
                            contents: bytemuck::cast_slice(&batch),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let buf_idx = gradient_quad_buffers.len();
                    gradient_quad_buffers.push(buf);
                    ops.push(RenderOp::GradientQuadBatch {
                        buf_idx,
                        count: batch.len() as u32,
                    });
                }
                DrawCmd::Flame(_) => {
                    // Drain the contiguous run of Flame cmds. Per-flame
                    // brightness + phase were already harvested into
                    // `flame_emitters` above, so here we just advance `i`
                    // and emit a single `FlameBatch` op that the dispatch
                    // side will expand into particle-system state.
                    while let Some(DrawCmd::Flame(_)) = frame.cmds.get(i) {
                        i += 1;
                    }
                    // Step the particle system once per frame and upload
                    // the live particles into a fresh instance buffer.
                    // Smoke-on paths skip the actual draw (see the
                    // `FlameBatch` branch below), but we still step so
                    // smoke → no-smoke toggles mid-game don't suddenly
                    // drop an empty pool into view.
                    self.flame_particles.step(&flame_emitters, self.frame_dt);
                    let count = self
                        .flame_particles
                        .fill_gpu_instances(&flame_emitters, &mut self.flame_particle_staging);
                    if count == 0 {
                        // Nothing to draw yet (first frame, or all
                        // particles expired during a pause). Still push
                        // the op so the downstream code has a consistent
                        // shape; the dispatch side handles count=0.
                        ops.push(RenderOp::FlameBatch {
                            buf_idx: usize::MAX,
                            count: 0,
                        });
                    } else {
                        let buf =
                            self.device
                                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                                    label: Some("flame-particles"),
                                    contents: bytemuck::cast_slice(
                                        &self.flame_particle_staging[..count],
                                    ),
                                    usage: wgpu::BufferUsages::VERTEX,
                                });
                        let buf_idx = flame_buffers.len();
                        flame_buffers.push(buf);
                        ops.push(RenderOp::FlameBatch {
                            buf_idx,
                            count: count as u32,
                        });
                    }
                }
                DrawCmd::Text(lbl) => {
                    if let Some(ref font) = self.ui_font {
                        let td = make_text_draw(
                            &self.device,
                            &self.queue,
                            &self.text_bind_group_layout,
                            &self.tile_sampler,
                            lbl,
                            font,
                            self.emoji_font.as_ref(),
                        );
                        let idx = text_draws.len();
                        text_draws.push(td);
                        ops.push(RenderOp::TextDraw(idx));
                    }
                    i += 1;
                }
                DrawCmd::TileFaceQuad(face) => {
                    let key = (
                        face.tile.suit,
                        face.tile.rank,
                        face.tile.enhancement,
                        face.tile.debuffed_visual,
                    );
                    if !self.tile_face_overlays.contains_key(&key) {
                        let overlay = make_tile_face_overlay_gpu(
                            &self.device,
                            &self.queue,
                            &self.text_bind_group_layout,
                            &self.tile_sampler,
                            self.ui_font.as_ref(),
                            self.emoji_font.as_ref(),
                            &face.tile,
                            self.tile_set.as_deref(),
                        );
                        self.tile_face_overlays.insert(key, overlay);
                    }
                    let buf = self
                        .device
                        .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                            label: Some("tile-face-quad"),
                            contents: bytemuck::cast_slice(&[face.inst]),
                            usage: wgpu::BufferUsages::VERTEX,
                        });
                    let idx = tile_face_quads.len();
                    tile_face_quads.push(*face);
                    tile_face_inst_buffers.push(buf);
                    ops.push(RenderOp::TileFaceQuad(idx));
                    i += 1;
                }
                DrawCmd::ShowcaseTileBatch(placements) => {
                    let idx = showcase_tile_batches.len();
                    showcase_tile_batches.push(placements.as_slice());
                    ops.push(RenderOp::ShowcaseTileBatch(idx));
                    i += 1;
                }
                DrawCmd::GlossaryAnchor { .. } => {
                    // Pure metadata for the tooltip overlay; no draw work.
                    i += 1;
                }
                DrawCmd::Object3d(obj) => {
                    object3d_cmds.push(std::slice::from_ref(obj));
                    // start/end will be filled in during the pre-pass; push placeholder
                    ops.push(RenderOp::Object3dBatch { start: 0, end: 0 });
                    i += 1;
                }
                DrawCmd::Object3dBatch(objs) => {
                    object3d_cmds.push(objs.as_slice());
                    ops.push(RenderOp::Object3dBatch { start: 0, end: 0 });
                    i += 1;
                }
            }
        }

        // ── Debug axes overlay labels ───────────────────────────────────
        // After walking the scene's cmds, append three text labels (one per
        // axis) projected from the world-space tip of each debug-axes bar.
        // These get rasterized into ordinary text draws so they ride along
        // in the same render pass as the bars themselves.
        if frame.debug_axes
            && let Some(ref font) = self.ui_font
        {
            let length = h * 0.35;
            let label_size = (h * 0.04).max(18.0);
            let label_w = label_size * 3.5;
            let label_h = label_size * 1.5;
            let labels: [(glam::Vec3, &str, [f32; 4]); 3] = [
                (
                    look_target + glam::Vec3::X * length,
                    "+X",
                    [1.0, 0.25, 0.25, 1.0],
                ),
                (
                    look_target + glam::Vec3::Y * length,
                    "+Y",
                    [0.25, 1.0, 0.25, 1.0],
                ),
                (
                    look_target + glam::Vec3::Z * length,
                    "+Z",
                    [0.45, 0.65, 1.0, 1.0],
                ),
            ];
            for (tip_world, text, color) in labels.iter() {
                let (sx, sy) = project_to_screen(*tip_world);
                let lbl = TextLabel {
                    rect: [sx - label_w * 0.5, sy - label_h * 0.5, label_w, label_h],
                    text: (*text).to_string(),
                    color: *color,
                    font_px: Some(label_size),
                    align: TextAlign::Center,
                    no_glossary: true,
                    ..Default::default()
                };
                let td = make_text_draw(
                    &self.device,
                    &self.queue,
                    &self.text_bind_group_layout,
                    &self.tile_sampler,
                    &lbl,
                    font,
                    None,
                );
                let idx = text_draws.len();
                text_draws.push(td);
                ops.push(RenderOp::TextDraw(idx));
            }
        }

        // ── Update procedural lit-mesh uniforms (table + candles) ───────
        // Written before the render pass begins, since the pass borrows
        // `self` immutably.
        let needs_table = ops.iter().any(|o| matches!(o, RenderOp::Table));
        if needs_table {
            // Horizontal table: mesh is local XY with +Z normal; Y-up mesh
            // chain uses Rx(-90°) so the felt normal is +Y in that basis, then
            // [`translate_rot_scale`] maps to world +Z. Wood grain is
            // evaluated in world XY in the shader.
            let table_extent = h * 30.0;
            let table_w = table_extent;
            let table_d = table_extent;
            let model = translate_rot_scale(
                glam::Vec3::ZERO,
                table_mesh_lay_flat(),
                glam::Vec3::new(table_w, table_d, 1.0),
            );
            self.table_instance.write_uniform(
                &self.queue,
                view_proj_arr,
                model,
                self.table_mesh.default_material,
            );
        }
        // Reset the debug pickable catch-all for this frame; each draw
        // loop below appends entries it wants to expose to
        // `pick_debug_object`.
        self.last_debug_pickables.clear();
        self.last_debug_trimesh_pickables.clear();

        // Candles migrated to Object3dKind::Candle.

        // ── Relic placeholders (migrated to Object3dKind::Relic) ──────
        self.last_relic_models.clear();
        self.last_pickable_relic_models.clear();
        let mut relic_slot_cursor: usize = 0;
        let _ = &mut relic_slot_cursor;

        // ── Pack placeholders (same mesh + pipeline as relics) ──────────
        self.proj.pack_rects.clear();
        // Pack placements migrated to Object3dKind::Pack.
        // ── Shrines (pick-blind scene). Each placement gets its own slot. ─
        // The shrine mesh is built in normalized -0.5..+0.5 local space, so
        // a per-instance scale by `extents` sizes Small/Big/Boss
        // independently. `world_pos` is the *base center*, so we lift the
        // model up by half the height to put the plinth on the ground.
        self.proj.shrine_rects.clear();
        {
            let mut shrine_cursor: usize = 0;
            for batch in &shrine_batches {
                for s in batch.iter() {
                    if shrine_cursor >= MAX_SHRINE_SLOTS {
                        break;
                    }
                    let slot_i = shrine_cursor;
                    shrine_cursor += 1;
                    let center = pixel_to_world(
                        w,
                        h,
                        s.world_pos[0],
                        s.world_pos[1],
                        s.world_pos[2] + s.extents[1] * 0.5,
                    );
                    let model = translate_rot_scale(
                        center,
                        Mat4::IDENTITY,
                        glam::Vec3::new(s.extents[0], s.extents[1], s.extents[2]),
                    );
                    // Project the shrine's 8 AABB corners to screen and
                    // take the bounding rect — gives the scene a 2D rect
                    // it can anchor labels to without re-projecting the
                    // perspective transform itself.
                    let hx = s.extents[0] * 0.5;
                    let hy = s.extents[1] * 0.5;
                    let hz = s.extents[2] * 0.5;
                    let mut mn_x = f32::INFINITY;
                    let mut mn_y = f32::INFINITY;
                    let mut mx_x = f32::NEG_INFINITY;
                    let mut mx_y = f32::NEG_INFINITY;
                    for cx in [-hx, hx] {
                        for cy in [-hy, hy] {
                            for cz in [-hz, hz] {
                                let world = center + glam::Vec3::new(cx, cy, cz);
                                let (px, py) = project_to_screen(world);
                                mn_x = mn_x.min(px);
                                mn_y = mn_y.min(py);
                                mx_x = mx_x.max(px);
                                mx_y = mx_y.max(py);
                            }
                        }
                    }
                    self.proj
                        .shrine_rects
                        .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                    // Glow gently brightens the shrine's tint so the
                    // upcoming shrine reads as the active choice at
                    // rest, but most of the warmth still comes from
                    // the warm spotlight tinting the stone — the
                    // shrine itself shouldn't self-illuminate.
                    let g = s.glow.clamp(0.0, 1.0);
                    let base_color = if g > 0.0 {
                        let target = [1.10, 1.05, 0.95, s.color[3]];
                        [
                            s.color[0] + (target[0] - s.color[0]) * g,
                            s.color[1] + (target[1] - s.color[1]) * g,
                            s.color[2] + (target[2] - s.color[2]) * g,
                            s.color[3],
                        ]
                    } else {
                        s.color
                    };
                    // Rough stone material: very low specular strength
                    // and a low specular power so any highlight that
                    // does catch is wide and soft (like weathered
                    // rock, not polished marble). Identical params for
                    // every shrine so cleared/future ones can't catch
                    // sharp glints from ambient fills.
                    let material = MaterialParams {
                        kind: MaterialKind::Plain,
                        base_color,
                        specular_strength: 0.06,
                        specular_power: 8.0,
                    };
                    self.shrine_instances[slot_i].write_uniform(
                        &self.queue,
                        view_proj_arr,
                        model,
                        material,
                    );
                }
            }
        }

        // Auxiliary dishes migrated to Object3dKind::Dish.

        // ── Ribbon batches (shop scene) ────────────────────────────────
        // Each textured ribbon uses up to 3 draw slots (top cap, tileable
        // middle, bottom cap) so its length is independent of texture aspect.
        // Untextured (plain) ribbons still use a single slot.
        self.last_ribbon_models.clear();
        self.last_ribbon_batch_slot_counts.clear();
        // Zodiac ribbons migrated to Object3dKind::ZodiacRibbon.

        // ── Talisman batches (shop scene) ──────────────────────────────
        self.last_talisman_models.clear();
        // Talismans migrated to Object3dKind::Talisman.

        // Coins migrated to Object3dKind::Coin.

        // ── Reset per-frame singletons owned by Object3d handlers ──────
        self.last_sell_tray_model = None;
        self.last_sell_card_model = None;

        // ── Skeuomorphic gameplay HUD uniform writes (phase 1) ─────────
        //
        // The new HUD meshes (plaque, ofuda, tablets, bowl, peg block, wall
        // stack) all share the lit-mesh pipeline. Each gets its
        // own slot pool above; per-frame we walk the cmds, write the
        // per-instance uniform, and (where the scene needs it for hit
        // testing in later phases) project the AABB to a screen-space rect.
        self.proj.yaku_tablet_rects.clear();
        self.proj.wood_tablet_rects.clear();
        self.proj.plaque_rects.clear();
        self.proj.bowl_rect = None;
        self.proj.mirror_rect = None;
        self.proj.dora_plinth_rect = None;
        self.proj.peg_rects = [None, None];
        self.proj.aux_dish_rects.clear();
        self.last_aux_dish_aabbs.clear();
        self.last_yaku_tablet_models.clear();
        self.last_wood_tablet_models.clear();
        self.last_bowl_model = None;
        self.last_mirror_model = None;

        // Helper closure: project the unit-cube AABB transformed by `model`
        // into a screen-space rect. Used by tablets/bowl for hit testing.
        let project_unit_cube_rect = |model: Mat4| -> [f32; 4] {
            let corners = [
                glam::Vec3::new(-0.5, -0.5, -0.5),
                glam::Vec3::new(0.5, -0.5, -0.5),
                glam::Vec3::new(-0.5, 0.5, -0.5),
                glam::Vec3::new(0.5, 0.5, -0.5),
                glam::Vec3::new(-0.5, -0.5, 0.5),
                glam::Vec3::new(0.5, -0.5, 0.5),
                glam::Vec3::new(-0.5, 0.5, 0.5),
                glam::Vec3::new(0.5, 0.5, 0.5),
            ];
            let mut mn_x = f32::INFINITY;
            let mut mn_y = f32::INFINITY;
            let mut mx_x = f32::NEG_INFINITY;
            let mut mx_y = f32::NEG_INFINITY;
            for c in corners {
                let w = model.transform_point3(c);
                let (sx, sy) = project_to_screen(w);
                mn_x = mn_x.min(sx);
                mn_y = mn_y.min(sy);
                mx_x = mx_x.max(sx);
                mx_y = mx_y.max(sy);
            }
            [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]
        };

        // Like `project_unit_cube_rect` but projects the actual mesh AABB
        // (given by half-extents and a Y-axis center offset) instead of
        // the full `[-0.5, 0.5]³` unit cube. This produces a much tighter
        // screen rect for flat objects like the river trough and mirror.
        let project_aabb_rect = |model: Mat4, half: [f32; 3], center_y: f32| -> [f32; 4] {
            let corners = [
                glam::Vec3::new(-half[0], center_y - half[1], -half[2]),
                glam::Vec3::new(half[0], center_y - half[1], -half[2]),
                glam::Vec3::new(-half[0], center_y + half[1], -half[2]),
                glam::Vec3::new(half[0], center_y + half[1], -half[2]),
                glam::Vec3::new(-half[0], center_y - half[1], half[2]),
                glam::Vec3::new(half[0], center_y - half[1], half[2]),
                glam::Vec3::new(-half[0], center_y + half[1], half[2]),
                glam::Vec3::new(half[0], center_y + half[1], half[2]),
            ];
            let mut mn_x = f32::INFINITY;
            let mut mn_y = f32::INFINITY;
            let mut mx_x = f32::NEG_INFINITY;
            let mut mx_y = f32::NEG_INFINITY;
            for c in corners {
                let w = model.transform_point3(c);
                let (sx, sy) = project_to_screen(w);
                mn_x = mn_x.min(sx);
                mn_y = mn_y.min(sy);
                mx_x = mx_x.max(sx);
                mx_y = mx_y.max(sy);
            }
            [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]
        };

        // Plaques (single instance per cmd).
        // Yaku tablet batches.
        let mut yaku_tablet_slot_cursor: usize = 0;
        for batch in &yaku_tablet_batches {
            for t in batch.iter() {
                if yaku_tablet_slot_cursor >= MAX_YAKU_TABLET_SLOTS {
                    break;
                }
                let slot_i = yaku_tablet_slot_cursor;
                yaku_tablet_slot_cursor += 1;
                // Hover lift along world Y so the tablet rises off the tray.
                let lift = t.hover.clamp(0.0, 1.0) * t.extents[1] * 0.4;
                let center = pixel_to_world(
                    w,
                    h,
                    t.world_pos[0],
                    t.world_pos[1],
                    t.world_pos[2] + t.extents[1] * 0.5 + lift,
                );
                // Tilt top face toward the camera (same Rx sign as wood tablets).
                let tilt_deg = -25.0_f32;
                let model = translate_rot_scale(
                    center,
                    rot_rz_rx_deg(tilt_deg, t.rotation_z_deg),
                    glam::Vec3::new(t.extents[0], t.extents[1], t.extents[2]),
                );
                let model = self.apply_arrange_override("gameplay.hand.yaku_tablet", model);
                // Active tablets warm up to a champagne tint; dim ones stay
                // bone. The decal pass (phase 2) will paint the engraved name
                // on top via a per-instance albedo texture.
                // Porcelain: bright cool-white when idle; a warmer cream
                // cast when this yaku is the selected target so it still
                // reads as "active" against the row.
                let base = if t.active {
                    [1.00, 0.97, 0.90, 1.0]
                } else {
                    [0.97, 0.96, 0.94, 1.0]
                };
                let material = MaterialParams {
                    kind: MaterialKind::Porcelain,
                    base_color: base,
                    // Shiny-chiclet glaze: strong pinpoint specular plus
                    // the shader's wet-glaze and rim terms. Hovering
                    // bumps the pinpoint so the lifted tablet looks
                    // freshly polished rather than just brighter.
                    specular_strength: 0.95 + 0.25 * t.hover.clamp(0.0, 1.0),
                    specular_power: 72.0,
                };
                // Engraved-name decal: rasterise on label change, then bind
                // it as the per-instance albedo overlay. Cached by label hash
                // so the steady-state cost is one compare per slot per frame.
                let label_hash = tablet_label_hash(&t.name, 256, 96);
                let inst = &mut self.yaku_tablet_instances[slot_i];
                if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                    let rgba = crate::render::decal::rasterize_yaku_tablet_decal(
                        &t.name,
                        self.ui_font.as_ref(),
                        self.emoji_font.as_ref(),
                    );
                    inst.set_decal(
                        crate::render::lit_mesh::DecalUploadCtx {
                            device: &self.device,
                            queue: &self.queue,
                            layout: &self.lit_mesh_material_layout,
                            sampler: &self.tile_sampler,
                            relief_view: &self.lit_mesh_relief_default_view,
                        },
                        &rgba,
                        256,
                        96,
                    );
                    inst.decal_label_hash = label_hash;
                }
                inst.write_uniform_with_decal(&self.queue, view_proj_arr, model, material, true);
                self.proj
                    .yaku_tablet_rects
                    .push(project_unit_cube_rect(model));
                self.last_yaku_tablet_models.push(model);
                self.last_debug_pickables.push((
                    "gameplay.hand.yaku_tablet".to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
            }
        }

        // Wood tablets migrated to Object3dKind::WoodTablet.

        // ── Object3d general-purpose placement pre-pass ──────────────────
        // Walk all Object3d batches and write uniforms into the appropriate
        // per-kind instance pools. Each kind uses its own slot cursor so
        // Object3d instances don't collide with legacy placement instances.
        // Also fills in the start/end range in the corresponding RenderOp.
        {
            let _camera_pitch_deg = {
                let look = look_target - cam_pos;
                look.z.atan2(look.y.abs()).to_degrees() + 180.0
            };

            let mut obj3d_primitive_slot: HashMap<crate::render::primitive::MeshId, usize> =
                HashMap::new();
            let mut obj3d_yaku_slot: usize = 0;
            let mut obj3d_wood_slot: usize = 0;
            let mut obj3d_relic_slot: usize = 0;
            let mut obj3d_pack_slot: usize = 0;
            let mut obj3d_talisman_slot: usize = 0;
            let mut obj3d_ribbon_slot: usize = 0;
            let mut obj3d_shrine_slot: usize = 0;
            let mut obj3d_dora_plinth_slot: usize = 0;
            let mut obj3d_orb_slot: usize = 0;
            let mut obj3d_bowl_slot: usize = 0;
            let mut obj3d_mirror_slot: usize = 0;
            let mut obj3d_tally_fan_idx: usize = 0;
            let mut obj3d_tally_stick_cursor: usize = 0;
            let mut obj3d_candle_slot: usize = 0;
            let mut obj3d_cascade_token_slot: usize = 0;
            let mut obj3d_glyph_slot: usize = 0;

            // Find the RenderOp::Object3dBatch ops to patch their start/end.
            let mut op_batch_idx: usize = 0;
            let mut obj3d_cmd_idx: usize = 0;

            for batch in &object3d_cmds {
                let batch_start = object3d_draw_list.len();

                for obj in batch.iter() {
                    use crate::render::draw_cmd::Object3dKind;
                    let center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                    let model = translate_rot_scale(
                        center,
                        obj.rotation, // Mat4 set directly by the scene
                        glam::Vec3::from(obj.extents),
                    );

                    match &obj.kind {
                        Object3dKind::Primitive {
                            shape,
                            material,
                            pick_id,
                            shadow_caster: _,
                            silhouette,
                        } => {
                            use crate::render::primitive::{
                                MeshId, resolve_material, shape_orientation,
                            };
                            // Slot bookkeeping is per-shape so two
                            // primitives of different shapes don't
                            // fight for the same pool index.
                            let cursor = obj3d_primitive_slot.entry(*shape).or_insert(0);
                            let slot_i = *cursor;
                            *cursor += 1;
                            // Lazily grow the per-shape instance pool.
                            // When a per-shape texture override is
                            // registered, bind it to the instance's
                            // albedo + relief slots so material
                            // branches that sample heightmaps (e.g.
                            // Metal coin) work.
                            let (albedo_v, relief_v) = match self.primitive_textures.get(shape) {
                                Some((a, r)) => (a, r),
                                None => (
                                    &self.lit_mesh_white_view,
                                    &self.lit_mesh_relief_default_view,
                                ),
                            };
                            let pool = self.primitive_instances.entry(*shape).or_default();
                            while pool.len() < slot_i + 1 {
                                pool.push(LitMeshInstance::new(
                                    &self.device,
                                    &self.lit_mesh_material_layout,
                                    &self.shadow_caster_layout,
                                    albedo_v,
                                    relief_v,
                                    &self.tile_sampler,
                                ));
                            }
                            // Decal rasterization + cache, unified for
                            // every shape via `rasterize_decal`.
                            let has_decal = if *silhouette {
                                false
                            } else if let Some(decal_spec) = &material.decal {
                                let (dw, dh) = crate::render::decal::decal_dimensions(
                                    &decal_spec.layout,
                                    obj.extents,
                                );
                                let label_hash = tablet_label_hash(&decal_spec.text, dw, dh);
                                let inst =
                                    &mut self.primitive_instances.get_mut(shape).unwrap()[slot_i];
                                if inst.decal_texture.is_none()
                                    || inst.decal_label_hash != label_hash
                                    || inst.decal_size != (dw, dh)
                                {
                                    let rgba = crate::render::decal::rasterize_decal(
                                        decal_spec,
                                        dw,
                                        dh,
                                        self.ui_font.as_ref(),
                                        self.emoji_font.as_ref(),
                                    );
                                    inst.set_decal(
                                        crate::render::lit_mesh::DecalUploadCtx {
                                            device: &self.device,
                                            queue: &self.queue,
                                            layout: &self.lit_mesh_material_layout,
                                            sampler: &self.tile_sampler,
                                            relief_view: &self.lit_mesh_relief_default_view,
                                        },
                                        &rgba,
                                        dw,
                                        dh,
                                    );
                                    inst.decal_label_hash = label_hash;
                                }
                                true
                            } else {
                                false
                            };
                            // Compose the per-shape mesh orientation
                            // (identity for most; Y-up-to-Z-up for
                            // Cylinder / DiscRound). Applied BEFORE
                            // extents scaling — i.e. rotate the local
                            // unit mesh into its canonical frame, then
                            // scale, then translate+rotate into world.
                            // Rebuild the model matrix here to preserve
                            // legacy ordering `T * R * O * S`.
                            let orient = shape_orientation(*shape);
                            let model = translate_rot_scale(
                                pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]),
                                obj.rotation * orient,
                                glam::Vec3::from(obj.extents),
                            );
                            // Arrange-name compat shim: for BeveledSlab
                            // without an explicit arrange_name,
                            // synthesise the legacy plaque name so
                            // saved arrange_overrides.json still works.
                            let arrange_name: String = if let Some(name) = obj.arrange_name {
                                name.to_string()
                            } else if *shape == MeshId::BeveledSlab {
                                match (self.active_scene_key, slot_i) {
                                    (Some("gameplay"), 0) => {
                                        "gameplay.score_panel.plaque".to_string()
                                    }
                                    (Some("gameplay"), 1) => {
                                        "gameplay.score_panel.scoring_placard".to_string()
                                    }
                                    (Some("shop"), i) => format!("shop.plaque[{i}]"),
                                    (_, i) => format!("plaque[{i}]"),
                                }
                            } else {
                                format!("primitive.{:?}[{}]", shape, slot_i)
                            };
                            let model = self.apply_arrange_override(&arrange_name, model);
                            if let Some(pid) = pick_id {
                                self.last_primitive_pick_models.insert(*pid, model);
                            }
                            let params = resolve_material(material, obj.color, *silhouette);
                            let tint = if *silhouette {
                                [0.04, 0.04, 0.05, obj.color[3]]
                            } else {
                                obj.color
                            };
                            let inst =
                                &mut self.primitive_instances.get_mut(shape).unwrap()[slot_i];
                            if *silhouette {
                                inst.write_uniform_tinted(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    params,
                                    tint,
                                );
                            } else {
                                inst.write_uniform_with_decal(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    params,
                                    has_decal,
                                );
                            }
                            self.last_debug_pickables.push((
                                arrange_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            // Screen-space rect for focus/hover hit
                            // testing. BeveledSlab projects only the
                            // +Z face (back is never seen); other
                            // shapes use the full AABB.
                            let corners: &[glam::Vec3] = if *shape == MeshId::BeveledSlab {
                                &[
                                    glam::Vec3::new(-0.5, -0.5, 0.5),
                                    glam::Vec3::new(0.5, -0.5, 0.5),
                                    glam::Vec3::new(-0.5, 0.5, 0.5),
                                    glam::Vec3::new(0.5, 0.5, 0.5),
                                ]
                            } else {
                                &[
                                    glam::Vec3::new(-0.5, -0.5, -0.5),
                                    glam::Vec3::new(0.5, -0.5, -0.5),
                                    glam::Vec3::new(-0.5, 0.5, -0.5),
                                    glam::Vec3::new(0.5, 0.5, -0.5),
                                    glam::Vec3::new(-0.5, -0.5, 0.5),
                                    glam::Vec3::new(0.5, -0.5, 0.5),
                                    glam::Vec3::new(-0.5, 0.5, 0.5),
                                    glam::Vec3::new(0.5, 0.5, 0.5),
                                ]
                            };
                            let mut mn_x = f32::INFINITY;
                            let mut mn_y = f32::INFINITY;
                            let mut mx_x = f32::NEG_INFINITY;
                            let mut mx_y = f32::NEG_INFINITY;
                            for c in corners {
                                let w_pt = model.transform_point3(*c);
                                let (sx, sy) = project_to_screen(w_pt);
                                mn_x = mn_x.min(sx);
                                mn_y = mn_y.min(sy);
                                mx_x = mx_x.max(sx);
                                mx_y = mx_y.max(sy);
                            }
                            if *shape == MeshId::BeveledSlab {
                                self.proj
                                    .plaque_rects
                                    .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                            }
                            // Dish-shaped primitives feed the pick/focus
                            // `aux_dish_rects` pipeline (shop trays,
                            // pick-blind altars, gameplay talisman dish)
                            // and the raycast AABB used by mouse picking.
                            // ShopActionProp reuses `aux_dish_rects` as
                            // the shop's focus-nav/click channel too —
                            // its `ShopHit::Dish(pid)` mapping is
                            // historical from when the props piggy-backed
                            // on the dish rect list.
                            if matches!(
                                *shape,
                                MeshId::DiscSquare | MeshId::DiscRound | MeshId::ShopActionProp
                            ) {
                                self.proj
                                    .aux_dish_rects
                                    .push((*pick_id, [mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]));
                                let center =
                                    pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                                let half = glam::Vec3::new(
                                    obj.extents[0] * 0.5,
                                    obj.extents[1] * 0.5,
                                    obj.extents[2] * 0.5,
                                );
                                self.last_aux_dish_aabbs.push((center, half));
                            }
                            object3d_draw_list.push((DrawKind::Primitive(*shape), slot_i));
                            // CabinetColumn emits a linked CabinetRails
                            // instance sharing the same world-space
                            // model matrix (post arrange override).
                            if *shape == MeshId::CabinetColumn {
                                let rails_cursor = obj3d_primitive_slot
                                    .entry(MeshId::CabinetRails)
                                    .or_insert(0);
                                let rails_slot = *rails_cursor;
                                *rails_cursor += 1;
                                let rails_pool = self
                                    .primitive_instances
                                    .entry(MeshId::CabinetRails)
                                    .or_default();
                                while rails_pool.len() < rails_slot + 1 {
                                    rails_pool.push(LitMeshInstance::new(
                                        &self.device,
                                        &self.lit_mesh_material_layout,
                                        &self.shadow_caster_layout,
                                        &self.lit_mesh_white_view,
                                        &self.lit_mesh_relief_default_view,
                                        &self.tile_sampler,
                                    ));
                                }
                                let rails_mesh = self
                                    .primitive_meshes
                                    .get(&MeshId::CabinetRails)
                                    .expect("CabinetRails mesh missing from registry");
                                rails_pool[rails_slot].write_uniform_with_decal(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    rails_mesh.default_material,
                                    false,
                                );
                                object3d_draw_list
                                    .push((DrawKind::Primitive(MeshId::CabinetRails), rails_slot));
                            }
                        }
                        Object3dKind::YakuTablet {
                            label,
                            active,
                            hover,
                        } => {
                            let slot_i = obj3d_yaku_slot;
                            obj3d_yaku_slot += 1;
                            if slot_i >= MAX_YAKU_TABLET_SLOTS {
                                continue;
                            }
                            let base = if *active {
                                [1.00_f32, 0.92, 0.72, 1.0]
                            } else {
                                [0.93_f32, 0.89, 0.78, 1.0]
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: base,
                                specular_strength: 0.30 + 0.20 * hover.clamp(0.0, 1.0),
                                specular_power: 32.0,
                            };
                            // All slots share one placement (gameplay.hand.yaku_tablet).
                            let _ = slot_i;
                            let yaku_name = "gameplay.hand.yaku_tablet";
                            let model = self.apply_arrange_override(yaku_name, model);
                            let label_hash = tablet_label_hash(label, 256, 96);
                            let inst = &mut self.yaku_tablet_instances[slot_i];
                            if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                                let rgba = crate::render::decal::rasterize_yaku_tablet_decal(
                                    label,
                                    self.ui_font.as_ref(),
                                    self.emoji_font.as_ref(),
                                );
                                inst.set_decal(
                                    crate::render::lit_mesh::DecalUploadCtx {
                                        device: &self.device,
                                        queue: &self.queue,
                                        layout: &self.lit_mesh_material_layout,
                                        sampler: &self.tile_sampler,
                                        relief_view: &self.lit_mesh_relief_default_view,
                                    },
                                    &rgba,
                                    256,
                                    96,
                                );
                                inst.decal_label_hash = label_hash;
                            }
                            inst.write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                                true,
                            );
                            self.last_debug_pickables.push((
                                yaku_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::YakuTablet, slot_i));
                        }
                        Object3dKind::WoodTablet { label, pick_id } => {
                            let slot_i = obj3d_wood_slot;
                            obj3d_wood_slot += 1;
                            if slot_i >= MAX_WOOD_TABLET_SLOTS {
                                continue;
                            }
                            // Explicit `arrange_name` wins; otherwise
                            // fall back to the legacy gameplay-slot
                            // convention so saved arrange overrides for
                            // the action bar keep loading.
                            let wood_name = if let Some(name) = obj.arrange_name {
                                name.to_string()
                            } else {
                                match slot_i {
                                    0 => "gameplay.action_bar.tablet_sort_suit".to_string(),
                                    1 => "gameplay.action_bar.tablet_sort_rank".to_string(),
                                    2 => "gameplay.action_bar.tablet_cash_in".to_string(),
                                    3 => "gameplay.action_bar.tablet_journal".to_string(),
                                    _ => "gameplay.action_bar.tablet".to_string(),
                                }
                            };
                            let model = self.apply_arrange_override(&wood_name, model);
                            let label_hash = tablet_label_hash(label, 512, 192);
                            let inst = &mut self.wood_tablet_instances[slot_i];
                            if inst.decal_texture.is_none() || inst.decal_label_hash != label_hash {
                                let rgba = crate::render::decal::rasterize_wood_tablet_decal(
                                    label,
                                    self.ui_font.as_ref(),
                                );
                                inst.set_decal(
                                    crate::render::lit_mesh::DecalUploadCtx {
                                        device: &self.device,
                                        queue: &self.queue,
                                        layout: &self.lit_mesh_material_layout,
                                        sampler: &self.tile_sampler,
                                        relief_view: &self.lit_mesh_relief_default_view,
                                    },
                                    &rgba,
                                    512,
                                    192,
                                );
                                inst.decal_label_hash = label_hash;
                            }
                            inst.write_uniform_with_decal(
                                &self.queue,
                                view_proj_arr,
                                model,
                                self.wood_tablet_mesh.default_material,
                                true,
                            );
                            self.proj
                                .wood_tablet_rects
                                .push(project_unit_cube_rect(model));
                            self.last_wood_tablet_models.push(model);
                            self.last_debug_pickables.push((
                                wood_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            // When a scene routes this tablet's click
                            // via `ShopHit::Dish(pid)` (shop journal
                            // button), publish the rect + model into
                            // the primitive-pick channels.
                            if let Some(pid) = pick_id {
                                self.proj
                                    .aux_dish_rects
                                    .push((Some(*pid), project_unit_cube_rect(model)));
                                self.last_primitive_pick_models.insert(*pid, model);
                            }
                            object3d_draw_list.push((DrawKind::WoodTablet, slot_i));
                        }
                        Object3dKind::Relic {
                            relic_id,
                            glow,
                            silhouette,
                            pick_id,
                        } => {
                            if obj3d_relic_slot >= MAX_RELIC_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_relic_slot;
                            obj3d_relic_slot += 1;
                            // Object3dKind::Relic fires for shop for-sale relics
                            // (single column Placement) and gameplay relics
                            // (single sidebar Placement).
                            let relic_arr_name = match self.active_scene_key {
                                Some("shop") => "shop.for_sale.relics".to_string(),
                                Some("gameplay") => "gameplay.relic_col".to_string(),
                                _ => format!("relic[{slot_i}]"),
                            };
                            let model = self.apply_arrange_override(&relic_arr_name, model);
                            // obj.rotation already encodes pitch/roll; extents are full.
                            let g = if *silhouette {
                                0.0
                            } else {
                                glow.clamp(0.0, 1.0)
                            };
                            let base_color = if *silhouette {
                                // Silhouette tint: the scene controls
                                // this via `obj.color`. Collection scene
                                // passes a muted rarity accent so locked
                                // relics still read as "earned-worth-
                                // chasing" rather than pure-black dots.
                                // Any caller that wants the old solid
                                // matte can pass `[0.04, 0.04, 0.05, 1]`
                                // explicitly — which is now the accent
                                // math's identity for near-zero tint.
                                obj.color
                            } else if g > 0.0 {
                                let target = [1.55, 1.32, 0.78, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = if *silhouette {
                                crate::render::lit_mesh::MaterialParams {
                                    kind: crate::render::lit_mesh::MaterialKind::Plain,
                                    base_color,
                                    specular_strength: 0.0,
                                    specular_power: 1.0,
                                }
                            } else {
                                relic_material_params(*relic_id, base_color, g)
                            };
                            if *silhouette {
                                self.relic_instances[slot_i].write_uniform_tinted(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    material,
                                    base_color,
                                );
                            } else {
                                self.relic_instances[slot_i].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    material,
                                );
                            }
                            // Silhouette pass skips the relic albedo/relief
                            // texture — we want the shape only, not the
                            // engraved artwork.
                            let want_tex = if *silhouette {
                                None
                            } else if self.relic_textures.contains_key(relic_id) {
                                Some(*relic_id)
                            } else {
                                None
                            };
                            if self.relic_slot_texture[slot_i] != want_tex {
                                let view = match want_tex {
                                    Some(rid) => &self.relic_textures[&rid].view,
                                    None => &self.lit_mesh_white_view,
                                };
                                let relief_view = match want_tex {
                                    Some(rid) => &self.relic_textures[&rid].relief_view,
                                    None => &self.lit_mesh_relief_default_view,
                                };
                                let inst = &mut self.relic_instances[slot_i];
                                inst.bind_group =
                                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("relic-bg-obj3d"),
                                        layout: &self.lit_mesh_material_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: inst.uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::TextureView(view),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: wgpu::BindingResource::Sampler(
                                                    &self.tile_sampler,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: wgpu::BindingResource::TextureView(
                                                    relief_view,
                                                ),
                                            },
                                        ],
                                    });
                                self.relic_slot_texture[slot_i] = want_tex;
                            }
                            self.last_relic_models.push((model, *relic_id));
                            if let Some(pid) = pick_id {
                                self.last_pickable_relic_models
                                    .push((*pid, model, *relic_id));
                            }
                            let projected_rect = project_unit_cube_rect(model);
                            self.proj.relic_rects.push(projected_rect);
                            self.last_debug_pickables.push((
                                relic_arr_name,
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            if g > 0.0 {
                                // Activation halo: champagne bloom inflated past
                                // the projected rect so the additive falloff
                                // spills out past the relic silhouette.
                                let [rx, ry, rw, rh] = projected_rect;
                                let pad_x = rw * 0.85;
                                let pad_y = rh * 0.95;
                                relic_glows.push(GpuInstance {
                                    rect: [
                                        rx - pad_x,
                                        ry - pad_y,
                                        rw + pad_x * 2.0,
                                        rh + pad_y * 2.0,
                                    ],
                                    color: [1.00, 0.82, 0.36, 1.20 * g],
                                });
                            }
                            object3d_draw_list.push((DrawKind::Relic, slot_i));
                        }
                        Object3dKind::Pack { kind, pick_id } => {
                            if obj3d_pack_slot >= self.pack_instances.len() {
                                continue;
                            }
                            let slot_i = obj3d_pack_slot;
                            obj3d_pack_slot += 1;
                            let _ = slot_i;
                            let pack_arr_name = obj.arrange_name.unwrap_or("shop.for_sale.packs");
                            let model = self.apply_arrange_override(pack_arr_name, model);
                            let material = MaterialParams {
                                kind: MaterialKind::Foil,
                                base_color: obj.color,
                                specular_strength: 0.70,
                                specular_power: 48.0,
                            };
                            self.pack_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            let want_tex = if self.pack_textures.contains_key(kind) {
                                Some(*kind)
                            } else {
                                None
                            };
                            if self.pack_slot_texture[slot_i] != want_tex {
                                let view = match want_tex {
                                    Some(k) => &self.pack_textures[&k].view,
                                    None => &self.lit_mesh_white_view,
                                };
                                let relief_view = match want_tex {
                                    Some(k) => &self.pack_textures[&k].relief_view,
                                    None => &self.lit_mesh_relief_default_view,
                                };
                                let inst = &mut self.pack_instances[slot_i];
                                inst.bind_group =
                                    self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                        label: Some("pack-bg-obj3d"),
                                        layout: &self.lit_mesh_material_layout,
                                        entries: &[
                                            wgpu::BindGroupEntry {
                                                binding: 0,
                                                resource: inst.uniform_buffer.as_entire_binding(),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 1,
                                                resource: wgpu::BindingResource::TextureView(view),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 2,
                                                resource: wgpu::BindingResource::Sampler(
                                                    &self.tile_sampler,
                                                ),
                                            },
                                            wgpu::BindGroupEntry {
                                                binding: 3,
                                                resource: wgpu::BindingResource::TextureView(
                                                    relief_view,
                                                ),
                                            },
                                        ],
                                    });
                                self.pack_slot_texture[slot_i] = want_tex;
                            }
                            // Project the 8 unit-cube corners via the model matrix to get
                            // the screen-space bounding rect. This feeds focus-nav and
                            // controller selection via aux_dish_rects (appended below).
                            self.proj
                                .pack_rects
                                .push((project_unit_cube_rect(model), *pick_id));
                            self.last_debug_pickables.push((
                                pack_arr_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Pack, slot_i));
                        }
                        Object3dKind::Talisman { kind } => {
                            if obj3d_talisman_slot >= MAX_TALISMAN_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_talisman_slot;
                            obj3d_talisman_slot += 1;
                            // extents already encode full size; scale to mesh local half-extents.
                            let sx = obj.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                            let sy = obj.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                            let sz = obj.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                            let _ = slot_i;
                            // Default to the for-sale stall arrange group, but
                            // let the caller opt into a different group (e.g.
                            // owned-inventory talismans, which shouldn't share
                            // the shop's Rx/Ry/Rz arrange rotation).
                            let talisman_name =
                                obj.arrange_name.unwrap_or("shop.for_sale.talismans");
                            let talisman_center_arr = self.apply_arrange_override(
                                talisman_name,
                                translate_rot_scale(
                                    center,
                                    obj.rotation,
                                    glam::Vec3::new(sx, sy, sz),
                                ),
                            );
                            // Re-decompose center after possible override; simpler: re-derive center from matrix.
                            let talisman_model = talisman_center_arr;
                            let material = talisman_material(*kind, obj.color);
                            let kind_idx = crate::core::talisman::TalismanKind::all()
                                .iter()
                                .position(|&k| k == *kind)
                                .unwrap_or(0) as u8;
                            if self.talisman_slot_kind[slot_i] != Some(kind_idx) {
                                self.talisman_instances[slot_i].rebind_texture(
                                    &self.device,
                                    &self.lit_mesh_material_layout,
                                    &self.talisman_height_views[kind_idx as usize],
                                    &self.lit_mesh_relief_default_view,
                                    &self.tile_sampler,
                                );
                                self.talisman_slot_kind[slot_i] = Some(kind_idx);
                            }
                            self.talisman_instances[slot_i].write_uniform_raw_w(
                                &self.queue,
                                view_proj_arr,
                                talisman_model,
                                material,
                                kind_idx as f32,
                            );
                            self.last_talisman_models.push(talisman_model);
                            self.proj
                                .talisman_rects
                                .push(project_unit_cube_rect(talisman_model));
                            self.last_debug_pickables.push((
                                talisman_name.to_string(),
                                talisman_model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Talisman, slot_i));
                        }
                        Object3dKind::ZodiacRibbon { kind } => {
                            // extents: [width, length, depth].
                            let eff_w = obj.extents[0];
                            let eff_l = obj.extents[1];
                            let depth = obj.extents[2];
                            // Push the overall ribbon AABB for arrange-mode picking.
                            // (Individual segments aren't separately selectable.)
                            let ribbon_arr_name =
                                obj.arrange_name.unwrap_or("shop.for_sale.ribbons");
                            let base_transform = self.apply_arrange_override(
                                ribbon_arr_name,
                                translate_rot_scale(center, obj.rotation, glam::Vec3::splat(1.0)),
                            );
                            let full_ribbon_model = ribbon_submesh(
                                base_transform,
                                0.0,
                                glam::Vec3::new(eff_w, eff_l, depth),
                            );
                            self.last_ribbon_models.push(full_ribbon_model);
                            self.proj
                                .ribbon_rects
                                .push(project_unit_cube_rect(full_ribbon_model));
                            self.last_debug_pickables.push((
                                ribbon_arr_name.to_string(),
                                full_ribbon_model,
                                glam::Vec3::new(0.5, 0.5, 0.5),
                                0.0,
                            ));
                            let cap_h = eff_w * 0.6;
                            let mid_h = (eff_l - cap_h * 2.0).max(0.0);
                            let silk_mat = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: obj.color,
                                specular_strength: 0.25,
                                specular_power: 16.0,
                            };
                            let zodiac_id: Option<u8> = kind.as_ref().and_then(|z| {
                                let tex_idx = crate::core::zodiac::ZodiacKind::all()
                                    .iter()
                                    .position(|&k| k == *z)?
                                    as u8;
                                Some(tex_idx)
                            });
                            // Emit segments: top cap (seg 0), optional mid (seg 1), bottom cap (seg 2).
                            let segments: &[(f32, f32, u8)] = if mid_h > 0.0 {
                                &[
                                    (0.0, cap_h, 0),
                                    (-cap_h, mid_h, 1),
                                    (-(cap_h + mid_h), cap_h, 2),
                                ]
                            } else {
                                &[(0.0, cap_h, 0), (-(cap_h), cap_h, 2)]
                            };
                            for &(offset, seg_h, seg_idx) in segments {
                                if obj3d_ribbon_slot >= MAX_RIBBON_SLOTS {
                                    break;
                                }
                                let slot_i = obj3d_ribbon_slot;
                                obj3d_ribbon_slot += 1;
                                let seg_model = ribbon_submesh(
                                    base_transform,
                                    offset,
                                    glam::Vec3::new(eff_w, seg_h, depth),
                                );
                                let rzod = zodiac_id.map(|ti| (ti, seg_idx));
                                if self.ribbon_slot_zodiac[slot_i] != rzod {
                                    let view: &wgpu::TextureView = match rzod {
                                        Some((idx, 0)) => {
                                            &self.ribbon_zodiac_tex.top_views[idx as usize]
                                        }
                                        Some((idx, 1)) => {
                                            &self.ribbon_zodiac_tex.mid_views[idx as usize]
                                        }
                                        Some((idx, _)) => {
                                            &self.ribbon_zodiac_tex.bot_views[idx as usize]
                                        }
                                        None => &self.lit_mesh_white_view,
                                    };
                                    let inst = &mut self.ribbon_instances[slot_i];
                                    inst.bind_group =
                                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                                            label: Some("ribbon-bg-obj3d"),
                                            layout: &self.lit_mesh_material_layout,
                                            entries: &[
                                                wgpu::BindGroupEntry {
                                                    binding: 0,
                                                    resource: inst
                                                        .uniform_buffer
                                                        .as_entire_binding(),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 1,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        view,
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 2,
                                                    resource: wgpu::BindingResource::Sampler(
                                                        &self.tile_sampler,
                                                    ),
                                                },
                                                wgpu::BindGroupEntry {
                                                    binding: 3,
                                                    resource: wgpu::BindingResource::TextureView(
                                                        &self.lit_mesh_relief_default_view,
                                                    ),
                                                },
                                            ],
                                        });
                                    self.ribbon_slot_zodiac[slot_i] = rzod;
                                }
                                self.ribbon_instances[slot_i].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    seg_model,
                                    silk_mat,
                                );
                                object3d_draw_list.push((DrawKind::Ribbon, slot_i));
                            }
                        }
                        Object3dKind::Shrine { glow } => {
                            if obj3d_shrine_slot >= MAX_SHRINE_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_shrine_slot;
                            obj3d_shrine_slot += 1;
                            // Shrines are pick-blind only; one placement per slot.
                            let shrine_name = match slot_i {
                                0 => "pick_blind.shrine[0]",
                                1 => "pick_blind.shrine[1]",
                                2 => "pick_blind.shrine[2]",
                                _ => "pick_blind.shrine",
                            };
                            // Shrine center is lifted by half-height; scene passes base pos.
                            let shrine_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5,
                            );
                            // The shrine mesh is built Y-up; rotate into Z-up world so it
                            // stands upright rather than lying flat. Compose with any
                            // scene-level obj.rotation (e.g. arrange-mode overrides).
                            let shrine_rot =
                                mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let shrine_model = self.apply_arrange_override(
                                shrine_name,
                                translate_rot_scale(
                                    shrine_center,
                                    shrine_rot,
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            let g = glow.clamp(0.0, 1.0);
                            let base_color = if g > 0.0 {
                                let target = [1.10, 1.05, 0.95, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color,
                                specular_strength: 0.06,
                                specular_power: 8.0,
                            };
                            self.shrine_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                shrine_model,
                                material,
                            );
                            // Project AABB for shrine_rects (label anchoring).
                            let shrine_world_center = shrine_model.w_axis.truncate();
                            let [hx, hy, hz] = [
                                obj.extents[0] * 0.5,
                                obj.extents[1] * 0.5,
                                obj.extents[2] * 0.5,
                            ];
                            let (mut mn_x, mut mn_y, mut mx_x, mut mx_y) = (
                                f32::INFINITY,
                                f32::INFINITY,
                                f32::NEG_INFINITY,
                                f32::NEG_INFINITY,
                            );
                            for cx in [-hx, hx] {
                                for cy in [-hy, hy] {
                                    for cz in [-hz, hz] {
                                        let world =
                                            shrine_world_center + glam::Vec3::new(cx, cy, cz);
                                        let (px, py) = project_to_screen(world);
                                        mn_x = mn_x.min(px);
                                        mn_y = mn_y.min(py);
                                        mx_x = mx_x.max(px);
                                        mx_y = mx_y.max(py);
                                    }
                                }
                            }
                            self.proj
                                .shrine_rects
                                .push([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                            self.last_debug_pickables.push((
                                shrine_name.to_string(),
                                shrine_model,
                                glam::Vec3::new(hx, hy, hz),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::Shrine, slot_i));
                        }
                        Object3dKind::DoraPlinth { glow } => {
                            if obj3d_dora_plinth_slot >= MAX_DORA_PLINTH_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_dora_plinth_slot;
                            obj3d_dora_plinth_slot += 1;
                            let plinth_name = "gameplay.dora_plinth";
                            // Mesh is built Y-up centered; lift the world position
                            // by half-height so `obj.pos` describes the plinth's
                            // base sitting on the table felt.
                            let plinth_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5,
                            );
                            let plinth_rot =
                                mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let plinth_model = self.apply_arrange_override(
                                plinth_name,
                                translate_rot_scale(
                                    plinth_center,
                                    plinth_rot,
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            let g = glow.clamp(0.0, 1.0);
                            let base_color = if g > 0.0 {
                                let target = [1.10, 0.95, 0.55, obj.color[3]];
                                [
                                    obj.color[0] + (target[0] - obj.color[0]) * g,
                                    obj.color[1] + (target[1] - obj.color[1]) * g,
                                    obj.color[2] + (target[2] - obj.color[2]) * g,
                                    obj.color[3],
                                ]
                            } else {
                                obj.color
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Metal,
                                base_color,
                                specular_strength: 0.85,
                                specular_power: 64.0,
                            };
                            self.dora_plinth_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                plinth_model,
                                material,
                            );
                            // Project AABB → screen rect for hover/focus.
                            let plinth_world_center = plinth_model.w_axis.truncate();
                            let [hx, hy, hz] = [
                                obj.extents[0] * 0.5,
                                obj.extents[1] * 0.5,
                                obj.extents[2] * 0.5,
                            ];
                            let (mut mn_x, mut mn_y, mut mx_x, mut mx_y) = (
                                f32::INFINITY,
                                f32::INFINITY,
                                f32::NEG_INFINITY,
                                f32::NEG_INFINITY,
                            );
                            for cx in [-hx, hx] {
                                for cy in [-hy, hy] {
                                    for cz in [-hz, hz] {
                                        let world =
                                            plinth_world_center + glam::Vec3::new(cx, cy, cz);
                                        let (px, py) = project_to_screen(world);
                                        mn_x = mn_x.min(px);
                                        mn_y = mn_y.min(py);
                                        mx_x = mx_x.max(px);
                                        mx_y = mx_y.max(py);
                                    }
                                }
                            }
                            self.proj.dora_plinth_rect =
                                Some([mn_x, mn_y, mx_x - mn_x, mx_y - mn_y]);
                            self.last_debug_pickables.push((
                                plinth_name.to_string(),
                                plinth_model,
                                glam::Vec3::new(hx, hy, hz),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::DoraPlinth, slot_i));
                        }
                        Object3dKind::SellTray { pick_id } => {
                            // Round dish mesh is built Y-up; rotate local Y
                            // into world Z so the rim sits flat on the table
                            // and `extents[1]` (rim) becomes vertical
                            // thickness. Compose with any scene rotation.
                            let oriented = mesh_y_thickness_along_local_y_to_z_up() * obj.rotation;
                            let model = translate_rot_scale(
                                center,
                                oriented,
                                glam::Vec3::from(obj.extents),
                            );
                            let model = self.apply_arrange_override("shop.shelf.sell_tray", model);
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: obj.color,
                                specular_strength: 0.3,
                                specular_power: 16.0,
                            };
                            self.sell_tray_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            if let Some(pid) = pick_id {
                                self.last_sell_tray_model = Some((model, *pid));
                            }
                            // Folded "SELL" tent card sits in the recess when
                            // the tray is focused (any control method). The
                            // shop scene encodes focus state via hover_target
                            // (≥0.5 = focused/hovered).
                            if obj.hover_target >= 0.5 {
                                if !self.sell_card_decal_ready {
                                    let rgba = crate::render::decal::rasterize_tablet_label_decal(
                                        "SELL",
                                        self.ui_font.as_ref(),
                                        self.emoji_font.as_ref(),
                                        256,
                                        128,
                                        [0.62, 0.18, 0.14, 1.0],
                                    );
                                    self.sell_card_instance.set_decal(
                                        crate::render::lit_mesh::DecalUploadCtx {
                                            device: &self.device,
                                            queue: &self.queue,
                                            layout: &self.lit_mesh_material_layout,
                                            sampler: &self.tile_sampler,
                                            relief_view: &self.lit_mesh_relief_default_view,
                                        },
                                        &rgba,
                                        256,
                                        128,
                                    );
                                    self.sell_card_decal_ready = true;
                                }
                                // Build the card model matrix anchored to the
                                // tray. Local card extents: x=-0.5..0.5,
                                // y=0..0.5, z=-0.5..0.5. The tray is a unit
                                // box with rim top at +0.5 and recess at +0.2;
                                // we shrink the card to fit inside the rim and
                                // sit on the recessed floor.
                                let (scale, rot, trans) = model.to_scale_rotation_translation();
                                // Card footprint: 60% of rim diameter, height
                                // ~70% of rim depth.
                                // Card height is decoupled from the (very
                                // shallow) rim thickness so it stays readable
                                // on the flat plate; sized off the plate
                                // footprint instead.
                                let footprint = scale.x.min(scale.z);
                                let card_scale = glam::Vec3::new(
                                    scale.x * 0.55,
                                    footprint * 0.55,
                                    scale.z * 0.55,
                                );
                                // Sit the card just above the rim top
                                // (local y=+0.5) so it doesn't poke through
                                // the shallow plate. Nudged back along local
                                // -z (world +y, deeper into scene) so the
                                // card stands toward the rear of the dish
                                // instead of centered in the recess.
                                let local_floor = glam::Vec3::new(0.0, 0.55, -0.15);
                                let world_floor = trans + rot * (local_floor * scale);
                                // Yaw the card 100° around world +Z so the
                                // crease faces the camera at a slight angle.
                                let yaw = glam::Quat::from_rotation_z(100.0_f32.to_radians());
                                let card_rot = yaw * rot;
                                let card_model = Mat4::from_scale_rotation_translation(
                                    card_scale,
                                    card_rot,
                                    world_floor,
                                );
                                let card_material = MaterialParams {
                                    kind: MaterialKind::Plain,
                                    base_color: [0.96, 0.93, 0.84, 1.0],
                                    specular_strength: 0.10,
                                    specular_power: 8.0,
                                };
                                self.sell_card_instance.write_uniform_with_decal(
                                    &self.queue,
                                    view_proj_arr,
                                    card_model,
                                    card_material,
                                    true,
                                );
                                self.last_sell_card_model = Some(card_model);
                            }
                            self.last_debug_pickables.push((
                                "shop.shelf.sell_tray".to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::SellTray, 0));
                        }
                        Object3dKind::ShopLamp { glow } => {
                            // Lamp mesh is in world-space Z-up convention: no corrective
                            // rotation needed. pos is the apex/cord-attachment point (high Z).
                            // The shade rim (wide, open end) hangs below at lower Z. ✓
                            let lamp_center =
                                pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let lamp_model = self.apply_arrange_override(
                                "shop.props.lamp",
                                translate_rot_scale(
                                    lamp_center,
                                    obj.rotation,
                                    glam::Vec3::from(obj.extents),
                                ),
                            );
                            // Body — brass Metal material.
                            self.lamp_body_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                lamp_model,
                                self.lamp_body_mesh.default_material,
                            );
                            object3d_draw_list.push((DrawKind::LampBody, 0));
                            // Bulb — Glass material. Push brightness well above
                            // 1.0 when glow is active so the HDR bulb color
                            // crosses the bloom extract threshold and glares.
                            let g = glow.clamp(0.0, 1.0);
                            let dm = &self.lamp_bulb_mesh.default_material;
                            let bulb_mat = MaterialParams {
                                kind: crate::render::lit_mesh::MaterialKind::Glass,
                                base_color: [
                                    dm.base_color[0] * (1.0 + g * 1.4),
                                    dm.base_color[1] * (1.0 + g * 1.0),
                                    dm.base_color[2] * (1.0 + g * 0.5),
                                    1.0,
                                ],
                                specular_strength: dm.specular_strength,
                                specular_power: dm.specular_power,
                            };
                            self.lamp_bulb_instance.write_uniform(
                                &self.queue,
                                view_proj_arr,
                                lamp_model,
                                bulb_mat,
                            );
                            object3d_draw_list.push((DrawKind::LampBulb, 0));
                            // Trimesh pick: AABB of extents [w,h,w] is a bad
                            // silhouette for a lamp (thin cord on top of a wide
                            // shade) and invites accidental grabs on empty air
                            // above the shade. Ray-cast against the actual cord
                            // + cone triangles so the pick region matches what
                            // the player sees.
                            self.last_debug_trimesh_pickables.push((
                                "shop.props.lamp".to_string(),
                                lamp_model,
                                TrimeshRef::LampBody,
                            ));
                        }
                        Object3dKind::Bug {
                            slot,
                            flap_rad,
                            live_wing_alpha,
                            blur_alpha,
                        } => {
                            let slot = *slot;
                            if slot >= MAX_BUG_SLOTS {
                                continue;
                            }
                            let bug_model = translate_rot_scale(
                                center,
                                obj.rotation,
                                glam::Vec3::from(obj.extents),
                            );
                            self.bug_body_instances[slot].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                self.bug_body_mesh.default_material,
                            );
                            object3d_draw_list.push((DrawKind::BugBody, slot));
                            // Live wing model matrices: the mesh lives in +Y,
                            // so the left wing is the identity and the
                            // right wing flips Y (mirror across body).
                            // Flap rotates about mesh +X, which is the
                            // body axis — the right wing uses -flap so
                            // the two counter-sweep like a moth's.
                            let flap_l = glam::Mat4::from_rotation_x(*flap_rad);
                            let flap_r = glam::Mat4::from_rotation_x(-*flap_rad)
                                * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0));
                            let live_a = live_wing_alpha.clamp(0.0, 1.0);
                            let wing_mat = self.bug_wing_mesh.default_material;
                            let live_tint = [
                                wing_mat.base_color[0],
                                wing_mat.base_color[1],
                                wing_mat.base_color[2],
                                wing_mat.base_color[3] * live_a,
                            ];
                            self.bug_wing_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * flap_l,
                                wing_mat,
                                live_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingL, slot));
                            self.bug_wing_r_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * flap_r,
                                wing_mat,
                                live_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingR, slot));
                            // Blur fans — the swept-volume mesh is drawn once per
                            // side with no flap rotation (the mesh itself is the
                            // full sweep). The right side reuses the same mesh
                            // with a Y-mirror transform, matching how the live
                            // wing pair is built.
                            let blur_a = blur_alpha.clamp(0.0, 1.0);
                            let blur_mat = self.bug_wing_blur_mesh.default_material;
                            let blur_tint = [
                                blur_mat.base_color[0],
                                blur_mat.base_color[1],
                                blur_mat.base_color[2],
                                blur_mat.base_color[3] * blur_a,
                            ];
                            self.bug_wing_blur_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model,
                                blur_mat,
                                blur_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingBlurL, slot));
                            self.bug_wing_blur_r_instances[slot].write_uniform_tinted(
                                &self.queue,
                                view_proj_arr,
                                bug_model * glam::Mat4::from_scale(glam::Vec3::new(1.0, -1.0, 1.0)),
                                blur_mat,
                                blur_tint,
                            );
                            object3d_draw_list.push((DrawKind::BugWingBlurR, slot));
                        }
                        Object3dKind::MaterialOrb { material } => {
                            if obj3d_orb_slot >= MAX_ORB_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_orb_slot;
                            obj3d_orb_slot += 1;
                            self.orb_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                *material,
                            );
                            object3d_draw_list.push((DrawKind::Orb, slot_i));
                        }
                        Object3dKind::Mirror {
                            rotation_x_deg,
                            rotation_z_deg,
                        } => {
                            if obj3d_mirror_slot >= MAX_MIRROR_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_mirror_slot;
                            obj3d_mirror_slot += 1;
                            let target = obj.hover_target.clamp(0.0, 1.0);
                            let anim_id = if obj.anim_id != 0 { obj.anim_id } else { 2 };
                            let k = 1.0 - (-14.0 * self.frame_dt).exp();
                            let e = self.obj3d_hover_state.entry(anim_id).or_insert(0.0);
                            *e += (target - *e) * k;
                            let anim = *e;
                            let lift = anim * obj.extents[1] * 0.15;
                            let tilt_deg = *rotation_x_deg + anim * 22.0;
                            let center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5 + lift,
                            );
                            let hover_model = translate_rot_scale(
                                center,
                                rot_rz_rx_deg(tilt_deg, *rotation_z_deg),
                                glam::Vec3::from(obj.extents),
                            );
                            let hover_model = self
                                .apply_arrange_override("gameplay.action_bar.mirror", hover_model);
                            self.mirror_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                hover_model,
                                self.mirror_mesh.default_material,
                            );
                            if slot_i == 0 {
                                self.proj.mirror_rect = Some(project_aabb_rect(
                                    hover_model,
                                    MIRROR_LOCAL_HALF,
                                    MIRROR_LOCAL_CENTER_Y,
                                ));
                                self.last_mirror_model = Some(hover_model);
                            }
                            self.last_debug_pickables.push((
                                "gameplay.action_bar.mirror".to_string(),
                                hover_model,
                                glam::Vec3::new(
                                    MIRROR_LOCAL_HALF[0],
                                    MIRROR_LOCAL_HALF[1],
                                    MIRROR_LOCAL_HALF[2],
                                ),
                                MIRROR_LOCAL_CENTER_Y,
                            ));
                            object3d_draw_list.push((DrawKind::Mirror, slot_i));
                        }
                        Object3dKind::ExtrudedGlyph {
                            scale: g_scale,
                            rotation_x: g_rx,
                            rotation_y: g_ry,
                            label,
                            emissive,
                            material: g_mat,
                        } => {
                            if obj3d_glyph_slot >= MAX_EXTRUDED_GLYPH_SLOTS {
                                continue;
                            }
                            if !self.extruded_glyph_meshes.contains_key(label) {
                                if let Some(cpu) = self.glyph_cpu_cache.mesh_for(label) {
                                    let gpu = LitMeshGpu::new(
                                        &self.device,
                                        cpu,
                                        &format!("glyph-{}", label),
                                    );
                                    self.extruded_glyph_meshes.insert(label.clone(), gpu);
                                } else {
                                    continue;
                                }
                            }
                            let slot_i = obj3d_glyph_slot;
                            obj3d_glyph_slot += 1;
                            let g_center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let glyph_model = translate_rot_scale(
                                g_center,
                                score_popup_glyph_rot_rad(
                                    *g_ry,
                                    -std::f32::consts::FRAC_PI_2 + *g_rx,
                                ),
                                glam::Vec3::splat(*g_scale),
                            );
                            let glyph_model =
                                self.apply_arrange_override("gameplay.score_popup", glyph_model);
                            let material = match g_mat {
                                crate::render::draw_cmd::GlyphMaterial::Metal => MaterialParams {
                                    kind: MaterialKind::Metal,
                                    base_color: obj.color,
                                    specular_strength: 1.0,
                                    specular_power: 128.0,
                                },
                                crate::render::draw_cmd::GlyphMaterial::Polychrome => {
                                    MaterialParams {
                                        kind: MaterialKind::Polychrome,
                                        base_color: obj.color,
                                        specular_strength: 0.85,
                                        specular_power: 48.0,
                                    }
                                }
                                crate::render::draw_cmd::GlyphMaterial::Plain => MaterialParams {
                                    kind: MaterialKind::Plain,
                                    base_color: obj.color,
                                    specular_strength: 0.35 + 0.20 * emissive.clamp(0.0, 1.0),
                                    specular_power: 96.0,
                                },
                            };
                            self.extruded_glyph_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                glyph_model,
                                material,
                            );
                            self.last_debug_pickables.push((
                                "gameplay.score_popup".to_string(),
                                glyph_model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::ExtrudedGlyph, slot_i));
                        }
                        Object3dKind::CascadeToken { kind: ck, pulse } => {
                            if obj3d_cascade_token_slot >= MAX_CASCADE_TOKEN_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_cascade_token_slot;
                            obj3d_cascade_token_slot += 1;
                            let pulse_f = pulse.clamp(0.0, 1.0);
                            let pulse_scale = 1.0 + 0.18 * pulse_f;
                            let center = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let cascade_token_name = match ck {
                                CascadeTokenKind::Chips => "gameplay.cascade_token.chips",
                                CascadeTokenKind::Mult => "gameplay.cascade_token.mult",
                            };
                            let model = translate_rot_scale(
                                center,
                                Mat4::IDENTITY,
                                glam::Vec3::new(
                                    obj.extents[0] * pulse_scale,
                                    obj.extents[1] * pulse_scale,
                                    obj.extents[2] * pulse_scale,
                                ),
                            );
                            let model = self.apply_arrange_override(cascade_token_name, model);
                            let base = match ck {
                                CascadeTokenKind::Chips => [0.55, 0.78, 1.00, 1.0],
                                CascadeTokenKind::Mult => [0.85, 0.32, 0.42, 1.0],
                            };
                            let material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: base,
                                specular_strength: 0.40 + 0.30 * pulse_f,
                                specular_power: 48.0,
                            };
                            self.cascade_token_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                model,
                                material,
                            );
                            self.last_debug_pickables.push((
                                cascade_token_name.to_string(),
                                model,
                                glam::Vec3::splat(0.5),
                                0.0,
                            ));
                            object3d_draw_list.push((DrawKind::CascadeToken, slot_i));
                        }
                        Object3dKind::Candle {
                            scale,
                            height_scale,
                        } => {
                            let slot_i = obj3d_candle_slot;
                            obj3d_candle_slot += 1;
                            if self.candle_instances.get(slot_i).is_none() {
                                continue;
                            }
                            let base = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let s = *scale;
                            let candle_model = translate_rot_scale(
                                base,
                                mesh_y_thickness_along_local_y_to_z_up(),
                                glam::Vec3::new(s, s * *height_scale, s),
                            );
                            let candle_name = self.scene_path("candle");
                            let candle_model =
                                self.apply_arrange_override(&candle_name, candle_model);
                            self.candle_instances[slot_i][0].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                candle_model,
                                self.candle_wax_mesh.default_material,
                            );
                            self.candle_instances[slot_i][1].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                candle_model,
                                self.candle_wick_mesh.default_material,
                            );
                            self.last_debug_pickables.push((
                                candle_name,
                                candle_model,
                                glam::Vec3::new(0.36, 0.305, 0.36),
                                0.305,
                            ));
                            object3d_draw_list.push((DrawKind::CandleWax, slot_i));
                            object3d_draw_list.push((DrawKind::CandleWick, slot_i));
                        }
                        Object3dKind::TallyFan {
                            stick_len,
                            stick_wide,
                            stick_thickness,
                            count,
                            max_count,
                            spread_deg,
                            tip_color,
                            rotation_y_deg,
                            kind: fan_kind,
                        } => {
                            let fan_i = obj3d_tally_fan_idx;
                            obj3d_tally_fan_idx += 1;
                            if fan_i >= MAX_TALLY_FAN_SLOTS {
                                continue;
                            }
                            let max_c: u32 = (*max_count).max(1);
                            let count_usize = (*count).min(max_c) as usize;
                            let spread_rad = spread_deg.to_radians();
                            let slot_angle = |k: u32| -> f32 {
                                if max_c <= 1 {
                                    0.0
                                } else {
                                    -spread_rad * 0.5
                                        + (k as f32) * (spread_rad / (max_c as f32 - 1.0))
                                }
                            };
                            let pivot = pixel_to_world(w, h, obj.pos[0], obj.pos[1], obj.pos[2]);
                            let base_orient = mesh_y_thickness_along_local_y_to_z_up();
                            let fan_yaw = Mat4::from_rotation_z(rotation_y_deg.to_radians());
                            let base_scale =
                                glam::Vec3::new(*stick_wide, *stick_len, *stick_thickness);
                            let base_material = self.tally_stick_base_mesh.default_material;
                            let tip_material = MaterialParams {
                                kind: MaterialKind::Plain,
                                base_color: *tip_color,
                                specular_strength: 0.40,
                                specular_power: 42.0,
                            };
                            let arrange_name = match fan_kind {
                                TallyFanKind::Draws => "gameplay.counter.draws_fan",
                                TallyFanKind::Discards => "gameplay.counter.discards_fan",
                            };
                            let missing = (max_c as usize).saturating_sub(count_usize);
                            let mut visible_slots: Vec<u32> = (0..max_c).collect();
                            for trim in 0..missing {
                                if trim % 2 == 0 {
                                    visible_slots.pop();
                                } else {
                                    visible_slots.remove(0);
                                }
                            }
                            for (stick_i, &k) in visible_slots.iter().enumerate() {
                                if obj3d_tally_stick_cursor + 1 >= MAX_TALLY_STICK_SLOTS * 2 {
                                    break;
                                }
                                let angle = slot_angle(k);
                                let rot = fan_yaw * Mat4::from_rotation_y(angle) * base_orient;
                                let model = translate_rot_scale(pivot, rot, base_scale);
                                let model = self.apply_arrange_override(arrange_name, model);
                                if stick_i == 0 {
                                    self.last_debug_pickables.push((
                                        arrange_name.to_string(),
                                        model,
                                        glam::Vec3::new(0.5, 0.5, 0.5),
                                        0.0,
                                    ));
                                }
                                self.tally_stick_instances[obj3d_tally_stick_cursor].write_uniform(
                                    &self.queue,
                                    view_proj_arr,
                                    model,
                                    base_material,
                                );
                                self.tally_stick_instances[obj3d_tally_stick_cursor + 1]
                                    .write_uniform(&self.queue, view_proj_arr, model, tip_material);
                                object3d_draw_list
                                    .push((DrawKind::TallyStickBase, obj3d_tally_stick_cursor));
                                object3d_draw_list
                                    .push((DrawKind::TallyStickTip, obj3d_tally_stick_cursor + 1));
                                obj3d_tally_stick_cursor += 2;
                            }
                            let fan_width =
                                *stick_len * (spread_rad * 0.5).sin() * 2.0 + *stick_wide;
                            let fan_height = *stick_len + *stick_wide * 0.5;
                            let fan_center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + *stick_len * 0.5,
                            );
                            let fan_model = translate_rot_scale(
                                fan_center,
                                fan_yaw,
                                glam::Vec3::new(fan_width, *stick_thickness * 2.0, fan_height),
                            );
                            let slot_idx = match fan_kind {
                                TallyFanKind::Draws => 0,
                                TallyFanKind::Discards => 1,
                            };
                            self.proj.peg_rects[slot_idx] = Some(project_unit_cube_rect(fan_model));
                        }
                        Object3dKind::Bowl => {
                            if obj3d_bowl_slot >= MAX_BOWL_SLOTS {
                                continue;
                            }
                            let slot_i = obj3d_bowl_slot;
                            obj3d_bowl_slot += 1;
                            // Ease hover in-place (can't call self.ease_hover mid-borrow).
                            let target = obj.hover_target.clamp(0.0, 1.0);
                            let anim_id = if obj.anim_id != 0 { obj.anim_id } else { 1 };
                            let k = 1.0 - (-14.0 * self.frame_dt).exp();
                            let e = self.obj3d_hover_state.entry(anim_id).or_insert(0.0);
                            *e += (target - *e) * k;
                            let anim = *e;
                            let lift = anim * obj.extents[1] * 0.15;
                            // Recompute model with hover lift + tilt baked in.
                            // Scene passes rotation_x_deg via obj.rotation (Mat4::from_rotation_x).
                            let tilt = anim * 18.0_f32.to_radians();
                            let center = pixel_to_world(
                                w,
                                h,
                                obj.pos[0],
                                obj.pos[1],
                                obj.pos[2] + obj.extents[1] * 0.5 + lift,
                            );
                            let hover_model = translate_rot_scale(
                                center,
                                glam::Mat4::from_rotation_x(tilt) * obj.rotation,
                                glam::Vec3::from(obj.extents),
                            );
                            let hover_model = self
                                .apply_arrange_override("gameplay.action_bar.bowl", hover_model);
                            self.bowl_instances[slot_i].write_uniform(
                                &self.queue,
                                view_proj_arr,
                                hover_model,
                                self.bowl_mesh.default_material,
                            );
                            if slot_i == 0 {
                                self.proj.bowl_rect = Some(project_aabb_rect(
                                    hover_model,
                                    BOWL_LOCAL_HALF,
                                    BOWL_LOCAL_CENTER_Y,
                                ));
                                self.last_bowl_model = Some(hover_model);
                            }
                            self.last_debug_pickables.push((
                                "gameplay.action_bar.bowl".to_string(),
                                hover_model,
                                glam::Vec3::new(
                                    BOWL_LOCAL_HALF[0],
                                    BOWL_LOCAL_HALF[1],
                                    BOWL_LOCAL_HALF[2],
                                ),
                                BOWL_LOCAL_CENTER_Y,
                            ));
                            object3d_draw_list.push((DrawKind::Bowl, slot_i));
                        }
                    }
                }

                let batch_end = object3d_draw_list.len();
                // Patch the placeholder RenderOp that was pushed during the cmd walk.
                // Find the correct Object3dBatch op by scanning from op_batch_idx.
                while op_batch_idx < ops.len() {
                    if let RenderOp::Object3dBatch { start, end } = &mut ops[op_batch_idx]
                        && *start == 0
                        && *end == 0
                    {
                        *start = batch_start;
                        *end = batch_end;
                        op_batch_idx += 1;
                        break;
                    }
                    op_batch_idx += 1;
                }
                obj3d_cmd_idx += 1;
            }
            let _ = obj3d_cmd_idx;
        }

        // Wall stack: facedown tiles laid out in a row at the back of the
        // table, height growing slightly as the stack thickens. Phase 1 uses
        // the bone tablet mesh (a plain box) — phase 7 may swap to the real
        // tile mesh.
        let mut wall_tile_slot_cursor: usize = 0;
        for w_cmd in wall_stack_cmds.iter() {
            let row_len = w_cmd.row_len.max(1);
            let total = w_cmd.remaining.min(MAX_WALL_TILE_SLOTS as u32);
            let tile_w = w_cmd.tile_extents[0];
            let tile_h = w_cmd.tile_extents[1];
            let tile_d = w_cmd.tile_extents[2];
            for k in 0..total {
                if wall_tile_slot_cursor >= MAX_WALL_TILE_SLOTS {
                    break;
                }
                let col = k % row_len;
                let layer = k / row_len;
                let px = w_cmd.world_pos[0] + col as f32 * tile_w;
                let py = w_cmd.world_pos[1] + layer as f32 * tile_d;
                let pz = w_cmd.world_pos[2] + tile_h * 0.5;
                let center = pixel_to_world(w, h, px, py, pz);
                let model = translate_rot_scale(
                    center,
                    Mat4::IDENTITY,
                    glam::Vec3::new(tile_w, tile_h, tile_d),
                );
                let model = self.apply_arrange_override("WallTile", model);
                let material = MaterialParams {
                    kind: MaterialKind::Plain,
                    base_color: [0.86, 0.81, 0.69, 1.0],
                    specular_strength: 0.20,
                    specular_power: 24.0,
                };
                self.wall_tile_instances[wall_tile_slot_cursor].write_uniform(
                    &self.queue,
                    view_proj_arr,
                    model,
                    material,
                );
                // Wall tiles aren't arrangeable — keep the legacy name so
                // the hit-test debug overlay still identifies them.
                self.last_debug_pickables.push((
                    "gameplay.wall_tile".to_string(),
                    model,
                    glam::Vec3::splat(0.5),
                    0.0,
                ));
                wall_tile_slot_cursor += 1;
            }
        }

        // Cascade scoring tokens migrated to Object3dKind::CascadeToken.

        // Extruded-glyph score popups migrated to Object3dKind::ExtrudedGlyph.

        // ── Arrange-mode bounding box overlay ──────────────────────────────
        // When an object is selected in arrange mode, draw a 2D screen-space
        // rectangle outline around its projected AABB so the user can see
        // exactly what they're moving.
        if let Some(ref ov) = self.debug_arrange_override {
            let aabb = self
                .last_debug_pickables
                .iter()
                .find(|(n, _, _, _)| n == &ov.name)
                .map(|(_, m, h, o)| (*m, *h, *o))
                .or_else(|| {
                    self.last_debug_trimesh_pickables
                        .iter()
                        .find(|(n, _, _)| n == &ov.name)
                        .map(|(_, m, mesh)| match mesh {
                            TrimeshRef::LampBody => {
                                (*m, self.lamp_body_local_half, self.lamp_body_local_center_y)
                            }
                        })
                });
            if let Some((model, half, center_y)) = aabb {
                let [rx, ry, rw, rh] = project_aabb_rect(model, [half.x, half.y, half.z], center_y);
                let t = (h * 0.003).max(2.0); // border thickness in pixels
                let color = [1.0_f32, 0.85, 0.25, 0.9]; // gold
                let border_quads: [GpuInstance; 4] = [
                    // top
                    GpuInstance {
                        rect: [rx, ry, rw, t],
                        color,
                    },
                    // bottom
                    GpuInstance {
                        rect: [rx, ry + rh - t, rw, t],
                        color,
                    },
                    // left
                    GpuInstance {
                        rect: [rx, ry, t, rh],
                        color,
                    },
                    // right
                    GpuInstance {
                        rect: [rx + rw - t, ry, t, rh],
                        color,
                    },
                ];
                let buf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("arrange-bbox"),
                        contents: bytemuck::cast_slice(&border_quads),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let buf_idx = quad_buffers.len();
                quad_buffers.push(buf);
                ops.push(RenderOp::QuadBatch { buf_idx, count: 4 });
            }

            // Clamp-band hint for the selected pickable. Two thin lines at
            // the clamp walls — dim gold when the current `center_frac` is
            // inside the band, red-thick on whichever wall is currently
            // pinning it. Tells the user at a glance why a nudge isn't
            // moving the object any further.
            if let Some(clamp) = frame.arrange_clamps.iter().find(|c| c.name == ov.name) {
                use crate::render::draw_cmd::ClampAxis;
                let dim = [1.0_f32, 0.85, 0.25, 0.35];
                let pin = [1.0_f32, 0.30, 0.25, 0.95];
                let line_t = (h * 0.0018).max(1.5);
                let pin_t = line_t * 3.0;
                let below = clamp.center_frac < clamp.lo_frac;
                let above = clamp.center_frac > clamp.hi_frac;
                let (lo_color, lo_thick) = if below { (pin, pin_t) } else { (dim, line_t) };
                let (hi_color, hi_thick) = if above { (pin, pin_t) } else { (dim, line_t) };
                let clamp_quads: [GpuInstance; 2] = match clamp.axis {
                    ClampAxis::Horizontal => {
                        let lo_px = clamp.lo_frac * w;
                        let hi_px = clamp.hi_frac * w;
                        [
                            GpuInstance {
                                rect: [lo_px - lo_thick * 0.5, 0.0, lo_thick, h],
                                color: lo_color,
                            },
                            GpuInstance {
                                rect: [hi_px - hi_thick * 0.5, 0.0, hi_thick, h],
                                color: hi_color,
                            },
                        ]
                    }
                };
                let buf = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("arrange-clamp"),
                        contents: bytemuck::cast_slice(&clamp_quads),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                let buf_idx = quad_buffers.len();
                quad_buffers.push(buf);
                ops.push(RenderOp::QuadBatch { buf_idx, count: 2 });
            }
        }

        // Relic activation halo buffer — built from `relic_glows` populated
        // during the relic projection loop above. Drawn through the same
        // additive `tile_glow_pipeline` as the selected-tile halos, right
        // after the 3D relic boxes so the bloom blossoms around them.
        let relic_glow_buffer = if relic_glows.is_empty() {
            None
        } else {
            Some(
                self.device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("relic-glow-instances"),
                        contents: bytemuck::cast_slice(&relic_glows),
                        usage: wgpu::BufferUsages::VERTEX,
                    }),
            )
        };

        // ── Volumetric smoke setup (camera, bounds, cursor) ─────────────
        // Done before the encoder is created so the per-tile impulses
        // queued during the tile-loop above are still pending. The grid
        // is sized to comfortably bracket the table area in world units.
        if let Some(ref mut fluid) = self.fluid {
            // Grid bounds: a box roughly enclosing the table with vertical
            // headroom for smoke to rise. World space is Z-up (see
            // `crate::render::world_space`): X is screen-horizontal, Y is
            // screen far/near, Z is up out of the felt. The buoyancy and
            // floor passes use world Z for height_frac, so the grid must be
            // *tall in Z* — not Y, which was the old Y-up convention.
            let half_w = w * 0.75;
            let half_y = h * 0.75;
            let smoke_box_h = h * 0.75 + 12.0;
            let grid_min = glam::Vec3::new(-half_w, -half_y, -12.0);
            let grid_max = glam::Vec3::new(half_w, half_y, grid_min.z + 2.0 * smoke_box_h);
            fluid.set_grid_bounds(grid_min, grid_max);

            // Scene-driven wind gusts. Scenes (currently gameplay) push these
            // when they want a deliberate, time-shaped breath of wind on the
            // smoke — e.g. blowing the post-deal smoke off the hand strip a
            // few seconds after dealing. Coordinates are layout pixels; we
            // run them through `pixel_to_world` so the gust lands on the
            // table plane.
            for g in frame.wind_gusts.iter() {
                let pos = pixel_to_world(w, h, g.center_px.0, g.center_px.1, g.lift);
                fluid.inject_impulse(
                    pos,
                    glam::Vec3::new(g.velocity[0], g.velocity[1], g.velocity[2]),
                    g.radius,
                    g.density * 0.35,
                    0.0,
                    0.0,
                );
            }

            // Opaque shadow casters (shop bugs, etc.). Project pixel-space
            // xy onto the table plane using the same mapping as wind
            // gusts so the shadows land where the meshes visibly are.
            let occluders: Vec<crate::render::fluid::BugOccluder> = frame
                .bug_occluders
                .iter()
                .map(|b| crate::render::fluid::BugOccluder {
                    world_pos: pixel_to_world(w, h, b.center_px.0, b.center_px.1, b.lift),
                    radius: b.radius,
                    strength: b.strength,
                })
                .collect();
            fluid.set_occluders(&occluders);

            // Cursor → table-plane impulse trail. Unproject the screen
            // cursor, intersect z=5, then interpolate between the previous
            // and current world positions to inject a *chain* of small
            // puffs so the trail has no gaps even at low frame rates or
            // fast flicks.
            if let Some((cx, cy)) = frame.cursor_pos {
                // Gate on actual screen-space pointer motion. Without this,
                // a stationary cursor over an orbiting/swaying camera would
                // emit continuous puffs as the unprojected table-plane hit
                // drifts with the camera.
                let screen_moved = match self.prev_cursor_screen {
                    Some((pcx, pcy)) => (cx - pcx).abs() > 0.01 || (cy - pcy).abs() > 0.01,
                    None => false,
                };
                self.prev_cursor_screen = Some((cx, cy));
                let inv_vp = view_proj.inverse();
                let nx = (cx / w) * 2.0 - 1.0;
                let ny = 1.0 - (cy / h) * 2.0;
                let near = inv_vp * glam::Vec4::new(nx, ny, 0.0, 1.0);
                let far = inv_vp * glam::Vec4::new(nx, ny, 1.0, 1.0);
                let near3 = glam::Vec3::new(near.x / near.w, near.y / near.w, near.z / near.w);
                let far3 = glam::Vec3::new(far.x / far.w, far.y / far.w, far.z / far.w);
                let dir = (far3 - near3).normalize_or_zero();
                if dir.z.abs() > 1e-4 {
                    let plane_z = 5.0;
                    let t = (plane_z - near3.z) / dir.z;
                    if t > 0.0 {
                        let hit = near3 + dir * t;
                        if let Some(prev) = self.prev_cursor_world {
                            let raw_delta = hit - prev;
                            let jump = raw_delta.length();
                            let win_scale = (h / 1080.0).max(0.5);
                            let max_jump = 42.0 * win_scale;
                            if screen_moved && jump.is_finite() && jump <= max_jump {
                                let speed_threshold = 0.4 * win_scale;
                                if jump > speed_threshold {
                                    // Drop a line of overlapping gaussian
                                    // puffs between the previous and
                                    // current cursor world positions. The
                                    // new density-only sim transports them
                                    // upward via its drift + curl field,
                                    // so we just need to seed enough mass
                                    // for the plume to read as solid smoke.
                                    let puff_radius = 18.0 * win_scale;
                                    // Spacing below the radius so adjacent
                                    // Gaussians overlap heavily (~e^-0.5 ≈ 60%
                                    // at the midpoint), leaving no visible
                                    // gaps along a fast flick. Cap raised
                                    // from 8 so long drags still fill.
                                    let step_size = puff_radius * 0.8;
                                    let n_puffs = ((jump / step_size).ceil() as u32).clamp(1, 24);

                                    // Perpendicular basis for in-plane jitter:
                                    // table-plane is z=5, so XY are the free axes.
                                    let tangent = raw_delta.normalize_or_zero();
                                    let perp = glam::Vec3::new(-tangent.y, tangent.x, 0.0);

                                    // Wake-vortex strength scales with cursor
                                    // speed — stronger flicks shed stronger
                                    // eddies. Divide by dt so `speed` is in
                                    // world units per second.
                                    let speed = jump / dt.max(1.0 / 120.0);
                                    // Rotational velocity applied to each
                                    // vortex puff along ±perp, producing a
                                    // counter-rotating pair behind the cursor.
                                    let swirl_vel = (speed * 1.1).min(640.0 * win_scale);
                                    // Small retrograde push so vortices sit
                                    // behind the leading edge rather than
                                    // racing ahead with the trail.
                                    let retrograde = speed * 0.12;

                                    use rand::RngExt;
                                    let mut rng = rand::rng();
                                    for i in 0..n_puffs {
                                        let frac = if n_puffs == 1 {
                                            1.0
                                        } else {
                                            (i as f32 + 1.0) / n_puffs as f32
                                        };
                                        let jitter_perp: f32 = rng.random_range(-1.0..1.0);
                                        let jitter_along: f32 = rng.random_range(-0.35..0.35);
                                        let jitter_z: f32 = rng.random_range(-0.4..0.4);
                                        let radius_mul: f32 = rng.random_range(0.75..1.25);
                                        let density_mul: f32 = rng.random_range(0.7..1.15);

                                        let center = prev
                                            + raw_delta * frac
                                            + perp * (jitter_perp * puff_radius * 0.35)
                                            + tangent * (jitter_along * step_size);
                                        let z_lift = glam::Vec3::new(
                                            0.0,
                                            0.0,
                                            (4.0 + jitter_z * 3.0) * win_scale,
                                        );
                                        fluid.inject_impulse(
                                            center + z_lift,
                                            glam::Vec3::ZERO,
                                            puff_radius * radius_mul,
                                            0.13 * density_mul * smoke_amount.density_mul(),
                                            0.0,
                                            0.0,
                                        );

                                        // Shed a counter-rotating vortex pair
                                        // per step, offset ±perp from the
                                        // trail. Alternate which side leads
                                        // per step so the wake staggers like
                                        // a Kármán vortex street instead of
                                        // reading as two parallel rails.
                                        let lead_sign = if i % 2 == 0 { 1.0 } else { -1.0 };
                                        let offset = puff_radius * 0.9;
                                        for side in [-1.0_f32, 1.0_f32] {
                                            let s = side * lead_sign;
                                            let pos = center + perp * (s * offset)
                                                - tangent * (offset * 0.4)
                                                + z_lift;
                                            let vel = perp * (s * swirl_vel) - tangent * retrograde;
                                            fluid.inject_impulse(
                                                pos,
                                                vel,
                                                puff_radius * 0.75 * radius_mul,
                                                0.07 * density_mul * smoke_amount.density_mul(),
                                                0.0,
                                                0.0,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        self.prev_cursor_world = Some(hit);
                    }
                }
            }

            // (Re)allocate the offscreen smoke target whenever the user
            // changes the detail dropdown OR the window resizes. Cheap
            // no-op when nothing changed. Reallocating invalidates the
            // render bgs (they bind the offscreen views as TAA history
            // inputs), so `render_bgs_need_rebuild()` picks that up in
            // the block below.
            fluid.set_detail(&self.device, smoke_quality, &self.depth_copy_view);

            // Build/rebuild the volume render bind groups on first use,
            // after every depth-texture recreation (resize), and after
            // any offscreen reallocation. The smoke pass samples a
            // SNAPSHOT of the depth (`depth_copy_view`) copied between
            // the pre-smoke and post-smoke passes — the live
            // `depth_view` would alias the active depth attachment.
            if self.fluid_render_bg_dirty || fluid.render_bgs_need_rebuild() {
                fluid.rebuild_render_bind_group(
                    &self.device,
                    &self.depth_copy_view,
                    &self.point_lights_buffer,
                );
                self.fluid_render_bg_dirty = false;
            }

            // Upload the per-frame camera uniform consumed by the volume
            // raymarch shader.
            fluid.upload_camera_uniform(
                &self.queue,
                view_proj,
                cam_pos,
                smoke_quality,
                smoke_amount,
            );
        }

        // Garbage-collect stale per-tile world cache entries — drop any uid
        // that wasn't seen in `tile_uids` this frame so the HashMap doesn't
        // grow unbounded across runs.
        if !self.prev_tile_world.is_empty() {
            let live: std::collections::HashSet<u32> = self.tile_uids.iter().copied().collect();
            self.prev_tile_world.retain(|k, _| live.contains(k));
        }

        // ── Shadow map setup ────────────────────────────────────────────
        // Build a single directional shadow camera anchored to the same
        // key direction the lit shaders use (`(0.25, 1.0, 0.35)` normalized).
        // The orthographic frustum is sized to cover the play area where
        // casters live, not the full table — most of the table is empty
        // wood and would burn shadow texels for nothing.
        const SHADOW_MAP_SIZE: f32 = 2048.0;
        let key_dir = glam::Vec3::new(0.25, 1.0, 0.35).normalize();
        // Half-extents in world units. Generous so candles + relics on the
        // sides of the play area stay inside the frustum at any window
        // aspect. World X spans ±w/2, world Z spans ±h/2 — make the
        // ortho box cover both with margin.
        let shadow_half_x = (w * 0.6).max(h * 0.6);
        let shadow_half_z = (w * 0.6).max(h * 0.6);
        // Orthographic basis: light eye sits along +key_dir from the
        // play-area center. The eye distance + far plane are kept TIGHT
        // around the scene (~80 world units of headroom along the light
        // axis) so the [0,1] light-space depth resolves the few units of
        // height between casters and the table well — a generous depth
        // range here would burn precision on empty space and force a
        // huge bias to fight acne.
        let shadow_center = glam::Vec3::new(0.0, 0.0, 0.0);
        let scene_height = 80.0_f32;
        let eye_dist = scene_height * 0.5;
        let shadow_eye = shadow_center + key_dir * eye_dist;
        let shadow_view = Mat4::look_at_rh(shadow_eye, shadow_center, glam::Vec3::Y);
        let shadow_proj = Mat4::orthographic_rh(
            -shadow_half_x,
            shadow_half_x,
            -shadow_half_z,
            shadow_half_z,
            0.1,
            scene_height,
        );
        let light_view_proj = shadow_proj * shadow_view;
        let light_view_proj_arr = light_view_proj.to_cols_array();
        let shadow_enabled_flag = if shadows_enabled { 1.0_f32 } else { 0.0 };
        self.queue.write_buffer(
            &self.shadow_globals_buffer,
            0,
            bytemuck::bytes_of(&ShadowGlobals {
                light_view_proj: light_view_proj_arr,
                params: [
                    shadow_enabled_flag,
                    // Depth bias in light-space [0,1] depth. With
                    // scene_height = 80, 0.005 ≈ 0.4 world units —
                    // big enough to hide self-shadow acne, small enough
                    // that 1-unit-tall tiles still cast onto the table.
                    0.005,
                    1.0 / SHADOW_MAP_SIZE,
                    0.0,
                ],
            }),
        );

        // ── Per-instance shadow caster uniforms ─────────────────────────
        // Mirror the model matrices written into the main lit-mesh +
        // hand-tile uniforms above so the shadow pre-pass can re-render
        // the same geometry from the light's POV.
        // Table is excluded — it's a flat receiver, not a caster.
        // Candle shadows: walk Object3dKind::Candle in the cmd list.
        {
            let mut slot_i = 0usize;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if let crate::render::draw_cmd::Object3dKind::Candle {
                        scale,
                        height_scale,
                    } = o.kind
                    {
                        let Some(instances) = self.candle_instances.get(slot_i) else {
                            break;
                        };
                        let base = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let model = translate_rot_scale(
                            base,
                            mesh_y_thickness_along_local_y_to_z_up(),
                            glam::Vec3::new(scale, scale * height_scale, scale),
                        );
                        let candle_name = self.scene_path("candle");
                        let model = self.apply_arrange_override(&candle_name, model);
                        instances[0].write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                        instances[1].write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                        slot_i += 1;
                    }
                }
            }
        }
        // Shrine shadow casters (pick-blind scene). Same model as the main
        // pass: scale by extents, lift base by half-height.
        {
            let mut shrine_shadow_cursor: usize = 0;
            for batch in &shrine_batches {
                for s in batch.iter() {
                    if shrine_shadow_cursor >= MAX_SHRINE_SLOTS {
                        break;
                    }
                    let slot_i = shrine_shadow_cursor;
                    shrine_shadow_cursor += 1;
                    let center = pixel_to_world(
                        w,
                        h,
                        s.world_pos[0],
                        s.world_pos[1],
                        s.world_pos[2] + s.extents[1] * 0.5,
                    );
                    let model = translate_rot_scale(
                        center,
                        Mat4::IDENTITY,
                        glam::Vec3::new(s.extents[0], s.extents[1], s.extents[2]),
                    );
                    self.shrine_instances[slot_i].write_shadow_uniform(
                        &self.queue,
                        light_view_proj_arr,
                        model,
                    );
                }
            }
        }
        // Ribbon shadow casters — walk Object3dKind::ZodiacRibbon.
        {
            let mut ribbon_shadow_cursor: usize = 0;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if let crate::render::draw_cmd::Object3dKind::ZodiacRibbon { kind } = &o.kind {
                        if ribbon_shadow_cursor >= MAX_RIBBON_SLOTS {
                            break;
                        }
                        let anchor = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let eff_w = o.extents[0];
                        let eff_l = o.extents[1];
                        let depth = o.extents[2];
                        let base_transform =
                            translate_rot_scale(anchor, o.rotation, glam::Vec3::splat(1.0));
                        if kind.is_some() {
                            let nominal_cap = eff_w * 0.33;
                            let cap_h = if eff_l < 2.0 * nominal_cap {
                                eff_l / 2.0
                            } else {
                                nominal_cap
                            };
                            let mid_h = (eff_l - 2.0 * cap_h).max(0.0);
                            let slots_needed = if mid_h > 0.0 { 3 } else { 2 };
                            if ribbon_shadow_cursor + slots_needed > MAX_RIBBON_SLOTS {
                                break;
                            }
                            let top_model = ribbon_submesh(
                                base_transform,
                                0.0,
                                glam::Vec3::new(eff_w, cap_h, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                top_model,
                            );
                            ribbon_shadow_cursor += 1;
                            if mid_h > 0.0 {
                                let mid_model = ribbon_submesh(
                                    base_transform,
                                    -cap_h,
                                    glam::Vec3::new(eff_w, mid_h, depth),
                                );
                                self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                    &self.queue,
                                    light_view_proj_arr,
                                    mid_model,
                                );
                                ribbon_shadow_cursor += 1;
                            }
                            let bot_model = ribbon_submesh(
                                base_transform,
                                -(cap_h + mid_h),
                                glam::Vec3::new(eff_w, cap_h, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                bot_model,
                            );
                            ribbon_shadow_cursor += 1;
                        } else {
                            let model = ribbon_submesh(
                                base_transform,
                                0.0,
                                glam::Vec3::new(eff_w, eff_l, depth),
                            );
                            self.ribbon_instances[ribbon_shadow_cursor].write_shadow_uniform(
                                &self.queue,
                                light_view_proj_arr,
                                model,
                            );
                            ribbon_shadow_cursor += 1;
                        }
                    }
                }
            }
        }
        // Talisman shadows: walk Object3dKind::Talisman in the cmd list.
        {
            let mut talisman_shadow_cursor: usize = 0;
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if matches!(
                        o.kind,
                        crate::render::draw_cmd::Object3dKind::Talisman { .. }
                    ) {
                        if talisman_shadow_cursor >= MAX_TALISMAN_SLOTS {
                            break;
                        }
                        let center = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                        let sx = o.extents[0] / (TALISMAN_LOCAL_HALF[0] * 2.0);
                        let sy = o.extents[1] / (TALISMAN_LOCAL_HALF[1] * 2.0);
                        let sz = o.extents[2] / (TALISMAN_LOCAL_HALF[2] * 2.0);
                        let model =
                            translate_rot_scale(center, o.rotation, glam::Vec3::new(sx, sy, sz));
                        self.talisman_instances[talisman_shadow_cursor].write_shadow_uniform(
                            &self.queue,
                            light_view_proj_arr,
                            model,
                        );
                        talisman_shadow_cursor += 1;
                    }
                }
            }
        }
        // Primitive shadow casters: walk every `Object3dKind::Primitive`
        // in frame cmds whose `shadow_caster` flag is true, and upload
        // the shadow uniform into the matching per-shape instance slot.
        // Slot cursors must track the main dispatch's
        // `obj3d_primitive_slot` exactly so each caster maps to the
        // instance the draw-pass will select.
        {
            use crate::render::primitive::{MeshId, shape_orientation};
            let mut cursors: HashMap<MeshId, usize> = HashMap::new();
            for cmd in frame.cmds.iter() {
                let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                    DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                    DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                    _ => Box::new(std::iter::empty()),
                };
                for o in objs {
                    if let crate::render::draw_cmd::Object3dKind::Primitive {
                        shape,
                        shadow_caster,
                        ..
                    } = &o.kind
                    {
                        // Step the cursor for every Primitive (matches
                        // the main dispatch). Only *write* a shadow
                        // uniform when `shadow_caster: true`.
                        let slot_i = *cursors.entry(*shape).or_insert(0);
                        *cursors.get_mut(shape).unwrap() += 1;
                        if *shadow_caster {
                            let center = pixel_to_world(w, h, o.pos[0], o.pos[1], o.pos[2]);
                            let orient = shape_orientation(*shape);
                            let model = translate_rot_scale(
                                center,
                                o.rotation * orient,
                                glam::Vec3::from(o.extents),
                            );
                            if let Some(pool) = self.primitive_instances.get_mut(shape)
                                && let Some(inst) = pool.get_mut(slot_i)
                            {
                                inst.write_shadow_uniform(&self.queue, light_view_proj_arr, model);
                            }
                        }
                        // CabinetColumn pairs with a CabinetRails
                        // instance in the main dispatch — bump the
                        // rails cursor so shadow slots stay in sync,
                        // but don't cast rails shadows (too thin).
                        if *shape == MeshId::CabinetColumn {
                            *cursors.entry(MeshId::CabinetRails).or_insert(0) += 1;
                        }
                    }
                }
            }
        }
        // Hand tile shadow uniforms — pull each tile's model matrix from
        // `tile_pick_models` (snapshot of the per-tile model written above).
        for (i, model) in &tile_pick_models {
            if let Some(htg) = self.hand_tiles.get(*i) {
                self.queue.write_buffer(
                    &htg.shadow_uniform_buffer,
                    0,
                    bytemuck::bytes_of(&ShadowCasterUniform {
                        light_view_proj: light_view_proj_arr,
                        model: model.to_cols_array(),
                    }),
                );
            }
        }

        // ── Showcase tile GPU resources + uniforms ────────────────────────
        // Grow or update the pool so each tile in every ShowcaseTileBatch has
        // a ready-to-draw ShowcaseTileGpu slot with the correct decal and
        // up-to-date model matrix.
        {
            let total_showcase: usize = showcase_tile_batches
                .iter()
                .map(|b| b.len())
                .sum::<usize>()
                .min(MAX_SHOWCASE_TILE_SLOTS);

            // Ensure we have enough slots.
            while self.showcase_tiles.len() < total_showcase {
                // Placeholder — will be rebuilt immediately below if tile_id
                // doesn't match, but we need *something* to hold the GPU
                // resources. Use the first tile from the first batch.
                let placeholder_tile = showcase_tile_batches
                    .iter()
                    .flat_map(|b| b.iter())
                    .next()
                    .map(|p| &p.tile);
                if let Some(tile) = placeholder_tile {
                    let ctx = ShowcaseTileCtx {
                        device: &self.device,
                        queue: &self.queue,
                        layout: &self.tile_material_layout,
                        shadow_caster_layout: &self.shadow_caster_layout,
                        primitives: &self.tile_primitives,
                        sampler: &self.tile_sampler,
                        ui_font: self.ui_font.as_ref(),
                        emoji_font: self.emoji_font.as_ref(),
                    };
                    let stg = make_showcase_tile_gpu(
                        &ctx,
                        self.tile_base_color_factor,
                        tile,
                        self.tile_set.as_deref(),
                    );
                    self.showcase_tiles.push(stg);
                } else {
                    break;
                }
            }

            // ── HandStrip arrange-mode pre-pass ────────────────────────────
            // When a "HandStrip" arrange override is active, compute the
            // strip's world-space pivot (centroid of all hand tiles — those
            // with a pick_id) and build a delta-rotation matrix so each
            // tile's center is rotated around that pivot before the
            // translation offset is added.
            let hand_strip_arrange: Option<(glam::Vec3, Mat4, glam::Vec3)> = {
                if let Some(ref ov) = self.debug_arrange_override {
                    if ov.name == "HandStrip" {
                        // Collect world centers of hand tiles (pick_id = Some).
                        let hand_centers: Vec<glam::Vec3> = showcase_tile_batches
                            .iter()
                            .flat_map(|b| b.iter())
                            .filter(|p| p.pick_id.is_some())
                            .map(|p| {
                                pixel_to_world(
                                    w,
                                    h,
                                    p.center_pos[0],
                                    p.center_pos[1],
                                    p.center_pos[2],
                                )
                            })
                            .collect();
                        if !hand_centers.is_empty() {
                            let count = hand_centers.len() as f32;
                            let pivot =
                                hand_centers.iter().fold(glam::Vec3::ZERO, |a, &c| a + c) / count;
                            // Delta rotation applied around the pivot in world space.
                            let r_delta = Mat4::from_rotation_z(ov.delta_rz_deg.to_radians())
                                * Mat4::from_rotation_y(ov.delta_ry_deg.to_radians())
                                * Mat4::from_rotation_x(ov.delta_rx_deg.to_radians());
                            // Translation offset: pixel_x → +world_x, pixel_y → -world_y.
                            let translation =
                                glam::Vec3::new(ov.delta_px, -ov.delta_py, ov.delta_lift);
                            Some((pivot, r_delta, translation))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            };

            // Track hand-tile world centers for the HandStrip debug pickable
            // (registered after the loop).
            let mut hand_strip_centers: Vec<glam::Vec3> = Vec::new();

            let mut slot_cursor = 0usize;
            for batch in &showcase_tile_batches {
                for p in batch.iter() {
                    if slot_cursor >= MAX_SHOWCASE_TILE_SLOTS {
                        break;
                    }
                    let wanted_id = (
                        p.tile.suit,
                        p.tile.rank,
                        p.tile.enhancement,
                        p.tile.debuffed_visual,
                    );
                    // Re-rasterise decal if the tile identity changed.
                    if self.showcase_tiles[slot_cursor].tile_id != wanted_id {
                        let ctx = ShowcaseTileCtx {
                            device: &self.device,
                            queue: &self.queue,
                            layout: &self.tile_material_layout,
                            shadow_caster_layout: &self.shadow_caster_layout,
                            primitives: &self.tile_primitives,
                            sampler: &self.tile_sampler,
                            ui_font: self.ui_font.as_ref(),
                            emoji_font: self.emoji_font.as_ref(),
                        };
                        self.showcase_tiles[slot_cursor] = make_showcase_tile_gpu(
                            &ctx,
                            self.tile_base_color_factor,
                            &p.tile,
                            self.tile_set.as_deref(),
                        );
                    }

                    // Build model matrix from the placement's explicit 3D transform.
                    let mut center =
                        pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2]);
                    let tile_short_px = p.size_px * 0.85;
                    let tile_long_px = tile_short_px * tile_preset.face_long_ratio();
                    let tile_thickness_px = tile_short_px * tile_preset.thickness_ratio();
                    let scale = glam::Vec3::new(
                        tile_long_px / LOCAL_X_EXTENT,
                        tile_thickness_px / LOCAL_Y_EXTENT,
                        tile_short_px / LOCAL_Z_EXTENT,
                    ) * p.scale;

                    let mut base_rotation =
                        rot_euler_xyz_rad(p.rotation[0], p.rotation[1], p.rotation[2]);

                    // Apply HandStrip arrange override: rotate each hand tile's
                    // center around the strip pivot, then add the translation.
                    if let (true, Some((pivot, r_delta, translation))) =
                        (p.pick_id.is_some(), &hand_strip_arrange)
                    {
                        let offset = center - *pivot;
                        let rotated_offset = r_delta.transform_vector3(offset);
                        center = *pivot + rotated_offset + *translation;
                        // Also rotate the tile's own orientation so the face
                        // tracks the strip rotation (e.g. ry spins tiles in
                        // place as well as revolving their centers).
                        base_rotation = *r_delta * base_rotation;
                        hand_strip_centers.push(center);
                    }

                    let oriented = base_rotation * tile_basis;
                    let model = translate_rot_scale(center, oriented, scale);

                    // Smoke impulse: compare world position to previous frame's
                    // position for this tile uid and inject velocity into fluid sim.
                    if let Some(pick_id) = p.pick_id {
                        let uid = p.tile.id;
                        if let Some(prev) = self.prev_tile_world.get(&uid).copied() {
                            let delta = center - prev;
                            let speed = delta.length();
                            if speed > 0.5
                                && let Some(ref mut fluid) = self.fluid
                            {
                                let inv_dt = 1.0 / dt.max(1.0 / 120.0);
                                fluid.inject_impulse(
                                    center,
                                    delta * inv_dt * 0.45,
                                    tile_short_px * 0.55,
                                    speed * 0.04,
                                    0.0,
                                    0.0,
                                );
                            }
                        }
                        self.prev_tile_world.insert(uid, center);

                        // Project the tile's 8 corners for the screen AABB,
                        // used for pick tracking and glow rect sizing.
                        let lx = tile_long_px * 0.5;
                        let ly = tile_thickness_px * 0.5;
                        let lz = tile_short_px * 0.5;
                        let sc_corners = [
                            glam::Vec3::new(-lx, -ly, -lz),
                            glam::Vec3::new(lx, -ly, -lz),
                            glam::Vec3::new(-lx, ly, -lz),
                            glam::Vec3::new(lx, ly, -lz),
                            glam::Vec3::new(-lx, -ly, lz),
                            glam::Vec3::new(lx, -ly, lz),
                            glam::Vec3::new(-lx, ly, lz),
                            glam::Vec3::new(lx, ly, lz),
                        ];
                        let mut sc_min_x = f32::INFINITY;
                        let mut sc_min_y = f32::INFINITY;
                        let mut sc_max_x = f32::NEG_INFINITY;
                        let mut sc_max_y = f32::NEG_INFINITY;
                        for c in sc_corners {
                            let world_c = center + oriented.transform_point3(c);
                            let (px, py) = project_to_screen(world_c);
                            sc_min_x = sc_min_x.min(px);
                            sc_min_y = sc_min_y.min(py);
                            sc_max_x = sc_max_x.max(px);
                            sc_max_y = sc_max_y.max(py);
                        }
                        let overlay_w = (sc_max_x - sc_min_x).max(16.0);
                        let overlay_h = (sc_max_y - sc_min_y).max(16.0);
                        let overlay_x = sc_min_x;
                        let overlay_y = sc_min_y;

                        tile_3d_rects.push((pick_id, [overlay_x, overlay_y, overlay_w, overlay_h]));
                        tile_pick_models.push((pick_id, model));

                        if p.glow {
                            let gw = overlay_w * 1.50;
                            let gh = overlay_h * 1.55;
                            let gx = overlay_x + (overlay_w - gw) * 0.5;
                            let gy = overlay_y + (overlay_h - gh) * 0.5;
                            tile_glows.push(GpuInstance {
                                rect: [gx, gy, gw, gh],
                                color: p.glow_color.unwrap_or([1.00, 0.78, 0.32, 0.55]),
                            });
                        }
                    }

                    let stg = &self.showcase_tiles[slot_cursor];
                    let mut sc_bcf = self.tile_base_color_factor;
                    sc_bcf[0] = p.brightness;
                    // 1.0 = selected (gold rim), 0.5 = hovered (cool rim),
                    // 0.0 = none. Hovered supersedes selected.
                    sc_bcf[1] = if p.hovered {
                        0.5
                    } else if p.selected {
                        1.0
                    } else {
                        0.0
                    };
                    sc_bcf[2] = p.tile.enhancement.map_or(0.0, |e| e.shader_id());
                    // Per-tile procedural variation in tile_3d.wgsl (e.g.
                    // tortoise shell mottling) is seeded from the tile's
                    // unique run-scoped id so a given tile keeps the same
                    // pattern across draws, shuffles, and reorders.
                    let tile_seed = p.tile.id as f32;
                    self.queue.write_buffer(
                        &stg.uniform_buffer,
                        0,
                        bytemuck::bytes_of(&CameraUniform {
                            view_proj: view_proj_arr,
                            model: model.to_cols_array(),
                            base_color_factor: sc_bcf,
                            cam_pos: cam_pos.to_array(),
                            tile_seed,
                        }),
                    );
                    // Outline shell: write inflated model matrix when requested.
                    if p.outline {
                        const OUTLINE_GROW: f32 = 1.055;
                        let outline_scale = scale * OUTLINE_GROW;
                        let outline_model = translate_rot_scale(center, oriented, outline_scale);
                        self.queue.write_buffer(
                            &stg.outline_uniform_buffer,
                            0,
                            bytemuck::bytes_of(&CameraUniform {
                                view_proj: view_proj_arr,
                                model: outline_model.to_cols_array(),
                                base_color_factor: sc_bcf,
                                cam_pos: cam_pos.to_array(),
                                tile_seed,
                            }),
                        );
                    }
                    self.queue.write_buffer(
                        &stg.shadow_uniform_buffer,
                        0,
                        bytemuck::bytes_of(&ShadowCasterUniform {
                            light_view_proj: light_view_proj_arr,
                            model: model.to_cols_array(),
                        }),
                    );

                    slot_cursor += 1;
                }
            }

            // Register the hand strip as a single debug-pickable so arrange
            // mode can select it by clicking any tile. The pickable is an AABB
            // that encloses all hand-tile centers (or their arrange-moved
            // positions when an override is already active).
            if !hand_strip_centers.is_empty() || {
                // Fallback: compute from batch placements when the override is
                // not yet active (first click selection).

                showcase_tile_batches
                    .iter()
                    .flat_map(|b| b.iter())
                    .any(|p| p.pick_id.is_some())
            } {
                // Use the centers we collected (post-override) if available,
                // otherwise derive directly from placements.
                let centers: Vec<glam::Vec3> = if !hand_strip_centers.is_empty() {
                    hand_strip_centers.clone()
                } else {
                    showcase_tile_batches
                        .iter()
                        .flat_map(|b| b.iter())
                        .filter(|p| p.pick_id.is_some())
                        .map(|p| {
                            pixel_to_world(w, h, p.center_pos[0], p.center_pos[1], p.center_pos[2])
                        })
                        .collect()
                };
                if !centers.is_empty() {
                    let count = centers.len() as f32;
                    let centroid = centers.iter().fold(glam::Vec3::ZERO, |a, &c| a + c) / count;
                    // Build half-extents that encompass all tile centers plus
                    // one tile-width of padding so clicking the end tiles works.
                    let tile_half = showcase_tile_batches
                        .iter()
                        .flat_map(|b| b.iter())
                        .find(|p| p.pick_id.is_some())
                        .map(|p| p.size_px * 0.5)
                        .unwrap_or(40.0);
                    let mut hx = tile_half;
                    let mut hy = tile_half;
                    let mut hz = tile_half;
                    for c in &centers {
                        let d = (*c - centroid).abs();
                        hx = hx.max(d.x + tile_half);
                        hy = hy.max(d.y + tile_half);
                        hz = hz.max(d.z + tile_half);
                    }
                    let strip_model =
                        translate_rot_scale(centroid, Mat4::IDENTITY, glam::Vec3::new(hx, hy, hz));
                    self.last_debug_pickables.push((
                        "gameplay.hand.strip".to_string(),
                        strip_model,
                        glam::Vec3::splat(0.5),
                        0.0,
                    ));
                }
            }
        }

        // Snapshot projected tile rects and pick models now that both the hand
        // pre-pass and showcase pre-pass have had a chance to push entries.
        self.proj.hand_rects = tile_3d_rects.clone();
        self.last_pick_models = tile_pick_models.clone();
        self.last_pick_camera = Some(PickCamera {
            inv_view_proj: view_proj.inverse(),
            viewport_w: w,
            viewport_h: h,
        });

        // Rebuild projected screen rects for relics/ribbons/talismans from
        // the authoritative `last_*_models` lists. Keeping this as a single
        // bulk step — instead of per-site pushes paired with each model
        // push — means mouse pick (model list) and focus nav (rect list)
        // always see the same set of items; a new draw path can't add a
        // model without a matching rect.
        self.proj.relic_rects.clear();
        for (model, _rid) in &self.last_relic_models {
            self.proj.relic_rects.push(project_unit_cube_rect(*model));
        }
        // Ribbons: mesh local AABB is x ∈ [-0.5, 0.5], y ∈ [-1, 0],
        // z ∈ [-0.05, 0.05] — not the unit cube. Project those bounds so the
        // screen rect lines up with the actual ribbon (otherwise it ends up
        // half-height and shifted up by half the ribbon length).
        self.proj.ribbon_rects.clear();
        for model in &self.last_ribbon_models {
            self.proj
                .ribbon_rects
                .push(project_aabb_rect(*model, [0.5, 0.5, 0.05], -0.5));
        }
        // Talismans: local mesh AABB is `TALISMAN_LOCAL_HALF` (y=0.7, z=0.09),
        // not ±0.5. The model already bakes the world scale (see sx/sy/sz
        // derivations against `TALISMAN_LOCAL_HALF * 2`), so we must project
        // the real local bounds — unit-cube projection clips ~30% off height
        // and 5.5× overstates depth.
        self.proj.talisman_rects.clear();
        for model in &self.last_talisman_models {
            self.proj
                .talisman_rects
                .push(project_aabb_rect(*model, TALISMAN_LOCAL_HALF, 0.0));
        }

        // Sync singleton shop-prop models (journal book, reroll prop, leave
        // prop, sell tray) into `aux_dish_rects` so focus nav can reach
        // them. Dishes authored via `DishExplicit` were already pushed
        // during their pass; these props come through Object3d kinds that
        // only update model snapshots, so we project them here. Packs live
        // in `pack_rects` (both the PackBatch and Object3d paths populate
        // it) and get appended last.
        if let Some((model, pid)) = self.last_sell_tray_model {
            self.proj
                .aux_dish_rects
                .push((Some(pid), project_unit_cube_rect(model)));
        }
        for (rect, pick_id) in &self.proj.pack_rects {
            self.proj.aux_dish_rects.push((*pick_id, *rect));
        }

        // Tile occluder buffer — analytic AABBs for the per-fragment ray
        // occlusion test that gives the candle pools their tile shadows.
        // Each tile contributes a single conservative world-space AABB
        // built from the 8 transformed local corners of its mesh extent.
        // Limited to MAX_TILE_OCCLUDERS so the uniform stays bounded.
        //
        // After collecting per-tile boxes we inflate adjacent tiles toward
        // each other so their AABBs touch along the row axis. Without this,
        // the back candles sit high above the table and their light threads
        // through the visible gaps between hand tiles, painting sharp
        // specular streaks on the table in front of the row (the row is
        // visually contiguous but physically gappy). The inflation per side
        // is half the gap to the nearest neighbour, so distant tiles never
        // smear into each other.
        {
            let hx = LOCAL_X_EXTENT * 0.5;
            let hy = LOCAL_Y_EXTENT * 0.5;
            let hz = LOCAL_Z_EXTENT * 0.5;
            let local_corners = [
                glam::Vec3::new(-hx, -hy, -hz),
                glam::Vec3::new(hx, -hy, -hz),
                glam::Vec3::new(-hx, hy, -hz),
                glam::Vec3::new(hx, hy, -hz),
                glam::Vec3::new(-hx, -hy, hz),
                glam::Vec3::new(hx, -hy, hz),
                glam::Vec3::new(-hx, hy, hz),
                glam::Vec3::new(hx, hy, hz),
            ];
            let mut tiles: Vec<(glam::Vec3, glam::Vec3)> =
                Vec::with_capacity(tile_pick_models.len().min(MAX_TILE_OCCLUDERS));
            for (_, model) in &tile_pick_models {
                if tiles.len() >= MAX_TILE_OCCLUDERS {
                    break;
                }
                let mut lo = glam::Vec3::splat(f32::INFINITY);
                let mut hi = glam::Vec3::splat(f32::NEG_INFINITY);
                for c in local_corners.iter() {
                    let w = model.transform_point3(*c);
                    lo = lo.min(w);
                    hi = hi.max(w);
                }
                tiles.push(((lo + hi) * 0.5, (hi - lo) * 0.5));
            }

            // Pick the dominant horizontal axis (X or Y on the felt; Z is up) by
            // comparing the spread of tile centers. The hand is laid out
            // along screen X — that's world X after `pixel_to_world` — but
            // detecting it from the data keeps this robust if the layout
            // ever rotates.
            if tiles.len() >= 2 {
                let (mut min_x, mut max_x) = (f32::INFINITY, f32::NEG_INFINITY);
                let (mut min_z, mut max_z) = (f32::INFINITY, f32::NEG_INFINITY);
                for (c, _) in &tiles {
                    min_x = min_x.min(c.x);
                    max_x = max_x.max(c.x);
                    min_z = min_z.min(c.z);
                    max_z = max_z.max(c.z);
                }
                let row_axis_x = (max_x - min_x) >= (max_z - min_z);

                let mut order: Vec<usize> = (0..tiles.len()).collect();
                order.sort_by(|&a, &b| {
                    let ka = if row_axis_x {
                        tiles[a].0.x
                    } else {
                        tiles[a].0.z
                    };
                    let kb = if row_axis_x {
                        tiles[b].0.x
                    } else {
                        tiles[b].0.z
                    };
                    ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
                });
                for win in order.windows(2) {
                    let (a, b) = (win[0], win[1]);
                    let (ca, cb) = (tiles[a].0, tiles[b].0);
                    let (ha, hb) = (tiles[a].1, tiles[b].1);
                    let gap = if row_axis_x {
                        (cb.x - ca.x) - (ha.x + hb.x)
                    } else {
                        (cb.z - ca.z) - (ha.z + hb.z)
                    };
                    if gap > 0.0 {
                        let pad = gap * 0.5;
                        if row_axis_x {
                            tiles[a].1.x += pad;
                            tiles[b].1.x += pad;
                        } else {
                            tiles[a].1.z += pad;
                            tiles[b].1.z += pad;
                        }
                    }
                }
            }

            let mut occ = TileOccludersBuf::empty();
            for (i, (center, half)) in tiles.iter().enumerate() {
                occ.boxes[i] = TileOccluderGpu {
                    center: [center.x, center.y, center.z, 0.0],
                    half_extents: [half.x, half.y, half.z, 0.0],
                };
            }
            occ.count[0] = tiles.len() as u32;
            self.queue
                .write_buffer(&self.tile_occluders_buffer, 0, bytemuck::bytes_of(&occ));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });

        // Run fluid simulation compute passes (before render pass).
        //
        // Use the real inter-frame `dt` captured at the top of `render()`.
        // The previous `self.last_frame.elapsed()` here was a bug: by this
        // point we've already reassigned `self.last_frame = now`, so the
        // elapsed value is just the time spent on render work earlier in
        // this same function — typically 5–15 ms regardless of FPS. That
        // made the sim advance only ~0.5–0.9 seconds of simulated time per
        // wall second, so the post-deal wind sweep (1.4s wall) only got
        // ~0.7s of advection and intermittently failed to push the opening
        // smoke curtain off-grid before the overlay finished fading.
        // `dt` is already capped at 50 ms above, which is plenty of
        // headroom for the semi-Lagrangian step to stay stable.
        if let Some(ref mut fluid) = self.fluid {
            let step_dt = dt.max(1.0 / 120.0);
            fluid.step(&mut encoder, &self.queue, step_dt, smoke_quality);
        }

        // ── Shadow pre-pass ─────────────────────────────────────────────
        // Render every caster (table excluded) into the shadow map from
        // the light's POV. Skipped entirely when shadows are disabled —
        // the lit shaders short-circuit on `params.x = 0` and the stale
        // map contents go unread.
        if shadows_enabled {
            let shadow_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::Shadow);
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-pre-pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_map_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: shadow_ts,
                multiview_mask: None,
            });
            shadow_pass.set_pipeline(&self.shadow_pipeline);

            // Candles (wax + wick) — pool is written above via Object3dKind::Candle.
            {
                let candle_count = frame
                    .cmds
                    .iter()
                    .flat_map(
                        |cmd| -> Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> {
                            match cmd {
                                DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                                DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                                _ => Box::new(std::iter::empty()),
                            }
                        },
                    )
                    .filter(|o| {
                        matches!(o.kind, crate::render::draw_cmd::Object3dKind::Candle { .. })
                    })
                    .count();
                for slot_i in 0..candle_count {
                    let Some(instances) = self.candle_instances.get(slot_i) else {
                        break;
                    };
                    shadow_pass.set_bind_group(0, &instances[0].shadow_bind_group, &[]);
                    shadow_pass.set_vertex_buffer(0, self.candle_wax_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.candle_wax_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    shadow_pass.draw_indexed(0..self.candle_wax_mesh.index_count, 0, 0..1);

                    shadow_pass.set_bind_group(0, &instances[1].shadow_bind_group, &[]);
                    shadow_pass.set_vertex_buffer(0, self.candle_wick_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.candle_wick_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    shadow_pass.draw_indexed(0..self.candle_wick_mesh.index_count, 0, 0..1);
                }
            }

            // Shrines (pick-blind scene).
            {
                let total_shrines = shrine_batches
                    .iter()
                    .map(|b| b.len())
                    .sum::<usize>()
                    .min(MAX_SHRINE_SLOTS);
                if total_shrines > 0 {
                    shadow_pass.set_vertex_buffer(0, self.shrine_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.shrine_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_shrines {
                        let Some(inst) = self.shrine_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.shrine_mesh.index_count, 0, 0..1);
                    }
                }
            }

            // (Dish shadow casting now flows through the generic
            // Primitive shadow block below.)

            // Ribbons (shop).
            {
                let total_ribbons = self.last_ribbon_slot_count;
                if total_ribbons > 0 {
                    shadow_pass.set_vertex_buffer(0, self.ribbon_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.ribbon_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_ribbons {
                        let Some(inst) = self.ribbon_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.ribbon_mesh.index_count, 0, 0..1);
                    }
                }
            }

            // Talismans — count Object3dKind::Talisman entries and draw their shadow instances.
            {
                let total_talismans = frame
                    .cmds
                    .iter()
                    .flat_map(
                        |cmd| -> Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> {
                            match cmd {
                                DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                                DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                                _ => Box::new(std::iter::empty()),
                            }
                        },
                    )
                    .filter(|o| {
                        matches!(
                            o.kind,
                            crate::render::draw_cmd::Object3dKind::Talisman { .. }
                        )
                    })
                    .count()
                    .min(MAX_TALISMAN_SLOTS);
                if total_talismans > 0 {
                    shadow_pass.set_vertex_buffer(0, self.talisman_mesh.vertex_buffer.slice(..));
                    shadow_pass.set_index_buffer(
                        self.talisman_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    for slot_i in 0..total_talismans {
                        let Some(inst) = self.talisman_instances.get(slot_i) else {
                            break;
                        };
                        shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                        shadow_pass.draw_indexed(0..self.talisman_mesh.index_count, 0, 0..1);
                    }
                }
            }

            // Primitive shadow casters — re-walk cmds to pair slot
            // indices with `shadow_caster: true` flags, then draw with
            // the registered mesh. Deterministic order (matches the
            // uniform-upload pass above).
            {
                use crate::render::primitive::MeshId;
                let mut cursors: std::collections::HashMap<MeshId, usize> =
                    std::collections::HashMap::new();
                for cmd in frame.cmds.iter() {
                    let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> =
                        match cmd {
                            DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                            DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                            _ => Box::new(std::iter::empty()),
                        };
                    for o in objs {
                        if let crate::render::draw_cmd::Object3dKind::Primitive {
                            shape,
                            shadow_caster,
                            ..
                        } = &o.kind
                        {
                            let slot_i = *cursors.entry(*shape).or_insert(0);
                            *cursors.get_mut(shape).unwrap() += 1;
                            if *shadow_caster {
                                let (Some(mesh), Some(inst)) = (
                                    self.primitive_meshes.get(shape).map(|a| a.as_ref()),
                                    self.primitive_instances
                                        .get(shape)
                                        .and_then(|pool| pool.get(slot_i)),
                                ) else {
                                    continue;
                                };
                                shadow_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                                shadow_pass.set_index_buffer(
                                    mesh.index_buffer.slice(..),
                                    wgpu::IndexFormat::Uint32,
                                );
                                shadow_pass.set_bind_group(0, &inst.shadow_bind_group, &[]);
                                shadow_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                            }
                            if *shape == MeshId::CabinetColumn {
                                *cursors.entry(MeshId::CabinetRails).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }

            // Hand tiles — one draw per (tile, primitive). Same multi-prim
            // walk the main pass uses, but only the position attribute is
            // read by the shadow shader so the bind group is the per-tile
            // shadow uniform, not the multi-prim main bind group.
            if !self.tile_primitives.is_empty() {
                for (i, _) in &tile_3d_rects {
                    let Some(htg) = self.hand_tiles.get(*i) else {
                        continue;
                    };
                    shadow_pass.set_bind_group(0, &htg.shadow_bind_group, &[]);
                    for prim in &self.tile_primitives {
                        shadow_pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                        shadow_pass.set_index_buffer(
                            prim.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        shadow_pass.draw_indexed(0..prim.index_count, 0, 0..1);
                    }
                }

                // Showcase tiles — same mesh, separate GPU resource pool.
                let total_showcase: usize = showcase_tile_batches
                    .iter()
                    .map(|b| b.len())
                    .sum::<usize>()
                    .min(MAX_SHOWCASE_TILE_SLOTS);
                for slot_i in 0..total_showcase {
                    let Some(stg) = self.showcase_tiles.get(slot_i) else {
                        break;
                    };
                    shadow_pass.set_bind_group(0, &stg.shadow_bind_group, &[]);
                    for prim in &self.tile_primitives {
                        shadow_pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                        shadow_pass.set_index_buffer(
                            prim.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        shadow_pass.draw_indexed(0..prim.index_count, 0, 0..1);
                    }
                }
            }
        }

        // Pre-create background image instance buffer (must outlive render pass).
        let bg_inst = GpuInstance {
            rect: [
                0.0,
                0.0,
                self.size.width.max(1) as f32,
                self.size.height.max(1) as f32,
            ],
            color: [1.0, 1.0, 1.0, 1.0],
        };
        let bg_inst_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("bg-inst"),
                contents: bytemuck::cast_slice(&[bg_inst]),
                usage: wgpu::BufferUsages::VERTEX,
            });

        // Find the FluidSmoke marker so we can split the render pass: the
        // smoke fragment shader samples the depth buffer, which can't alias
        // the live depth attachment, so we end pass A right before the
        // smoke draw, copy depth → depth_copy, then start pass B (loading
        // color & depth) with the smoke as its first draw.
        let split_idx = ops.iter().position(|o| matches!(o, RenderOp::FluidSmoke));
        let split_end = split_idx.unwrap_or(ops.len());
        // Closure that processes one render op against the supplied pass.
        // Captures self + per-frame locals immutably (Rust 2021 disjoint
        // capture). Used twice — once for ops before smoke, once for ops
        // from smoke onwards.
        let process_op = |pass: &mut wgpu::RenderPass<'_>, op: &RenderOp| {
            match op {
                RenderOp::Background(id) => {
                    if let Some(bg_tex) = self.background_textures.get(id) {
                        pass.set_pipeline(&self.image_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &bg_tex.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, bg_inst_buf.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..1);
                    }
                }
                RenderOp::Starfield => {
                    pass.set_pipeline(&self.starfield_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                RenderOp::EmberDrift => {
                    pass.set_pipeline(&self.ember_drift_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                RenderOp::GoldenDust => {
                    pass.set_pipeline(&self.golden_dust_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                RenderOp::MoonlitWater => {
                    pass.set_pipeline(&self.moonlit_water_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &self.moon_albedo_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                RenderOp::MountainHaze => {
                    pass.set_pipeline(&self.mountain_haze_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &self.haze_uniform_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                RenderOp::SunlitWater => {
                    pass.set_pipeline(&self.sunlit_water_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                RenderOp::ShootingStarCascade => {
                    // Cascade was rendered into the half-res offscreen target
                    // in the pre-pass above; here we just sample+additively
                    // composite it onto the main scene target.
                    pass.set_pipeline(&self.cascade_composite_pipeline);
                    pass.set_bind_group(0, &self.cascade_composite_bind_group, &[]);
                    pass.draw(0..3, 0..1);
                }
                RenderOp::Table => {
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(0, &self.table_instance.bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.table_mesh.vertex_buffer.slice(..));
                    pass.set_index_buffer(
                        self.table_mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    pass.draw_indexed(0..self.table_mesh.index_count, 0, 0..1);
                }
                RenderOp::Object3dBatch { start, end } => {
                    pass.set_pipeline(&self.lit_mesh_pipeline);
                    pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                    pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                    pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                    let mut current_blended = false;
                    for &(kind, slot_i) in &object3d_draw_list[*start..*end] {
                        // Live wings carry per-frame `live_wing_alpha` tinting
                        // (crisp at turnarounds, faded at mid-stroke) and the
                        // blur fans carry `blur_alpha` (inverse). Both need the
                        // alpha-blended pipeline now that they're tinted.
                        let want_blended = matches!(
                            kind,
                            DrawKind::BugWingL
                                | DrawKind::BugWingR
                                | DrawKind::BugWingBlurL
                                | DrawKind::BugWingBlurR
                        );
                        if want_blended != current_blended {
                            if want_blended {
                                pass.set_pipeline(&self.lit_mesh_blended_pipeline);
                            } else {
                                pass.set_pipeline(&self.lit_mesh_pipeline);
                            }
                            current_blended = want_blended;
                        }
                        // Relic mesh is looked up per relic_id stored in relic_slot_texture.
                        if matches!(kind, DrawKind::Relic) {
                            let mesh = match self.relic_slot_texture.get(slot_i).copied().flatten()
                            {
                                Some(rid) => self.relic_mesh_for(rid),
                                None => &self.relic_box_mesh,
                            };
                            if let Some(inst) = self.relic_instances.get(slot_i) {
                                pass.set_bind_group(0, &inst.bind_group, &[]);
                                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                                pass.set_index_buffer(
                                    mesh.index_buffer.slice(..),
                                    wgpu::IndexFormat::Uint32,
                                );
                                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                            }
                            continue;
                        }
                        // Extruded glyph mesh is per-label. Look it up from the scan
                        // of the cmd list; slot_i maps to the Nth ExtrudedGlyph in
                        // draw order.
                        if matches!(kind, DrawKind::ExtrudedGlyph) {
                            let label: Option<&str> = frame
                                .cmds
                                .iter()
                                .flat_map(
                                    |cmd| -> Box<
                                        dyn Iterator<Item = &crate::render::draw_cmd::Object3d>,
                                    > {
                                        match cmd {
                                            DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                                            DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                                            _ => Box::new(std::iter::empty()),
                                        }
                                    },
                                )
                                .filter_map(|o| match &o.kind {
                                    crate::render::draw_cmd::Object3dKind::ExtrudedGlyph {
                                        label,
                                        ..
                                    } => Some(label.as_str()),
                                    _ => None,
                                })
                                .nth(slot_i);
                            if let (Some(lbl), Some(inst)) =
                                (label, self.extruded_glyph_instances.get(slot_i))
                                && let Some(mesh) = self.extruded_glyph_meshes.get(lbl)
                            {
                                pass.set_bind_group(0, &inst.bind_group, &[]);
                                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                                pass.set_index_buffer(
                                    mesh.index_buffer.slice(..),
                                    wgpu::IndexFormat::Uint32,
                                );
                                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                            }
                            continue;
                        }
                        // Candle uses a [LitMeshInstance; 2] pair indexed by slot_i;
                        // wax = pair[0], wick = pair[1].
                        if matches!(kind, DrawKind::CandleWax | DrawKind::CandleWick) {
                            let (mesh, idx) = match kind {
                                DrawKind::CandleWax => (&self.candle_wax_mesh, 0),
                                _ => (&self.candle_wick_mesh, 1),
                            };
                            if let Some(pair) = self.candle_instances.get(slot_i) {
                                pass.set_bind_group(0, &pair[idx].bind_group, &[]);
                                pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                                pass.set_index_buffer(
                                    mesh.index_buffer.slice(..),
                                    wgpu::IndexFormat::Uint32,
                                );
                                pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                            }
                            continue;
                        }
                        let (mesh, inst_opt): (&LitMeshGpu, Option<&LitMeshInstance>) = match kind {
                            DrawKind::YakuTablet => (
                                &self.bone_tablet_mesh,
                                self.yaku_tablet_instances.get(slot_i),
                            ),
                            DrawKind::WoodTablet => (
                                &self.wood_tablet_mesh,
                                self.wood_tablet_instances.get(slot_i),
                            ),
                            DrawKind::Pack => (&self.pack_mesh, self.pack_instances.get(slot_i)),
                            DrawKind::Ribbon => {
                                (&self.ribbon_mesh, self.ribbon_instances.get(slot_i))
                            }
                            DrawKind::Talisman => {
                                (&self.talisman_mesh, self.talisman_instances.get(slot_i))
                            }
                            DrawKind::Shrine => {
                                (&self.shrine_mesh, self.shrine_instances.get(slot_i))
                            }
                            DrawKind::SellTray => {
                                (&self.round_dish_mesh, Some(&self.sell_tray_instance))
                            }
                            DrawKind::LampBody => {
                                (&self.lamp_body_mesh, Some(&self.lamp_body_instance))
                            }
                            DrawKind::LampBulb => {
                                (&self.lamp_bulb_mesh, Some(&self.lamp_bulb_instance))
                            }
                            DrawKind::BugBody => {
                                (&self.bug_body_mesh, self.bug_body_instances.get(slot_i))
                            }
                            DrawKind::BugWingL => {
                                (&self.bug_wing_mesh, self.bug_wing_instances.get(slot_i))
                            }
                            DrawKind::BugWingBlurL => (
                                &self.bug_wing_blur_mesh,
                                self.bug_wing_blur_instances.get(slot_i),
                            ),
                            DrawKind::BugWingR => {
                                (&self.bug_wing_mesh, self.bug_wing_r_instances.get(slot_i))
                            }
                            DrawKind::BugWingBlurR => (
                                &self.bug_wing_blur_mesh,
                                self.bug_wing_blur_r_instances.get(slot_i),
                            ),
                            DrawKind::Orb => (&self.orb_mesh, self.orb_instances.get(slot_i)),
                            DrawKind::DoraPlinth => (
                                &self.dora_plinth_mesh,
                                self.dora_plinth_instances.get(slot_i),
                            ),
                            DrawKind::Bowl => (&self.bowl_mesh, self.bowl_instances.get(slot_i)),
                            DrawKind::Mirror => {
                                (&self.mirror_mesh, self.mirror_instances.get(slot_i))
                            }
                            DrawKind::TallyStickBase => (
                                &self.tally_stick_base_mesh,
                                self.tally_stick_instances.get(slot_i),
                            ),
                            DrawKind::TallyStickTip => (
                                &self.tally_stick_tip_mesh,
                                self.tally_stick_instances.get(slot_i),
                            ),
                            DrawKind::CascadeToken => (
                                &self.bone_tablet_mesh,
                                self.cascade_token_instances.get(slot_i),
                            ),
                            DrawKind::Primitive(mid) => {
                                let mesh = self
                                    .primitive_meshes
                                    .get(&mid)
                                    .map(|a| a.as_ref())
                                    .expect("primitive mesh missing from registry");
                                let inst = self
                                    .primitive_instances
                                    .get(&mid)
                                    .and_then(|pool| pool.get(slot_i));
                                (mesh, inst)
                            }
                            // Handled by the early-out blocks above.
                            DrawKind::Relic
                            | DrawKind::ExtrudedGlyph
                            | DrawKind::CandleWax
                            | DrawKind::CandleWick => unreachable!(),
                        };
                        let Some(inst) = inst_opt else { continue };
                        pass.set_bind_group(0, &inst.bind_group, &[]);
                        pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                    }
                    // Sell-tray "SELL" tent card — drawn last in the same
                    // pipeline state when the tray was focused this frame.
                    if self.last_sell_card_model.is_some() && self.sell_card_decal_ready {
                        if current_blended {
                            pass.set_pipeline(&self.lit_mesh_pipeline);
                        }
                        pass.set_bind_group(0, &self.sell_card_instance.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.sell_card_mesh.vertex_buffer.slice(..));
                        pass.set_index_buffer(
                            self.sell_card_mesh.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        pass.draw_indexed(0..self.sell_card_mesh.index_count, 0, 0..1);
                    }
                    // Relic activation halos — additive bloom rects drawn
                    // after the relic meshes so the falloff spills past the
                    // silhouette. Fires whenever any relic in the scene this
                    // frame had glow > 0 (Object3dKind::Relic accumulates).
                    if let Some(ref rgb) = relic_glow_buffer {
                        pass.set_pipeline(&self.tile_glow_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, rgb.slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..relic_glows.len() as u32);
                    }
                }
                RenderOp::ShowcaseTileBatch(batch_idx) => {
                    if !self.tile_primitives.is_empty() {
                        let batch = showcase_tile_batches[*batch_idx];
                        if !batch.is_empty() {
                            pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                            pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                            pass.set_bind_group(3, &self.spot_lights_bind_group, &[]);
                            let start_slot: usize = showcase_tile_batches
                                .iter()
                                .take(*batch_idx)
                                .map(|b| b.len())
                                .sum();

                            // Glow halos for selected hand tiles (additive, drawn before mesh).
                            let has_glow = batch.iter().any(|p| p.glow);
                            if has_glow && let Some(ref tgb) = tile_glow_buffer {
                                pass.set_pipeline(&self.tile_glow_pipeline);
                                pass.set_bind_group(0, &self.globals_bind_group, &[]);
                                pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                                pass.set_vertex_buffer(1, tgb.slice(..));
                                pass.set_index_buffer(
                                    self.index_buffer.slice(..),
                                    wgpu::IndexFormat::Uint16,
                                );
                                pass.draw_indexed(0..6, 0, 0..tile_glows.len() as u32);
                            }

                            // Pass A: gold outline shells for tiles with outline=true.
                            let has_outline = batch.iter().any(|p| p.outline);
                            if has_outline {
                                pass.set_pipeline(&self.tile_outline_pipeline);
                                for (i, p) in batch.iter().enumerate() {
                                    if !p.outline {
                                        continue;
                                    }
                                    let slot_i = start_slot + i;
                                    if slot_i >= MAX_SHOWCASE_TILE_SLOTS {
                                        break;
                                    }
                                    let Some(stg) = self.showcase_tiles.get(slot_i) else {
                                        break;
                                    };
                                    for (pi, prim) in self.tile_primitives.iter().enumerate() {
                                        let Some(bg) = stg.outline_bind_groups.get(pi) else {
                                            continue;
                                        };
                                        pass.set_bind_group(0, bg, &[]);
                                        pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                                        pass.set_index_buffer(
                                            prim.index_buffer.slice(..),
                                            wgpu::IndexFormat::Uint32,
                                        );
                                        pass.draw_indexed(0..prim.index_count, 0, 0..1);
                                    }
                                }
                            }

                            // Pass B: regular textured tile meshes.
                            pass.set_pipeline(&self.tile_pipeline);
                            for (i, _) in batch.iter().enumerate() {
                                let slot_i = start_slot + i;
                                if slot_i >= MAX_SHOWCASE_TILE_SLOTS {
                                    break;
                                }
                                let Some(stg) = self.showcase_tiles.get(slot_i) else {
                                    break;
                                };
                                for (pi, prim) in self.tile_primitives.iter().enumerate() {
                                    let Some(bg) = stg.bind_groups.get(pi) else {
                                        continue;
                                    };
                                    pass.set_bind_group(0, bg, &[]);
                                    pass.set_vertex_buffer(0, prim.vertex_buffer.slice(..));
                                    pass.set_index_buffer(
                                        prim.index_buffer.slice(..),
                                        wgpu::IndexFormat::Uint32,
                                    );
                                    pass.draw_indexed(0..prim.index_count, 0, 0..1);
                                }
                            }
                        }
                    }
                }
                RenderOp::FluidSmoke => {
                    if smoke_quality != crate::persistence::SmokeQuality::Off
                        && let Some(ref fluid) = self.fluid
                    {
                        // Composite the offscreen smoke target onto the
                        // swap chain. The actual raymarch ran earlier in
                        // its own offscreen pass; this is just a
                        // bilinear sample + premultiplied blend.
                        fluid.draw_composite(pass);
                    }
                }
                RenderOp::QuadBatch { buf_idx, count } => {
                    pass.set_pipeline(&self.quad_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, quad_buffers[*buf_idx].slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..*count);
                }
                RenderOp::GradientQuadBatch { buf_idx, count } => {
                    pass.set_pipeline(&self.gradient_quad_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, gradient_quad_buffers[*buf_idx].slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..*count);
                }
                RenderOp::FlameBatch { buf_idx, count } => {
                    // When the volumetric smoke sim is active, candle flames
                    // are rendered as 3D emission inside the volume lightbake
                    // pass — skip the particle billboards so we don't
                    // double-draw. With smoke Off, the fluid sim doesn't
                    // step and volumetric flames wouldn't appear, so we
                    // drive the 3D particle system instead.
                    if smoke_quality == crate::persistence::SmokeQuality::Off
                        && *count > 0
                        && *buf_idx != usize::MAX
                    {
                        pass.set_pipeline(&self.flame_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &self.flame_view_bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, flame_buffers[*buf_idx].slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..*count);
                    }
                }
                RenderOp::TextDraw(idx) => {
                    let td = &text_draws[*idx];
                    pass.set_pipeline(&self.text_pipeline);
                    pass.set_bind_group(0, &self.globals_bind_group, &[]);
                    pass.set_bind_group(1, &td.bind_group, &[]);
                    pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                    pass.set_vertex_buffer(1, td.inst_buf.slice(..));
                    pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    pass.draw_indexed(0..6, 0, 0..1);
                }
                RenderOp::TileFaceQuad(idx) => {
                    let face = &tile_face_quads[*idx];
                    let key = (
                        face.tile.suit,
                        face.tile.rank,
                        face.tile.enhancement,
                        face.tile.debuffed_visual,
                    );
                    if let Some(gpu) = self.tile_face_overlays.get(&key) {
                        pass.set_pipeline(&self.image_pipeline);
                        pass.set_bind_group(0, &self.globals_bind_group, &[]);
                        pass.set_bind_group(1, &gpu.bind_group, &[]);
                        pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
                        pass.set_vertex_buffer(1, tile_face_inst_buffers[*idx].slice(..));
                        pass.set_index_buffer(
                            self.index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        pass.draw_indexed(0..6, 0, 0..1);
                    }
                }
            }
        }; // end process_op closure

        // ── Pre-pass: shooting-star cascade into half-res offscreen ─────
        // The cascade shader is extremely heavy per-pixel, so it renders at
        // quarter-area (half dims) and is additively composited up to the
        // main scene target inside `Pass A`. Skip the pass entirely when no
        // cascade op is queued so the clear isn't paid for on every frame.
        let cascade_active = ops
            .iter()
            .any(|o| matches!(o, RenderOp::ShootingStarCascade));
        if cascade_active {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cascade-offscreen-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.cascade_offscreen_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.shooting_star_cascade_pipeline);
            pass.set_bind_group(0, &self.globals_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // ── Pass A: clear + draw everything that lives behind the smoke ──
        {
            let main_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::Main);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scene_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: main_ts,
                multiview_mask: None,
            });
            for op in &ops[..split_end] {
                // 2D HUD text labels are drawn into the swapchain in a
                // separate overlay pass after the SSR snapshot, so they
                // don't end up in `scene_prev_texture` and get reflected by
                // the lacquered table. See the overlay pass below the
                // end-of-frame copies.
                //
                // `Plaque` ops are also held back: the score plaque now
                // carries an engraved decal texture (the score header
                // text) baked onto its +Z face, and if it were drawn here
                // the lacquered-table SSR would reflect that engraved
                // text into the table — recreating the exact ghost-text
                // artefact the overlay pass was originally introduced to
                // avoid. We snapshot `scene_prev` + `ssr_prev_depth`
                // immediately after this loop and *then* draw the plaques
                // in a sibling pass that loads the swapchain.
                if matches!(op, RenderOp::TextDraw(_)) {
                    continue;
                }
                process_op(&mut pass, op);
            }

            // Debug world-axes overlay: draw three colored bars after the
            // normal pass-A 3D ops so they sit on top of the table. Uses
            // the shared `relic_box_mesh` unit cube; per-instance uniforms
            // were written above.
            if frame.debug_axes {
                pass.set_pipeline(&self.lit_mesh_pipeline);
                pass.set_bind_group(3, &self.lit_mesh_ssr_bind_group, &[]);
                pass.set_bind_group(1, &self.point_lights_bind_group, &[]);
                pass.set_bind_group(2, &self.shadow_sample_bind_group, &[]);
                pass.set_vertex_buffer(0, self.relic_box_mesh.vertex_buffer.slice(..));
                pass.set_index_buffer(
                    self.relic_box_mesh.index_buffer.slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                for inst in self.debug_axes_instances.iter() {
                    pass.set_bind_group(0, &inst.bind_group, &[]);
                    pass.draw_indexed(0..self.relic_box_mesh.index_count, 0, 0..1);
                }
            }
        }

        // ── SSR snapshot ────────────────────────────────────────────────
        // Capture the swapchain colour and depth buffers BEFORE the
        // hanging plaques are drawn. The lacquered-table SSR samples
        // these textures next frame, so plaques (and the engraved score
        // text decal on their +Z face) never end up in the table's
        // reflection. The smoke pass below still gets a fresh, full
        // (with-plaques) depth via its own `depth_copy_texture` copy.
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.scene_color_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.scene_prev_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.size.width.max(1),
                height: self.size.height.max(1),
                depth_or_array_layers: 1,
            },
        );
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.depth_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.ssr_prev_depth_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::DepthOnly,
            },
            wgpu::Extent3d {
                width: self.size.width.max(1),
                height: self.size.height.max(1),
                depth_or_array_layers: 1,
            },
        );

        // ── Pass B: only created when there's a FluidSmoke marker. The
        // ── live depth buffer is copied into a sibling texture so the
        // ── smoke fragment shader can sample it without aliasing the
        // ── still-bound depth attachment.
        if let Some(split) = split_idx {
            encoder.copy_texture_to_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.depth_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &self.depth_copy_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::DepthOnly,
                },
                wgpu::Extent3d {
                    width: self.size.width.max(1),
                    height: self.size.height.max(1),
                    depth_or_array_layers: 1,
                },
            );

            // ── Smoke-only timing pass (debug profiling only) ────────
            // When a GPU profile session is active, render smoke with
            // flames disabled into the offscreen target first. The real
            // smoke-offscreen pass below overwrites the target with the
            // correct smoke+flames result, so this has no visual effect.
            // Placed here (before smoke-offscreen) so multiple subsequent
            // render passes flush the end-of-pass timestamp on Metal.
            #[cfg(debug_assertions)]
            if self.gpu_profiler.is_sampling()
                && smoke_quality != crate::persistence::SmokeQuality::Off
                && let Some(ref fluid) = self.fluid
            {
                fluid.set_render_mode_encoder(&mut encoder, true);
                // Smoke-only timing: no flame AABB needed
                // because the shader skips flames in this mode.
                let scissor = fluid.screen_aabb_rect(view_proj, None);
                let smoke_only_ts = self
                    .gpu_profiler
                    .pass_writes(crate::render::gpu_profiler::PassSlot::SmokeOnly);
                fluid.render_offscreen(
                    &mut encoder,
                    &self.globals_bind_group,
                    scissor,
                    smoke_only_ts,
                );
                fluid.set_render_mode_encoder(&mut encoder, false);
            }

            // ── Offscreen smoke raymarch pass ──────────────────────────
            // Run the volumetric ray-march into the (reduced-resolution)
            // smoke target BEFORE the swap-chain pass-B begins. The depth
            // copy above means the shader can sample scene depth without
            // aliasing the live depth attachment, and rendering offscreen
            // means the next pass-B can simply sample + bilinear-upsample
            // the result instead of paying for full-screen ray-marching.
            //
            // Skipped entirely when smoke is disabled — the post-smoke
            // pass below still runs so any UI/text ops queued after the
            // FluidSmoke marker draw correctly.
            if smoke_quality != crate::persistence::SmokeQuality::Off
                && let Some(ref fluid) = self.fluid
            {
                // Flame AABB: the raymarch runs its per-candle SDF
                // sub-march inside the same pass, so we have to
                // include the flame bounding spheres in the scissor
                // or flames disappear when the smoke field is empty.
                let flame_aabb = compute_flame_world_aabb(
                    &frame.point_lights[..frame
                        .candle_light_count
                        .min(frame.point_lights.len() as u32)
                        as usize],
                    frame.flame_height_world,
                    self.size.width.max(1) as f32,
                    self.size.height.max(1) as f32,
                );
                let scissor = fluid.screen_aabb_rect(view_proj, flame_aabb);
                let smoke_ts = self
                    .gpu_profiler
                    .pass_writes(crate::render::gpu_profiler::PassSlot::SmokeOffscreen);
                // `None` means both smoke and flames contribute
                // nothing — clear the offscreen target and skip the
                // raymarch. The composite still runs (sampling a
                // transparent texture) so queued ops after the
                // FluidSmoke marker draw correctly.
                if scissor.is_some() {
                    fluid.render_offscreen(
                        &mut encoder,
                        &self.globals_bind_group,
                        scissor,
                        smoke_ts,
                    );
                } else {
                    fluid.clear_offscreen(&mut encoder, smoke_ts);
                }
            }

            let post_smoke_ts = self
                .gpu_profiler
                .pass_writes(crate::render::gpu_profiler::PassSlot::PostSmoke);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("post-smoke-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: scene_view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: post_smoke_ts,
                multiview_mask: None,
            });
            for op in &ops[split..] {
                if matches!(op, RenderOp::TextDraw(_)) {
                    continue;
                }
                process_op(&mut pass, op);
            }
        }

        let bloom_w = (self.size.width.max(1) / 2).max(1);
        let bloom_h = (self.size.height.max(1) / 2).max(1);
        let bloom_threshold = if bloom_active { 1.05 } else { 9999.0 };
        let bloom_strength = if bloom_active { 0.92 } else { 0.0 };
        let make_bloom_bg =
            |label: &'static str, params: BloomParams, texture_view: &wgpu::TextureView| {
                let buffer = self
                    .device
                    .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents: bytemuck::bytes_of(&params),
                        usage: wgpu::BufferUsages::UNIFORM,
                    });
                let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some(label),
                    layout: &self.bloom_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: buffer.as_entire_binding(),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::TextureView(texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                        },
                    ],
                });
                (buffer, bind_group)
            };
        let extract_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [0.0; 4],
        };
        let blur_h_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [1.0, 0.0, 0.0, 0.0],
        };
        let blur_v_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [0.0, 1.0, 0.0, 0.0],
        };
        let fisheye_strength = frame.fisheye_strength.max(0.0);
        // Vignette tracks fisheye so the warp's corner squish fades into
        // darkness (hiding the clamp seam) and reinforces the "looking
        // down a long cabinet" feel without swamping the image when
        // fisheye is off.
        let vignette_strength = (fisheye_strength * 1.4).min(0.85);
        let composite_params = BloomParams {
            data0: [
                bloom_threshold,
                bloom_strength,
                1.0 / bloom_w as f32,
                1.0 / bloom_h as f32,
            ],
            data1: [fisheye_strength, vignette_strength, 0.0, 0.0],
        };
        let (_extract_params_buf, bloom_scene_bg) = make_bloom_bg(
            "bloom-scene-pass-bg",
            extract_params,
            &self.scene_color_view,
        );
        let (_blur_h_params_buf, bloom_ping_bg) =
            make_bloom_bg("bloom-ping-pass-bg", blur_h_params, &self.bloom_ping_view);
        let (_blur_v_params_buf, bloom_pong_bg) =
            make_bloom_bg("bloom-pong-pass-bg", blur_v_params, &self.bloom_pong_view);
        let composite_params_buf =
            self.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("bloom-composite-params"),
                    contents: bytemuck::bytes_of(&composite_params),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
        let bloom_composite_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("bloom-composite-pass-bg"),
            layout: &self.bloom_composite_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: composite_params_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&self.scene_color_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&self.bloom_ping_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.bloom_sampler),
                },
            ],
        });

        if bloom_active {
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom-extract-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_ping_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_extract_pipeline);
                pass.set_bind_group(0, &bloom_scene_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom-blur-h-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_pong_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_blur_pipeline);
                pass.set_bind_group(0, &bloom_ping_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom-blur-v-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.bloom_ping_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&self.bloom_blur_pipeline);
                pass.set_bind_group(0, &bloom_pong_bg, &[]);
                pass.draw(0..3, 0..1);
            }
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene-composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.bloom_composite_pipeline);
            pass.set_bind_group(0, &bloom_composite_bg, &[]);
            pass.draw(0..3, 0..1);
        }

        // (The SSR colour + depth snapshots that used to live here have
        // moved up to between pass A and the new plaque pass — see the
        // "SSR snapshot" block above. The smoke pass already maintains
        // its own `depth_copy_texture` copy, so nothing else needs the
        // end-of-frame depth dump.)

        // ── Overlay pass: 2D HUD text labels ────────────────────────────
        // Drawn AFTER the end-of-frame swapchain → scene_prev snapshot so
        // the text doesn't end up in next frame's SSR reflection sample.
        // The lacquered table reflects whatever's in scene_prev, and a
        // text label rasterised onto the plaque's screen rect would
        // otherwise appear as a phantom duplicate in the table reflection
        // immediately below the plaque (text doesn't write depth, so the
        // SSR ray hits the plaque's depth and samples the colour buffer
        // there — which has the text on top). Loading the swapchain (no
        // clear) lets us composite text on top of the just-finished scene.
        if ops.iter().any(|o| matches!(o, RenderOp::TextDraw(_))) {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("text-overlay-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            for op in &ops {
                if matches!(op, RenderOp::TextDraw(_)) {
                    process_op(&mut pass, op);
                }
            }
        }

        // Flip the smoke TAA ping-pong for next frame — the slot we
        // just rendered into (and the composite just read) becomes
        // next frame's history input. Skipped when smoke is Off so we
        // don't mark undefined texture contents as valid history.
        if smoke_quality != crate::persistence::SmokeQuality::Off
            && let Some(fluid) = self.fluid.as_mut()
        {
            fluid.advance_taa_frame();
        }

        // GPU profiler: resolve query set + stage readback before submit,
        // then block on map after submit so the readback is frame-accurate.
        // Both calls are no-ops when no profiling session is active.
        self.gpu_profiler.before_submit(&mut encoder);

        // Headless screenshot capture: if a path is queued, copy the
        // surface texture into a staging buffer in the same submission.
        // After submit + poll(Wait), map and PNG-encode synchronously.
        // The surface texture is still owned by us until present(), so
        // this is safe. Tied into the same encoder so no extra submit.
        let screenshot_path = self.pending_screenshot.take();
        let screenshot_staging = if let Some(ref path) = screenshot_path {
            log::info!("screenshot: encoding capture for {}", path.display());
            Some(self.encode_screenshot_copy(&mut encoder, frame_texture, path))
        } else {
            None
        };

        self.queue.submit(std::iter::once(encoder.finish()));

        if let (Some(path), Some(staging)) = (screenshot_path, screenshot_staging) {
            match self.finalize_screenshot(staging, &path) {
                Ok(()) => log::info!("screenshot: wrote {}", path.display()),
                Err(e) => log::error!("screenshot finalize failed: {e:?}"),
            }
        }

        if let Some(sf) = surface_frame {
            sf.present();
        }
        self.gpu_profiler.after_submit(&self.device);
        Ok(())
    }

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
